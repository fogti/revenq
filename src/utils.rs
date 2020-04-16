use std::sync::atomic::{AtomicPtr, Ordering};
pub use std::sync::Arc;
use std::{fmt, mem, ops::Deref, ptr};

/// An AtomSetOnce wraps an AtomicPtr, it allows for safe mutation of an atomic
/// into common Rust Types.
pub struct AtomSetOnce<T>(AtomicPtr<T>);

impl<T> fmt::Debug for AtomSetOnce<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "atom({:?})", self.0.load(Ordering::Relaxed))
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

/// Trait abstracting over owning reference types pointing to a revision
pub trait RevisionRefTrait: Deref + Sized {
    /// Try to detach this revision from the following.
    /// Only works if this `RevisionRef` is the last reference to this revision.
    /// This is the case if no RevisionRef to a revision with precedes this
    /// revision exist and this is the last ptr to this revision, and all queue
    /// references have already consumed this revision.
    /// Use this method to reduce queue memory usage if you want to store this
    /// object long-term.
    fn try_detach(this: &mut Self) -> Result<(), RevisionDetachError>;

    /// Maps the value (the revision derefs to) to a sub-reference
    #[inline(always)]
    fn map<U, F>(this: Self, mapfn: F) -> MappedRevisionRef<Self, F>
    where
        F: Fn(&<Self as Deref>::Target) -> &U,
    {
        MappedRevisionRef { inner: this, mapfn }
    }
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

impl<T> Deref for RevisionRef<T> {
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
    pub(crate) fn new(nr: &NextRevision<T>, order: Ordering) -> Option<Self> {
        ptr::NonNull::new(nr.0.load(order)).map(|rptr| {
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
        order: Ordering,
    ) -> Option<(Self, Box<RevisionNode<T>>)> {
        let new = Box::into_raw(revnode);
        let old = latest.0.compare_and_swap(ptr::null_mut(), new, order);
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
        unsafe { &*this.inner.0.load(Ordering::Relaxed) }
    }

    #[inline]
    pub(crate) fn next(this: &Self) -> NextRevision<T> {
        Arc::clone(&Self::deref_to_rn(this).next)
    }
}

#[inline]
pub fn next_mut_of<T>(
    this: &mut NextRevision<T>,
) -> Result<Option<&mut NextRevision<T>>, RevisionDetachError> {
    // get ownership over the Arc of revision $this.inner
    let ptr_this = Arc::get_mut(this).ok_or(RevisionDetachError)?;
    // no other reference to *us* exists.
    // in case of RevisionRef:
    // the following is safe because we know that we point to valid data
    // (with lifetime = as long as $this.inner exists with the current Arc)
    Ok(if let Some(x) = unsafe { ptr_this.0.get_mut().as_mut() } {
        let mut_this: &mut RevisionNode<T> = x;
        Some(&mut mut_this.next)
    } else {
        // this code may be run on a Queue, and thus not be a valid RevisionRef
        None
    })
}

impl<T> RevisionRefTrait for RevisionRef<T> {
    fn try_detach(this: &mut Self) -> Result<(), RevisionDetachError> {
        let mut_next = next_mut_of(&mut this.inner)?;
        // override our $next ptr, thus decoupling this node from the following
        *mut_next.expect("revenq internal error: revision ref contract violated") =
            Arc::new(AtomSetOnce::empty());
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct MappedRevisionRef<T, F> {
    inner: T,
    mapfn: F,
}

impl<T, U, F> Deref for MappedRevisionRef<T, F>
where
    T: Deref,
    F: Fn(&<T as Deref>::Target) -> &U,
{
    type Target = U;

    #[inline]
    fn deref(&self) -> &U {
        (self.mapfn)(Deref::deref(&self.inner))
    }
}

impl<T, U, F> RevisionRefTrait for MappedRevisionRef<T, F>
where
    T: RevisionRefTrait,
    F: Fn(&<T as Deref>::Target) -> &U,
{
    #[inline(always)]
    fn try_detach(this: &mut Self) -> Result<(), RevisionDetachError> {
        RevisionRefTrait::try_detach(&mut this.inner)
    }
}

impl<T, F> MappedRevisionRef<T, F> {
    #[inline(always)]
    pub fn into_inner(this: Self) -> T {
        this.inner
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
    while let Some(x) = RevisionRef::new(&cur, Ordering::Relaxed) {
        writeln!(&mut writer, "{} {}. {:?}", prefix, cnt, &*x)?;
        cur = RevisionRef::next(&x);
        cnt += 1;
    }
    Ok(())
}
