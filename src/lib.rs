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

mod utils;
pub use utils::{RevisionDetachError, RevisionRef};

mod queue;
pub use queue::Queue;

mod woke_queue;
pub use woke_queue::WokeQueue;

/// Common interface for all provided event / revision queues;
/// implements the `Iterator` interface over newly published revisions.
pub trait QueueInterface:
    Clone + Default + Iterator<Item = RevisionRef<<Self as QueueInterface>::RevisionIn>>
{
    type RevisionIn: Send + 'static;

    /// This method enqueues the pending revision for publishing.
    /// The iterator **must** be "collected"/"polled"
    /// (calling [`Iterator::next`] until it returns None) to publish them.
    #[inline(always)]
    fn enqueue(&mut self, pending: Self::RevisionIn) {
        self.pending_mut().push_back(pending);
    }

    /// This method allows direct access of the pending revisions queue.
    fn pending(&self) -> &std::collections::VecDeque<Self::RevisionIn>;

    /// This method allows direct modification of the pending revisions queue.
    /// This is useful if you want to withdraw a revision
    /// based on newly received revisions.
    fn pending_mut(&mut self) -> &mut std::collections::VecDeque<Self::RevisionIn>;

    /// Discards all newly published revisions and enforces publishing
    /// of our pending revisions.
    #[inline(always)]
    fn skip_and_publish(&mut self) {
        while self.next().is_some() {}
    }
}

pub mod prelude {
    pub use crate::QueueInterface;
}
