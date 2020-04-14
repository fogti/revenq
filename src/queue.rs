#![forbid(unsafe_code)]

use crate::{utils::*, QueueInterface};
use std::{collections::VecDeque, sync::atomic::Ordering};

/// A simple event / revision queue
#[derive(Debug)]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,

    // currently pending revisions
    pub(crate) pending: VecDeque<T>,
}

impl<T> Clone for Queue<T> {
    #[inline]
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
            pending: Default::default(),
        }
    }
}

impl<T> Default for Queue<T> {
    #[inline]
    fn default() -> Self {
        Queue {
            next: Arc::new(AtomSetOnce::empty()),
            pending: Default::default(),
        }
    }
}

impl<T: Send + 'static> Iterator for Queue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        while let Some(data) = self.pending.pop_front() {
            // 1. prepare revision
            let latest = Arc::new(AtomSetOnce::empty());
            let revnode = Box::new(RevisionNode {
                data,
                next: Arc::clone(&latest),
            });

            // 2. try to publish revision
            // e.g. append to the first 'None' ptr in the 'latest' chain
            if let Some((old, new)) =
                RevisionRef::new_cas(&mut self.next, revnode, Ordering::AcqRel)
            {
                // publishing failed
                self.pending.push_front((*new).data);

                // we discovered a new revision, return that
                return Some(old);
            }

            // publishing succeeded
            // RevisionRef::new_cas doesn't update self.next in that case
            self.next = latest;
            // continue publishing until another thread interrupts us
        }

        assert!(self.pending.is_empty());

        RevisionRef::new(&self.next, Ordering::Relaxed).map(|x| {
            self.next = x.next();
            x
        })
    }
}

impl<T: Send + 'static> QueueInterface for Queue<T> {
    type RevisionIn = T;

    #[inline(always)]
    fn pending_mut(&mut self) -> &mut VecDeque<T> {
        &mut self.pending
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
        Ok(())
    }
}
