use criterion::{criterion_group, criterion_main, Criterion};
use revenq::QueueInterface;

fn queue_bench(c: &mut Criterion) {
    use revenq::Queue;
    c.bench_function("queue-simple", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            q.publish(0);
            let mut l = q.clone();
            l.with(|_evs| {});
            q.publish(1);
            l.with(|_evs| {});
        })
    });

    c.bench_function("queue-multi", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.publish(0);
            l1.with(|_evs| {});
            l2.with(|_evs| {});
        })
    });
}

fn woke_queue_bench(c: &mut Criterion) {
    use revenq::WokeQueue;
    use std::thread;

    c.bench_function("woke-queue-simple", |b| {
        b.iter(|| {
            let mut q = WokeQueue::new();
            q.publish(0);
            let mut l = q.clone();
            l.with(|_evs| {});
            q.publish(1);
            l.with(|_evs| {});
        })
    });

    c.bench_function("woke-queue-multi", |b| {
        b.iter(|| {
            let mut q = WokeQueue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.publish(0);
            l1.with(|_evs| {});
            l2.with(|_evs| {});
        })
    });

    c.bench_function("woke-queue-blocking", |b| {
        b.iter(|| {
            let spt = |mut q: WokeQueue<u32>, publiv: Vec<u32>| {
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
                })
            };

            let q1 = WokeQueue::new();
            let q2 = q1.clone();
            let th1 = spt(q1, vec![1, 3]);
            let th2 = spt(q2, vec![2, 4]);
            th1.join().unwrap();
            th2.join().unwrap();
        })
    });
}

criterion_group!(benches, queue_bench, woke_queue_bench);
criterion_main!(benches);
