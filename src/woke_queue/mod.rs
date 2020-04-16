use crate::utils::{Arc, MappedRevisionRef, RevisionRef, RevisionRefTrait};
use crate::{Queue, QueueInterface};
use futures_core::stream;
use std::task::{Context, Poll};
use std::{collections::VecDeque, io, marker::Unpin, pin::Pin, sync::atomic::Ordering};

mod utils;
pub use self::utils::WokeQueueNextFuture;
use self::utils::*;

#[derive(Debug)]
pub enum WokeIntercept<T> {
    Wake(WakeEntry),
    Data(T),
}

impl<T> WokeIntercept<T> {
    fn wokeit(&self) -> &T {
        match self {
            WokeIntercept::Data(ref y) => y,
            WokeIntercept::Wake(_) => unreachable!("revenq internal error in wokeit"),
        }
    }

    fn wake_by_ref(&self) {
        if let WokeIntercept::Wake(ref wke) = self {
            wke.wake_by_ref();
        }
    }

    fn is_wake(&self) -> bool {
        if let WokeIntercept::Wake(_) = self {
            true
        } else {
            false
        }
    }
}

/// An event / revision queue with the ability to wait for new events
#[derive(Debug)]
#[must_use = "WokeQueue does nothing unless you call .next() or some variation of it, or poll it"]
pub struct WokeQueue<T> {
    inner: Queue<WokeIntercept<T>>,
    pending: VecDeque<T>,
    // store pending wakers
    wakers: Vec<RevisionRef<WokeIntercept<T>>>,
}

fn notify_all_mut<T>(wakers: &mut Vec<RevisionRef<WokeIntercept<T>>>) {
    for i in std::mem::take(wakers) {
        (*i).wake_by_ref();
    }
}

fn notify_all<T>(wakers: &[RevisionRef<WokeIntercept<T>>]) {
    for i in wakers {
        (*i).wake_by_ref();
    }
}

impl<T> Clone for WokeQueue<T> {
    fn clone(&self) -> Self {
        WokeQueue {
            inner: Queue::clone(&self.inner),
            pending: Default::default(),
            wakers: self.wakers.clone(),
        }
    }
}

impl<T> Default for WokeQueue<T> {
    fn default() -> Self {
        WokeQueue {
            inner: Queue::default(),
            pending: Default::default(),
            wakers: Default::default(),
        }
    }
}

impl<T: Unpin> Unpin for WokeQueue<T> {}

impl<T> Drop for WokeQueue<T> {
    fn drop(&mut self) {
        fn inner_drop<T>(this: Pin<&mut WokeQueue<T>>) {
            let this_ref = this.into_ref();
            let inner = WokeQueue::pin_get_inner(this_ref);
            if Arc::strong_count(&inner.next) <= 2 {
                // there are no other senders out there...
                // notify all hanging queues
                let wakers = WokeQueue::pin_get_wakers(this_ref);
                notify_all(&wakers);
            }
        }

        // `new_unchecked` is okay because we know this value is never used
        // again after being dropped.
        inner_drop(unsafe { Pin::new_unchecked(self) });
    }
}

impl<T: Send + 'static> Iterator for WokeQueue<T> {
    type Item = MappedRevisionRef<RevisionRef<WokeIntercept<T>>, fn(&WokeIntercept<T>) -> &T>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.meta_next(None)
    }
}

