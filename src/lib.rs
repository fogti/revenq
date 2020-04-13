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
struct RevisionNode<T: Clone> {
    data: Vec<T>,
    next: NextRevision<T>,
}

pub struct Queue<T: Clone> {
    pub pending: Vec<T>,
    latest: NextRevision<T>,
}

pub struct QueueListener<T: Clone> {
    next: NextRevision<T>,
}

impl<T: Clone> Default for Queue<T> {
    fn default() -> Self {
        Queue {
            pending: Vec::new(),
            latest: Arc::new(AtomSetOnce::empty()),
        }
    }
}

impl<T: Clone> Queue<T> {
    #[inline(always)]
    pub fn new() -> Self {
        Default::default()
    }

    pub fn listen(&self) -> QueueListener<T> {
        QueueListener {
            next: Arc::clone(&self.latest),
        }
    }

    /// This method publishes the current pending events and finishes the revision
    pub fn publish(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        // 1. prepare revision
        let mut latest = Arc::new(AtomSetOnce::empty());
        let mut revnode = Some(Box::new(RevisionNode {
            data: std::mem::take(&mut self.pending),
            next: Arc::clone(&latest),
        }));

        // 2. create dangling $self.latest
        //    (this is ok because we have a [&mut self] reference)
        std::mem::swap(&mut latest, &mut self.latest);
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

            latest = {
                // This is safe since ptr cannot be changed once it is set
                // which means that this is now a Box.
                // use the next revision
                Arc::clone(&(unsafe { &*old }).next)
            };
        }
    }

    /// This is a convenience method to push an event into the queue
    #[inline]
    pub fn push(&mut self, val: T) {
        self.pending.push(val);
    }
}

impl<T: Clone> QueueListener<T> {
    /// For each revision, applies a function to the list of new events.
    pub fn with<F: FnMut(&[T])>(&mut self, mut f: F) {
        loop {
            let ptr = self.next.0.load(Ordering::Acquire);
            if ptr.is_null() {
                break;
            } else {
                unsafe {
                    // This is safe since ptr cannot be changed once it is set
                    // which means that this is now a Arc or a Box.
                    let x: &RevisionNode<T> = mem::transmute(&*ptr);
                    f(&x.data[..]);
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
        q.push(0);
        q.publish();

        let mut l = q.listen();
        let mut marker = Vec::new();
        l.with(|evs| marker.extend(evs.iter().copied()));
        assert!(marker.is_empty());

        q.push(1);
        q.publish();

        l.with(|evs| marker.extend(evs.iter().copied()));
        assert_eq!(marker, &[1]);
    }

    #[test]
    fn queue_multi() {
        let mut q = Queue::new();
        let mut l1 = q.listen();
        let mut l2 = q.listen();

        q.push(0);
        q.publish();

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
            let mut lx = q.listen();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                let mut marker = Vec::new();
                lx.with(|evs| marker.extend(evs.iter().copied()));
                assert_eq!(marker, &[0]);
                marker.clear();
                thread::sleep(Duration::from_millis(20));
                lx.with(|evs| marker.extend(evs.iter().copied()));
                assert_eq!(marker, &[1]);
                thread::sleep(Duration::from_millis(40));
                let mut marker = Vec::new();
                lx.with(|evs| marker.push(evs.to_vec()));
                assert_eq!(marker, &[[2], [3]]);
            })
        };

        let th1 = spt(&q);
        let th2 = spt(&q);
        q.push(0);
        q.publish();
        thread::sleep(Duration::from_millis(60));
        q.push(1);
        q.publish();
        thread::sleep(Duration::from_millis(30));
        q.push(2);
        q.publish();
        q.push(3);
        q.publish();
        th1.join().unwrap();
        th2.join().unwrap();
    }
}
