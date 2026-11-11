//! OS gestures supplement terminal events without depending on a terminal brand.
use ratatui::layout::Position;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug)]
pub(super) struct PinchEvent {
    pub factor: f64,
    pub position: Position,
    pub finished: bool,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct ScrollEvent {
    pub pixels: [f64; 2],
    pub position: Position,
    pub modifiers: crossterm::event::KeyModifiers,
    pub finished: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum GestureEvent {
    Pinch(PinchEvent),
    Scroll(ScrollEvent),
}

impl GestureEvent {
    fn merge(&mut self, next: Self) -> bool {
        match (self, next) {
            (Self::Pinch(previous), Self::Pinch(next))
                if !previous.finished && previous.position == next.position =>
            {
                previous.factor = (previous.factor * next.factor).clamp(0.000001, 1_000_000.0);
                previous.finished = next.finished;
            }
            (Self::Scroll(previous), Self::Scroll(next))
                if !previous.finished
                    && previous.position == next.position
                    && previous.modifiers == next.modifiers =>
            {
                let pixels = std::array::from_fn(|axis| previous.pixels[axis] + next.pixels[axis]);
                if pixels.iter().any(|value| !value.is_finite()) {
                    return false;
                }
                previous.pixels = pixels;
                previous.finished = next.finished;
            }
            _ => return false,
        }
        true
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy)]
struct Pointer {
    position: Position,
    screen: [f64; 2],
    host: i32,
    canvas: ratatui::layout::Rect,
    cell: [f64; 2],
    scale: f64,
    pinch: bool,
    scroll: bool,
}

impl Pointer {
    fn locate(self, screen: [f64; 2], host: i32) -> Option<Position> {
        if host <= 0 || host != self.host || !self.canvas.contains(self.position) {
            return None;
        }
        let mut position = [self.position.x, self.position.y];
        let bounds = [
            (self.canvas.left(), self.canvas.right()),
            (self.canvas.top(), self.canvas.bottom()),
        ];
        for axis in 0..2 {
            if !screen[axis].is_finite()
                || !self.screen[axis].is_finite()
                || !self.cell[axis].is_finite()
                || self.cell[axis] <= 0.0
            {
                return None;
            }
            let delta = (screen[axis] - self.screen[axis]) / self.cell[axis];
            let (start, end) = bounds[axis];
            // The terminal reports a cell, not the pointer's offset within it.
            // Use the intersection of all possible canvas bounds, so a stale
            // cell report never expands ownership into a neighboring pane.
            if !delta.is_finite()
                || delta < f64::from(start) - f64::from(position[axis])
                || delta > f64::from(end - 1) - f64::from(position[axis])
            {
                return None;
            }
            position[axis] = (f64::from(position[axis]) + delta).round() as u16;
        }
        Some(Position::new(position[0], position[1]))
    }
}

struct Calibration {
    pointer: Option<Pointer>,
    focused: bool,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            pointer: None,
            // Terminals without focus reporting can still use native input.
            focused: true,
        }
    }
}

