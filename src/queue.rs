#![forbid(unsafe_code)]

use crate::utils::*;
use event_listener::Event;
use std::{collections::VecDeque, marker::Unpin};

/// A simple event / revision queue
#[derive(Debug)]
#[must_use = "Queue does nothing unless you call .next() or some variation of it"]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,

    // currently pending revisions
    pub pending: VecDeque<T>,

    // waiting next... calls
    next_ops: Arc<Event>,
}

impl<T> Clone for Queue<T> {
    #[inline]
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
            pending: Default::default(),
            next_ops: Arc::clone(&self.next_ops),
        }
    }
}

impl<T> Default for Queue<T> {
    #[inline]
    fn default() -> Self {
        Queue {
            next: Arc::new(AtomSetOnce::empty()),
            pending: Default::default(),
            next_ops: Arc::new(Default::default()),
        }
    }
}

impl<T: Unpin> Unpin for Queue<T> {}

impl<T: Send + 'static> Iterator for Queue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        fn intern_<T: Send + 'static>(this: &mut Queue<T>) -> Option<RevisionRef<T>> {
            while let Some(data) = this.pending.pop_front() {
                // 1. prepare revision
                let latest = Arc::new(AtomSetOnce::empty());
                let revnode = Box::new(RevisionNode {
                    data,
                    next: Arc::clone(&latest),
                });

                // 2. try to publish revision
                // e.g. append to the first 'None' ptr in the 'latest' chain
                if let Some((old, new)) =
                    RevisionRef::new_cas(&mut this.next, revnode)
                {
                    // this publishing failed
                    this.pending.push_front((*new).data);

                    // we discovered a new revision, return that
                    return Some(old);
                }

                // publishing succeeded
                // RevisionRef::new_cas doesn't update this.next in that case
                this.next = latest;
                // continue publishing until another thread interrupts us
            }

            RevisionRef::new(&this.next).map(|x| {
                this.next = RevisionRef::next(&x);
                x
            })
        }

        let orig_pending_len = self.pending.len();
        let ret = intern_(self);

        // may have published something
        if orig_pending_len != self.pending.len() {
            self.next_ops.notify(usize::MAX);
        }

        ret
    }
}

impl<T: Send + 'static> Queue<T> {
    /// Similiar to [`Queue::next_blocking`], but `async`.
    /// Only returns `None` if no other reference to the queue
    /// exists anymore, thus, otherwise nothing could wake this up.
    pub async fn next_async(&mut self) -> Option<RevisionRef<T>> {
        loop {
            // put ourselves into the waiting list
            let listener = self.next_ops.listen();

            if let ret @ Some(_) = self.next() {
                // we got something, return
                return ret;
            } else if Arc::get_mut(&mut self.next_ops).is_some() {
                // cancel if no one is listening
                // skip publishing + notifying phase bc no one is listening
                // we need to re-check to catch a race-condition between
                // the call to $self.next and the check of $self.next_ops
                // in between other queue instances may have been destroyed,
                // but messages are still in the queue.
                return RevisionRef::new(&self.next).map(|x| {
                    self.next = RevisionRef::next(&x);
                    x
                });
            } else {
                listener.await;
            }
        }
    }

    /// Similiar to [`next`](Iterator::next), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready.
    /// Only returns None if no other reference to the queue exists anymore,
    /// to prevent a deadlock, because nothing could wake this up.
    #[inline(always)]
    pub fn next_blocking(&mut self) -> Option<RevisionRef<T>> {
        futures_lite::future::block_on(self.next_async())
    }

    /// This method enqueues the pending revision for publishing.
    /// The iterator **must** be "collected"/"polled"
    /// (calling [`Iterator::next`] until it returns None) to publish them.
    #[inline(always)]
    pub fn enqueue(&mut self, pending: T) {
        self.pending.push_back(pending);
    }

    /// Discards all newly published revisions and enforces publishing
    /// of our pending revisions.
    #[inline(always)]
    pub fn skip_and_publish(&mut self) {
        while self.next().is_some() {}
    }
}

impl<T> Queue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }
}

impl<T: std::fmt::Debug> Queue<T> {
    /// Helper function, prints all unprocessed, newly published revisions
    #[cold]
    pub fn print_debug<W: std::io::Write>(
        &self,
        mut writer: W,
        prefix: &str,
    ) -> std::io::Result<()> {
        print_queue(&mut writer, Arc::clone(&self.next), prefix)?;
        writeln!(writer, "{} pending = {:?}", prefix, &self.pending)?;
        writeln!(
            writer,
            "{} next_ops = {:?} x{}",
            prefix,
            &self.next_ops,
            Arc::strong_count(&self.next_ops)
        )?;
        Ok(())
    }
}
