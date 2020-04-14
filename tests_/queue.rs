fn accumulate<T>(rv: Vec<revenq::RevisionRef<T>>) -> T
where
    T: Copy + std::iter::Sum,
{
    rv.into_iter().map(|i| *i).sum()
}

#[test]
fn simple() {
    let mut q = Queue::new();
    q.publish(vec![0]);

    let mut l = q.clone();
    let mut marker = Vec::new();
    marker.extend(l.recv().into_iter().map(|i| (*i).clone()).flatten());
    assert!(marker.is_empty());

    q.publish(vec![1]);

    marker.extend(l.recv().into_iter().map(|i| (*i).clone()).flatten());
    assert_eq!(marker, [1]);
}

#[test]
fn multi() {
    let mut q = Queue::new();
    let mut l1 = q.clone();
    let mut l2 = q.clone();

    q.publish(0);

    let mut marker = Vec::new();
    marker.extend(l1.recv().into_iter().map(|i| *i));
    assert_eq!(marker, [0]);
    marker.clear();
    marker.extend(l2.recv().into_iter().map(|i| *i));
    assert_eq!(marker, [0]);
}

#[test]
#[cfg_attr(miri, ignore)]
fn multithreaded() {
    use std::{thread, time::Duration};
    let mut q = Queue::new();

    let spt = |q: &Queue<u32>| {
        let mut lx = q.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            let marker = accumulate(lx.recv());
            assert_eq!(marker, 1);
            thread::sleep(Duration::from_millis(20));
            let marker = accumulate(lx.recv());
            assert_eq!(marker, 2);
            thread::sleep(Duration::from_millis(40));
            let marker: Vec<_> = lx.recv().into_iter().map(|i| *i).collect();
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
fn mp() {
    let mut q1 = Queue::new();
    let mut q2 = q1.clone();

    let (mut c1, mut c2) = (0, 0);
    c1 += accumulate(q1.publish(1));
    c2 += accumulate(q2.publish(2));
    c1 += accumulate(q1.publish(3));
    c2 += accumulate(q2.publish(4));
    c1 += accumulate(q1.recv());
    c2 += accumulate(q2.recv());
    assert_eq!(c1, 6);
    assert_eq!(c2, 4);
}

#[test]
#[cfg_attr(miri, ignore)]
fn mtmp() {
    use std::{thread, time::Duration};
    let q1 = Queue::new();
    let q2 = q1.clone();

    let spt = |mut q: Queue<u32>, publiv: Vec<u32>| {
        thread::spawn(move || {
            let mut c = 0;
            for i in publiv {
                c += accumulate(q.publish(i));
                thread::sleep(Duration::from_millis(20));
            }
            c += accumulate(q.recv());
            c
        })
    };

    let th1 = spt(q1, vec![1, 3]);
    let th2 = spt(q2, vec![2, 4]);
    assert_eq!(th1.join().unwrap(), 6);
    assert_eq!(th2.join().unwrap(), 4);
}
