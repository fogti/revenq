use super::WokeQueue;
use crossbeam_utils::sync::Parker;
use futures_core::future::FusedFuture;
use futures_core::stream::FusedStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::{future::Future, marker::Unpin, pin::Pin};

// source: https://github.com/rust-lang/futures-rs/blob/master/futures-util/src/stream/stream/next.rs
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WokeQueueNextFuture<'a, T>(pub(super) &'a mut WokeQueue<T>);

impl<T: Unpin> Unpin for WokeQueueNextFuture<'_, T> {}

impl<T: Send + Unpin + 'static> FusedFuture for WokeQueueNextFuture<'_, T> {
    #[inline(always)]
    fn is_terminated(&self) -> bool {
        self.0.is_terminated()
    }
}

impl<T: Send + Unpin + 'static> Future for WokeQueueNextFuture<'_, T>
where
    T: Send + Unpin + 'static,
{
    type Output = Option<<WokeQueue<T> as Iterator>::Item>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        futures_core::stream::Stream::poll_next(Pin::new(self.get_mut().0), cx)
    }
}

#[derive(Debug)]
pub struct WakeEntry {
    active: AtomicBool,
    waker: Waker,
}

impl WakeEntry {
    #[inline]
    pub fn new(waker: Waker) -> Self {
        Self {
            active: AtomicBool::new(true),
            waker,
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    #[inline]
    pub fn wake_by_ref(&self) {
        // check if entry was already consumed
        if self.active.compare_and_swap(true, false, Ordering::AcqRel) == true {
            // we can consume this entry, it wasn't already consumed
            self.waker.wake_by_ref();
        }
    }
}

// source: https://github.com/stjepang/byo-block-on/blob/master/examples/v4.rs
pub fn block_on<F: Future>(future: F) -> F::Output {
    use std::cell::RefCell;

    // Pin the future on the stack.
    pin_utils::pin_mut!(future);

    thread_local! {
        // Parker and waker associated with the current thread.
        static CACHE: RefCell<(Parker, Waker)> = {
            let parker = Parker::new();
            let unparker = parker.unparker().clone();
            let waker = async_task::waker_fn(move || unparker.unpark());
            RefCell::new((parker, waker))
        };
    }

    CACHE.with(|cache| {
        // Panic if `block_on()` is called recursively.
        let (parker, waker) = &mut *cache.try_borrow_mut().expect("recursive `block_on`");

        // Create the task context.
        let cx = &mut Context::from_waker(&waker);

        // Keep polling the future until completion.
        loop {
            match future.as_mut().poll(cx) {
                Poll::Ready(output) => return output,
                Poll::Pending => parker.park(),
            }
        }
    })
}