impl<T: Send + 'static> WokeQueue<T> {
    fn cleanup_wakers(&mut self) {
        self.wakers.retain(|i| {
            if let WokeIntercept::Wake(ref w) = &**i {
                w.is_active()
            } else {
                false
            }
        });
    }

    fn pending_to_inner(&mut self, wke: Option<WakeEntry>) {
        let real_pending = std::mem::take(&mut self.pending)
            .into_iter()
            .map(WokeIntercept::Data);
        let inner_pending = self.inner.pending_mut();
        inner_pending.extend(real_pending);
        if let Some(wke) = wke {
            inner_pending.push_back(WokeIntercept::Wake(wke));
        }
    }

    fn pending_from_inner(&mut self) {
        let inner_pending = std::mem::take(self.inner.pending_mut())
            .into_iter()
            .filter_map(|i| {
                // drop all wakers
                match i {
                    WokeIntercept::Data(d) => Some(d),
                    WokeIntercept::Wake(_) => None,
                }
            });
        self.pending.extend(inner_pending);
        self.cleanup_wakers();
    }

    fn meta_next(&mut self, wke: Option<WakeEntry>) -> Option<<Self as Iterator>::Item> {
        self.pending_to_inner(wke);
        let orig_pending_len = self.inner.pending().len();

        let ret = loop {
            // unmangle and cache all wakers
            match self.inner.next() {
                None => break None,
                Some(pkt) => {
                    if pkt.is_wake() {
                        self.wakers.push(pkt);
                    } else {
                        // maybe we can clear the cached waker list at this point
                        break Some(RevisionRef::map::<_, fn(&WokeIntercept<T>) -> &T>(
                            pkt,
                            WokeIntercept::wokeit,
                        ));
                    }
                }
            }
        };

        // may have published something
        if orig_pending_len != self.inner.pending().len() {
            notify_all_mut(&mut self.wakers);
        }

        self.pending_from_inner();

        ret
    }
}

impl<T: Send + 'static> QueueInterface for WokeQueue<T> {
    type RevisionIn = T;

    #[inline(always)]
    fn has_listeners(&mut self) -> bool {
        self.inner.has_listeners()
    }

    #[inline(always)]
    fn pending(&self) -> &VecDeque<T> {
        &self.pending
    }

    #[inline(always)]
    fn pending_mut(&mut self) -> &mut VecDeque<T> {
        &mut self.pending
    }
}

impl<T> WokeQueue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }

    #[inline(always)]
    fn pin_get_wakers(self: Pin<&Self>) -> &[RevisionRef<WokeIntercept<T>>] {
        &self.get_ref().wakers
    }

    #[inline(always)]
    fn pin_get_inner(self: Pin<&Self>) -> &Queue<WokeIntercept<T>> {
        &self.get_ref().inner
    }
}

impl<T: std::fmt::Debug> WokeQueue<T> {
    /// Helper function, prints all unprocessed, newly published revisions
    #[cold]
    pub fn print_debug<W: io::Write>(&self, mut writer: W, prefix: &str) -> io::Result<()> {
        self.inner.print_debug(&mut writer, prefix)?;
        writeln!(
            writer,
            "{} wakers = {:?} xq{}",
            prefix,
            &self.wakers,
            Arc::strong_count(&self.inner.next)
        )?;
        Ok(())
    }
}

impl<T> stream::Stream for WokeQueue<T>
where
    T: Send + Unpin + 'static,
{
    type Item = <Self as Iterator>::Item;

    // This is the main implementation. All other semi-blocking "aliases" forward to this one.
    /// Similiar to [`WokeQueue::next_blocking`], but `async`.
    /// Only returns `Poll::Ready(None)` if no other reference to the queue
    /// exists anymore, thus, otherwise nothing could wake this up.
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<<Self as Iterator>::Item>> {
        let this = Pin::into_inner(self);
        let ret = this.meta_next(Some(WakeEntry::new(cx.waker().clone())));

        if ret.is_none() && this.has_listeners() {
            Poll::Pending
        } else {
            // either we got something, return;
            // or cancel if no one is listening
            Poll::Ready(ret)
        }
    }
}

impl<T> stream::FusedStream for WokeQueue<T>
where
    T: Send + Unpin + 'static,
{
    #[inline]
    fn is_terminated(&self) -> bool {
        // this may be not exact, but the user can't access $self.inner.next
        // directly, anyway
        Arc::strong_count(&self.inner.next) == 1 && {
            RevisionRef::new(&self.inner.next, Ordering::Acquire)
                .map(|nrev| Arc::strong_count(&RevisionRef::next(&nrev)) <= 2)
                .unwrap_or(true)
        }
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
    pub fn next_blocking(&mut self) -> Option<<Self as Iterator>::Item> {
        block_on(self.next_async())
    }
}
