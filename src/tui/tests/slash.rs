// Included in tui::tests to share isolated document and adapter fixtures.

#[test]
fn slash_completion_filters_conversation_actions_and_retains_the_permission_alias() {
    use commands::completion::{candidates, query};
    assert_eq!(candidates("").len(), 12);
    assert_eq!(
        candidates("cl")
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["clear-queue", "clear-images"]
    );
    assert_eq!(candidates("mo")[0].name, "permissions");
    for prefix in ["theme", "history", "play", "commands", "quit", "日本語"] {
        assert!(candidates(prefix).is_empty());
    }
    for input in ["hello /con", "/attach 日本語.png", "/con\ntext", "/日本語"] {
        assert!(query(&editor(input)).is_none());
    }
    assert_eq!(query(&editor("/")), Some(""));
}

#[test]
fn slash_completion_clicks_fill_without_execution_and_reject_stale_queries() {
    use crossterm::event::{MouseButton, MouseEventKind as M};
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.focus = Focus::Prompt;
        app.editor = editor("/");
        let picker = Backend::halfblocks();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            196,
        )
        .unwrap();
        paint_mouse(&mut app, 130, 34);
        let row = app
            .pointer
            .area(mouse::Target::CommandCompletion(Some(3)))
            .unwrap();
        app.mouse_event(
            mouse_input(
                M::Down(MouseButton::Left),
                (row.x, row.y),
                KeyModifiers::NONE,
            ),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        app.editor = editor("/att");
        paint_mouse(&mut app, 130, 34);
        app.mouse_event(
            mouse_input(M::Up(MouseButton::Left), (row.x, row.y), KeyModifiers::NONE),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        assert_eq!(app.editor.lines(), ["/att"]);
        let row = app
            .pointer
            .area(mouse::Target::CommandCompletion(Some(0)))
            .unwrap();
        for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
            app.mouse_event(
                mouse_input(kind, (row.x, row.y), KeyModifiers::NONE),
                &picker,
                &worker,
            )
            .await
            .unwrap();
        }
        assert_eq!(app.editor.lines(), ["/attach "]);
        assert_eq!(app.focus, Focus::Prompt);
        assert!(app.input.is_none());
        assert!(!app.conversation.active());
    });
}

#[test]
fn slash_completion_fills_without_running_preserves_arguments_and_dismisses_until_edit() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.focus = Focus::Prompt;
        let document = std::fs::read(app.conversation.store().path()).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            193,
        )
        .unwrap();
        app.editor = editor("/cl");
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/clear-images"]);
        press(&mut app, KeyCode::Char('_'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines(), ["/cl"]);
        app.editor = editor("/mo");
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/mode"]);
        assert!(app.permission_ui.is_none());
        assert_eq!(app.focus, Focus::Prompt);
        assert!(!app.conversation.active());
        app.editor = editor("/att 日本語の画像.png");
        app.editor.move_cursor(CursorMove::Jump(0, 4));
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/attach 日本語の画像.png"]);
        assert!(app.input.is_none());
        app.editor = editor("/att");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/attach "]);
        assert!(app.input.is_none());
        app.editor = editor("/con");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/con"]);
        app.insert_text("v");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["/conversations"]);
        assert!(app.document_task.is_none());
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(app.document_task.is_some());
        document_result(&mut app, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::Saved(_))));
        assert!(!temp.path().join("sent-turn.txt").exists());
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            document
        );
    });
}

#[test]
fn slash_help_is_scoped_and_removed_commands_are_never_submitted_with_custom_keys() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.focus = Focus::Prompt;
        app.keys.submit = vec!["F5".into()];
        app.navigation.commands = vec!["F8".into()];
        app.navigation.validate(&app.keys).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            194,
        )
        .unwrap();
        for key in [KeyCode::Enter, KeyCode::F(5)] {
            for input in ["/theme light", "/history", "/quit", "/unknown"] {
                app.editor = editor(input);
                let error = app
                    .key(
                        KeyEvent::new(key, KeyModifiers::NONE),
                        &Backend::halfblocks(),
                        &worker,
                    )
                    .await
                    .unwrap_err();
                assert!(error.message.contains("F8"));
                assert_eq!(app.editor.lines(), [input]);
                assert!(!app.conversation.active());
            }
        }
        app.editor = editor("/help");
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        assert!(app.help && app.slash_help);
        assert!(app.help_text().contains("/attach"));
        assert!(!app.help_text().contains("Preview: pan"));
        assert!(!app.help_text().contains("/theme"));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(1), KeyModifiers::NONE, &worker).await;
        assert!(app.help && !app.slash_help);
        assert!(app.help_text().contains("Preview: pan"));
        assert!(!temp.path().join("sent-turn.txt").exists());
    });
}

#[test]
fn slash_completion_scrolls_and_keeps_the_prompt_caret_at_every_supported_size() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.focus = Focus::Prompt;
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            195,
        )
        .unwrap();
        for (width, height) in [(50, 16), (80, 24), (130, 34), (170, 44)] {
            app.editor = editor("/");
            app.completion = Default::default();
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            for _ in 0..11 {
                press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
            }
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                text.contains("/help"),
                "Selected completion must be visible at {width}x{height}"
            );
            assert!(!text.contains("/theme"));
            let position = terminal.get_cursor_position().unwrap();
            let prompt = app
                .pointer
                .area(mouse::Target::Focus(Focus::Prompt))
                .unwrap();
            assert!(
                prompt.contains(position),
                "Completion must retain the prompt's IME caret"
            );
            crate::tui::test_artifacts::artifact(
                terminal.backend().buffer(),
                &format!("slash-completion-{width}"),
            );
            press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
            assert_eq!(app.editor.lines(), ["/help"]);
            assert!(!app.help);
        }
        app.editor = editor("/con\nliteral text");
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
        assert_ne!(app.focus, Focus::Prompt);
    });
}
