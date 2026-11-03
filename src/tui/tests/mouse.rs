fn mouse_input(kind: crossterm::event::MouseEventKind, point: (u16, u16), modifiers: KeyModifiers) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent { kind, column: point.0, row: point.1, modifiers }
}

fn paint_mouse(app: &mut App, width: u16, height: u16) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| { app.cells = app.draw(frame); }).unwrap();
}

async fn mouse_fixture(temp: &TempDir) -> (App, Worker, Backend) {
    let mut app = app(temp, "echo");
    app.mouse.show_controls = true;
    let children = (0..64).map(|index| json!({"type":"rectangle", "id":format!("mouse-{index}"), "name":format!("Mouse row {index}"), "x":index*100, "width":80, "height":80, "fill":"#ff0000"})).collect::<Vec<_>>();
    std::fs::write(app.conversation.store().path(), json!({"version":"2.21", "children":children}).to_string()).unwrap();
    let picker = Backend::halfblocks();
    let worker = Worker::start(app.conversation.store().clone(), vec![], picker.clone(), 198).unwrap();
    paint_mouse(&mut app, 130, 34);
    app.request(&worker, false, false);
    let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
    app.accept_interactive_preview(output, &worker, &picker);
    paint_mouse(&mut app, 130, 34);
    (app, worker, picker)
}

