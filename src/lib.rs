/*!
# Nomenclature
This library generally is designed to handle events. It doesn't "pin" the
user to a single event container, instead, it abstracts away from this and
generally handles so-called revisions, which may contain one event at a time,
or a `Vec<Event>`, the only requirements are that the revisions must be safe
to [send across threads](std::marker::Send), contain no depending lifetimes
(e.g. is `'static`), and have a [size known at compile time](std::marker::Sized)
(due to limitations of [`AtomicPtr`](std::sync::atomic::AtomicPtr)).
**/

use std::sync::Arc;

/// This struct contains the parameters for the `with_f` function,
/// given to the [`QueueInterface::publish_with`] method.
pub struct PendingMap<'a, T: 'static> {
    pub current: &'a T,
    pub pending: &'a mut T,
}

/// Common interface for all provided event / revision queues
pub trait QueueInterface: Clone + Default {
    type Item: Send + 'static;

    #[inline(always)]
    fn new() -> Self {
        Default::default()
    }

    /// This method publishes the given pending revision,
    /// while also calling a helper function for each otherwise skipped revision,
    /// thus, no events are lost.
    fn publish_with<F>(&mut self, pending: Self::Item, with_f: F)
    where
        F: FnMut(PendingMap<'_, Self::Item>);

    /// This method publishes the pending revision.
    ///
    /// # Usage Warning
    /// Some revisons may be skipped when using this method.
    /// The following code does probably not what you want
    /// (e.g. some revisions may be lost):
    /// ```
    /// # use revenq::{prelude::*, Queue};
    /// fn handler(x: &u32) {
    ///     // user-specific code
    /// }
    ///
    /// let mut q = Queue::<u32>::new();
    /// let _q2 = Queue::clone(&q);
    ///
    /// // send _q2 to another thread, the other thread may publish revisions
    /// q.with(handler);
    ///
    /// // the following call is practically a "safe" race-condition,
    /// // e.g. some revisions may be skipped and are practically lost.
    /// q.publish(0);
    /// ```
    ///
    /// The following code/behavoir is probably more desired:
    /// ```
    /// # use revenq::{prelude::*, Queue};
    /// fn handler(x: &u32) {
    ///     // user-specific code
    /// }
    ///
    /// let mut q = Queue::<u32>::new();
    /// let _q2 = Queue::clone(&q);
    ///
    /// // send _q2 to another thread, the other thread may publish revisions
    /// // the following call combines `with` and `publish` without the
    /// // possibility of skipped revisions.
    /// q.publish_with(0, |pm| handler(pm.current));
    /// ```
    #[inline(always)]
    fn publish(&mut self, pending: Self::Item) {
        self.publish_with(pending, |_| {});
    }

    /// Applies a function for each revision.
    fn with<F: FnMut(&Self::Item)>(&mut self, f: F);
}

pub mod prelude {
    pub use crate::QueueInterface;
}

mod queue;
pub use queue::Queue;

mod woke_queue;
pub use woke_queue::WokeQueue;
