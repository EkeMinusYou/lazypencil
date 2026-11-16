//! An owned terminal session, including its bounded shutdown and child reaping.
use std::{
    io,
    os::{fd::RawFd, unix::process::CommandExt},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub struct Session {
    child: Child,
}

impl Session {
    /// Start a new session, optionally acquiring an already configured PTY.
    pub fn spawn(command: &mut Command, controlling_terminal: Option<RawFd>) -> io::Result<Self> {
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if let Some(fd) = controlling_terminal
                    && libc::ioctl(fd, libc::TIOCSCTTY as _, 0) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(Self {
            child: command.spawn()?,
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Once reaped, the PID may belong to a different process. Never signal it.
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let group = -(self.child.id() as i32);
        // Only Session::spawn can construct this guard, so the group is owned.
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