#[test]
fn mouse_scroll_keeps_selection_and_clicks_the_rendered_row() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.nodes.select(Some(0));
        paint_mouse(&mut app, 130, 34);
        let area = app.pointer.area(mouse::Target::List(mouse::ListTarget::Nodes)).unwrap();
        let point = (area.x + 3, area.y);
        app.mouse_event(mouse_input(M::ScrollDown, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.nodes.selected(), Some(0));
        assert_eq!(app.nodes.offset(), 2);
        assert_eq!(app.focus, Focus::Prompt);
        app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.nodes.offset(), 2, "Typing in Prompt must not reset another panel's scroll");
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        }
        assert_eq!(app.selected_node().unwrap().id, "mouse-2");
        assert_eq!(app.focus, Focus::Nodes);
        assert!(app.outbox.references.is_empty());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_zoom_anchors_the_pointer_and_pan_preserves_the_prompt_and_document() {
    use crossterm::event::MouseEventKind as M;
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.insert_text("日本語の下書き");
        let original = std::fs::read(app.conversation.store().path()).unwrap();
        let canvas = app.pointer.area(mouse::Target::Canvas).unwrap();
        let point = (canvas.x + canvas.width / 3, canvas.y + canvas.height / 3);
        let before = app.preview.as_ref().unwrap().view;
        let font = picker.font_size();
        let offset = [
            (f64::from(point.0 - canvas.x) + 0.5 - f64::from(canvas.width)/2.0) * f64::from(font.width),
            (f64::from(point.1 - canvas.y) + 0.5 - f64::from(canvas.height)/2.0) * f64::from(font.height),
        ];
        app.mouse_event(mouse_input(M::ScrollUp, point, KeyModifiers::CONTROL), &picker, &worker).await.unwrap();
        for (axis, offset) in offset.into_iter().enumerate() {
            let old_world = before.center[axis] + offset/f64::from(before.zoom.unwrap());
            let new_world = app.view.center[axis] + offset/f64::from(app.view.zoom.unwrap());
            assert!((old_world-new_world).abs() < 1e-6);
        }
        let before = app.view;
        app.mouse_event(mouse_input(M::ScrollRight, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(app.view.center[0] > before.center[0]);
        assert_eq!(app.view.center[1], before.center[1]);
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.editor.lines(), &["日本語の下書き"]);
        assert_eq!(std::fs::read(app.conversation.store().path()).unwrap(), original);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_space_drag_is_temporary_and_plain_drag_does_not_pan() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.focus = Focus::Preview;
        app.effective_camera();
        let canvas = app.pointer.area(mouse::Target::Canvas).unwrap();
        let start = (canvas.x + 10, canvas.y + 5);
        let end = (start.0 + 3, start.1 + 2);
        let before = app.view;
        for (kind, point) in [(M::Down(MouseButton::Left), start), (M::Drag(MouseButton::Left), end), (M::Up(MouseButton::Left), end)] {
            app.mouse_event(mouse_input(kind, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        }
        assert_eq!(app.view, before);
        app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(!app.playback.active());
        app.mouse_event(mouse_input(M::Down(MouseButton::Left), start, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        app.pointer.space = false;
        app.mouse_event(mouse_input(M::Drag(MouseButton::Left), end, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        let font = picker.font_size();
        assert!((app.view.center[0] - (before.center[0] - 3.0*f64::from(font.width)/f64::from(before.zoom.unwrap()))).abs() < 1e-6);
        assert!((app.view.center[1] - (before.center[1] - 2.0*f64::from(font.height)/f64::from(before.zoom.unwrap()))).abs() < 1e-6);
        app.mouse_event(mouse_input(M::Up(MouseButton::Left), end, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        app.preview.as_mut().unwrap().time_dependent = true;
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(app.playback.active());
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(!app.playback.active());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_modal_rows_and_buttons_survive_redraw_and_block_the_workspace() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        let original = app.view;
        app.modal = Some(checkpoints::Modal::commands());
        paint_mouse(&mut app, 130, 34);
        let row = app.pointer.area(mouse::Target::Row(mouse::ListTarget::Commands, 8)).unwrap();
        let point = (row.x + 3, row.y);
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
            paint_mouse(&mut app, 130, 34);
        }
        assert!(matches!(app.modal, Some(checkpoints::Modal::Commands { selected: 8 })));
        assert!(!app.nodes_collapsed);
        app.mouse_event(mouse_input(M::ScrollDown, (0, 5), KeyModifiers::CONTROL), &picker, &worker).await.unwrap();
        assert_eq!(app.view, original);
        let button = app.pointer.area(mouse::Target::Button(mouse::Action::Accept)).unwrap();
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
            paint_mouse(&mut app, 130, 34);
        }
        assert!(app.nodes_collapsed);
        assert!(app.modal.is_none());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_menu_restores_panels_at_minimum_size_and_disabled_input_is_ignored() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.nodes_collapsed = true;
        app.agent_collapsed = true;
        paint_mouse(&mut app, 50, 16);
        if app.pointer.area(mouse::Target::Button(mouse::Action::ToggleAgent)).is_none() {
            let menu = app.pointer.area(mouse::Target::Button(mouse::Action::More)).unwrap();
            for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
                app.mouse_event(mouse_input(kind, (menu.x, menu.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
                paint_mouse(&mut app, 50, 16);
            }
        }
        let button = app.pointer.area(mouse::Target::Button(mouse::Action::ToggleAgent)).unwrap();
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
            paint_mouse(&mut app, 50, 16);
        }
        assert!(!app.agent_collapsed);
        let button = app.pointer.area(mouse::Target::Button(mouse::Action::More)).unwrap();
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
            paint_mouse(&mut app, 50, 16);
        }
        assert!(app.pointer.more);
        assert!(app.pointer.area(mouse::Target::Button(mouse::Action::FitAll)).is_some());
        app.pointer.more = false;
        paint_mouse(&mut app, 50, 16);
        app.mouse.enabled = false;
        let before = app.view;
        let canvas = app.pointer.area(mouse::Target::Canvas).unwrap();
        app.mouse_event(mouse_input(M::ScrollUp, (canvas.x, canvas.y), KeyModifiers::CONTROL), &picker, &worker).await.unwrap();
        assert_eq!(app.view, before);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_changed_modal_cannot_reuse_an_old_press_or_scroll_target() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        let button = app.pointer.area(mouse::Target::Button(mouse::Action::More)).unwrap();
        app.mouse_event(mouse_input(M::Down(MouseButton::Left), (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        app.help = true;
        // The overlay may appear between input events, before its first draw.
        app.mouse_event(mouse_input(M::Up(MouseButton::Left), (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(!app.nodes_collapsed);
        paint_mouse(&mut app, 130, 34);
        let button = app.pointer.area(mouse::Target::Button(mouse::Action::Close)).unwrap();
        app.mouse_event(mouse_input(M::Down(MouseButton::Left), (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        app.pointer.clear();
        paint_mouse(&mut app, 100, 24);
        app.mouse_event(mouse_input(M::Up(MouseButton::Left), (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(app.help);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_scroll_batches_match_individual_steps_and_history_stays_anchored() {
    use crossterm::event::MouseEventKind as M;
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.effective_camera();
        let canvas = app.pointer.area(mouse::Target::Canvas).unwrap();
        let point = (canvas.x + 5, canvas.y + 5);
        let before = app.view;
        let event = mouse_input(M::ScrollDown, point, KeyModifiers::NONE);
        app.mouse_events(event, 32, &picker, &worker).await.unwrap();
        let batched = app.view;
        app.view = before;
        for _ in 0..32 { app.mouse_event(event, &picker, &worker).await.unwrap(); }
        assert_eq!(app.view.zoom, batched.zoom);
        for axis in 0..2 { assert!((app.view.center[axis] - batched.center[axis]).abs() < 1e-9); }
        app.log = (0..80).map(|n| format!("History line {n}\n")).collect();
        paint_mouse(&mut app, 130, 34);
        let history = app.pointer.area(mouse::Target::History).unwrap();
        app.mouse_events(mouse_input(M::ScrollUp, (history.x, history.y), KeyModifiers::NONE), 5, &picker, &worker).await.unwrap();
        paint_mouse(&mut app, 130, 34);
        let offset = app.scroll;
        let lines = app.history_metrics.1;
        app.append("New message\nAnother line\n");
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.scroll, offset + app.history_metrics.1 - lines);
        assert_eq!(app.focus, Focus::Prompt);
        app.mouse_events(mouse_input(M::ScrollDown, (history.x, history.y), KeyModifiers::NONE), 64, &picker, &worker).await.unwrap();
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.scroll, 0);
        app.append("Followed message\n");
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.scroll, 0);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_wheel_preferences_and_middle_right_drags_preserve_camera_directions() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        let canvas = app.pointer.area(mouse::Target::Canvas).unwrap();
        let start = (canvas.x + 4, canvas.y + 4);
        app.effective_camera();
        let before = app.view;
        app.mouse.wheel_zoom = true;
        app.mouse_event(mouse_input(M::ScrollUp, start, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(app.view.zoom.unwrap() > before.zoom.unwrap());
        let before = app.view;
        app.mouse_event(mouse_input(M::ScrollDown, start, KeyModifiers::CONTROL | KeyModifiers::SHIFT), &picker, &worker).await.unwrap();
        assert_eq!(app.view.zoom, before.zoom);
        assert!(app.view.center[0] > before.center[0]);
        assert_eq!(app.view.center[1], before.center[1]);
        let font = picker.font_size();
        for button in [MouseButton::Middle, MouseButton::Right] {
            let before = app.view;
            let end = (start.0 + 2, start.1 + 1);
            for (kind, point) in [(M::Down(button), start), (M::Drag(button), end), (M::Up(button), end)] {
                app.mouse_event(mouse_input(kind, point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
            }
            assert!((app.view.center[0] - before.center[0] + 2.0 * f64::from(font.width)/f64::from(before.zoom.unwrap())).abs() < 1e-6);
            assert!((app.view.center[1] - before.center[1] + f64::from(font.height)/f64::from(before.zoom.unwrap())).abs() < 1e-6);
            assert!(!app.pointer.space);
        }
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn mouse_disclosure_toggles_filtered_branches_without_fitting_or_adding_references() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        std::fs::write(app.conversation.store().path(), json!({"version":"2.21","children":[{"type":"frame","id":"root","width":100,"height":100,"children":[{"type":"rectangle","id":"child","name":"Child label","width":20,"height":20}]}]}).to_string()).unwrap();
        app.request(&worker, false, false);
        loop {
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
            if output.generation == app.generation {
                app.accept_interactive_preview(output, &worker, &picker);
                break;
            }
        }
        paint_mouse(&mut app, 130, 34);
        let before = app.view;
        assert_eq!(app.filtered_nodes().len(), 1);
        for expected in [2, 1] {
            let disclosure = app.pointer.area(mouse::Target::Branch(0)).unwrap();
            for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
                app.mouse_event(mouse_input(kind, (disclosure.x, disclosure.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
                paint_mouse(&mut app, 130, 34);
            }
            assert_eq!(app.filtered_nodes().len(), expected);
        }
        app.filter = "Child label".into();
        app.search_nodes(None);
        paint_mouse(&mut app, 130, 34);
        assert_eq!(app.filtered_nodes().len(), 2);
        let disclosure = app.pointer.area(mouse::Target::Branch(0)).unwrap();
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(mouse_input(kind, (disclosure.x, disclosure.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
            paint_mouse(&mut app, 130, 34);
        }
        assert_eq!(app.filtered_nodes().len(), 1);
        assert_eq!(app.view, before);
        assert!(app.outbox.references.is_empty());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn pointer_capture_keeps_the_default_workspace_appearance() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    assert!(!app.mouse.show_controls);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 34)).unwrap();
    terminal.draw(|frame| { app.cells = app.draw(frame); }).unwrap();
    let enabled = terminal.backend().buffer().clone();
    app.mouse.enabled = false;
    terminal.draw(|frame| { app.cells = app.draw(frame); }).unwrap();
    assert_eq!(&enabled, terminal.backend().buffer());
    assert!(app.pointer.area(mouse::Target::Button(mouse::Action::More)).is_none());
}

#[test]
fn view_menu_is_available_without_persistent_buttons() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, _) = mouse_fixture(&temp).await;
        app.mouse.show_controls = false;
        app.modal = Some(checkpoints::Modal::Commands { selected: checkpoints::COMMANDS.len() - 1 });
        app.activate_modal(&worker).unwrap();
        paint_mouse(&mut app, 130, 34);
        assert!(app.pointer.more);
        assert!(app.pointer.area(mouse::Target::Button(mouse::Action::WheelMode)).is_some());
        app.pointer.more = false;
        app.mouse.enabled = false;
        app.modal = Some(checkpoints::Modal::Commands { selected: checkpoints::COMMANDS.len() - 1 });
        app.activate_modal(&worker).unwrap();
        assert!(!app.pointer.more, "Disabled pointer input must not open an invisible menu");
        app.conversation.close().await.unwrap();
    });
}
