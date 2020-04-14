use criterion::{criterion_group, criterion_main, Criterion};
use revenq::QueueInterface;

fn queue_bench(c: &mut Criterion) {
    use revenq::Queue;
    c.bench_function("queue-simple", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            q.enqueue(0);
            q.skip_and_publish();
            let mut l = q.clone();
            l.next();
            q.enqueue(1);
            q.skip_and_publish();
            l.next();
        })
    });

    c.bench_function("queue-multi", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.enqueue(0);
            q.skip_and_publish();
            l1.next();
            l2.next();
        })
    });
}

fn woke_queue_bench(c: &mut Criterion) {
    use revenq::WokeQueue;
    use std::thread;

    c.bench_function("woke-queue-simple", |b| {
        b.iter(|| {
            let mut q = WokeQueue::new();
            q.enqueue(0);
            q.skip_and_publish();
            let mut l = q.clone();
            l.next();
            q.enqueue(1);
            q.skip_and_publish();
            l.next();
        })
    });

    c.bench_function("woke-queue-multi", |b| {
        b.iter(|| {
            let mut q = WokeQueue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.enqueue(0);
            q.skip_and_publish();
            l1.next();
            l2.next();
        })
    });

    c.bench_function("woke-queue-blocking", |b| {
        b.iter(|| {
            let spt = |mut q: WokeQueue<u32>, publiv: &[u32]| {
                *q.pending_mut() = publiv.iter().map(|i| *i).collect();
                let plvl = publiv.len();
                thread::spawn(move || {
                    let mut c = Vec::with_capacity(plvl);
                    while c.len() < plvl {
                        if let Some(x) = q.next_blocking() {
                            c.push(*x);
                        }
                    }
                    c.extend((&mut q).map(|i| *i));
                })
            };

            let q1 = WokeQueue::new();
            let q2 = q1.clone();
            let th1 = spt(q1, &[1, 3]);
            let th2 = spt(q2, &[2, 4]);
            th1.join().unwrap();
            th2.join().unwrap();
        })
    });
}

criterion_group!(benches, queue_bench, woke_queue_bench);
criterion_main!(benches);
