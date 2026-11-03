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
    deferred: Option<io::Result<Batch>>,
    pending: Option<io::Result<Batch>>,
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
                    let mut event = if let Some(event) = pending.take() { event } else { match crossterm::event::poll(Duration::from_millis(50)) {
                        Ok(false) => continue,
                        Ok(true) => crossterm::event::read(),
                        Err(error) => Err(error),
                    }};
                    // Native gestures use hover to identify the owning TTY.
                    if matches!(&event, Ok(Event::Mouse(event)) if event.kind == MouseEventKind::Moved) {
                        for _ in 0..64 {
                            if !crossterm::event::poll(Duration::ZERO).unwrap_or(false) { break; }
                            let next = crossterm::event::read();
                            if matches!(&next, Ok(Event::Mouse(event)) if event.kind == MouseEventKind::Moved) { event = next; }
                            else { pending = Some(next); break; }
                        }
                    }
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
            deferred: None,
            pending: None,
            stop,
            thread: Some(thread),
        })
    }

    pub fn defer(&mut self, event: io::Result<Batch>) {
        debug_assert!(self.deferred.is_none());
        self.deferred = Some(event);
    }
    fn pop_ready(&mut self) -> Option<io::Result<Batch>> {
        self.deferred
            .take()
            .or_else(|| self.pending.take())
            .or_else(|| self.receiver.try_recv().ok())
    }

    pub fn try_next(&mut self) -> Option<io::Result<Batch>> {
        let event = self.pop_ready()?;
        Some(self.coalesce(event))
    }

    pub async fn next(&mut self) -> Option<io::Result<Batch>> {
        let event = if let Some(event) = self.pop_ready() {
            event
        } else {
            self.receiver.recv().await?
        };
        Some(self.coalesce(event))
    }

    fn coalesce(&mut self, event: io::Result<Batch>) -> io::Result<Batch> {
        let mut batch = event?;
        if !scroll(&batch.event) {
            return Ok(batch);
        }
        // The input thread may have received each event separately while a
        // fallback image was drawn. Merge the now-ready burst before repeating
        // expensive camera feedback, retaining every nonmatching boundary.
        for _ in 1..64 {
            let Some(next) = self.pop_ready() else {
                break;
            };
            match next {
                Ok(next) if next.event == batch.event => {
                    if let Some(count) = batch.count.checked_add(next.count) {
                        batch.count = count;
                    } else {
                        self.pending = Some(Ok(next));
                        break;
                    }
                }
                boundary => {
                    self.pending = Some(boundary);
                    break;
                }
            }
        }
        Ok(batch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

    fn pointer(kind: MouseEventKind, column: u16, modifiers: KeyModifiers) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row: 9,
            modifiers,
        })
    }

    fn right() -> Event {
        pointer(MouseEventKind::ScrollRight, 48, KeyModifiers::NONE)
    }

    fn queued(events: impl IntoIterator<Item = io::Result<Batch>>) -> Events {
        let (sender, receiver) = mpsc::channel(512);
        for event in events {
            assert!(sender.try_send(event).is_ok());
        }
        Events {
            receiver,
            deferred: None,
            pending: None,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    fn batch(event: Event, count: u16) -> io::Result<Batch> {
        Ok(Batch { event, count })
    }

    #[test]
    fn ready_trackpad_bursts_keep_every_step_before_following_text() {
        let text = Event::Paste(" 続き".into());
        let mut events = queued(
            (0..256)
                .map(|_| batch(right(), 1))
                .chain([batch(text.clone(), 1)]),
        );
        let mut steps = 0;
        let mut redraws = 0;
        loop {
            let next = events.try_next().unwrap().unwrap();
            if next.event == text {
                break;
            }
            assert_eq!(next.event, right());
            steps += next.count;
            redraws += 1;
        }
        assert_eq!(steps, 256);
        assert_eq!(redraws, 4);
        assert!(events.try_next().is_none());
    }

    #[tokio::test]
    async fn async_input_keeps_direction_position_modifiers_and_key_boundaries() {
        let expected = [
            (right(), 5),
            (
                pointer(MouseEventKind::ScrollLeft, 48, KeyModifiers::NONE),
                1,
            ),
            (
                pointer(MouseEventKind::ScrollRight, 49, KeyModifiers::NONE),
                1,
            ),
            (
                pointer(MouseEventKind::ScrollRight, 48, KeyModifiers::SHIFT),
                1,
            ),
            (
                Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
                1,
            ),
            (right(), 7),
        ];
        let mut events = queued([
            batch(right(), 2),
            batch(right(), 3),
            batch(expected[1].0.clone(), 1),
            batch(expected[2].0.clone(), 1),
            batch(expected[3].0.clone(), 1),
            batch(expected[4].0.clone(), 1),
            batch(right(), 7),
        ]);
        for (event, count) in expected {
            let actual = events.next().await.unwrap().unwrap();
            assert_eq!(actual.event, event);
            assert_eq!(actual.count, count);
        }
        assert!(events.next().await.is_none());
    }

    #[test]
    fn count_overflow_and_read_errors_preserve_the_following_events() {
        let mut events = queued([
            batch(right(), u16::MAX),
            batch(right(), 1),
            Err(io::Error::from(io::ErrorKind::BrokenPipe)),
            batch(right(), 2),
        ]);
        assert_eq!(events.try_next().unwrap().unwrap().count, u16::MAX);
        assert_eq!(events.try_next().unwrap().unwrap().count, 1);
        assert!(
            matches!(events.try_next(), Some(Err(error)) if error.kind() == io::ErrorKind::BrokenPipe)
        );
        assert_eq!(events.try_next().unwrap().unwrap().count, 2);
        assert!(events.try_next().is_none());
    }

    #[tokio::test]
    async fn deferred_hit_geometry_replays_before_hover_and_paste_boundaries() {
        let hover = pointer(MouseEventKind::Moved, 49, KeyModifiers::NONE);
        let text = Event::Paste(" 続き".into());
        let mut events = queued([
            batch(right(), 1),
            batch(right(), 2),
            batch(hover.clone(), 1),
            batch(text.clone(), 1),
            batch(right(), 4),
        ]);
        let first = events.try_next().unwrap();
        assert_eq!(first.as_ref().unwrap().count, 3);
        events.defer(first);
        let replay = events.next().await.unwrap().unwrap();
        assert_eq!(replay.event, right());
        assert_eq!(replay.count, 3);
        assert_eq!(events.try_next().unwrap().unwrap().event, hover);
        assert_eq!(events.next().await.unwrap().unwrap().event, text);
        assert_eq!(events.try_next().unwrap().unwrap().count, 4);
        assert!(events.next().await.is_none());
    }
}
