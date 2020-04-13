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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_simple() {
        let mut q = Queue::new();
        q.publish(vec![0]);

        let mut l = q.clone();
        let mut marker = Vec::new();
        l.with(|evs| marker.extend(evs.iter().copied()));
        assert!(marker.is_empty());

        q.publish(vec![1]);

        l.with(|evs| marker.extend(evs.iter().copied()));
        assert_eq!(marker, &[1]);
    }

    #[test]
    fn queue_multi() {
        let mut q = Queue::new();
        let mut l1 = q.clone();
        let mut l2 = q.clone();

        q.publish(vec![0]);

        let mut marker = Vec::new();
        l1.with(|evs| marker.extend(evs.iter().copied()));
        assert_eq!(marker, &[0]);
        marker.clear();
        l2.with(|evs| marker.extend(evs.iter().copied()));
        assert_eq!(marker, &[0]);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn queue_multithreaded() {
        use std::{thread, time::Duration};
        let mut q = Queue::new();

        let spt = |q: &Queue<u32>| {
            let mut lx = q.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                let mut marker = 0;
                lx.with(|evs| marker += evs);
                assert_eq!(marker, 1);
                marker = 0;
                thread::sleep(Duration::from_millis(20));
                lx.with(|evs| marker += evs);
                assert_eq!(marker, 2);
                thread::sleep(Duration::from_millis(40));
                let mut marker = Vec::new();
                lx.with(|evs| marker.push(*evs));
                assert_eq!(marker, &[3, 4]);
            })
        };

        let th1 = spt(&q);
        let th2 = spt(&q);
        q.publish(1);
        thread::sleep(Duration::from_millis(60));
        q.publish(2);
        thread::sleep(Duration::from_millis(30));
        q.publish(3);
        q.publish(4);
        th1.join().unwrap();
        th2.join().unwrap();
    }

    #[test]
    fn queue_mp() {
        let mut q1 = Queue::new();
        let mut q2 = q1.clone();

        let (mut c1, mut c2) = (0, 0);
        let (mut cl1, mut cl2) = (|pm: PendingMap<u32>| c1 += *pm.current, |pm: PendingMap<u32>| c2 += *pm.current);
        q1.publish_with(1, &mut cl1);
        q2.publish_with(2, &mut cl2);
        q1.publish_with(3, &mut cl1);
        q2.publish_with(4, &mut cl2);
        q1.with(|cur| c1 += *cur);
        q2.with(|cur| c2 += *cur);
        assert_eq!(c1, 6);
        assert_eq!(c2, 4);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn queue_mtmp() {
        use std::{thread, time::Duration};
        let q1 = Queue::new();
        let q2 = q1.clone();

        let spt = |mut q: Queue<u32>, publiv: Vec<u32>| {
            thread::spawn(move || {
                let mut c = 0;
                for i in publiv {
                    q.publish_with(i, |pm: PendingMap<u32>| c += *pm.current);
                    thread::sleep(Duration::from_millis(20));
                }
                q.with(|cur| c += *cur);
                c
            })
        };

        let th1 = spt(q1, vec![1, 3]);
        let th2 = spt(q2, vec![2, 4]);
        assert_eq!(th1.join().unwrap(), 6);
        assert_eq!(th2.join().unwrap(), 4);
    }
}
