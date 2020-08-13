/*!
# Nomenclature
This library generally is designed to handle events. It doesn't "pin" the
user to a single event container, instead, it abstracts away from this and
generally handles so-called revisions, which may contain one event at a time,
or a `Vec<Event>`, the only requirements are that the revisions must be safe
to [send across threads](core::marker::Send), contain no depending lifetimes
(e.g. is `'static`), and have a [size known at compile time](core::marker::Sized)
(due to limitations of [`AtomicPtr`](core::sync::atomic::AtomicPtr)).
**/

#![deny(clippy::as_conversions, clippy::cast_ptr_alignment, trivial_casts)]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

mod utils;
pub use utils::{RevisionDetachError, RevisionRef};

mod queue;
pub use queue::Queue;
