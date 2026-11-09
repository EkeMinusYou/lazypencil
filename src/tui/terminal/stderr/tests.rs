use super::*;
use std::time::Instant;

#[test]
fn native_writes_stay_off_the_terminal_until_capture_is_dropped() {
    let (mut master, mut slave) = (-1, -1);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut master = unsafe { File::from_raw_fd(master) };
    let mut slave = unsafe { File::from_raw_fd(slave) };
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
        0
    );
    let capture = Capture::redirect(slave.as_raw_fd()).unwrap();
    slave.write_all(b"native diagnostic\n").unwrap();
    let mut buffer = [0; 1024];
    assert_eq!(
        master.read(&mut buffer).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(capture);
    slave.write_all(b"restored descriptor\n").unwrap();
    let mut output = Vec::new();
    let end = Instant::now() + Duration::from_secs(2);
    while !output.ends_with(b"restored descriptor\r\n") {
        match master.read(&mut buffer) {
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < end, "Missing restored terminal output");
                thread::sleep(Duration::from_millis(5));
            }
            result => panic!("Unexpected terminal read: {result:?}"),
        }
    }
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Diagnostics captured while the TUI was active"));
    assert!(output.contains("native diagnostic"));
}

#[test]
fn diagnostic_retention_is_bounded_and_shutdown_does_not_wait_for_inherited_writers() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let reader = thread::spawn(move || drain(reader, stopping));
    writer.write_all(&vec![b'x'; RETAINED_BYTES * 4]).unwrap();
    writer.write_all(b"last native diagnostic\n").unwrap();
    stop.store(true, Ordering::Release);
    let end = Instant::now() + Duration::from_secs(2);
    while !reader.is_finished() {
        assert!(Instant::now() < end, "Waited for an inherited writer's EOF");
        thread::sleep(Duration::from_millis(5));
    }
    let tail = reader.join().unwrap();
    assert_eq!(tail.len(), RETAINED_BYTES);
    let tail = tail.into_iter().collect::<Vec<_>>();
    assert!(tail.ends_with(b"last native diagnostic\n"));
    drop(writer);
}
