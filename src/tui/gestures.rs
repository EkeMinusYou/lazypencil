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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy)]
struct Pointer {
    position: Position,
    screen: [f64; 2],
    host: i32,
}
#[derive(Default)]
pub(super) struct Shared {
    pointer: Mutex<Option<Pointer>>,
    pending: Mutex<Option<PinchEvent>>,
    wake: Notify,
}
impl Shared {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn receive(&self, delta: f64, screen: [f64; 2], host: i32, finished: bool) -> bool {
        if !delta.is_finite() || !(-0.99..=10.0).contains(&delta) {
            return false;
        }
        let Ok(pointer) = self.pointer.lock() else {
            return false;
        };
        let Some(pointer) = *pointer else {
            return false;
        };
        // Only claim a gesture at a position reported by this TTY, in the same
        // foreground host. Other panes and applications retain their gestures.
        if host <= 0
            || host != pointer.host
            || screen
                .iter()
                .zip(pointer.screen)
                .any(|(a, b)| !a.is_finite() || (a - b).abs() > 4.0)
        {
            return false;
        }
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let factor = 1.0 + delta;
        let previous = pending.take();
        *pending = Some(PinchEvent {
            factor: previous.map_or(factor, |previous| {
                (previous.factor * factor).clamp(0.000001, 1_000_000.0)
            }),
            position: pointer.position,
            finished,
        });
        self.wake.notify_one();
        true
    }
    pub async fn next(&self) -> PinchEvent {
        loop {
            let notified = self.wake.notified();
            if let Some(event) = self.pending.lock().unwrap().take() {
                return event;
            }
            notified.await;
        }
    }
}

pub(super) struct Pinch {
    pub shared: Arc<Shared>,
    #[cfg(target_os = "macos")]
    handle: *mut std::ffi::c_void,
    #[cfg(target_os = "macos")]
    context: *const Shared,
}
impl Default for Pinch {
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
impl Pinch {
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
    pub fn observe_pointer(&self, position: Position, canvas: bool) {
        if !self.active() {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let mut screen = [0.0; 2];
            let mut host = 0;
            if canvas {
                unsafe {
                    lp_pinch_pointer(&mut screen[0], &mut screen[1], &mut host);
                }
            }
            *self.shared.pointer.lock().unwrap() = (canvas && host > 0).then_some(Pointer {
                position,
                screen,
                host,
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (position, canvas);
        }
    }
    pub fn cancel(&self) {
        *self.shared.pointer.lock().unwrap() = None;
        *self.shared.pending.lock().unwrap() = None;
    }
}
impl Drop for Pinch {
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
        callback: extern "C" fn(*mut std::ffi::c_void, f64, f64, f64, i32, bool) -> bool,
        context: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn lp_pinch_stop(handle: *mut std::ffi::c_void);
    fn lp_pinch_pointer(x: *mut f64, y: *mut f64, host: *mut i32);
}
#[cfg(target_os = "macos")]
extern "C" fn receive(
    context: *mut std::ffi::c_void,
    delta: f64,
    x: f64,
    y: f64,
    host: i32,
    finished: bool,
) -> bool {
    // The native source is joined before this strong Arc reference is released.
    unsafe { &*context.cast::<Shared>() }.receive(delta, [x, y], host, finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinch_is_scoped_to_its_tty_and_coalesces_fractional_scale() {
        let source = Shared::default();
        assert!(!source.receive(0.1, [100.0, 100.0], 42, false));
        *source.pointer.lock().unwrap() = Some(Pointer {
            position: Position::new(20, 10),
            screen: [100.0, 100.0],
            host: 42,
        });
        assert!(!source.receive(0.1, [100.0, 100.0], 43, false));
        assert!(!source.receive(0.1, [200.0, 100.0], 42, false));
        assert!(!source.receive(f64::NAN, [100.0, 100.0], 42, false));
        assert!(source.receive(0.01, [100.0, 100.0], 42, false));
        assert!(source.receive(0.02, [100.0, 100.0], 42, false));
        assert!(source.receive(0.0, [100.0, 100.0], 42, true));
        let event = source.pending.lock().unwrap().take().unwrap();
        assert!((event.factor - 1.01 * 1.02).abs() < 1e-12);
        assert_eq!(event.position, Position::new(20, 10));
        assert!(event.finished);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod native_tests {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventSetType(event: *mut c_void, kind: u32);
        fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
        fn CGEventSetDoubleValueField(event: *mut c_void, field: u32, value: f64);
        fn CFRelease(object: *mut c_void);
        fn lp_pinch_decode(event: *mut c_void, delta: *mut f64, ended: *mut bool) -> bool;
    }
    #[test]
    fn native_decoder_reads_magnification_and_phase_without_posting_input() {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            assert!(!event.is_null());
            // Construct a gesture fixture in memory; it is never posted to the desktop.
            // Gesture subtype/phase fields match native trackpad packets.
            CGEventSetType(event, 29);
            CGEventSetIntegerValueField(event, 110, 8);
            CGEventSetDoubleValueField(event, 113, 0.125);
            CGEventSetIntegerValueField(event, 132, 8);
            let mut delta = 0.0;
            let mut ended = false;
            let decoded = lp_pinch_decode(event, &mut delta, &mut ended);
            CFRelease(event);
            assert!(decoded);
            assert!((delta - 0.125).abs() < 1e-6);
            assert!(ended);
        }
    }
}
