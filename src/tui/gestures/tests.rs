use super::*;
use crossterm::event::KeyModifiers;
use ratatui::layout::Rect;

fn source(scale: f64) -> Shared {
    let source = Shared::default();
    source.observe_pointer(Some(Pointer {
        position: Position::new(20, 10),
        screen: [100.0, 100.0],
        host: 42,
        canvas: Rect::new(10, 5, 20, 10),
        cell: [10.0 / scale, 20.0 / scale],
        scale,
        pinch: true,
        scroll: true,
    }));
    source
}

#[test]
fn pinch_survives_small_pointer_motion_and_coalesces_fractional_scale() {
    let source = source(1.0);
    assert!(source.receive(0, [0.01, 0.0], [106.0, 106.0], 42, 0, false));
    assert!(source.receive(0, [0.02, 0.0], [106.0, 106.0], 42, 0, false));
    assert!(source.receive(0, [0.0, 0.0], [106.0, 106.0], 42, 0, true));
    let pending = source.pending.lock().unwrap();
    assert_eq!(pending.len(), 1);
    let GestureEvent::Pinch(event) = pending[0] else {
        panic!("Expected pinch")
    };
    assert!((event.factor - 1.01 * 1.02).abs() < 1e-12);
    assert_eq!(event.position, Position::new(21, 10));
    assert!(event.finished);
}

#[test]
fn gestures_stay_in_their_canvas_and_foreground_host_at_both_display_scales() {
    for scale in [1.0, 2.0] {
        let source = source(scale);
        for host in [-1, 0, 43] {
            for kind in [0, 1] {
                assert!(!source.receive(kind, [0.1, 0.0], [100.0, 100.0], host, 0, false));
            }
        }
        for outside in [[-1.0, 100.0], [201.0, 100.0], [100.0, -1.0], [100.0, 201.0]] {
            assert!(!source.receive(0, [0.1, 0.0], outside, 42, 0, false));
            assert!(!source.receive(1, [0.125, 0.25], outside, 42, 0, false));
        }
        assert!(source.receive(0, [0.1, 0.0], [106.0, 106.0], 42, 0, false));
    }
}

#[test]
fn canvas_bounds_never_include_a_neighbor_for_any_offset_in_the_reported_cell() {
    for scale in [1.0, 2.0] {
        let pointer = source(scale).calibration.lock().unwrap().pointer.unwrap();
        for position in [
            Position::new(10, 5),
            Position::new(20, 10),
            Position::new(29, 14),
        ] {
            let pointer = Pointer {
                position,
                ..pointer
            };
            for fraction in [0.0, 0.01, 0.5, 0.99] {
                let origin = [
                    pointer.screen[0] - (f64::from(position.x) + fraction) * pointer.cell[0],
                    pointer.screen[1] - (f64::from(position.y) + fraction) * pointer.cell[1],
                ];
                for (axis, boundary) in [(0, 10.0), (0, 30.0), (1, 5.0), (1, 15.0)] {
                    let mut screen = pointer.screen;
                    let outside = boundary
                        + if boundary == 10.0 || boundary == 5.0 {
                            -0.01
                        } else {
                            0.01
                        };
                    screen[axis] = origin[axis] + outside * pointer.cell[axis];
                    assert!(pointer.locate(screen, 42).is_none());
                }
            }
        }
    }
}

