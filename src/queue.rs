use crate::utils::{Arc, AtomSetOnce, NextRevision, RevisionNode, RevisionRef};
use crate::QueueInterface;
use std::sync::atomic::Ordering;

/// A simple event / revision queue
#[derive(Debug)]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,
}

impl<T> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Queue {
            next: Arc::new(AtomSetOnce::empty()),
        }
    }
}

impl<T: Send + 'static> QueueInterface for Queue<T> {
    type Item = T;

    fn publish(&mut self, pending: T) -> Vec<RevisionRef<T>> {
        // 1. prepare revision
        let mut latest = Arc::new(AtomSetOnce::empty());
        let mut revnode = Some(Box::new(RevisionNode {
            data: pending,
            next: Arc::clone(&latest),
        }));

        // 2. create dangling $self.latest
        //    (this is ok because we have a [&mut self] reference)
        std::mem::swap(&mut latest, &mut self.next);
        // $latest points now at the previous-latest NextRevision

        // 3. publish revision (e.g. append to the first 'None' ptr in the 'latest' chain)
        // we should be the owner of the ptr, but catch the case that other
        // threads concurrently append to the structure, just in case,
        // to avoid corruption of revisions
        let mut ret = Vec::new();
        while let Some((old, new)) =
            RevisionRef::new_cas(&mut latest, revnode.take().unwrap(), Ordering::AcqRel)
        {
            ret.push(old);
            revnode = Some(new);
        }
        ret
    }

    fn recv(&mut self) -> Vec<RevisionRef<T>> {
        let mut ret = Vec::new();
        while let Some(x) = RevisionRef::new(&self.next, Ordering::Relaxed) {
            self.next = x.next();
            ret.push(x);
        }
        ret
    }
}
