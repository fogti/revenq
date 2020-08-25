use criterion::{criterion_group, criterion_main, Criterion};
use reclutch_event::prelude::*;

fn queue_bench(c: &mut Criterion) {
    c.bench_function("rqueue-simple", |b| {
        b.iter(|| {
            use reclutch_event::nonts::Queue;
            let q: Queue<u32> = Default::default();
            q.emit_owned(0);
            let l = q.listen();
            l.with(|_| ());
            q.emit_owned(1);
            l.with(|_| ());
        })
    });

    c.bench_function("rqueue-multi", |b| {
        b.iter(|| {
            use reclutch_event::nonts::Queue;
            let q: Queue<u32> = Default::default();
            let l1 = q.listen();
            let l2 = q.listen();

            q.emit_owned(0);
            l1.with(|_| ());
            l2.with(|_| ());
        })
    });

    c.bench_function("rcc-queue-blocking", |b| {
        b.iter(|| {
            use crossbeam_channel::{Sender, Receiver, unbounded};
            use std::sync::Arc;
            use std::thread;
            let spt = |qo: Sender<Arc<u32>>, qi: Receiver<Arc<u32>>, publiv: &[u32]| {
                let plvl = publiv.len();
                for &i in publiv.iter() {
                    let _ = qo.send(Arc::new(i));
                }
                thread::spawn(move || {
                    let mut c = Vec::with_capacity(plvl);
                    while c.len() < plvl {
                        if let Ok(x) = qi.recv() {
                            c.push(*x);
                        }
                    }
                    c.extend(qi.iter().map(|i| *i));
                })
            };

            let (s1, r1) = unbounded();
            let (s2, r2) = unbounded();
            let th1 = spt(s1, r2, &[1, 3]);
            let th2 = spt(s2, r1, &[2, 4]);
            th1.join().unwrap();
            th2.join().unwrap();
        })
    });
}

criterion_group!(benches, queue_bench);
criterion_main!(benches);
