use crate::{Arc, PendingMap, Queue, QueueInterface};
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

    fn publish_with<F>(&mut self, pending: T, with_f: F)
    where
        F: FnMut(PendingMap<'_, T>),
    {
        self.inner.publish_with(pending, with_f);
        let mut wakers = self.wakers.lock().unwrap();
        let ctid = current_thread_id();
        for th in std::mem::take(&mut *wakers) {
            if th.id() != ctid {
                th.unpark();
            }
        }
    }

    #[inline(always)]
    fn with<F: FnMut(&T)>(&mut self, f: F) {
        self.inner.with(f);
    }
}

impl<T: Send + 'static> WokeQueue<T> {
    /// Similiar to [`with`](QueueInterface::with), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready. The return value of 'f' determines
    /// if an event is considered ready/usable (`true` -> ready).
    pub fn with_blocking<F>(&mut self, mut f: F)
    where
        F: FnMut(&T) -> bool,
    {
        let mut got_anything = false;
        loop {
            let mut wakers = self.wakers.lock().unwrap();
            self.inner.with(|x| got_anything |= f(x));
            if got_anything {
                break;
            }
            wakers.push(thread::current());
            std::mem::drop(wakers);
            std::thread::park();
        }
    }

    /// Similiar to [`with`](QueueInterface::with), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready. The return value of 'with_f' determines
    /// if an event is considered ready/usable (`true` -> ready).
    pub fn publish_with_blocking<F>(&mut self, pending: T, mut with_f: F)
    where
        F: FnMut(&T) -> bool,
    {
        let mut got_anything = false;
        self.publish_with(pending, |pm| got_anything |= with_f(pm.current));
        if !got_anything {
            self.with_blocking(with_f);
        }
    }
}
