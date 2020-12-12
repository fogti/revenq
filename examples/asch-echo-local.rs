use async_executor::Executor;
use std::mem::drop;
use std::sync::Arc;
use std::time::Instant;

async fn server(conns: usize, ex: &Executor<'static>) {
    let msgs: usize = 900_000;
    let msg_per_client = msgs / conns;

    let mut clients = vec![];
    let now = Instant::now();
    for _ in 0..conns {
        clients.push(ex.spawn(async move {
            let (cs, sr) = async_channel::unbounded();
            let (ss, cr) = async_channel::unbounded();
            for _ in 0..msg_per_client {
                cs.send(b'a').await.unwrap();
            }
            drop(cs);
            while let Ok(x) = sr.recv().await {
                ss.send(x).await.unwrap();
            }
            drop(sr);
            drop(ss);
            for _ in 0..msg_per_client {
                assert_eq!(cr.recv().await.unwrap(), b'a');
            }
        }))
    }

    for c in clients {
        c.await;
    }

    let delta = now.elapsed();
    println!(
        "Sent {} messages in {} ms. {:.2} msg/s",
        msgs,
        delta.as_millis(),
        msgs as f64 / delta.as_secs_f64()
    );
}

fn main() {
    use futures_lite::future::block_on as fblon;
    let ex = Arc::new(Executor::new());
    let shutdown = event_listener::Event::new();

    for _ in 0..num_cpus::get() {
        let ex = ex.clone();
        let listener = shutdown.listen();
        std::thread::spawn(move || fblon(ex.run(listener)));
    }

    let ex2 = ex.clone();
    fblon(ex.run(async move {
        server(1, &ex2).await;
        // This should drive the CPU utilization to 100%.
        // Asynchronous execution needs parallelism to thrive!
        server(5, &ex2).await;
    }));
    shutdown.notify(usize::MAX);
}
