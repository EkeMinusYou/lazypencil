//! A paused-by-default timeline with no catch-up frame queue.

use crate::{Error, Result};
use std::time::{Duration, Instant};

pub const DEFAULT_FPS: u32 = 5;
pub const MAX_FPS: u32 = 10;

#[derive(Default)]
pub struct Playback {
    running: Option<Running>,
}

struct Running {
    started: Instant,
    time: f64,
    fps: u32,
    period: Duration,
    next: Instant,
}

impl Playback {
    pub fn start(&mut self, time: f64, fps: u32, now: Instant) -> Result<()> {
        if !time.is_finite() || time < 0.0 || !(1..=MAX_FPS).contains(&fps) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Playback requires finite nonnegative shader time and an integer frame rate from 1 to 10",
            ));
        }
        let period = Duration::from_nanos(1_000_000_000_u64.div_ceil(u64::from(fps)));
        self.running = Some(Running {
            started: now,
            time,
            fps,
            period,
            next: now + period,
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running = None;
    }

    pub fn active(&self) -> bool {
        self.running.is_some()
    }

    pub fn fps(&self) -> Option<u32> {
        self.running.as_ref().map(|running| running.fps)
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.running.as_ref().map(|running| running.next)
    }

    /// Advancing the deadline before the caller checks renderer capacity drops
    /// missed frames instead of spinning or cancelling unfinished rendering.
    pub fn frame(&mut self, now: Instant) -> Option<f64> {
        let running = self.running.as_mut()?;
        if now < running.next {
            return None;
        }
        running.next = now + running.period;
        Some(running.time + now.saturating_duration_since(running.started).as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_uses_elapsed_time_and_skips_missed_frames_without_catch_up() {
        let now = Instant::now();
        let mut playback = Playback::default();
        assert!(!playback.active());
        assert_eq!(playback.deadline(), None);
        assert_eq!(playback.frame(now), None);
        playback.start(1.25, DEFAULT_FPS, now).unwrap();
        assert_eq!(playback.fps(), Some(5));
        assert_eq!(playback.deadline(), Some(now + Duration::from_millis(200)));
        assert_eq!(playback.frame(now + Duration::from_millis(199)), None);
        assert_eq!(playback.frame(now + Duration::from_millis(200)), Some(1.45));
        assert_eq!(playback.frame(now + Duration::from_secs(8)), Some(9.25));
        assert_eq!(playback.frame(now + Duration::from_secs(8)), None);
        assert_eq!(playback.deadline(), Some(now + Duration::from_millis(8200)));
        playback.stop();
        assert_eq!(playback.deadline(), None);
        assert_eq!(playback.frame(now + Duration::from_secs(100)), None);
    }

    #[test]
    fn invalid_changes_preserve_the_current_clock_and_restart_uses_the_requested_time() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.start(7.0, 10, now).unwrap();
        let deadline = playback.deadline();
        for (time, fps) in [
            (0.0, 0),
            (0.0, 11),
            (-1.0, 5),
            (f64::NAN, 5),
            (f64::INFINITY, 5),
        ] {
            assert_eq!(
                playback.start(time, fps, now).unwrap_err().code,
                "INVALID_ARGUMENT"
            );
            assert_eq!(playback.deadline(), deadline);
            assert_eq!(playback.fps(), Some(10));
        }
        playback.stop();
        playback
            .start(2.0, 1, now + Duration::from_secs(20))
            .unwrap();
        assert_eq!(playback.frame(now + Duration::from_secs(21)), Some(3.0));
    }
}
