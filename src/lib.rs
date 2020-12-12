/*!
# Nomenclature
This library generally is designed to handle events. It doesn't "pin" the
user to a single event container, instead, it abstracts away from this and
generally handles so-called revisions, which may contain one event at a time,
or a `Vec<Event>`, the only requirements are that the revisions must
have a [size known at compile time](core::marker::Sized)
(due to limitations of the used backend).
**/

#![forbid(clippy::as_conversions, clippy::cast_ptr_alignment, trivial_casts)]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

use alloc::{collections::VecDeque, sync::Arc};
use core::{fmt, marker::Unpin};
use event_listener::Event;
use once_cell::sync::OnceCell;

type NextRevision<T> = Arc<OnceCell<RevisionNode<T>>>;

#[derive(Clone, Debug)]
struct RevisionNode<T> {
    next: NextRevision<T>,
    data: T,
}

/// A owning reference to a revision.
///
/// Warning: Objects of this type must not be leaked, otherwise all future
/// revisions will be leaked, too, and thus the memory of the queue is never freed.
#[derive(Debug)]
pub struct RevisionRef<T> {
    inner: NextRevision<T>,
}

/// Error indicating a failed [`RevisionRef::try_detach`] call.
#[derive(Clone, Debug)]
pub struct RevisionDetachError;

impl fmt::Display for RevisionDetachError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "failed to detach revision")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RevisionDetachError {}

impl<T> Clone for RevisionRef<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> core::ops::Deref for RevisionRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // This pointer should never change once RevisionRef is created until
        // it's dropped.
        &unsafe { self.inner.get_unchecked() }.data
    }
}

unsafe impl<T> stable_deref_trait::StableDeref for RevisionRef<T> {}
unsafe impl<T> stable_deref_trait::CloneStableDeref for RevisionRef<T> {}

impl<T> RevisionRef<T> {
    fn new_and_forward(nr: &mut NextRevision<T>) -> Option<Self> {
        match nr.get() {
            Some(_) => {
                let x = Self {
                    inner: Arc::clone(&nr),
                };
                *nr = Arc::clone(&unsafe { nr.get_unchecked() }.next);
                Some(x)
            }
            None => None,
        }
    }

    /// Try to detach this revision from the following.
    /// Only works if this `RevisionRef` is the last reference to this revision.
    /// This is the case if no RevisionRef to a revision with precedes this
    /// revision exist and this is the last ptr to this revision, and all queue
    /// references have already consumed this revision.
    /// Use this method to reduce queue memory usage if you want to store this
    /// object long-term.
    pub fn try_detach<'a>(this: &'a mut Self) -> Result<&'a mut T, RevisionDetachError> {
        // get ownership over the Arc of revision $this.inner
        // (with lifetime = as long as $this.inner exists with the current Arc)
        let mut_this: &mut RevisionNode<T> = Arc::get_mut(&mut this.inner)
            .ok_or(RevisionDetachError)?
            .get_mut()
            .unwrap();
        // no other reference to *us* exists.
        // override our $next ptr, thus decoupling this node from the following
        mut_this.next = Arc::new(OnceCell::default());
        Ok(&mut mut_this.data)
    }

    /// Similiar to [`try_detach`](RevisionRef::try_detach), detach this revision
    /// if possible, but then unwrap the inner value
    pub fn try_into_inner(mut this: Self) -> Result<T, Self> {
        // get ownership over the Arc of revision $this.inner
        let mut mut_this: RevisionNode<T> = match Arc::get_mut(&mut this.inner) {
            Some(x) => x.take().unwrap(),
            None => return Err(this),
        };
        // no other reference to *us* exists.
        // override our $next ptr, thus decoupling this node from the following
        mut_this.next = Arc::new(OnceCell::default());
        Ok(mut_this.data)
    }
}

/// A simple event / revision queue
#[derive(Debug)]
#[must_use = "Queue does nothing unless you call .next() or some variation of it"]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,

    // waiting next... calls
    next_ops: Arc<Event>,

    // currently pending revisions
    pub pending: VecDeque<T>,
}

impl<T> Clone for Queue<T> {
    #[inline]
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
            next_ops: Arc::clone(&self.next_ops),
            pending: Default::default(),
        }
    }
}

impl<T> Default for Queue<T> {
    #[inline]
    fn default() -> Self {
        Queue {
            next: Arc::new(Default::default()),
            next_ops: Arc::new(Default::default()),
            pending: Default::default(),
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.next_ops) == 2 {
            self.next_ops.notify(1);
        }
    }
}

