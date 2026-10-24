use super::*;
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};
use tungstenite::Message;

enum Reply {
    Header,
    Frame,
    Echo,
    Unread,
}

struct Peer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    sent: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    fn new(reply: Reply, cancel: Option<CancellationToken>) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicUsize::new(0));
        let stop_peer = stop.clone();
        let sent_peer = sent.clone();
        let thread = thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                if stop_peer.load(Ordering::Acquire) || Instant::now() >= end {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Cannot accept CDP fixture: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            if matches!(reply, Reply::Header) {
                stream
                    .write_all(b"HTTP/1.1 101 Switching Protocols\r\nX-Pending: ")
                    .unwrap();
            } else {
                let mut socket = tungstenite::accept(stream).unwrap();
                if matches!(reply, Reply::Echo) {
                    let text = socket.read().unwrap();
                    socket.send(text).unwrap();
                    return;
                }
                stream = socket.into_inner();
                if matches!(reply, Reply::Frame) {
                    // Start a 16 KiB text frame, then keep its body incomplete.
                    stream.write_all(&[0x81, 126, 0x40, 0x00]).unwrap();
                }
            }
            let end = Instant::now() + Duration::from_secs(3);
            // Keep header chunks above the protocol's average-packet defense;
            // the response stays incomplete and must stop at our own budget.
            let header_chunk = [b'x'; 256];
            let chunk: &[u8] = if matches!(reply, Reply::Header) {
                &header_chunk
            } else {
                b"x"
            };
            while Instant::now() < end && !stop_peer.load(Ordering::Acquire) {
                if !matches!(reply, Reply::Unread) {
                    if stream.write_all(chunk).is_err() {
                        return;
                    }
                    let count = sent_peer.fetch_add(1, Ordering::AcqRel) + 1;
                    if count == 3
                        && let Some(cancel) = &cancel
                    {
                        cancel.cancel();
                    }
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        Self {
            address,
            stop,
            sent,
            thread: Some(thread),
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let joined = self.thread.take().unwrap().join();
        if !thread::panicking() {
            joined.unwrap();
        }
    }
}

#[test]
fn partial_http_headers_cannot_extend_the_deadline_or_hide_cancellation() {
    for cancel in [false, true] {
        let token = CancellationToken::new();
        let peer = Peer::new(Reply::Header, cancel.then(|| token.clone()));
        let budget = Budget::new(
            Duration::from_secs(if cancel { 5 } else { 1 }),
            Some(&token),
        );
        let started = Instant::now();
        let error = connect(peer.address, "/devtools/browser/fixture", &budget)
            .err()
            .expect("An incomplete HTTP header must not finish handshaking");
        assert_eq!(
            error.code,
            if cancel {
                "CANCELLED"
            } else {
                "RENDER_TIMEOUT"
            }
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            peer.sent.load(Ordering::Acquire) >= 3,
            "The peer must keep sending partial response bytes"
        );
    }
}

#[test]
fn partial_websocket_frames_cannot_extend_the_deadline_or_hide_cancellation() {
    for cancel in [false, true] {
        let token = CancellationToken::new();
        let peer = Peer::new(Reply::Frame, cancel.then(|| token.clone()));
        let mut socket = connect(
            peer.address,
            "/devtools/browser/fixture",
            &Budget::new(Duration::from_secs(5), None),
        )
        .unwrap();
        let budget = Budget::new(
            Duration::from_secs(if cancel { 5 } else { 1 }),
            Some(&token),
        );
        socket.get_mut().set_budget(&budget);
        let started = Instant::now();
        let error = socket.read().unwrap_err();
        assert!(matches!(error, tungstenite::Error::Io(ref error) if pending(error)));
        assert_eq!(
            budget.check().unwrap_err().code,
            if cancel {
                "CANCELLED"
            } else {
                "RENDER_TIMEOUT"
            }
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            peer.sent.load(Ordering::Acquire) >= 3,
            "The peer must keep sending an incomplete frame"
        );
    }
}

#[test]
fn a_new_command_budget_replaces_the_expired_handshake_budget() {
    let peer = Peer::new(Reply::Echo, None);
    let handshake = Budget::new(Duration::from_millis(500), None);
    let mut socket = connect(peer.address, "/devtools/browser/fixture", &handshake).unwrap();
    thread::sleep(Duration::from_millis(550));
    assert_eq!(handshake.check().unwrap_err().code, "RENDER_TIMEOUT");
    socket
        .get_mut()
        .set_budget(&Budget::new(Duration::from_secs(2), None));
    socket.send(Message::Text("next operation".into())).unwrap();
    assert_eq!(
        socket.read().unwrap(),
        Message::Text("next operation".into())
    );
}

#[cfg(unix)]
#[test]
fn a_peer_that_stops_reading_cannot_block_websocket_writes_past_the_deadline() {
    use std::os::fd::AsRawFd;
    let peer = Peer::new(Reply::Unread, None);
    let mut socket = connect(
        peer.address,
        "/devtools/browser/fixture",
        &Budget::new(Duration::from_secs(5), None),
    )
    .unwrap();
    let size: libc::c_int = 4096;
    // SAFETY: The socket is live and the pointer/length describe one c_int.
    assert_eq!(
        unsafe {
            libc::setsockopt(
                socket.get_ref().stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                std::ptr::from_ref(&size).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        },
        0
    );
    let budget = Budget::new(Duration::from_secs(1), None);
    socket.get_mut().set_budget(&budget);
    let started = Instant::now();
    let error = socket
        .send(Message::Binary(vec![b'x'; 16 * 1024 * 1024].into()))
        .unwrap_err();
    assert!(matches!(error, tungstenite::Error::Io(ref error) if pending(error)));
    assert_eq!(budget.check().unwrap_err().code, "RENDER_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(2));
}
