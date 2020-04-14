use crate::utils::{Arc, RevisionRef};
use crate::{Queue, QueueInterface};
use futures_core::stream;
use std::{io, marker::Unpin, pin::Pin, sync::Mutex, task};

mod utils;
pub use self::utils::WokeQueueNextFuture;
use self::utils::*;

/// An event / revision queue with the ability to wait for new events
#[derive(Debug)]
#[must_use = "WokeQueue does nothing unless you call .next() or some variation of it, or poll it"]
pub struct WokeQueue<T> {
    inner: Queue<T>,
    wakers: Arc<Mutex<Vec<Waker>>>,
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

impl<T: Unpin> Unpin for WokeQueue<T> {}

impl<T> Drop for WokeQueue<T> {
    fn drop(&mut self) {
        fn inner_drop<T>(this: Pin<&mut WokeQueue<T>>) {
            let wakers = WokeQueue::pin_get_wakers(this.into_ref());
            if Arc::strong_count(wakers) == 2 {
                // there are no other senders out there...
                if let Ok(mut wakers) = wakers.lock() {
                    // notify all hanging queues
                    notify_all(&mut wakers);
                }
            }
        }

        // `new_unchecked` is okay because we know this value is never used
        // again after being dropped.
        inner_drop(unsafe { Pin::new_unchecked(self) });
    }
}

impl<T: Send + 'static> Iterator for WokeQueue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        let orig_pending_len = self.pending().len();
        let ret = self.inner.next();

        // may have published something
        if orig_pending_len != self.pending().len() {
            notify_all(&mut self.wakers.lock().unwrap());
        }

        ret
    }
}

impl<T: Send + 'static> QueueInterface for WokeQueue<T> {
    type RevisionIn = T;

    #[inline(always)]
    fn pending(&self) -> &std::collections::VecDeque<T> {
        self.inner.pending()
    }

    #[inline(always)]
    fn pending_mut(&mut self) -> &mut std::collections::VecDeque<T> {
        self.inner.pending_mut()
    }
}

impl<T> WokeQueue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }

    #[inline(always)]
    fn pin_get_wakers(self: Pin<&Self>) -> &Arc<Mutex<Vec<Waker>>> {
        &self.get_ref().wakers
    }
}

impl<T: std::fmt::Debug> WokeQueue<T> {
    /// Helper function, prints all unprocessed, newly published revisions
    #[cold]
    pub fn print_debug<W: io::Write>(&self, mut writer: W, prefix: &str) -> io::Result<()> {
        self.inner.print_debug(&mut writer, prefix)?;
        writeln!(
            writer,
            "{} wakers = {:?} x{}",
            prefix,
            &self.wakers,
            Arc::strong_count(&self.wakers)
        )?;
        Ok(())
    }
}

impl<T> stream::Stream for WokeQueue<T>
where
    T: Send + Unpin + 'static,
{
    type Item = RevisionRef<T>;

    // This is the main implementation. All other semi-blocking "aliases" forward to this one.
    /// Similiar to [`WokeQueue::next_blocking`], but `async`.
    /// Only returns task::Poll::Ready(None) if no other reference to the queue
    /// exists anymore, thus, otherwise nothing could wake this up.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<RevisionRef<T>>> {
        let this = Pin::into_inner(self);
        // we do lock $wakers here via a non-future-aware mutex,
        // but the hold time should be small enough that the
        // increased latency is compensated because we don't need
        // to include the rather big 'futures-util' library here.
        let mut wakers = this.wakers.lock().unwrap();
        let orig_pending_len = this.inner.pending().len();
        let ret = this.inner.next();

        // may have published something
        if orig_pending_len != this.inner.pending().len() {
            notify_all(&mut wakers);
        }

        // we got something, return
        if ret.is_some() {
            return task::Poll::Ready(ret);
        }

        // put ourselves into the waiting list
        wakers.push(cx.waker().clone());
        std::mem::drop(wakers);

        // cancel if no one is listening
        if let Some(x) = Arc::get_mut(&mut this.wakers) {
            // remove ourselves from the waiting list
            x.get_mut().unwrap().pop();
            task::Poll::Ready(None)
        } else {
            task::Poll::Pending
        }
    }
}

impl<T> stream::FusedStream for WokeQueue<T>
where
    T: Send + Unpin + 'static,
{
    #[inline]
    fn is_terminated(&self) -> bool {
        // this may be not exact, but the user can't access $self.wakers
        // directly, anyway
        Arc::strong_count(&self.wakers) == 1
    }
}

impl<T> WokeQueue<T>
where
    T: Send + Unpin + 'static,
{
    /// Similiar to [`WokeQueue::next_blocking`], but `async`.
    /// Only returns `None` if no other reference to the queue
    /// exists anymore, thus, otherwise nothing could wake this up.
    #[inline(always)]
    pub fn next_async(&mut self) -> WokeQueueNextFuture<'_, T> {
        WokeQueueNextFuture(self)
    }

    /// Similiar to [`next`](Iterator::next), but
    /// waits for an event on the WokeQueue, until at least one event
    /// (or event block) got ready.
    /// Only returns None if no other reference to the queue exists anymore,
    /// to prevent a deadlock, because nothing could wake this up.
    #[inline(always)]
    pub fn next_blocking(&mut self) -> Option<RevisionRef<T>> {
        block_on(self.next_async())
    }
}
