//! Owned transport sinks observe routing, without pretending to complete handshakes.

use super::super::reference;
use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Server {
    pub tcp: reference::Server,
    pub stun: SocketAddr,
    pub quic: SocketAddr,
    pub worker_quic: SocketAddr,
    counts: Arc<[AtomicUsize; 3]>,
    stop: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
}

impl Server {
    pub fn new() -> Self {
        let mut server = Self {
            tcp: reference::Server::new(),
            stun: SocketAddr::from(([127, 0, 0, 1], 0)),
            quic: SocketAddr::from(([127, 0, 0, 1], 0)),
            worker_quic: SocketAddr::from(([127, 0, 0, 1], 0)),
            counts: Arc::new(std::array::from_fn(|_| AtomicUsize::new(0))),
            stop: Arc::new(AtomicBool::new(false)),
            tasks: Vec::new(),
        };
        for index in 0..3 {
            let socket = UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            if index == 0 {
                server.stun = socket.local_addr().unwrap();
            } else if index == 1 {
                server.quic = socket.local_addr().unwrap();
            } else {
                server.worker_quic = socket.local_addr().unwrap();
            }
            socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let counts = server.counts.clone();
            let stop = server.stop.clone();
            server.tasks.push(thread::spawn(move || {
                let mut buffer = [0; 2048];
                while !stop.load(Ordering::SeqCst) {
                    match socket.recv_from(&mut buffer) {
                        Ok((size, _)) => {
                            let valid = if index == 0 {
                                size >= 20 && buffer[4..8] == [0x21, 0x12, 0xa4, 0x42]
                            } else {
                                size >= 1200 && buffer[0] & 0xc0 == 0xc0 && buffer[1..5] != [0; 4]
                            };
                            if valid {
                                counts[index].fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock
                                    | std::io::ErrorKind::TimedOut
                                    | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            continue;
                        }
                        Err(error) => panic!("Owned UDP transport sink failed: {error}"),
                    }
                }
            }));
        }
        server
    }

    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.counts[0].load(Ordering::SeqCst),
            self.tcp.turn_requests(),
            self.counts[1].load(Ordering::SeqCst),
            self.counts[2].load(Ordering::SeqCst),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let mut failed = false;
        for task in self.tasks.drain(..) {
            failed |= task.join().is_err();
        }
        if !thread::panicking() {
            assert!(!failed, "Owned transport sink panicked");
        }
    }
}
