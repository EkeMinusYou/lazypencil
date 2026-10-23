//! A bounded loopback HTTP fixture with post-load and unfinished fetches.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Server {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) if requests.len() < 64 => {
                        let stopped = stopped.clone();
                        requests.push(thread::spawn(move || serve(stream, &stopped)));
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Cannot accept fixture connection: {error}"),
                }
            }
            for request in requests {
                request.join().unwrap();
            }
        });
        Self {
            address,
            stop,
            task: Some(task),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}

fn serve(mut stream: TcpStream, stop: &AtomicBool) {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut header = Vec::new();
    while header.len() < 8192 && !header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 512];
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => header.extend_from_slice(&buffer[..count]),
        }
    }
    let request = String::from_utf8_lossy(&header);
    let path = request.split_whitespace().nth(1).unwrap_or("");
    let (kind, body, delay) = match path {
        "/delayed" => (
            "text/html",
            "<style>html{background:#f00}</style><script>addEventListener('load',()=>fetch('/color').then(r=>r.text()).then(color=>document.documentElement.style.background=color));</script>",
            0,
        ),
        "/busy" => (
            "text/html",
            "<style>html{background:#f00}</style><script>addEventListener('load',()=>fetch('/never').then(r=>r.text()));</script>",
            0,
        ),
        "/color" => ("text/plain", "#00ff00", 1200),
        "/never" => ("text/plain", "ok", 10000),
        _ => ("text/plain", "", 0),
    };
    if write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len()).is_err() {
        return;
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(delay) {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let _ = stream.write_all(body.as_bytes());
}
