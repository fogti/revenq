use criterion::{criterion_group, criterion_main, Criterion};
use revenq::Queue;

#[inline]
fn skip_and_publish<T: Send + 'static>(q: &mut Queue<T>) {
    while q.next().is_some() {}
}

fn queue_bench(c: &mut Criterion) {
    c.bench_function("queue-simple", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            q.enqueue(0);
            skip_and_publish(&mut q);
            let mut l = q.clone();
            l.next();
            q.enqueue(1);
            skip_and_publish(&mut q);
            l.next();
        })
    });

    c.bench_function("queue-multi", |b| {
        b.iter(|| {
            let mut q = Queue::new();
            let mut l1 = q.clone();
            let mut l2 = q.clone();

            q.enqueue(0);
            skip_and_publish(&mut q);
            l1.next();
            l2.next();
        })
    });

    c.bench_function("queue-blocking", |b| {
        b.iter(|| {
            use std::thread;
            let spt = |mut q: Queue<u32>, publiv: &[u32]| {
                q.pending = publiv.iter().map(|i| *i).collect();
                let plvl = publiv.len();
                thread::spawn(move || {
                    let mut c = Vec::with_capacity(plvl);
                    while c.len() < plvl {
                        if let Some(x) = futures_lite::future::block_on(q.next_async()) {
                            c.push(*x);
                        }
                    }
                    c.extend((&mut q).map(|i| *i));
                })
            };

            let q1 = Queue::new();
            let q2 = q1.clone();
            let th1 = spt(q1, &[1, 3]);
            let th2 = spt(q2, &[2, 4]);
            th1.join().unwrap();
            th2.join().unwrap();
        })
    });
}

criterion_group!(benches, queue_bench);
criterion_main!(benches);
