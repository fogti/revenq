use std::sync::Arc;

pub struct PendingMap<'a, T: 'static> {
    pub current: &'a T,
    pub pending: &'a mut T,
}

/// Common interface for all provided event queues
pub trait QueueInterface: Clone + Default {
    type Item: Send + 'static;

    #[inline(always)]
    fn new() -> Self {
        Default::default()
    }

    /// This method publishes the pending event and finishes the revision,
    /// while also calling a helper function for each skipped revision,
    /// thus, no events are lost.
    fn publish_with<F>(&mut self, pending: Self::Item, with_f: F)
    where
        F: FnMut(PendingMap<'_, Self::Item>);

    /// This method publishes the pending event and finishes the revision
    #[inline(always)]
    fn publish(&mut self, pending: Self::Item) {
        self.publish_with(pending, |_| {});
    }

    /// For each revision, applies a function to the list of new events.
    fn with<F: FnMut(&Self::Item)>(&mut self, f: F);
}

pub mod prelude {
    pub use crate::QueueInterface;
}

mod queue;
pub use queue::Queue;

mod woke_queue;
pub use woke_queue::WokeQueue;
