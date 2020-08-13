use crate::utils::*;
use alloc::collections::VecDeque;
use core::{marker::Unpin, mem, ptr, sync::atomic::Ordering};
use event_listener::Event;

/// A simple event / revision queue
#[derive(Debug)]
#[must_use = "Queue does nothing unless you call .next() or some variation of it"]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,

    // waiting next... calls
    next_ops: Arc<Event>,

    // currently pending revisions
    pub pending: VecDeque<T>,
}

impl<T> Clone for Queue<T> {
    #[inline]
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
            next_ops: Arc::clone(&self.next_ops),
            pending: Default::default(),
        }
    }
}

impl<T> Default for Queue<T> {
    #[inline]
    fn default() -> Self {
        Queue {
            next: Arc::new(AtomSetOnce::empty()),
            next_ops: Arc::new(Default::default()),
            pending: Default::default(),
        }
    }
}

impl<T: Unpin> Unpin for Queue<T> {}

fn next_intern_<T: Send + 'static>(this: &mut Queue<T>) -> Option<RevisionRef<T>> {
    while let Some(data) = this.pending.pop_front() {
        // 1. prepare revision
        let latest = Arc::new(AtomSetOnce::empty());
        let revnode = Box::new(RevisionNode {
            data,
            next: Arc::clone(&latest),
        });

        // 2. try to publish revision
        // e.g. append to the first 'None' ptr in the 'latest' chain

        // try to append revnode, if CAS succeeds, continue, otherwise:
        // return a RevisionRef for the failed CAS ptr, and the revnode;
        // set $latest to the next ptr

        let new = Box::into_raw(revnode);
        let old = this
            .next
            .0
            .compare_and_swap(ptr::null_mut(), new, Ordering::AcqRel);

        if let Some(rptr) = ptr::NonNull::new(old) {
            let real_old: &RevisionNode<T> = unsafe { rptr.as_ref() };

            let old = RevisionRef {
                // This is safe since ptr cannot be changed once it is set
                // which means that this is now a Box.
                // use the next revision
                inner: mem::replace(&mut this.next, Arc::clone(&real_old.next)),
            };
            RevisionRef::check_against_rptr(&old, rptr);

            // this publishing failed
            this.pending
                .push_front((*unsafe { Box::from_raw(new) }).data);

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

impl<T: Send + 'static> Iterator for Queue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        let orig_pending_len = self.pending.len();
        let ret = next_intern_(self);

        // may have published something
        if orig_pending_len != self.pending.len() {
            self.next_ops.notify(usize::MAX);
        }

        ret
    }
}

impl<T: Send + 'static> Queue<T> {
    /// Waits asynchronously for an event to be published on the queue.
    /// Only returns `None` if no other reference to the queue
    /// exists anymore, because otherwise nothing could wake this up.
    /// Tries to publish pending revisions while waiting.
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

    /// This method enqueues the pending revision for publishing.
    /// The iterator **must** be "collected"/"polled"
    /// (calling [`Iterator::next`] until it returns None) to publish them.
    #[inline(always)]
    pub fn enqueue(&mut self, pending: T) {
        self.pending.push_back(pending);
    }
}

impl<T> Queue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }
}

#[cfg(feature = "std")]
impl<T: std::fmt::Debug> Queue<T> {
    /// Helper function, prints all unprocessed, newly published revisions
    #[cold]
    pub fn print_debug<W: std::io::Write>(
        &self,
        mut writer: W,
        prefix: &str,
    ) -> std::io::Result<()> {
        #[cold]
        let mut cur = Arc::clone(&self.next);
        let mut cnt = 0;
        while let Some(x) = RevisionRef::new(&cur) {
            writeln!(&mut writer, "{} {}. {:?}", prefix, cnt, &*x)?;
            cur = RevisionRef::next(&x);
            cnt += 1;
        }
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
