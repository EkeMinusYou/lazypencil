// Included in tui::tests to share isolated application fixtures.

fn modal_review_layouts(app: &mut App, name: &str, title: &str) {
    use crate::config::keys::Theme;
    let previous = app.theme;
    for theme in [Theme::Dark, Theme::Light, Theme::Monochrome] {
        let _theme = theme::activate(theme, None);
        app.theme = theme;
        for (width, height) in [(50, 16), (80, 24), (130, 34)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let screen = buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains(title), "{name} {width}x{height}: {screen}");
            assert!(screen.contains("[Esc]"));
            crate::tui::test_artifacts::artifact(
                buffer,
                &format!("review-{name}-{theme:?}-{width}x{height}"),
            );
        }
    }
    app.theme = previous;
}

#[test]
fn modal_command_forms_keep_the_input_error_cursor_and_actions_visible_after_resizing() {
    use crate::config::keys::Theme;
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.insert_text("Keep the conversation draft");
        for theme in [Theme::Dark, Theme::Light, Theme::Monochrome] {
            let _theme = theme::activate(theme, None);
            app.theme = theme;
            for command in commands::COMMANDS
                .iter()
                .filter(|command| !command.arguments.is_empty())
            {
                for error in [false, true] {
                    let mut input = commands::Input::new(command.action);
                    input.insert("command argument");
                    if error {
                        input.error = Some("Invalid value; correct the arguments".into());
                    }
                    app.modal = Some(checkpoints::Modal::CommandInput(Box::new(input)));
                    for (width, height) in [(50, 16), (80, 24), (100, 16), (130, 34), (150, 42)] {
                        let mut terminal = ratatui::Terminal::new(
                            ratatui::backend::TestBackend::new(width, height),
                        )
                        .unwrap();
                        terminal
                            .draw(|frame| {
                                app.draw(frame);
                            })
                            .unwrap();
                        let cursor = terminal.get_cursor_position().unwrap();
                        let buffer = terminal.backend().buffer();
                        let screen = buffer
                            .content
                            .iter()
                            .map(|cell| cell.symbol())
                            .collect::<String>();
                        assert!(
                            screen.contains("command argument"),
                            "{theme:?} {:?} {width}x{height}: {screen}",
                            command.action
                        );
                        assert!(screen.contains("[Enter Run]"));
                        if error {
                            assert!(
                                screen.contains("Invalid value; correct the arguments"),
                                "{screen}"
                            );
                        }
                        assert!(modal::area(buffer.area).contains(cursor));
                        assert_eq!(buffer[cursor].bg, theme::colors().prompt_background);
                        assert!(
                            !buffer
                                .content
                                .iter()
                                .any(|cell| cell.modifier.contains(Modifier::HIDDEN))
                        );
                        let accept = app
                            .pointer
                            .area(mouse::Target::Button(mouse::Action::Accept))
                            .unwrap();
                        let close = app
                            .pointer
                            .area(mouse::Target::Button(mouse::Action::Close))
                            .unwrap();
                        assert!(close.y < cursor.y && accept.y > cursor.y);
                        if command.action == commands::Action::Queue {
                            crate::tui::test_artifacts::artifact(
                                buffer,
                                &format!("form-{theme:?}-{width}x{height}-error-{error}"),
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(app.editor.lines(), ["Keep the conversation draft"]);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn modal_workflows_keep_their_content_and_controls_visible_in_each_theme_and_size() {
    use crate::config::keys::Theme;
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions");
        permission_ready(&mut app).await;
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 198).unwrap();
        let checkpoint = checkpoints::Page {
            revision: "current-document-revision-0123456789abcdef".into(),
            dependency_revision: "dependency-revision-0123456789abcdef".into(),
            entries: vec![checkpoints::Checkpoint { id: "checkpoint-0123456789abcdef".into(), sequence: 2, created_at: "2026-11-10 12:34:56 UTC".into(), request_id: "request-0123456789abcdef".into(), revision: "target-document-revision-0123456789abcdef".into() }],
            next_before: Some(1), cursors: vec![None], selected: 0, notice: None, review_scroll: 0, review_maximum_scroll: 0,
        };
        for theme in [Theme::Dark, Theme::Light, Theme::Monochrome] {
            let _theme = theme::activate(theme, None);
            app.theme = theme;
            for (width, height) in [(50, 16), (80, 24), (130, 34), (150, 42)] {
                for workflow in ["loading", "history", "confirm", "sessions", "turns", "log", "raw", "help", "view", "modes", "scope", "full"] {
                    app.modal = None; app.help = false; app.permission_ui = None; app.pointer.more = false;
                    let title = match workflow {
                        "loading" => { app.modal = Some(checkpoints::Modal::Loading { message: "Loading saved logs…" }); "Document operation" }
                        "history" => { app.modal = Some(checkpoints::Modal::History(checkpoint.clone())); "Checkpoints" }
                        "confirm" => { app.modal = Some(checkpoints::Modal::Confirm(checkpoint.clone())); "Review restoration" }
                        "sessions" | "turns" | "log" | "raw" => {
                            let mut page = saved::Page::sessions("session-0123456789abcdef".into());
                            page.data = json!({"entries":[{"id":"saved-entry-0123456789abcdef", "agent":"Codex", "requested_auth":"login", "bytes":128}], "text":"Saved response with 日本語", "revision":"revision-0123456789abcdef"});
                            if workflow == "turns" { page.location = saved::Location::Turns("session-0123456789abcdef".into()); }
                            if matches!(workflow, "log" | "raw") { page.location = saved::Location::Text { session: "session-0123456789abcdef".into(), turn: "turn-0123456789abcdef".into(), raw: workflow == "raw" }; }
                            app.modal = Some(checkpoints::Modal::Saved(Box::new(page)));
                            match workflow { "sessions" => "Conversations", "turns" => "Turns", _ => "Saved log" }
                        }
                        "help" => { app.show_help(); "Help" }
                        "view" => { app.pointer.more = true; "View controls" }
                        "modes" | "scope" | "full" => {
                            app.open_agent_permissions();
                            if workflow != "modes" { app.permission_click(permissions::Hit::Row(if workflow == "full" { 2 } else { 1 })).unwrap(); app.permission_click(permissions::Hit::Continue).unwrap(); }
                            if workflow == "full" { app.permission_click(permissions::Hit::Continue).unwrap(); }
                            match workflow { "modes" => "Agent permissions", "scope" => "Apply permissions", _ => "Allow full access?" }
                        }
                        _ => unreachable!(),
                    };
                    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| { app.draw(frame); }).unwrap();
                    let buffer = terminal.backend().buffer();
                    let screen = buffer.content.iter().map(|cell| cell.symbol()).collect::<String>();
                    assert!(screen.contains(title), "{workflow} {width}x{height}: {screen}");
                    assert!(screen.contains("[Esc]"));
                    if matches!(workflow, "log" | "raw") { assert!(screen.contains("Saved response"), "{screen}"); }
                    if workflow == "view" { assert!(app.pointer.area(mouse::Target::Button(mouse::Action::ToggleAgent)).is_some()); }
                    crate::tui::test_artifacts::artifact(buffer, &format!("workflow-{workflow}-{theme:?}-{width}x{height}"));
                    if workflow == "confirm" && width == 50 {
                        let Some(checkpoints::Modal::Confirm(page)) = &app.modal else { panic!() };
                        assert!(page.review_maximum_scroll > 0);
                        app.modal_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), &worker).unwrap();
                        terminal.draw(|frame| { app.draw(frame); }).unwrap();
                        let screen = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
                        assert!(screen.contains("restoration."), "{screen}");
                    }
                }
            }
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn modal_workflows_reuse_native_preview_pixels_outside_their_surface() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        command_palette_preview(&mut app, true).await;
        for workflow in ["input", "loading", "saved", "help", "permissions", "view"] {
            app.modal = None;
            app.help = false;
            app.permission_ui = None;
            app.pointer.more = false;
            command_palette_frame(&mut app);
            match workflow {
                "input" => {
                    app.modal = Some(checkpoints::Modal::CommandInput(Box::new(
                        commands::Input::new(commands::Action::Queue),
                    )))
                }
                "loading" => {
                    app.modal = Some(checkpoints::Modal::Loading {
                        message: "Loading…",
                    })
                }
                "saved" => {
                    app.modal = Some(checkpoints::Modal::Saved(Box::new(saved::Page::sessions(
                        app.conversation.id().into(),
                    ))))
                }
                "help" => app.show_help(),
                "permissions" => app.open_agent_permissions(),
                "view" => app.pointer.more = true,
                _ => unreachable!(),
            }
            let buffer = command_palette_frame(&mut app);
            let wire = buffer[(0, 0)].symbol();
            assert_eq!(wire.matches("a=p,").count(), 2, "{workflow}: {wire}");
            assert!(
                !wire.contains("a=t,"),
                "{workflow} must reuse the uploaded image"
            );
            let serial = app.presentation.serial();
            command_palette_frame(&mut app);
            assert_eq!(app.presentation.serial(), serial);
        }
        app.conversation.close().await.unwrap();
    });
}
