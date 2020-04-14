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

mod queue;
pub use queue::{Queue, RevisionRef, RevisionDetachError};

mod woke_queue;
pub use woke_queue::WokeQueue;

/// Common interface for all provided event / revision queues
pub trait QueueInterface: Clone + Default {
    type Item: Send + 'static;

    #[inline(always)]
    fn new() -> Self {
        Default::default()
    }

    /// This method publishes the pending revision and returns all skipped revisions.
    fn publish(&mut self, pending: Self::Item) -> Vec<RevisionRef<Self::Item>>;

    /// Returns a list of newly published revisions.
    fn recv(&mut self) -> Vec<RevisionRef<Self::Item>>;
}

pub mod prelude {
    pub use crate::QueueInterface;
}
