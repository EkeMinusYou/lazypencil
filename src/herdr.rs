//! Optional, best-effort lifecycle reports from the foreground CLI or TUI.

use std::{
    ffi::OsString,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Idle,
    Working,
    Blocked,
}

impl State {
    fn argument(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
        }
    }
}

struct Target {
    binary: PathBuf,
    pane: OsString,
}

impl Target {
    fn from_environment(mut get: impl FnMut(&str) -> Option<OsString>) -> Option<Self> {
        if get("HERDR_ENV")?.as_os_str() != "1" {
            return None;
        }
        let pane = get("HERDR_PANE_ID").filter(|value| !value.is_empty())?;
        let binary = PathBuf::from(get("HERDR_BIN_PATH")?);
        binary.is_absolute().then_some(Self { binary, pane })
    }

    fn arguments(&self, state: Option<State>, sequence: u64) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "pane",
            if state.is_some() {
                "report-agent"
            } else {
                "release-agent"
            },
        ]
        .map(OsString::from)
        .into();
        args.push(self.pane.clone());
        args.extend(["--source", "lazypencil", "--agent", "lazypencil"].map(OsString::from));
        if let Some(state) = state {
            args.extend(["--state", state.argument()].map(OsString::from));
            if state == State::Blocked {
                args.extend(["--message", "Waiting for permission"].map(OsString::from));
            }
        }
        args.extend([OsString::from("--seq"), sequence.to_string().into()]);
        args
    }

    fn report(&self, state: Option<State>, sequence: u64) {
        let Ok(mut child) = Command::new(&self.binary)
            .args(self.arguments(state, sequence))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return;
        };
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
            }
        }
    }
}

#[derive(Default)]
struct Mailbox {
    pending: Option<State>,
    closing: bool,
}

type Shared = Arc<(Mutex<Mailbox>, Condvar)>;

pub(crate) struct Reporter {
    shared: Shared,
    worker: Option<JoinHandle<()>>,
    last: State,
}

impl Reporter {
    pub(crate) fn new(state: State) -> Self {
        let target = Target::from_environment(|key| std::env::var_os(key));
        Self::start(state, target)
    }

    fn start(state: State, target: Option<Target>) -> Self {
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker = target.and_then(|target| {
            let shared = shared.clone();
            thread::Builder::new()
                .name("lazypencil-herdr".into())
                .spawn(move || run(shared, |state, seq| target.report(state, seq)))
                .ok()
        });
        let reporter = Self {
            shared,
            worker,
            last: state,
        };
        reporter.publish(state);
        reporter
    }

    pub(crate) fn update(&mut self, state: State) {
        if self.last != state {
            self.last = state;
            self.publish(state);
        }
    }

    fn publish(&self, state: State) {
        if self.worker.is_some() {
            let (mailbox, wake) = &*self.shared;
            mailbox.lock().unwrap().pending = Some(state);
            wake.notify_one();
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let (mailbox, wake) = &*self.shared;
            mailbox.lock().unwrap().closing = true;
            wake.notify_one();
            // Only shutdown waits: at most the in-flight report and the release timeout.
            let _ = worker.join();
        }
    }
}

fn run(shared: Shared, mut report: impl FnMut(Option<State>, u64)) {
    let mut sequence = 0;
    loop {
        let (mailbox, wake) = &*shared;
        let mut mailbox = mailbox.lock().unwrap();
        while mailbox.pending.is_none() && !mailbox.closing {
            mailbox = wake.wait(mailbox).unwrap();
        }
        let closing = mailbox.closing;
        let state = if closing {
            None
        } else {
            mailbox.pending.take()
        };
        drop(mailbox);
        // Timestamps keep the stable source ordered across process restarts, too.
        sequence = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64)
            .max(sequence + 1);
        report(state, sequence);
        if closing {
            return;
        }
    }
}

#[cfg(test)]
#[path = "herdr/tests.rs"]
mod tests;