#[test]
fn precise_scroll_keeps_fractional_diagonal_deltas_and_display_scale() {
    let source = source(2.0);
    for delta in [[0.125, -0.25], [0.0625, -0.125]] {
        assert!(source.receive(1, delta, [100.0, 100.0], 42, 3, false));
    }
    assert!(source.receive(1, [0.0, 0.0], [100.0, 100.0], 42, 3, true));
    let pending = source.pending.lock().unwrap();
    assert_eq!(pending.len(), 1);
    let GestureEvent::Scroll(event) = pending[0] else {
        panic!("Expected scroll")
    };
    assert_eq!(event.pixels, [-0.375, 0.75]);
    assert_eq!(event.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert!(event.finished);
}

#[test]
fn coalescing_preserves_modifiers_gesture_endings_and_event_order() {
    let source = source(1.0);
    for (kind, modifiers, finished) in [
        (1, 0, false),
        (1, 2, false),
        (0, 0, true),
        (1, 2, true),
        (1, 2, false),
    ] {
        assert!(source.receive(kind, [0.125, 0.0], [100.0, 100.0], 42, modifiers, finished));
    }
    assert_eq!(source.pending.lock().unwrap().len(), 5);
}

#[test]
fn changed_canvas_or_modal_discards_calibration_and_pending_native_input() {
    for canvas in [Some(Rect::new(30, 5, 20, 10)), None] {
        let input = Input {
            shared: Arc::new(source(1.0)),
            ..Default::default()
        };
        assert!(
            input
                .shared
                .receive(0, [0.01, 0.0], [100.0, 100.0], 42, 0, false)
        );
        input.retain_canvas(Some(Rect::new(10, 5, 20, 10)));
        assert_eq!(input.shared.pending.lock().unwrap().len(), 1);
        input.retain_canvas(canvas);
        assert!(input.shared.pending.lock().unwrap().is_empty());
        assert!(
            !input
                .shared
                .receive(1, [0.125, 0.25], [100.0, 100.0], 42, 0, false)
        );
    }
}

#[test]
fn focus_loss_rejects_delayed_mouse_reports_until_focus_returns_and_pointer_is_recalibrated() {
    let input = Input {
        shared: Arc::new(source(1.0)),
        ..Default::default()
    };
    let pointer = input.shared.calibration.lock().unwrap().pointer;
    let scroll = || {
        input
            .shared
            .receive(1, [0.125, 0.25], [100.0, 100.0], 42, 0, false)
    };
    assert!(scroll());
    input.set_focused(false);
    assert!(input.shared.pending.lock().unwrap().is_empty());
    input.shared.observe_pointer(pointer);
    assert!(
        !scroll(),
        "A delayed mouse report must not reclaim background input"
    );
    input.cancel();
    input.retain_canvas(Some(Rect::new(10, 5, 20, 10)));
    input.shared.observe_pointer(pointer);
    assert!(
        !scroll(),
        "Redraws and cancellations must preserve lost focus"
    );
    input.set_focused(true);
    assert!(
        !scroll(),
        "Returning focus must not restore stale calibration"
    );
    input.shared.observe_pointer(pointer);
    assert!(scroll());
}

#[test]
fn disabled_unscoped_invalid_and_overflowing_packets_remain_on_the_terminal_path() {
    let source = source(1.0);
    assert!(!source.receive(0, [f64::NAN, 0.0], [100.0, 100.0], 42, 0, false));
    assert!(!source.receive(1, [0.0, f64::INFINITY], [100.0, 100.0], 42, 0, false));
    assert!(!source.receive(0, [0.1, 0.0], [f64::NAN, 100.0], 42, 0, false));
    source
        .calibration
        .lock()
        .unwrap()
        .pointer
        .as_mut()
        .unwrap()
        .pinch = false;
    assert!(!source.receive(0, [0.1, 0.0], [100.0, 100.0], 42, 0, false));
    source
        .calibration
        .lock()
        .unwrap()
        .pointer
        .as_mut()
        .unwrap()
        .scroll = false;
    assert!(!source.receive(1, [0.1, 0.0], [100.0, 100.0], 42, 0, false));
    source
        .calibration
        .lock()
        .unwrap()
        .pointer
        .as_mut()
        .unwrap()
        .scroll = true;
    for index in 0..128 {
        assert!(source.receive(1, [0.1, 0.0], [100.0, 100.0], 42, index % 2, false));
    }
    assert!(!source.receive(1, [0.1, 0.0], [100.0, 100.0], 42, 0, false));
    assert_eq!(source.pending.lock().unwrap().len(), 128);
    let pinch = Input {
        shared: Arc::new(source),
        ..Default::default()
    };
    pinch.cancel();
    assert!(pinch.shared.pending.lock().unwrap().is_empty());
    assert!(
        !pinch
            .shared
            .receive(1, [0.1, 0.0], [100.0, 100.0], 42, 0, false)
    );
}

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_void;

    #[test]
    fn native_foreground_tracks_app_switches_and_declines_unknown_owners_without_posting_input() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("foreground.m");
        let executable = temp.path().join("foreground");
        std::fs::write(&source, include_str!("tests/foreground.m")).unwrap();
        let output = std::process::Command::new("clang")
            .args(["-fobjc-arc", "-fblocks", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/gestures"))
            .arg(&source)
            .args([
                "-framework",
                "AppKit",
                "-framework",
                "ApplicationServices",
                "-o",
            ])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let output = std::process::Command::new(executable).output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventCreateScrollWheelEvent(
            source: *const c_void,
            units: u32,
            wheels: u32,
            wheel1: i32,
            ...
        ) -> *mut c_void;
        fn CGEventSetType(event: *mut c_void, kind: u32);
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
        fn CGEventSetDoubleValueField(event: *mut c_void, field: u32, value: f64);
        fn CFRelease(object: *mut c_void);
        fn lp_pinch_decode(event: *mut c_void, delta: *mut f64, ended: *mut bool) -> bool;
        fn lp_pinch_event_mask() -> u64;
        fn lp_scroll_decode(
            event: *mut c_void,
            dx: *mut f64,
            dy: *mut f64,
            modifiers: *mut u32,
            ended: *mut bool,
        ) -> bool;
    }

    #[test]
    fn native_event_mask_uses_core_graphics_gesture_packets_instead_of_appkit_magnify() {
        assert_eq!(unsafe { lp_pinch_event_mask() }, (1 << 29) | (1 << 22));
    }

    #[test]
    fn native_decoder_declines_foreign_event_types_without_appkit_warnings() {
        const CHILD: &str = "LAZYPENCIL_TEST_NATIVE_EVENT_TYPES";
        if std::env::var_os(CHILD).is_none() {
            // Isolate stderr so warnings emitted directly by AppKit fail the test.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tui::gestures::tests::native::native_decoder_declines_foreign_event_types_without_appkit_warnings",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            assert!(output.stderr.is_empty(), "{output:?}");
            return;
        }
        unsafe {
            let (mut dx, mut dy, mut modifiers, mut ended) = (0.0, 0.0, 0, false);
            assert!(!lp_pinch_decode(std::ptr::null_mut(), &mut dx, &mut ended));
            assert!(!lp_scroll_decode(
                std::ptr::null_mut(),
                &mut dx,
                &mut dy,
                &mut modifiers,
                &mut ended,
            ));
            for kind in [0, 1, 22, 30, 31] {
                let event = CGEventCreate(std::ptr::null());
                assert!(!event.is_null());
                CGEventSetType(event, kind);
                assert!(!lp_pinch_decode(event, &mut dx, &mut ended), "{kind}");
                if kind != 22 {
                    assert!(!lp_scroll_decode(
                        event,
                        &mut dx,
                        &mut dy,
                        &mut modifiers,
                        &mut ended,
                    ));
                }
                CFRelease(event);
            }
        }
    }

    #[test]
    fn native_decoder_reads_magnification_and_phase_without_posting_input() {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            assert!(!event.is_null());
            // Construct a gesture fixture in memory; never post desktop input.
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

    #[test]
    fn native_scroll_reads_precise_axes_modifiers_and_momentum_but_declines_wheel_detents() {
        unsafe {
            for units in [0, 1] {
                let event =
                    CGEventCreateScrollWheelEvent(std::ptr::null(), units, 2, 13_i32, -7_i32);
                assert!(!event.is_null());
                CGEventSetFlags(event, (1 << 17) | (1 << 18));
                // CoreGraphics momentum phase 3 denotes the last momentum packet.
                CGEventSetIntegerValueField(event, 123, 3);
                let (mut dx, mut dy, mut modifiers, mut ended) = (0.0, 0.0, 0, false);
                let decoded = lp_scroll_decode(event, &mut dx, &mut dy, &mut modifiers, &mut ended);
                CFRelease(event);
                assert_eq!(decoded, units == 0);
                if decoded {
                    assert_eq!([dx, dy], [-7.0, 13.0]);
                    assert_eq!(modifiers, 3);
                    assert!(ended);
                }
            }
        }
    }
}
