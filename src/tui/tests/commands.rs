// Included in tui::tests to share the isolated workspace fixtures.

#[test]
fn workspace_commands_keep_legacy_names_and_literal_queue_text() {
    use commands::Action;
    for (name, action) in [
        ("history", Action::Checkpoints),
        ("checkpoints", Action::Checkpoints),
        ("logs", Action::Logs),
        ("conversations", Action::Conversations),
        ("resume-queue", Action::ResumeQueue),
        ("clear-queue", Action::ClearQueue),
        ("clear-images", Action::ClearImages),
        ("open-preview", Action::OpenPreview),
        ("pause", Action::Pause),
        ("enable-pinch", Action::EnablePinch),
        ("enable-native-input", Action::EnablePinch),
        ("commands", Action::Commands),
        ("help", Action::Help),
        ("quit", Action::Quit),
    ] {
        assert_eq!(
            commands::parse(&format!("/{name}")).unwrap().0.action,
            action
        );
        assert!(commands::parse(&format!("/{name} extra")).is_err());
    }
    let (_, args) = commands::parse("/attach '画像 samples/reference.png' 800x600").unwrap();
    assert_eq!(args, ["画像 samples/reference.png", "800x600"]);
    let text = "Check the layout's spacing\nthen keep `$(literal)` and \"quotes\"";
    let (_, args) = commands::parse(&format!("/queue {text}")).unwrap();
    assert_eq!(args, [text]);
    for invalid in [
        "/queue",
        "/queue   ",
        "/attach",
        "/attach 'unfinished",
        "/resize-image 1",
        "/time 1 2",
        "/unknown",
    ] {
        assert_eq!(
            commands::parse(invalid).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
    }
}

#[test]
fn workspace_command_input_keeps_plain_navigation_bindings_as_text() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.insert_text("Keep the prompt draft");
        app.focus = Focus::Preview;
        app.navigation.commands = vec!["x".into()];
        app.navigation.validate(&app.keys).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            174,
        )
        .unwrap();
        app.modal = Some(command_modal(commands::Action::Time));
        app.activate_modal(&worker).unwrap();
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE, &worker).await;
        let Some(checkpoints::Modal::CommandInput(input)) = &app.modal else {
            panic!("A navigation key must remain text inside the input dialog");
        };
        assert_eq!(input.editor.lines(), ["x"]);
        assert_eq!(app.editor.lines(), ["Keep the prompt draft"]);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(matches!(
            app.modal,
            Some(checkpoints::Modal::Commands { .. })
        ));
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE, &worker).await;
        assert!(app.modal.is_none());
        assert_eq!(app.focus, Focus::Preview);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn workspace_command_palette_queues_without_consuming_the_draft_and_cancels_input() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.insert_text("未送信の下書き");
        app.save_draft().unwrap();
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        let images = app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>();
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 171).unwrap();
        app.modal = Some(command_modal(commands::Action::Queue));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else { panic!("Expected queue input"); };
        input.insert("次の依頼は layout's spacing を確認\u{1b}");
        crate::tui::test_artifacts::snapshot(&mut app, "command-input");
        press(&mut app, KeyCode::Enter, KeyModifiers::ALT, &worker).await;
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else { panic!(); };
        input.insert("画像は変更しない");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(app.modal.is_none());
        assert_eq!(app.outbox.queued[0].text, "次の依頼は layout's spacing を確認\n画像は変更しない");
        assert!(app.outbox.queued[0].image_ids.is_empty());
        assert!(app.command_unavailable(commands::Action::Attach.command()).is_some());
        assert!(app.command_unavailable(commands::Action::PasteImage.command()).is_none(), "Clipboard text remains available while prompts are queued");
        assert_eq!(outbox::Outbox::load(app.conversation.directory()).unwrap().queued[0].text, app.outbox.queued[0].text);
        assert_eq!(app.editor.lines(), ["未送信の下書き"]);
        assert_eq!(app.conversation.draft(), "未送信の下書き");
        assert_eq!(app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>(), images);

        app.modal = Some(command_modal(commands::Action::Time));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else { panic!(); };
        input.insert("NaN");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        let Some(checkpoints::Modal::CommandInput(input)) = &app.modal else { panic!(); };
        assert!(input.error.is_some());
        assert_eq!(input.editor.lines(), ["NaN"]);
        assert_eq!(app.view.time, 0.0);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::Commands { selected }) if selected == command_index(commands::Action::Time)));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(app.modal.is_none());
        assert_eq!(app.editor.lines(), ["未送信の下書き"]);
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn workspace_command_palette_and_slash_share_playback_and_image_actions() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let (mut app, worker, _) = mouse_fixture(&temp).await;
        app.input_reader = fake_input;
        app.insert_text("Retain this draft");
        app.save_draft().unwrap();
        app.modal = Some(command_modal(commands::Action::PasteImage));
        app.activate_modal(&worker).unwrap();
        app.activate_modal(&worker).unwrap();
        let input = tokio::time::timeout(
            Duration::from_secs(10),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        assert!(app.accept_prepared(input));
        assert_eq!(app.conversation.images().len(), 2);
        assert_eq!(app.editor.lines(), ["Retain this draft"]);
        app.modal = Some(command_modal(commands::Action::ResizeImage));
        app.activate_modal(&worker).unwrap();
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else {
            panic!();
        };
        input.insert("2 original");
        app.activate_modal(&worker).unwrap();
        let input = tokio::time::timeout(
            Duration::from_secs(10),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        assert!(app.accept_prepared(input));
        app.modal = Some(command_modal(commands::Action::ClearImages));
        app.activate_modal(&worker).unwrap();
        assert!(app.conversation.images().is_empty());
        assert_eq!(app.conversation.draft(), "Retain this draft");

        app.preview.as_mut().unwrap().time_dependent = true;
        app.modal = Some(command_modal(commands::Action::Play));
        app.activate_modal(&worker).unwrap();
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else {
            panic!();
        };
        input.insert("3");
        app.activate_modal(&worker).unwrap();
        assert_eq!(app.playback.fps(), Some(3));
        app.modal = Some(command_modal(commands::Action::Pause));
        app.activate_modal(&worker).unwrap();
        assert!(!app.playback.active());
        assert_eq!(app.editor.lines(), ["Retain this draft"]);
        app.editor = editor("/play 3");
        app.command(&worker).await.unwrap();
        assert_eq!(app.playback.fps(), Some(3));
        app.editor = editor("/pause");
        app.command(&worker).await.unwrap();
        assert!(!app.playback.active());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn workspace_command_palette_explains_unavailable_actions_and_scrolls_at_minimum_size() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            172,
        )
        .unwrap();
        app.modal = Some(command_modal(commands::Action::ResizeImage));
        let buffer = crate::tui::test_artifacts::snapshot(&mut app, "commands-unavailable");
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Attach an image before resizing it"));
        app.activate_modal(&worker).unwrap();
        assert!(matches!(
            app.modal,
            Some(checkpoints::Modal::Commands { .. })
        ));
        assert!(
            app.command_unavailable(commands::Action::Play.command())
                .is_some()
        );
        app.mouse.enabled = false;
        assert!(
            app.command_unavailable(commands::Action::ViewControls.command())
                .is_some()
        );
        assert!(
            app.help_text()
                .contains("/checkpoints: Browse committed edits")
        );
        assert!(app.help_text().contains("/history: alias for /checkpoints"));
        for category in [
            "Conversation",
            "Send queue",
            "Images",
            "Preview",
            "View",
            "Checkpoints",
            "General",
        ] {
            assert!(app.help_text().contains(&format!("\n{category}\n")));
        }
        for (width, height) in [(50, 16), (130, 34)] {
            app.modal = Some(checkpoints::Modal::commands());
            press(&mut app, KeyCode::End, KeyModifiers::NONE, &worker).await;
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("General · Quit"));
            assert!(rendered.contains("/quit"));
            let rows = (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>();
            let selected = rows
                .iter()
                .position(|row| row.contains("General · Quit"))
                .unwrap();
            let usage = rows.iter().position(|row| row.contains("/quit")).unwrap();
            assert_eq!(
                usage,
                selected + 1,
                "Resizing must fill the command list without an empty gap above its details"
            );
            crate::tui::test_artifacts::artifact(
                terminal.backend().buffer(),
                &format!("commands-{width}"),
            );
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn workspace_checkpoint_commands_keep_review_and_busy_operation_guards() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        let checkpoint = checkpoint(&app, json!("#FF0000"), "command-checkpoint");
        let document = std::fs::read(app.conversation.store().path()).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            173,
        )
        .unwrap();
        for name in ["/checkpoints", "/history"] {
            app.editor = editor(name);
            app.command(&worker).await.unwrap();
            assert!(app.document_task.is_some());
            app.modal = Some(command_modal(commands::Action::NewConversation));
            app.activate_modal(&worker).unwrap();
            assert!(app.pending_session.is_none());
            assert!(matches!(
                app.modal,
                Some(checkpoints::Modal::Commands { .. })
            ));
            assert!(
                app.command_unavailable(commands::Action::Checkpoints.command())
                    .is_some()
            );
            assert!(
                app.command_unavailable(commands::Action::Logs.command())
                    .is_some()
            );
            document_result(&mut app, &worker).await;
            let Some(checkpoints::Modal::History(page)) = &app.modal else {
                panic!("Expected checkpoints");
            };
            assert_eq!(
                page.entries[0].id,
                checkpoint["checkpoint_id"].as_str().unwrap()
            );
            app.activate_modal(&worker).unwrap();
            assert!(matches!(app.modal, Some(checkpoints::Modal::Confirm(_))));
            press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
            press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
            assert_eq!(
                std::fs::read(app.conversation.store().path()).unwrap(),
                document
            );
            assert_eq!(app.conversation.store().history(10, None).unwrap().len(), 1);
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn theme_commands_share_the_palette_preserve_drafts_and_reject_invalid_values() {
    runtime().block_on(async {
        use crate::config::keys::Theme;
        let _theme = theme::activate(Theme::Dark, Some([17, 34, 51]));
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.terminal_background = Some([17, 34, 51]);
        app.insert_text("Keep this draft");
        app.save_draft().unwrap();
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 175).unwrap();
        app.modal = Some(command_modal(commands::Action::Theme));
        app.activate_modal(&worker).unwrap();
        let Some(checkpoints::Modal::CommandInput(input)) = &mut app.modal else { panic!("Expected theme input"); };
        input.insert("light");
        app.activate_modal(&worker).unwrap();
        assert_eq!(app.theme, Theme::Light);
        let buffer = crate::tui::test_artifacts::snapshot(&mut app, "theme-light");
        assert_eq!(buffer[(0, 0)].bg, theme::colors().canvas);
        assert_eq!(theme::colors().canvas, Color::Rgb(250, 250, 252));
        assert_eq!(app.editor.lines(), ["Keep this draft"]);
        assert_eq!(app.conversation.draft(), "Keep this draft");
        for name in ["monochrome", "auto", "dark"] {
            let (command, args) = commands::parse(&format!("/theme {name}")).unwrap();
            app.execute_command(command.action, &args, &worker).unwrap();
            crate::tui::test_artifacts::snapshot(&mut app, &format!("theme-{name}"));
        }
        assert_eq!(app.theme, Theme::Dark);
        let (command, args) = commands::parse("/theme invalid").unwrap();
        assert_eq!(app.execute_command(command.action, &args, &worker).unwrap_err().code, "INVALID_ARGUMENT");
        assert_eq!(app.theme, Theme::Dark);
        assert!(commands::parse("/theme dark extra").is_err());
        app.execute_command(command.action, &[], &worker).unwrap();
        assert!(app.status.contains("UI theme: Dark"));
        assert_eq!(app.editor.lines(), ["Keep this draft"]);
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}
