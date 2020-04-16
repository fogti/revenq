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

#![deny(clippy::as_conversions, clippy::cast_ptr_alignment, trivial_casts)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod utils;
pub use utils::{MappedRevisionRef, RevisionDetachError, RevisionRef, RevisionRefTrait};

mod queue;
pub use queue::Queue;

#[cfg(feature = "woke-queue")]
mod woke_queue;

#[cfg(feature = "woke-queue")]
#[cfg_attr(docsrs, doc(cfg(feature = "woke-queue")))]
pub use woke_queue::{WokeQueue, WokeQueueNextFuture};

/// Common interface for all provided event / revision queues;
/// implements the `Iterator` interface over newly published revisions.
pub trait QueueInterface: Clone + Default + Iterator
where
    <Self as Iterator>::Item: RevisionRefTrait,
{
    type RevisionIn: Send + 'static;

    /// This method enqueues the pending revision for publishing.
    /// The iterator **must** be "collected"/"polled"
    /// (calling [`Iterator::next`] until it returns None) to publish them.
    #[inline(always)]
    fn enqueue(&mut self, pending: Self::RevisionIn) {
        self.pending_mut().push_back(pending);
    }

    /// Checks if any possible listener can receive messages from us.
    ///
    /// Needs mutable access because the user shouldn't be able to clone this instance.
    fn has_listeners(&mut self) -> bool;

    // TODO(zserik): Maybe replace the pending*() methods with something
    // with lower overhead in book-keeping (this does incur an active cost in WokeQueue)
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