#[derive(Default)]
pub(super) struct Shared {
    calibration: Mutex<Calibration>,
    pending: Mutex<std::collections::VecDeque<GestureEvent>>,
    wake: Notify,
}
impl Shared {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn observe_pointer(&self, pointer: Option<Pointer>) {
        let mut calibration = self.calibration.lock().unwrap();
        if calibration.focused {
            calibration.pointer = pointer;
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn receive(
        &self,
        kind: u32,
        delta: [f64; 2],
        screen: [f64; 2],
        host: i32,
        modifiers: u32,
        finished: bool,
    ) -> bool {
        if delta.iter().any(|value| !value.is_finite()) {
            return false;
        }
        let Ok(calibration) = self.calibration.lock() else {
            return false;
        };
        let Some(pointer) = calibration.pointer.filter(|_| calibration.focused) else {
            return false;
        };
        let Some(position) = pointer.locate(screen, host) else {
            return false;
        };
        let event = match kind {
            0 if pointer.pinch && (-0.99..=10.0).contains(&delta[0]) => {
                GestureEvent::Pinch(PinchEvent {
                    factor: 1.0 + delta[0],
                    position,
                    finished,
                })
            }
            1 if pointer.scroll => {
                let pixels = delta.map(|value| -value * pointer.scale);
                if pixels.iter().any(|value| !value.is_finite()) {
                    return false;
                }
                GestureEvent::Scroll(ScrollEvent {
                    pixels,
                    position,
                    finished,
                    modifiers: crossterm::event::KeyModifiers::from_bits_truncate(modifiers as u8),
                })
            }
            _ => return false,
        };
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        if !pending
            .back_mut()
            .is_some_and(|previous| previous.merge(event))
        {
            // Declining a packet leaves it on the terminal's normal input path.
            if pending.len() >= 128 {
                return false;
            }
            pending.push_back(event);
        }
        self.wake.notify_one();
        true
    }
    pub async fn next(&self) -> GestureEvent {
        loop {
            let notified = self.wake.notified();
            if let Some(event) = self.pending.lock().unwrap().pop_front() {
                return event;
            }
            notified.await;
        }
    }
}

pub(super) struct Input {
    pub shared: Arc<Shared>,
    #[cfg(target_os = "macos")]
    handle: *mut std::ffi::c_void,
    #[cfg(target_os = "macos")]
    context: *const Shared,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            shared: Arc::default(),
            #[cfg(target_os = "macos")]
            handle: std::ptr::null_mut(),
            #[cfg(target_os = "macos")]
            context: std::ptr::null(),
        }
    }
}
impl Input {
    pub fn start(&mut self, request: bool) -> bool {
        #[cfg(target_os = "macos")]
        {
            if self.active() {
                return true;
            }
            if std::env::var_os("SSH_CONNECTION").is_some()
                || std::env::var_os("LAZYPENCIL_DISABLE_NATIVE_INPUT").is_some()
            {
                return false;
            }
            // The OS permission is requested only by the explicit palette action.
            if !unsafe { lp_pinch_authorized(request) } {
                return false;
            }
            let context = Arc::into_raw(self.shared.clone());
            let handle = unsafe { lp_pinch_start(receive, context.cast_mut().cast()) };
            if handle.is_null() {
                unsafe {
                    drop(Arc::from_raw(context));
                }
                return false;
            }
            self.handle = handle;
            self.context = context;
            true
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = request;
            false
        }
    }
    pub fn authorized(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            unsafe { lp_pinch_authorized(false) }
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
    pub fn active(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            !self.handle.is_null()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
    pub fn observe_pointer(
        &self,
        position: Position,
        canvas: Option<ratatui::layout::Rect>,
        font: ratatui_image::FontSize,
        settings: &crate::config::keys::MouseSettings,
    ) {
        if !self.active() {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let pointer = canvas.and_then(|canvas| {
                let mut screen = [0.0; 2];
                let mut host = 0;
                let mut scale = 1.0;
                unsafe {
                    lp_pinch_pointer(&mut screen[0], &mut screen[1], &mut host, &mut scale);
                }
                (host > 0 && scale.is_finite() && scale > 0.0).then_some(Pointer {
                    position,
                    screen,
                    host,
                    canvas,
                    scale,
                    cell: [
                        f64::from(font.width) / scale,
                        f64::from(font.height) / scale,
                    ],
                    pinch: settings.native_pinch,
                    scroll: settings.native_scroll,
                })
            });
            self.shared.observe_pointer(pointer);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (position, canvas, font, settings);
        }
    }

    pub fn cancel(&self) {
        self.shared.calibration.lock().unwrap().pointer = None;
        self.shared.pending.lock().unwrap().clear();
    }

    pub fn set_focused(&self, focused: bool) {
        let mut calibration = self.shared.calibration.lock().unwrap();
        calibration.focused = focused;
        calibration.pointer = None;
        self.shared.pending.lock().unwrap().clear();
    }

    pub fn retain_canvas(&self, canvas: Option<ratatui::layout::Rect>) {
        let mut calibration = self.shared.calibration.lock().unwrap();
        if calibration
            .pointer
            .is_some_and(|pointer| Some(pointer.canvas) != canvas)
        {
            calibration.pointer = None;
            self.shared.pending.lock().unwrap().clear();
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if !self.handle.is_null() {
            unsafe {
                lp_pinch_stop(self.handle);
                drop(Arc::from_raw(self.context));
            }
        }
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn lp_pinch_authorized(request: bool) -> bool;
    fn lp_pinch_start(
        callback: extern "C" fn(
            *mut std::ffi::c_void,
            u32,
            f64,
            f64,
            f64,
            f64,
            i32,
            u32,
            bool,
        ) -> bool,
        context: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn lp_pinch_stop(handle: *mut std::ffi::c_void);
    fn lp_pinch_pointer(x: *mut f64, y: *mut f64, host: *mut i32, scale: *mut f64);
}
#[cfg(target_os = "macos")]
extern "C" fn receive(
    context: *mut std::ffi::c_void,
    kind: u32,
    dx: f64,
    dy: f64,
    x: f64,
    y: f64,
    host: i32,
    modifiers: u32,
    finished: bool,
) -> bool {
    // The native source is joined before this strong Arc reference is released.
    unsafe { &*context.cast::<Shared>() }.receive(kind, [dx, dy], [x, y], host, modifiers, finished)
}

#[cfg(test)]
mod tests;
