fn panel_widths(app: &App) -> [u16; 3] {
    workspace::sized_areas(Rect::new(0, 0, 130, 34), &app.panel_layout, app.resize.widths,
        app.nodes_collapsed, app.agent_collapsed, app.footer_height()).panes.map(|p| p.width)
}

#[test]
fn panel_resize_keyboard_moves_directional_edges_and_keeps_the_draft() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.mouse.enabled = false;
        app.insert_text("Keep this draft");
        let original = std::fs::read(app.conversation.store().path()).unwrap();
        let initial = panel_widths(&app);
        for (focus, key, expected) in [
            (Focus::Nodes, KeyCode::Left, [-7, 7, 0]),
            (Focus::Nodes, KeyCode::Char('l'), [7, -7, 0]),
            (Focus::Preview, KeyCode::Char('h'), [-7, 7, 0]),
            (Focus::Preview, KeyCode::Right, [0, 7, -7]),
            (Focus::Prompt, KeyCode::Char('h'), [0, -7, 7]),
            (Focus::Prompt, KeyCode::Right, [0, 7, -7]),
        ] {
            app.resize.widths = None;
            app.focus = focus;
            app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT), &picker, &worker).await.unwrap();
            assert!(app.resize.keyboard);
            app.key(KeyEvent::new(key, KeyModifiers::NONE), &picker, &worker).await.unwrap();
            let widths = panel_widths(&app);
            for i in 0..3 { assert_eq!(i32::from(widths[i]) - i32::from(initial[i]), expected[i]); }
            // Esc keeps the live change, matching Herdr.
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &picker, &worker).await.unwrap();
            assert!(!app.resize.active());
            assert_eq!(panel_widths(&app), widths);
        }
        app.resize.widths = None;
        app.focus = Focus::Prompt;
        app.start_panel_resize();
        app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert_eq!(app.focus, Focus::Nodes);
        for _ in 0..2 { app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &picker, &worker).await.unwrap(); }
        assert_eq!(panel_widths(&app)[0], initial[0] + 14, "Queued repeat keys must accumulate before drawing");
        for key in [KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Char('q'), KeyCode::Up, KeyCode::Down] {
            app.key(KeyEvent::new(key, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        }
        assert_eq!(panel_widths(&app)[0], initial[0] + 14);
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert!(!app.resize.active());
        assert_eq!(app.editor.lines(), ["Keep this draft"]);
        assert_eq!(std::fs::read(app.conversation.store().path()).unwrap(), original);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn panel_resize_drag_survives_redraws_clamps_and_can_be_undone() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.insert_text("Unsent draft");
        let focus = app.focus;
        let initial = panel_widths(&app);
        for boundary in [resize::Boundary::Nodes, resize::Boundary::Agent] {
            let area = app.pointer.area(mouse::Target::Resize(boundary)).unwrap();
            let point = (area.x, area.y + 3);
            app.mouse_event(mouse_input(M::Down(MouseButton::Left), point, KeyModifiers::NONE), &picker, &worker).await.unwrap();
            for delta in [5, 10, 15] {
                app.mouse_event(mouse_input(M::Drag(MouseButton::Left), (point.0 + delta, point.1), KeyModifiers::NONE), &picker, &worker).await.unwrap();
                paint_mouse(&mut app, 130, 34);
                assert!(app.resize.active());
            }
            app.mouse_event(mouse_input(M::Up(MouseButton::Left), (point.0 + 15, point.1), KeyModifiers::NONE), &picker, &worker).await.unwrap();
            assert!(!app.resize.active());
            let after = panel_widths(&app);
            if boundary == resize::Boundary::Nodes { assert_eq!(after, [initial[0]+15, initial[1]-15, initial[2]]); }
            else { assert_eq!(after, [initial[0], 130 - initial[0] - 18, 18]); }
            assert_eq!(app.focus, focus);
            app.resize.widths = None;
            paint_mouse(&mut app, 130, 34);
        }
        let boundary = app.pointer.area(mouse::Target::Resize(resize::Boundary::Nodes)).unwrap();
        let point = (boundary.x, boundary.y + 2);
        for kind in [M::Down(MouseButton::Left), M::Drag(MouseButton::Left)] {
            let x = if matches!(kind, M::Drag(_)) { 0 } else { point.0 };
            app.mouse_event(mouse_input(kind, (x, point.1), KeyModifiers::NONE), &picker, &worker).await.unwrap();
        }
        assert_eq!(panel_widths(&app)[0], 12);
        paint_mouse(&mut app, 130, 34);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert_eq!(panel_widths(&app), initial);
        assert!(!app.resize.active());
        assert_eq!(app.editor.lines(), ["Unsent draft"]);
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn panel_resize_preserves_hidden_preferences_and_resets_from_the_palette() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, picker) = mouse_fixture(&temp).await;
        app.focus = Focus::Nodes;
        app.start_panel_resize();
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        app.resize.stop();
        let adjusted = panel_widths(&app);
        app.toggle_panel(Nav::ToggleNodesPanel, &worker);
        assert_eq!(panel_widths(&app), [0, adjusted[0] + adjusted[1], adjusted[2]]);
        app.start_panel_resize();
        app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), &picker, &worker).await.unwrap();
        assert_eq!(app.focus, Focus::Prompt);
        app.resize.stop();
        app.toggle_panel(Nav::ToggleNodesPanel, &worker);
        assert_eq!(panel_widths(&app), adjusted);
        app.execute_command(commands::Action::ResetPanelSizes, &[], &worker).unwrap();
        assert!(app.resize.widths.is_none());
        assert_ne!(panel_widths(&app), adjusted);
        app.execute_command(commands::Action::ResizePanels, &[], &worker).unwrap();
        assert!(app.resize.keyboard);
        app.resize.widths = Some(workspace::Widths { nodes: 0.35, agent: 0.4 });
        let preferred = app.resize.widths;
        paint_mouse(&mut app, 50, 16);
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &picker, &worker).await.unwrap();
        assert_eq!(app.resize.widths, preferred, "A blocked movement must preserve preferred proportions");
        app.help = true;
        paint_mouse(&mut app, 130, 34);
        assert!(!app.resize.active());
        assert!(app.pointer.area(mouse::Target::Resize(resize::Boundary::Nodes)).is_none());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}
