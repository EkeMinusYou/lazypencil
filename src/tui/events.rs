//! Keep terminal input moving independently of synchronous image output.

use crossterm::event::Event;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::mpsc;

pub(super) struct Events {
    receiver: mpsc::Receiver<io::Result<Event>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Events {
    pub fn new() -> crate::Result<Self> {
        let (sender, receiver) = mpsc::channel(128);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("lazypencil-input".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let event = match crossterm::event::poll(Duration::from_millis(50)) {
                        Ok(false) => continue,
                        Ok(true) => crossterm::event::read(),
                        Err(error) => Err(error),
                    };
                    let failed = event.is_err();
                    if sender.blocking_send(event).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(super::terminal::io)?;
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
        })
    }

    pub fn try_next(&mut self) -> Option<io::Result<Event>> {
        self.receiver.try_recv().ok()
    }

    pub async fn next(&mut self) -> Option<io::Result<Event>> {
        self.receiver.recv().await
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        // Closing also releases a producer waiting on a full bounded queue.
        self.receiver.close();
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
