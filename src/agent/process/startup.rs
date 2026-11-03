//! Private adapter startup framing and the gated stderr relay shared by hosts.
use crate::{Error, Result};
use std::{
    io::{self, Read, Write},
    process::ChildStderr,
    sync::mpsc,
    thread::JoinHandle,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Child};

const STATUS_BYTES: usize = 13;

pub(super) struct Protocol {
    magic: &'static [u8; 8],
    phases: &'static [&'static str],
    invalid_success: &'static str,
}

impl Protocol {
    pub(super) const fn new(
        magic: &'static [u8; 8],
        phases: &'static [&'static str],
        invalid_success: &'static str,
    ) -> Self {
        Self {
            magic,
            phases,
            invalid_success,
        }
    }

    pub(super) async fn confirm(
        &self,
        child: &mut Child,
        limit: Duration,
        failure: fn(&str, io::Error) -> Error,
    ) -> Result<()> {
        let mut frame = [0u8; STATUS_BYTES];
        tokio::time::timeout(
            limit,
            child
                .stderr
                .as_mut()
                .expect("piped stderr")
                .read_exact(&mut frame),
        )
        .await
        .map_err(|_| Error::new("AGENT_ERROR", "Agent initialization timed out"))?
        .map_err(|error| failure("supervisor_status", error))?;
        self.decode(&frame, failure)
    }

    fn decode(
        &self,
        frame: &[u8; STATUS_BYTES],
        failure: fn(&str, io::Error) -> Error,
    ) -> Result<()> {
        if &frame[..8] != self.magic {
            return Err(Error::new(
                "AGENT_UNAVAILABLE",
                "Invalid adapter supervisor startup status",
            ));
        }
        let code = i32::from_le_bytes(frame[9..].try_into().expect("fixed status size"));
        if frame[8] != 0 {
            let phase = self
                .phases
                .get(frame[8] as usize - 1)
                .copied()
                .unwrap_or("supervisor_startup");
            return Err(failure(
                phase,
                if code == 0 {
                    io::Error::other("Supervisor startup failed")
                } else {
                    io::Error::from_raw_os_error(code)
                },
            ));
        }
        if code != 0 {
            return Err(Error::new("AGENT_UNAVAILABLE", self.invalid_success));
        }
        Ok(())
    }

    fn frame(&self, phase: u8, code: i32) -> [u8; STATUS_BYTES] {
        let mut frame = [0u8; STATUS_BYTES];
        frame[..8].copy_from_slice(self.magic);
        frame[8] = phase;
        frame[9..].copy_from_slice(&code.to_le_bytes());
        frame
    }

    pub(super) fn write(&self, phase: u8, code: i32) -> io::Result<()> {
        let mut output = io::stderr().lock();
        output.write_all(&self.frame(phase, code))?;
        output.flush()
    }

    pub(super) fn report_error(&self, error: &Error) {
        let details = error.details.as_ref();
        let phase = details
            .and_then(|value| value["supervisor_phase"].as_str())
            .and_then(|name| self.phases.iter().position(|phase| *phase == name))
            .map_or(1, |index| index as u8 + 1);
        let code = details
            .and_then(|value| value["os_error"].as_i64())
            .unwrap_or(0) as i32;
        let _ = self.write(phase, code);
    }
}

/// The private status frame must reach the host before any adapter stderr.
pub(super) fn relay(mut stderr: ChildStderr) -> io::Result<(mpsc::Sender<()>, JoinHandle<()>)> {
    let (release, gate) = mpsc::channel();
    let relay = std::thread::Builder::new()
        .name("adapter-stderr".into())
        .spawn(move || {
            if gate.recv().is_ok() {
                let mut buffer = [0u8; 8192];
                let mut output = io::stderr();
                while let Ok(size) = stderr.read(&mut buffer) {
                    if size == 0 || output.write_all(&buffer[..size]).is_err() {
                        break;
                    }
                }
            }
        })?;
    Ok((release, relay))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(phase: &str, error: io::Error) -> Error {
        Error::new("AGENT_UNAVAILABLE", phase)
            .details(serde_json::json!({"os_error":error.raw_os_error()}))
    }

    #[test]
    fn startup_frames_reject_other_protocols_and_nonzero_success_codes() {
        let unix = Protocol::new(
            b"LPAP\x02\0\r\n",
            &[
                "owner_start",
                "adapter_spawn",
                "stderr_relay",
                "child_wait_policy",
            ],
            "invalid success",
        );
        let windows = Protocol::new(
            b"LPAP\x01\0\r\n",
            &["job_join", "adapter_spawn", "stderr_relay"],
            "invalid success",
        );
        for protocol in [&unix, &windows] {
            protocol.decode(&protocol.frame(0, 0), failure).unwrap();
            assert!(protocol.decode(&protocol.frame(0, 1), failure).is_err());
            let error = protocol.decode(&protocol.frame(2, 5), failure).unwrap_err();
            assert_eq!(error.message, "adapter_spawn");
            assert_eq!(error.details.unwrap()["os_error"], 5);
            assert_eq!(
                protocol
                    .decode(&protocol.frame(255, 0), failure)
                    .unwrap_err()
                    .message,
                "supervisor_startup"
            );
        }
        assert!(unix.decode(&windows.frame(0, 0), failure).is_err());
        assert!(windows.decode(&unix.frame(0, 0), failure).is_err());
        assert_eq!(
            unix.decode(&unix.frame(4, 0), failure).unwrap_err().message,
            "child_wait_policy"
        );
        assert_eq!(
            windows
                .decode(&windows.frame(4, 0), failure)
                .unwrap_err()
                .message,
            "supervisor_startup"
        );
    }
}
