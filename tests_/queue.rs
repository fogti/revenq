use revenq::PendingMap;

#[test]
fn simple() {
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
fn multi() {
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
fn multithreaded() {
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
fn mp() {
    let mut q1 = Queue::new();
    let mut q2 = q1.clone();

    let (mut c1, mut c2) = (0, 0);
    let (mut cl1, mut cl2) = (
        |pm: PendingMap<u32>| c1 += *pm.current,
        |pm: PendingMap<u32>| c2 += *pm.current,
    );
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
fn mtmp() {
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
