use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct WaitingProcess {
    child: Option<Child>,
    reader: Option<JoinHandle<Vec<u8>>>,
}

impl WaitingProcess {
    pub fn spawn(command: &mut Command) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            for line in BufReader::new(stderr).split(b'\n') {
                let Ok(line) = line else { break };
                if line.starts_with(b"Waiting for file lock:") {
                    let _ = sender.send(());
                }
                output.extend(line);
                output.push(b'\n');
            }
            output
        });
        let mut process = Self {
            child: Some(child),
            reader: Some(reader),
        };
        if receiver.recv_timeout(Duration::from_secs(10)).is_err() {
            process.child.as_mut().unwrap().kill().ok();
            let output = process.wait_with_output();
            panic!(
                "Process never reported a lock wait: status={}, stdout={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        process
    }
    #[cfg(unix)]
    pub fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }
    pub fn wait_with_output(mut self) -> Output {
        let started = Instant::now();
        let mut timed_out = false;
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(10) {
                self.child.as_mut().unwrap().kill().ok();
                timed_out = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut output = self.child.take().unwrap().wait_with_output().unwrap();
        output.stderr = self.reader.take().unwrap().join().unwrap();
        assert!(
            !timed_out,
            "Process did not finish after a lock wait: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}
impl Drop for WaitingProcess {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            child.kill().ok();
            child.wait().ok();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().ok();
        }
    }
}
