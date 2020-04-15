use super::WokeQueue;
use crate::utils::RevisionRef;
use crossbeam_utils::sync::Parker;
use futures_core::future::FusedFuture;
use futures_core::stream::FusedStream;
use std::future::Future;
use std::{marker::Unpin, pin::Pin, task};

pub use std::task::Waker;

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
    type Output = Option<RevisionRef<T>>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        futures_core::stream::Stream::poll_next(Pin::new(self.get_mut().0), cx)
    }
}

pub fn notify_all(wakers: &mut std::sync::MutexGuard<'_, Vec<Waker>>) {
    let wakers: &mut Vec<_> = &mut *wakers;
    let wcnt = wakers.len();
    for i in std::mem::replace(wakers, Vec::with_capacity(wcnt)) {
        i.wake();
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
        let cx = &mut task::Context::from_waker(&waker);

        // Keep polling the future until completion.
        loop {
            match future.as_mut().poll(cx) {
                task::Poll::Ready(output) => return output,
                task::Poll::Pending => parker.park(),
            }
        }
    })
}
