use criterion::{criterion_group, criterion_main, Criterion};
use revenq::QueueInterface;

fn queue_bench(c: &mut Criterion) {
    use revenq::Queue;
    c.bench_function("queue-simple", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            q.publish(0);
            let mut l = q.clone();
            l.recv();
            q.publish(1);
            l.recv();
        })
    });

    c.bench_function("queue-multi", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.publish(0);
            l1.recv();
            l2.recv();
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
            l.recv();
            q.publish(1);
            l.recv();
        })
    });

    c.bench_function("woke-queue-multi", |b| {
        b.iter(|| {
            let mut q = WokeQueue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.publish(0);
            l1.recv();
            l2.recv();
        })
    });

    c.bench_function("woke-queue-blocking", |b| {
        b.iter(|| {
            let spt = |mut q: WokeQueue<u32>, publiv: Vec<u32>| {
                thread::spawn(move || {
                    let mut c = Vec::new();
                    let plvl = publiv.len();
                    for i in publiv {
                        c.extend(q.publish(i).into_iter().map(|i| *i));
                    }
                    while c.len() < plvl {
                        c.extend(q.recv_blocking().into_iter().map(|i| *i));
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
