fn accumulate<T>(rv: &mut impl Iterator<Item = revenq::RevisionRef<T>>) -> T
where
    T: Copy + std::iter::Sum,
{
    rv.map(|i| *i).sum()
}

#[test]
fn simple() {
    let mut q = Queue::new();
    q.enqueue(vec![0]);
    q.skip_and_publish();

    let mut l = q.clone();
    let mut marker = Vec::new();
    marker.extend((&mut l).map(|i| (*i).clone()).flatten());
    assert!(marker.is_empty());

    q.enqueue(vec![1]);
    q.skip_and_publish();

    marker.extend((&mut l).map(|i| (*i).clone()).flatten());
    assert_eq!(marker, [1]);
}

#[test]
fn multi() {
    let mut q = Queue::new();
    let l1 = q.clone();
    let mut l2 = q.clone();

    q.enqueue(0);
    q.enqueue(1);
    q.skip_and_publish();

    let mut marker = Vec::new();
    marker.extend(l1.map(|i| *i));
    assert_eq!(marker, [0, 1]);
    marker.clear();
    let mut fi = l2.next().unwrap();
    marker.push(*fi);
    marker.extend(l2.map(|i| *i));
    assert_eq!(marker, [0, 1]);
    // detach fi
    assert!(revenq::RevisionRef::try_detach(&mut fi).is_ok());
    assert_eq!(*fi, 0);
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
            let marker = accumulate(&mut lx);
            assert_eq!(marker, 1);
            thread::sleep(Duration::from_millis(20));
            let marker = accumulate(&mut lx);
            assert_eq!(marker, 2);
            thread::sleep(Duration::from_millis(40));
            let marker: Vec<_> = lx.map(|i| *i).collect();
            assert_eq!(marker, &[3, 4]);
        })
    };

    let th1 = spt(&q);
    let th2 = spt(&q);
    q.enqueue(1);
    q.skip_and_publish();
    thread::sleep(Duration::from_millis(60));
    q.enqueue(2);
    q.skip_and_publish();
    thread::sleep(Duration::from_millis(30));
    q.enqueue(3);
    q.enqueue(4);
    q.skip_and_publish();
    th1.join().unwrap();
    th2.join().unwrap();
}

#[test]
fn mp() {
    let mut q1 = Queue::new();
    let mut q2 = q1.clone();

    let (mut c1, mut c2) = (0, 0);
    q1.enqueue(1);
    c1 += accumulate(&mut q1);
    q2.enqueue(2);
    c2 += accumulate(&mut q2);
    q1.enqueue(3);
    c1 += accumulate(&mut q1);
    q2.enqueue(4);
    c2 += accumulate(&mut q2);
    c1 += accumulate(&mut q1);
    c2 += accumulate(&mut q2);
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
                q.enqueue(i);
                c += accumulate(&mut q);
                thread::sleep(Duration::from_millis(20));
            }
            c += accumulate(&mut q);
            c
        })
    };

    let th1 = spt(q1, vec![1, 3]);
    let th2 = spt(q2, vec![2, 4]);
    assert_eq!(th1.join().unwrap(), 6);
    assert_eq!(th2.join().unwrap(), 4);
}
