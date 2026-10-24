//! Bound cleanup waits while retaining standard no-follow filesystem removal.

use super::deletion::{Budget, finish_trash};
use crate::{Error, Result, conversation::storage::RegistryGuard};
use std::{
    path::PathBuf,
    sync::{Mutex, mpsc},
    time::Duration,
};

struct Job {
    path: PathBuf,
    lease: RegistryGuard,
    reply: mpsc::Sender<Result<()>>,
}

// One running job and one queued job bound threads and retained leases. The
// worker exits after five idle seconds. Exiting the process leaves recoverable
// journals, never an independent helper process.
static WORKER: Mutex<Option<mpsc::SyncSender<Job>>> = Mutex::new(None);

fn submit(mut job: Job) -> Result<()> {
    let mut sender = WORKER
        .lock()
        .map_err(|_| Error::new("IO_ERROR", "Conversation cleanup worker lock was poisoned"))?;
    for _ in 0..2 {
        if sender.is_none() {
            let (queue, receiver) = mpsc::sync_channel::<Job>(1);
            std::thread::Builder::new()
                .name("lazypencil-conversation-cleanup".into())
                .spawn(move || {
                    while let Ok(Job { path, lease, reply }) =
                        receiver.recv_timeout(Duration::from_secs(5))
                    {
                        let result = finish_trash(&path).map_err(|error| Error::io(&path, error));
                        drop(lease);
                        let _ = reply.send(result);
                    }
                })
                .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
            *sender = Some(queue);
        }
        match sender
            .as_ref()
            .expect("started cleanup worker")
            .try_send(job)
        {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(_job)) => {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Conversation cleanup queue is full; retry conversations cleanup",
                ));
            }
            Err(mpsc::TrySendError::Disconnected(returned)) => {
                job = returned;
                *sender = None;
            }
        }
    }
    Err(Error::new(
        "IO_ERROR",
        "Conversation cleanup worker is unavailable",
    ))
}

fn wait(receiver: mpsc::Receiver<Result<()>>, budget: &Budget<'_>) -> Result<()> {
    loop {
        budget.check()?;
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::new(
                    "IO_ERROR",
                    "Conversation cleanup stopped; retry cleanup from its journal",
                ));
            }
        }
    }
}

pub(super) fn finish(path: PathBuf, lease: RegistryGuard, budget: &Budget<'_>) -> Result<()> {
    let (reply, receiver) = mpsc::channel();
    submit(Job { path, lease, reply })?;
    wait(receiver, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn cleanup_wait_observes_cancellation_and_deadline_without_waiting_for_filesystem_work() {
        let (_sender, receiver) = mpsc::channel();
        let token = CancellationToken::new();
        token.cancel();
        let started = Instant::now();
        assert_eq!(
            wait(
                receiver,
                &Budget {
                    started,
                    cancel: Some(&token)
                }
            )
            .unwrap_err()
            .code,
            "CANCELLED"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        let (_sender, receiver) = mpsc::channel();
        assert_eq!(
            wait(
                receiver,
                &Budget {
                    started: Instant::now() - Duration::from_secs(31),
                    cancel: None
                }
            )
            .unwrap_err()
            .code,
            "AGENT_ERROR"
        );
    }
}
