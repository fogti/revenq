use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;
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
    fn empty() -> AtomSetOnce<T> {
        AtomSetOnce(AtomicPtr::new(ptr::null_mut()))
    }
}

impl<T> Drop for AtomSetOnce<T> {
    fn drop(&mut self) {
        unsafe {
            let ptr = self.0.load(Ordering::Relaxed);
            if !ptr.is_null() {
                let _: Box<T> = Box::from_raw(ptr as *mut T);
            }
        }
    }
}

unsafe impl<T> Send for AtomSetOnce<T> {}
unsafe impl<T> Sync for AtomSetOnce<T> {}

type NextRevision<T> = Arc<AtomSetOnce<RevisionNode<T>>>;

#[derive(Clone)]
struct RevisionNode<T> {
    next: NextRevision<T>,
    data: T,
}

pub struct Queue<T> {
    next: NextRevision<T>,
}

pub struct PendingMap<'a, T: 'a> {
    pub current: &'a T,
    pub pending: &'a mut T,
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

impl<T> Queue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }
}

impl<T: Send> Queue<T> {
    /// This method publishes the pending event and finishes the revision,
    /// while also calling a helper function for each skipped revision,
    /// thus, no events are lost.
    pub fn publish_with<F>(&mut self, pending: T, mut with_f: F)
    where
        F: FnMut(PendingMap<'_, T>),
    {
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
        loop {
            let new = Box::into_raw(revnode.take().unwrap());
            let old = latest
                .0
                .compare_and_swap(ptr::null_mut(), new, Ordering::AcqRel);
            if old.is_null() {
                break;
            }
            revnode = Some(unsafe { Box::from_raw(new) });

            {
                let old: &RevisionNode<T> = unsafe { &*old };
                with_f(PendingMap {
                    current: &old.data,
                    pending: &mut revnode.as_mut().unwrap().data,
                });

                // This is safe since ptr cannot be changed once it is set
                // which means that this is now a Box.
                // use the next revision
                latest = Arc::clone(&old.next);
            }
        }
    }

    /// This method publishes the pending event and finishes the revision
    #[inline(always)]
    pub fn publish(&mut self, pending: T) {
        self.publish_with(pending, |_| {});
    }

    /// For each revision, applies a function to the list of new events.
    pub fn with<F: FnMut(&T)>(&mut self, mut f: F) {
        loop {
            let ptr = self.next.0.load(Ordering::Acquire);
            if ptr.is_null() {
                break;
            } else {
                unsafe {
                    // This is safe since ptr cannot be changed once it is set
                    // which means that this is now a Arc or a Box.
                    let x: &RevisionNode<T> = mem::transmute(&*ptr);
                    f(&x.data);
                    self.next = Arc::clone(&x.next);
                }
            }
        }
    }
}
