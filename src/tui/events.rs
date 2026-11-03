//! Keep terminal input moving independently of synchronous image output.

use crossterm::event::{Event, MouseEventKind};
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
    receiver: mpsc::Receiver<io::Result<Batch>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub(super) struct Batch {
    pub event: Event,
    pub count: u16,
}

fn scroll(event: &Event) -> bool {
    matches!(event, Event::Mouse(event) if matches!(event.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight))
}

impl Events {
    pub fn new() -> crate::Result<Self> {
        let (sender, receiver) = mpsc::channel(128);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("lazypencil-input".into())
            .spawn(move || {
                let mut pending = None;
                while !stopping.load(Ordering::Acquire) {
                    let event = if let Some(event) = pending.take() { event } else { match crossterm::event::poll(Duration::from_millis(50)) {
                        Ok(false) => continue,
                        Ok(true) => crossterm::event::read(),
                        Err(error) => Err(error),
                    }};
                    // Hover has no application action and must not delay typing.
                    if matches!(&event, Ok(Event::Mouse(event)) if event.kind == MouseEventKind::Moved) { continue; }
                    let mut count = 1;
                    if event.as_ref().is_ok_and(scroll) {
                        // Preserve direction, position, modifiers and every input
                        // boundary. Never wait for another event to form a batch.
                        for _ in 1..64 {
                            if !crossterm::event::poll(Duration::ZERO).unwrap_or(false) { break; }
                            let next = crossterm::event::read();
                            if next.as_ref().ok() == event.as_ref().ok() && next.is_ok() { count += 1; }
                            else { pending = Some(next); break; }
                        }
                    }
                    let failed = event.is_err();
                    if sender.blocking_send(event.map(|event| Batch { event, count })).is_err() || failed {
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

    pub fn try_next(&mut self) -> Option<io::Result<Batch>> {
        self.receiver.try_recv().ok()
    }

    pub async fn next(&mut self) -> Option<io::Result<Batch>> {
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
