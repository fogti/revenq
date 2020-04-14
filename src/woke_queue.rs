#![forbid(unsafe_code)]

use crate::utils::{Arc, RevisionRef};
use crate::{Queue, QueueInterface};
use std::sync::Mutex;
use std::thread::{self, Thread, ThreadId};

/// An event / revision queue with the ability to wait for new events
#[derive(Debug)]
pub struct WokeQueue<T> {
    inner: Queue<T>,
    wakers: Arc<Mutex<Vec<Thread>>>,
}

impl<T> Clone for WokeQueue<T> {
    fn clone(&self) -> Self {
        WokeQueue {
            inner: Queue::clone(&self.inner),
            wakers: Arc::clone(&self.wakers),
        }
    }
}

impl<T> Default for WokeQueue<T> {
    fn default() -> Self {
        WokeQueue {
            inner: Queue::default(),
            wakers: Default::default(),
        }
    }
}

impl<T: Send + 'static> Iterator for WokeQueue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        let orig_pending_len = self.inner.pending.len();
        let ret = self.inner.next();

        if ret.is_none() {
            // may have published something
            if orig_pending_len != self.inner.pending.len() {
                notify_all(&mut self.wakers.lock().unwrap());
            }
        }

        ret
    }
}

impl<T: Send + 'static> QueueInterface for WokeQueue<T> {
    type RevisionIn = T;

    #[inline(always)]
    fn pending_mut(&mut self) -> &mut std::collections::VecDeque<T> {
        self.inner.pending_mut()
    }
}

impl<T: Send + 'static> WokeQueue<T> {
    /// Similiar to [`next`](Iterator::next), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready.
    /// Only returns None if no other reference to the queue exists anymore
    pub fn next_blocking(&mut self) -> Option<RevisionRef<T>> {
        while Arc::get_mut(&mut self.wakers).is_none() {
            let mut wakers = self.wakers.lock().unwrap();
            let orig_pending_len = self.inner.pending.len();

            if let ret @ Some(_) = self.inner.next() {
                return ret;
            }

            // may have published something
            if orig_pending_len != self.inner.pending.len() {
                notify_all(&mut wakers);
            }

            // put ourselves into the waiting list
            wakers.push(thread::current());
            std::mem::drop(wakers);
            std::thread::park();
        }
        None
    }
}

impl<T> WokeQueue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }
}

fn notify_all(wakers: &mut std::sync::MutexGuard<'_, Vec<Thread>>) {
    let wakers: &mut Vec<Thread> = &mut *wakers;
    let ctid = current_thread_id();
    let wcnt = wakers.len();
    for th in std::mem::replace(wakers, Vec::with_capacity(wcnt)) {
        if th.id() != ctid {
            th.unpark();
        }
    }
}

// source: crossbeam-channel/src/waker.rs
/// Returns the id of the current thread.
#[inline]
fn current_thread_id() -> ThreadId {
    thread_local! {
        /// Cached thread-local id.
        static THREAD_ID: ThreadId = thread::current().id();
    }

    THREAD_ID
        .try_with(|id| *id)
        .unwrap_or_else(|_| thread::current().id())
}
