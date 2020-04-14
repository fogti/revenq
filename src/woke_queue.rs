use crate::{Arc, Queue, QueueInterface, RevisionRef};
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

impl<T: Send + 'static> QueueInterface for WokeQueue<T> {
    type Item = T;

    fn publish(&mut self, pending: T) -> Vec<RevisionRef<T>> {
        let ret = self.inner.publish(pending);
        let mut wakers = self.wakers.lock().unwrap();
        let ctid = current_thread_id();
        for th in std::mem::take(&mut *wakers) {
            if th.id() != ctid {
                th.unpark();
            }
        }
        ret
    }

    #[inline(always)]
    fn recv(&mut self) -> Vec<RevisionRef<T>> {
        self.inner.recv()
    }
}

impl<T: Send + 'static> WokeQueue<T> {
    /// Similiar to [`recv`](QueueInterface::recv), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready.
    pub fn recv_blocking(&mut self) -> Vec<RevisionRef<T>> {
        loop {
            let mut wakers = self.wakers.lock().unwrap();
            let ret = self.inner.recv();
            if !ret.is_empty() {
                return ret;
            }
            wakers.push(thread::current());
            std::mem::drop(wakers);
            std::thread::park();
        }
    }
}
