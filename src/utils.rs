use std::sync::atomic::{AtomicPtr, Ordering};
pub use std::sync::Arc;
use std::{fmt, mem, ptr};

/// An AtomSetOnce wraps an AtomicPtr, it allows for safe mutation of an atomic
/// into common Rust Types.
pub struct AtomSetOnce<T>(AtomicPtr<T>);

impl<T> fmt::Debug for AtomSetOnce<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "atom({:?})", self.0.load(Ordering::Acquire))
    }
}

impl<T> AtomSetOnce<T> {
    /// Create a empty AtomSetOnce
    #[inline]
    pub fn empty() -> AtomSetOnce<T> {
        AtomSetOnce(AtomicPtr::new(ptr::null_mut()))
    }
}

impl<T> Drop for AtomSetOnce<T> {
    fn drop(&mut self) {
        let ptr = *self.0.get_mut();
        if !ptr.is_null() {
            unsafe {
                let _: Box<T> = Box::from_raw(ptr);
            }
        }
    }
}

unsafe impl<T: Send + 'static> Send for AtomSetOnce<T> {}
unsafe impl<T: Sync + 'static> Sync for AtomSetOnce<T> {}

pub type NextRevision<T> = Arc<AtomSetOnce<RevisionNode<T>>>;

#[derive(Clone, Debug)]
pub struct RevisionNode<T> {
    pub(crate) next: NextRevision<T>,
    pub(crate) data: T,
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

impl std::error::Error for RevisionDetachError {}

impl<T> Clone for RevisionRef<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> std::ops::Deref for RevisionRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // This pointer should never change once RevisionRef is created until
        // it's dropped.
        &Self::deref_to_rn(self).data
    }
}

#[cfg(feature = "stable_deref_trait")]
unsafe impl<T> stable_deref_trait::StableDeref for RevisionRef<T> {}

#[cfg(feature = "stable_deref_trait")]
unsafe impl<T> stable_deref_trait::CloneStableDeref for RevisionRef<T> {}

impl<T> RevisionRef<T> {
    pub(crate) fn new(nr: &NextRevision<T>) -> Option<Self> {
        ptr::NonNull::new(nr.0.load(Ordering::Acquire)).map(|rptr| {
            let ret = Self {
                inner: Arc::clone(&nr),
            };
            Self::check_against_rptr(&ret, rptr);
            ret
        })
    }

    /// try to append revnode, if CAS succeeds, return None, otherwise:
    /// return a RevisionRef for the failed CAS ptr, and the revnode;
    /// set $latest to the next ptr
    pub(crate) fn new_cas(
        latest: &mut NextRevision<T>,
        revnode: Box<RevisionNode<T>>,
    ) -> Option<(Self, Box<RevisionNode<T>>)> {
        let new = Box::into_raw(revnode);
        let old = latest.0.compare_and_swap(ptr::null_mut(), new, Ordering::AcqRel);
        let rptr = ptr::NonNull::new(old)?;
        let real_old: &RevisionNode<T> = unsafe { rptr.as_ref() };

        let ret_self = Self {
            // This is safe since ptr cannot be changed once it is set
            // which means that this is now a Box.
            // use the next revision
            inner: mem::replace(latest, Arc::clone(&real_old.next)),
        };
        Self::check_against_rptr(&ret_self, rptr);
        Some((ret_self, unsafe { Box::from_raw(new) }))
    }

    #[inline]
    fn check_against_rptr(this: &Self, rptr: ptr::NonNull<RevisionNode<T>>) {
        assert!(std::ptr::eq(&**this, &unsafe { rptr.as_ref() }.data));
    }

    #[inline]
    fn deref_to_rn(this: &Self) -> &RevisionNode<T> {
        unsafe { &*this.inner.0.load(Ordering::Acquire) }
    }

    /// Try to detach this revision from the following.
    /// Only works if this `RevisionRef` is the last reference to this revision.
    /// This is the case if no RevisionRef to a revision with precedes this
    /// revision exist and this is the last ptr to this revision, and all queue
    /// references have already consumed this revision.
    /// Use this method to reduce queue memory usage if you want to store this
    /// object long-term.
    pub fn try_detach(this: &mut Self) -> Result<(), RevisionDetachError> {
        // get ownership over the Arc of revision $this.inner
        let ptr_this = Arc::get_mut(&mut this.inner).ok_or(RevisionDetachError)?;
        // no other reference to *us* exists.
        // the following is safe because we know that we point to valid data
        // (with lifetime = as long as $this.inner exists with the current Arc)
        let mut_this: &mut RevisionNode<T> = unsafe { &mut **ptr_this.0.get_mut() };
        // override our $next ptr, thus decoupling this node from the following
        mut_this.next = Arc::new(AtomSetOnce::empty());
        Ok(())
    }

    #[inline]
    pub(crate) fn next(this: &Self) -> NextRevision<T> {
        Arc::clone(&Self::deref_to_rn(this).next)
    }
}

/// This is a helper function to debug queues.
#[cold]
pub fn print_queue<W, T>(mut writer: W, start: NextRevision<T>, prefix: &str) -> std::io::Result<()>
where
    W: std::io::Write,
    T: fmt::Debug,
{
    let mut cur = start;
    let mut cnt = 0;
    while let Some(x) = RevisionRef::new(&cur) {
        writeln!(&mut writer, "{} {}. {:?}", prefix, cnt, &*x)?;
        cur = RevisionRef::next(&x);
        cnt += 1;
    }
    Ok(())
}