impl<T: Unpin> Unpin for Queue<T> {}

impl<T> Iterator for Queue<T> {
    type Item = RevisionRef<T>;

    fn next(&mut self) -> Option<RevisionRef<T>> {
        let orig_pending_len = self.pending.len();

        let ret = match self.publish_intern() {
            None => RevisionRef::new_and_forward(&mut self.next),
            x => x,
        };

        // may have published something
        if orig_pending_len != self.pending.len() {
            self.next_ops.notify(usize::MAX);
        }

        ret
    }
}

impl<T> Queue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }

    fn publish_intern(&mut self) -> Option<RevisionRef<T>> {
        enum State<T> {
            ToPublish { data: T },
            Published { latest: NextRevision<T> },
        }

        while let Some(data) = self.pending.pop_front() {
            // : try append to the first 'None' ptr in the 'latest' chain
            // try to append revnode, if CAS succeeds, continue, otherwise:
            // return a RevisionRef for the failed CAS ptr, and the revnode;
            // set $latest to the next ptr

            let mut state = State::ToPublish { data };
            let maybe_real_old = self.next.get_or_init(|| {
                let latest = Arc::new(OnceCell::default());
                if let State::ToPublish { data } = core::mem::replace(
                    &mut state,
                    State::Published {
                        latest: Arc::clone(&latest),
                    },
                ) {
                    RevisionNode {
                        data,
                        next: Arc::clone(&latest),
                    }
                } else {
                    unreachable!();
                }
            });
            match state {
                State::Published { latest } => {
                    // CAS / publishing succeeded
                    self.next = latest;
                    // continue publishing until another thread interrupts us
                }
                State::ToPublish { data } => {
                    // CAS failed
                    // we need to split this assignment to prevent rustc E0502
                    let new_next = Arc::clone(&maybe_real_old.next);

                    // this publishing failed
                    self.pending.push_front(data);

                    // we discovered a new revision, return that
                    return Some(RevisionRef {
                        // This is safe since the cell cannot be changed once it is set.
                        // use the next revision
                        inner: core::mem::replace(&mut self.next, new_next),
                    });
                }
            }
        }
        None
    }

    /// Waits asynchronously for an event to be published on the queue.
    /// Only returns `None` if no other reference to the queue
    /// exists anymore, because otherwise nothing could wake this up.
    /// Tries to publish pending revisions while waiting.
    pub async fn next_async(&mut self) -> Option<RevisionRef<T>> {
        loop {
            // put ourselves into the waiting list
            let listener = self.next_ops.listen();

            if let ret @ Some(_) = self.next() {
                // we got something, return
                return ret;
            } else if Arc::get_mut(&mut self.next_ops).is_some() {
                // cancel if no one is listening
                // skip publishing + notifying phase bc no one is listening
                // we need to re-check to catch a race-condition between
                // the call to $self.next and the check of $self.next_ops
                // in between other queue instances may have been destroyed,
                // but messages are still in the queue.
                return RevisionRef::new_and_forward(&mut self.next);
            } else {
                listener.await;
            }
        }
    }

    /// This method enqueues the pending revision for publishing.
    /// The iterator **must** be "collected"/"polled"
    /// (calling [`Iterator::next`] until it returns None) to publish them.
    #[inline(always)]
    pub fn enqueue(&mut self, pending: T) {
        self.pending.push_back(pending);
    }
}

#[cfg(feature = "std")]
impl<T: std::fmt::Debug> Queue<T> {
    /// Helper function, prints all unprocessed, newly published revisions
    #[cold]
    pub fn print_debug<W: std::io::Write>(
        &self,
        mut writer: W,
        prefix: &str,
    ) -> std::io::Result<()> {
        #[cold]
        let mut cur = Arc::clone(&self.next);
        let mut fi = true;
        let mut tmpstr = String::new();
        while let Some(x) = RevisionRef::new_and_forward(&mut cur) {
            if !fi {
                tmpstr.push(',');
                tmpstr.push(' ');
            }
            tmpstr += &format!("{:?}", &*x);
            fi = false;
        }
        writeln!(
            writer,
            "{} [{}] pending = {:?}; next_ops x{}",
            prefix,
            tmpstr,
            &self.pending,
            Arc::strong_count(&self.next_ops)
        )?;
        Ok(())
    }
}
