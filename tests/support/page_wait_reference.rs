//! A bounded loopback HTTP fixture with post-load and unfinished fetches.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Server {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
    #[allow(
        dead_code,
        reason = "Inspected by the browser_readiness integration target"
    )]
    network: Arc<NetworkCounts>,
}

#[derive(Default)]
struct NetworkCounts {
    http: AtomicUsize,
    websocket: AtomicUsize,
    tls: AtomicUsize,
    turn: AtomicUsize,
}

impl Server {
    #[allow(
        dead_code,
        reason = "Used by browser_readiness, not the shared dynamic_render fixture"
    )]
    pub fn turn_requests(&self) -> usize {
        self.network.turn.load(Ordering::SeqCst)
    }

    #[allow(
        dead_code,
        reason = "Used by browser_readiness, not the shared dynamic_render fixture"
    )]
    pub fn network_counts(&self) -> (usize, usize, usize) {
        (
            self.network.http.load(Ordering::SeqCst),
            self.network.websocket.load(Ordering::SeqCst),
            self.network.tls.load(Ordering::SeqCst),
        )
    }

    pub fn new() -> Self {
        Self::with_selector_probe(Arc::new(AtomicBool::new(false)))
    }

    pub fn with_selector_probe(probe: Arc<AtomicBool>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let network = Arc::new(NetworkCounts::default());
        let requests_network = network.clone();
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) if requests.len() < 64 => {
                        let stopped = stopped.clone();
                        let probe = probe.clone();
                        let network = requests_network.clone();
                        requests.push(thread::spawn(move || {
                            serve(stream, &stopped, &probe, &network)
                        }));
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
            network,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}

fn serve(mut stream: TcpStream, stop: &AtomicBool, probe: &AtomicBool, network: &NetworkCounts) {
    // Accepted sockets can inherit the listener's nonblocking mode.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut header = Vec::new();
    let header_end = Instant::now() + Duration::from_secs(2);
    while header.len() < 8192 && !header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        if stop.load(Ordering::SeqCst) || Instant::now() >= header_end {
            return;
        }
        let mut buffer = [0; 512];
        match stream.read(&mut buffer) {
            Ok(0) => return,
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
            Err(_) => return,
            Ok(count) => {
                // An intentional TLS sink proves HTTPS/WSS routing before
                // the expected handshake failure, without installing a test CA.
                if header.is_empty() && buffer[0] == 0x16 {
                    network.tls.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                header.extend_from_slice(&buffer[..count]);
                if header.len() >= 20 && header[4..8] == [0x21, 0x12, 0xa4, 0x42] {
                    network.turn.fetch_add(1, Ordering::SeqCst);
                    return;
                }
            }
        }
    }
    if !header.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        return;
    }
    let request = String::from_utf8_lossy(&header);
    let path = request.split_whitespace().nth(1).unwrap_or("");
    if path.starts_with("/network/socket")
        && let Some(key) = request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, value)| {
                name.eq_ignore_ascii_case("Sec-WebSocket-Key")
                    .then_some(value.trim())
            })
    {
        network.websocket.fetch_add(1, Ordering::SeqCst);
        let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
        let _ = write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        let _ = stream.write_all(&[0x88, 0]);
        let _ = stream.read(&mut [0; 128]);
        return;
    }
    if path.starts_with("/network/") {
        network.http.fetch_add(1, Ordering::SeqCst);
    }
    if path == "/polled" {
        probe.store(true, Ordering::SeqCst);
    }
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
        "/selector" => (
            "text/html",
            "<style>html{background:#f00}#ready{visibility:hidden;width:1px;height:1px}</style><div id='ready'></div><script>addEventListener('load',()=>fetch('/color').then(r=>r.text()).then(color=>{document.documentElement.style.background=color;document.getElementById('ready').style.visibility='visible'}));</script>",
            0,
        ),
        "/selector-cancel" => (
            "text/html",
            "<style>html{background:#0f0}</style><script>const query=document.querySelector.bind(document);let polled=false;document.querySelector=selector=>{if(selector==='#never'&&!polled){polled=true;fetch('/polled')}return query(selector)};</script>",
            0,
        ),
        "/color" => ("text/plain", "#00ff00", 1200),
        "/wait-image-page" => (
            "text/html",
            "<style>html{background:#f00;overflow:hidden}body{margin:0}img{display:block}</style><script>addEventListener('load',()=>setTimeout(()=>{const image=new Image();image.src='/wait-image.svg';document.body.appendChild(image)},50));</script>",
            0,
        ),
        "/wait-image.svg" => (
            "image/svg+xml",
            "<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'><path fill='#00ff00' d='M0 0h32v32H0z'/></svg>",
            1200,
        ),
        "/additional-wait-cancel" => (
            "text/html",
            "<style>#ready{width:1px;height:1px}</style><div id='ready'></div><script>const query=document.querySelector.bind(document);let polled=false;document.querySelector=selector=>{if(selector==='#ready'&&!polled){polled=true;fetch('/polled')}return query(selector)};</script>",
            0,
        ),
        "/never" => ("text/plain", "ok", 10000),
        "/network/image" => (
            "image/svg+xml",
            "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><path fill='green' d='M0 0h8v8H0z'/></svg>",
            0,
        ),
        "/network/green" => (
            "text/html",
            "<html data-network='yes' style='background:#00ff00'></html>",
            0,
        ),
        "/network/fetch" | "/network/worker" => ("text/plain", "ok", 0),
        _ => ("text/plain", "", 0),
    };
    let cors = if path.starts_with("/network/") {
        "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\n"
    } else {
        ""
    };
    if write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{cors}Cache-Control: no-store\r\nConnection: close\r\n\r\n", body.len()).is_err() {
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
