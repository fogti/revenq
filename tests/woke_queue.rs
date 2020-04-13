use revenq::{QueueInterface, WokeQueue as Queue};

include!("../tests_/queue.rs");

#[test]
#[cfg_attr(miri, ignore)]
fn blocking() {
    use std::thread;
    let q1 = Queue::new();
    let q2 = q1.clone();

    let spt = |mut q: Queue<u32>, publiv: Vec<u32>| {
        thread::spawn(move || {
            let mut c = Vec::new();
            let plvl = publiv.len();
            for i in publiv {
                q.publish_with(i, |pm| c.push(*pm.current));
            }
            while c.len() < plvl {
                q.with_blocking(|cur| {
                    c.push(*cur);
                    true
                });
            }
            c
        })
    };

    let th1 = spt(q1, vec![1, 3]);
    let th2 = spt(q2, vec![2, 4]);
    assert_eq!(th1.join().unwrap(), [2, 4]);
    assert_eq!(th2.join().unwrap(), [1, 3]);
}
