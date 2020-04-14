use revenq::QueueInterface;

fn main() {
    use revenq::WokeQueue;
    use std::thread;

    loop {
        let spt = |mut q: WokeQueue<u32>, publiv: Vec<u32>| {
            thread::spawn(move || {
                let mut c = Vec::new();
                let plvl = publiv.len();
                for i in publiv {
                    q.enqueue(i);
                }
                while c.len() < plvl {
                    q.print_debug(
                        std::io::stdout(),
                        &format!("{:?} |+", std::thread::current().id()),
                    )
                    .unwrap();
                    match q.next_blocking() {
                        Some(x) => c.push(*x),
                        None => {
                            println!("{:?} | ouch; c = {:?}", std::thread::current().id(), c);
                            q.print_debug(
                                std::io::stdout(),
                                &format!("{:?} |+", std::thread::current().id()),
                            )
                            .unwrap();
                            panic!();
                        }
                    }
                }
                c.extend((&mut q).map(|i| *i));
                println!("{:?} | done; c = {:?}", std::thread::current().id(), c);
                q.print_debug(
                    std::io::stdout(),
                    &format!("{:?} |+", std::thread::current().id()),
                )
                .unwrap();
            })
        };

        let q1 = WokeQueue::new();
        let q2 = q1.clone();
        let th1 = spt(q1, vec![1, 3]);
        let th2 = spt(q2, vec![2, 4]);
        th1.join().unwrap();
        th2.join().unwrap();
        println!();
    }
}