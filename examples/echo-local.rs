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
            let mut queue = revenq::Queue::<u8>::new();
            let mut queue2 = queue.clone();
            for _ in 0..msg_per_client {
                queue.enqueue(b'a');
            }
            queue.next();
            drop(queue);
            let mut queue_answ = revenq::Queue::<u8>::new();
            let mut queue_answ2 = queue_answ.clone();
            while let Some(x) = queue2.next_async().await {
                queue_answ.enqueue(*x);
            }
            queue_answ.next();
            drop(queue_answ);
            for _ in 0..msg_per_client {
                assert_eq!(*queue_answ2.next_async().await.unwrap(), b'a');
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
