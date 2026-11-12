//! Best-effort Program Status Protocol reports from the foreground CLI or TUI.

use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Write},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Idle,
    Working,
    Blocked,
}

fn core_environment(mut get: impl FnMut(&str) -> Option<OsString>) -> bool {
    get("HC_TERMINAL_ID").is_some_and(|value| !value.is_empty())
}

fn open_terminal() -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Reports must never wait for terminal backpressure.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
        options.open("/dev/tty")
    }
    #[cfg(windows)]
    {
        options.open("CONOUT$")
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn report_terminal(
    mut terminal: impl Write,
    state: Option<State>,
    completed: bool,
) -> io::Result<()> {
    let label = match state {
        Some(State::Idle) if completed => "done",
        Some(State::Idle) => "idle",
        Some(State::Working) => "working",
        Some(State::Blocked) => "blocked",
        None => "clear",
    };
    let detail = if state == Some(State::Blocked) {
        ":kind=permission"
    } else {
        ""
    };
    terminal.write_all(format!("\x1b]7501;state={label}:app=lazypencil{detail}\x1b\\").as_bytes())
}

pub(crate) struct Reporter {
    terminal: Option<File>,
    last: State,
}

impl Reporter {
    pub(crate) fn new(state: State) -> Self {
        let terminal = core_environment(|key| std::env::var_os(key))
            .then(open_terminal)
            .and_then(Result::ok);
        Self::start(state, terminal)
    }

    fn start(state: State, terminal: Option<File>) -> Self {
        let mut reporter = Self {
            terminal,
            last: state,
        };
        reporter.publish(Some(state), false);
        reporter
    }

    pub(crate) fn update(&mut self, state: State) {
        if self.last != state {
            let completed = state == State::Idle && self.last != State::Idle;
            self.last = state;
            self.publish(Some(state), completed);
        }
    }

    fn publish(&mut self, state: Option<State>, completed: bool) {
        if let Some(terminal) = &mut self.terminal {
            // The controlling terminal bypasses redirected JSON output and the
            // TUI's stderr capture. Never write status bytes into either stream.
            let _ = report_terminal(terminal, state, completed);
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.publish(None, false);
    }
}

#[cfg(test)]
#[path = "herdr/tests.rs"]
mod tests;
