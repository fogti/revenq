use crate::{Arc, QueueInterface};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::{fmt, mem, ptr};

/// An AtomSetOnce wraps an AtomicPtr, it allows for safe mutation of an atomic
/// into common Rust Types.
struct AtomSetOnce<T>(AtomicPtr<T>);

impl<T> fmt::Debug for AtomSetOnce<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "atom({:?})", self.0.load(Ordering::Relaxed))
    }
}

impl<T> AtomSetOnce<T> {
    /// Create a empty AtomSetOnce
    #[inline]
    fn empty() -> AtomSetOnce<T> {
        AtomSetOnce(AtomicPtr::new(ptr::null_mut()))
    }
}

impl<T> Drop for AtomSetOnce<T> {
    fn drop(&mut self) {
        let ptr = *self.0.get_mut();
        if !ptr.is_null() {
            unsafe {
                let _: Box<T> = Box::from_raw(ptr as *mut T);
            }
        }
    }
}

unsafe impl<T> Send for AtomSetOnce<T> {}
unsafe impl<T> Sync for AtomSetOnce<T> {}

type NextRevision<T> = Arc<AtomSetOnce<RevisionNode<T>>>;

#[derive(Clone, Debug)]
struct RevisionNode<T> {
    next: NextRevision<T>,
    data: T,
}

/// A owning reference to a revision.
///
/// Warning: Objects of this type must not be leaked, otherwise all future
/// revisions will be leaked, too, and thus the memory of the queue is never freed.
pub struct RevisionRef<T> {
    keep_alive: NextRevision<T>,
    // contract / invariant: rptr is valid as long as _keep_alive is valid
    rptr: ptr::NonNull<RevisionNode<T>>,
}

/// Error indicating a failed [`RevisionRef::try_detach`] call.
#[derive(Clone, Debug)]
pub struct RevisionDetachError;

impl fmt::Display for RevisionDetachError {
   fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
       write!(f, "failed to detach revision")
   }
}

impl std::error::Error for RevisionDetachError {}

impl<T> std::ops::Deref for RevisionRef<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &unsafe { self.rptr.as_ref() }.data
    }
}

impl<T> RevisionRef<T> {
    fn new(nr: &NextRevision<T>, order: Ordering) -> Option<Self> {
        let rptr = nr.0.load(order);
        ptr::NonNull::new(rptr).map(|rptr| Self {
            keep_alive: Arc::clone(&nr),
            rptr,
        })
    }

    /// try to append revnode, if CAS succeeds, return None, otherwise:
    /// return a RevisionRef for the failed CAS ptr, and the revnode.
    fn new_cas(
        latest: &mut NextRevision<T>,
        revnode: Box<RevisionNode<T>>,
        order: Ordering,
    ) -> Option<(Self, Box<RevisionNode<T>>)> {
        let new = Box::into_raw(revnode);
        let old = latest.0.compare_and_swap(ptr::null_mut(), new, order);
        let rptr = ptr::NonNull::new(old)?;
        let real_old: &RevisionNode<T> = unsafe { rptr.as_ref() };
        Some((
            Self {
                // This is safe since ptr cannot be changed once it is set
                // which means that this is now a Box.
                // use the next revision
                keep_alive: mem::replace(latest, Arc::clone(&real_old.next)),
                rptr,
            },
            unsafe { Box::from_raw(new) },
        ))
    }

    fn try_acquire_ownership(
        this: &mut NextRevision<T>,
    ) -> Result<&mut *mut RevisionNode<T>, RevisionDetachError> {
        // get ownership over the Arc of revision $this
        let ptr_this = Arc::get_mut(this).ok_or(RevisionDetachError)?;
        // no other reference to *us* exists.
        // we need to be sure that we are the *only node with access to next*
        Ok(ptr_this.0.get_mut())
    }

    /// Try to detach this revision from the following.
    /// Only works if this `RevisionRef` is the last reference to this revision,
    /// and the same is true for the following revision.
    /// Use this method to reduce queue memory usage if you want to store this
    /// object long-term.
    pub fn try_detach(this: &mut Self) -> Result<(), RevisionDetachError> {
        // 1. get ownership over the revision we point at
        let mut_this = *Self::try_acquire_ownership(&mut this.keep_alive)?;
        //    take this chance and validate rptr
        assert_eq!(mut_this, this.rptr.as_ptr());
        let mut_this = unsafe { &mut *mut_this };

        // 2. make sure we have ownership over the next revision
        let mut_next = Self::try_acquire_ownership(&mut mut_this.next)?;

        let old_next = mem::replace(mut_next, ptr::null_mut());

        // destroy old_next
        if !old_next.is_null() {
            unsafe {
                let _: Box<T> = Box::from_raw(old_next as *mut T);
            }
        }

        Ok(())
    }

    fn next(&self) -> NextRevision<T> {
        Arc::clone(&unsafe { self.rptr.as_ref() }.next)
    }
}

/// A simple event / revision queue
#[derive(Debug)]
pub struct Queue<T> {
    // the $next field is partially shared, e.g. all queues derived from the same
    // original queue can find the current $next value, but may be a bit behind
    // (e.g. have unconsumed revisions,
    //  which should be iterated to get the current value)
    next: NextRevision<T>,
}

impl<T> Clone for Queue<T> {
    fn clone(&self) -> Self {
        Queue {
            next: Arc::clone(&self.next),
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Queue {
            next: Arc::new(AtomSetOnce::empty()),
        }
    }
}

impl<T: Send + 'static> QueueInterface for Queue<T> {
    type Item = T;

    fn publish(&mut self, pending: T) -> Vec<RevisionRef<T>> {
        // 1. prepare revision
        let mut latest = Arc::new(AtomSetOnce::empty());
        let mut revnode = Some(Box::new(RevisionNode {
            data: pending,
            next: Arc::clone(&latest),
        }));

        // 2. create dangling $self.latest
        //    (this is ok because we have a [&mut self] reference)
        std::mem::swap(&mut latest, &mut self.next);
        // $latest points now at the previous-latest NextRevision

        // 3. publish revision (e.g. append to the first 'None' ptr in the 'latest' chain)
        // we should be the owner of the ptr, but catch the case that other
        // threads concurrently append to the structure, just in case,
        // to avoid corruption of revisions
        let mut ret = Vec::new();
        while let Some((old, new)) =
            RevisionRef::new_cas(&mut latest, revnode.take().unwrap(), Ordering::AcqRel)
        {
            ret.push(old);
            revnode = Some(new);
        }
        ret
    }

    fn recv(&mut self) -> Vec<RevisionRef<T>> {
        let mut ret = Vec::new();
        while let Some(x) = RevisionRef::new(&self.next, Ordering::Relaxed) {
            self.next = x.next();
            ret.push(x);
        }
        ret
    }
}
