// Included in tui::tests to preserve the existing test identities.

#[test]
fn prompt_terminal_cursor_tracks_display_cells_scroll_resize_and_focus_for_imes() {
    use ratatui::{Terminal, backend::TestBackend, layout::Position};

    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        // Exact caret positions and unwrapped labels require this fixed pane geometry.
        app.panel_layout = crate::config::keys::PanelLayout {
            nodes: 22,
            preview: 48,
            agent: 30,
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(74, 30));
        assert_eq!(terminal.backend().buffer()[(74, 30)].symbol(), "D");
        assert_eq!(terminal.backend().buffer()[(75, 30)].symbol(), "e");
        app.focus = Focus::Nodes;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        assert_eq!(terminal.backend().buffer()[(74, 30)].symbol(), "D");
        app.focus = Focus::Prompt;
        for (text, expected) in [
            ("ちょっと".to_owned(), Position::new(82, 30)),
            ("か\u{3099} 😀".to_owned(), Position::new(79, 30)),
            ("か\u{3099} 👩‍💻".to_owned(), Position::new(79, 30)),
            ("🇯🇵".to_owned(), Position::new(76, 30)),
            ("👨‍👩‍👧‍👦".to_owned(), Position::new(76, 30)),
            ("日\tA".to_owned(), Position::new(79, 30)),
            ("日".repeat(80), Position::new(95, 30)),
            ("一\n二\n三\n四\n五".to_owned(), Position::new(76, 32)),
        ] {
            app.editor = editor(&text);
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            assert!(terminal.backend().cursor_visible(), "{text:?}");
            assert_eq!(terminal.backend().cursor_position(), expected, "{text:?}");
            let cell = &terminal.backend().buffer()[expected];
            assert!(
                !cell.modifier.contains(Modifier::REVERSED),
                "Use the native cursor, not a second inverse cell"
            );
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| !cell.modifier.contains(Modifier::HIDDEN)),
                "The temporary caret marker must never reach the terminal"
            );
            assert_eq!(app.editor.lines().join("\n"), text);
        }
        let saved_editor = std::mem::replace(&mut app.editor, editor("abc"));
        app.editor.move_cursor(CursorMove::Head);
        app.editor.start_selection();
        app.editor.move_cursor(CursorMove::Forward);
        app.editor.move_cursor(CursorMove::Forward);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(76, 30));
        assert_eq!(terminal.backend().buffer()[(74, 30)].bg, Color::LightBlue);
        assert_eq!(terminal.backend().buffer()[(75, 30)].bg, Color::LightBlue);
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| !cell.modifier.contains(Modifier::HIDDEN))
        );
        app.editor = saved_editor;
        terminal.backend_mut().resize(50, 16);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(41, 12));
        app.editor = editor(&"日".repeat(80));
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(45, 10));
        let draft = app.editor.lines().join("\n");
        app.help = true;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        app.help = false;
        app.focus = Focus::Preview;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        app.focus = Focus::Prompt;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(terminal.backend().cursor_visible());
        terminal.backend_mut().resize(49, 15);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        assert_eq!(app.editor.lines().join("\n"), draft);
        assert!(!app.conversation.active());
    });
}

#[test]
fn explicit_send_resize_preserves_the_draft_original_and_persisted_dimensions() {
    use crate::image_input::ImageBounds;
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.conversation
            .attach_file(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
            )
            .unwrap();
        let original = app.conversation.images()[0].clone();
        let bytes = original
            .read_original(app.conversation.directory())
            .unwrap();
        app.editor = editor("日本語の下書きを保持");
        app.start_resize(0, Some(ImageBounds::new(80, 80).unwrap()))
            .unwrap();
        assert_eq!(
            (
                app.conversation.images()[0].width,
                app.conversation.images()[0].height
            ),
            (320, 160)
        );
        assert!(
            app.start_resize(0, Some(ImageBounds::new(40, 40).unwrap()))
                .is_err()
        );
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        let resized = &app.conversation.images()[0];
        assert_eq!((resized.width, resized.height), (80, 40));
        assert_eq!(
            resized.read_original(app.conversation.directory()).unwrap(),
            bytes
        );
        assert!(super::images::dimensions(resized).contains("320×160 → 80×40"));
        assert_eq!(app.editor.lines().join("\n"), "日本語の下書きを保持");
        let saved: Value = serde_json::from_slice(
            &std::fs::read(app.conversation.directory().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["pending_attachments"][0]["width"], 80);
        assert_eq!(saved["pending_attachments"][0]["original"]["width"], 320);
        app.start_resize(0, None).unwrap();
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images()[0].id, original.id);
    });
}

#[test]
fn cancelling_a_prepared_resize_reclaims_only_the_unadopted_send_variant() {
    use crate::image_input::ImageBounds;
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.conversation
            .attach_file(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
            )
            .unwrap();
        let original = app.conversation.images()[0].clone();
        let directory = app.conversation.directory().join("attachments");
        let names = || {
            std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        app.start_resize(0, Some(ImageBounds::new(80, 80).unwrap()))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !app.input.as_ref().unwrap().is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(names().len() > before.len());
        app.input.take();
        assert_eq!(names(), before);
        assert_eq!(app.conversation.images()[0].id, original.id);
        original
            .read_original(app.conversation.directory())
            .unwrap();
        original.read(app.conversation.directory()).unwrap();
    });
}

#[test]
fn failed_background_image_batches_leave_existing_images_and_blobs_unchanged() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        let directory = app.conversation.directory().join("attachments");
        let names = || {
            std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        let ids = app
            .conversation
            .images()
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>();
        app.input_reader = incomplete_image_batch;
        app.start_input(None).unwrap();
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert!(app.status.contains("ATTACHMENT_INVALID"));
        assert_eq!(names(), before);
        assert_eq!(
            app.conversation
                .images()
                .iter()
                .map(|image| image.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    });
}

#[test]
fn control_v_attaches_ordered_rgba_images_and_two_turns_share_one_agent() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.input_reader = fake_input;
        let picker = Backend::halfblocks();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            123,
        )
        .unwrap();
        for character in "日本語の見出し\nhjkl+-q".chars() {
            app.key(
                KeyEvent::new(
                    if character == '\n' {
                        KeyCode::Enter
                    } else {
                        KeyCode::Char(character)
                    },
                    if character == '\n' {
                        KeyModifiers::ALT
                    } else {
                        KeyModifiers::NONE
                    },
                ),
                &picker,
                &worker,
            )
            .await
            .unwrap();
        }
        assert_eq!(app.editor.lines().join("\n"), "日本語の見出し\nhjkl+-q");
        app.key(
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        assert_eq!(app.conversation.images().len(), 0);
        let content = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images().len(), 2);
        // Removing a pending image never changes its source; re-paste preserves order.
        app.conversation.remove_image(0).unwrap();
        assert_eq!(app.conversation.images()[0].display_name, "blue.png");
        app.conversation.clear_images().unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "complete");
        assert!(app.conversation.draft().is_empty());
        assert!(app.conversation.images().is_empty());
        for (index, expected) in [[255, 0, 0, 255], [0, 0, 255, 128]].iter().enumerate() {
            let image = image::open(temp.path().join(format!("received-image-1-{index}.png")))
                .unwrap()
                .to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, *expected);
        }
        let first: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("received-turn-1.json")).unwrap(),
        )
        .unwrap();
        assert!(
            first["text"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "日本語の見出し\nhjkl+-q")
        );
        app.editor = editor("次の依頼");
        app.draft_changed = true;
        app.submit().await.unwrap();
        let events = complete(&mut app).await;
        assert!(
            !events.iter().any(|value| value["type"] == "ready"),
            "Second turn must reuse the live connection"
        );
        assert!(
            events
                .iter()
                .any(|value| value["notification"]["update"]["content"]["text"] == "完了 2\n")
        );
        app.conversation.close().await.unwrap();
        let second: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("received-turn-2.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(second["images"], json!([]));
        let logs = std::fs::read_dir(app.conversation.directory())
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl"))
                .then(|| std::fs::read_to_string(path).unwrap())
            })
            .collect::<String>();
        assert!(
            !logs.contains("iVBOR"),
            "Transcripts must omit image payloads"
        );
    });
}

#[test]
fn failed_image_turn_keeps_draft_for_restart_and_does_not_send_text_only() {
    let temp = TempDir::new().unwrap();
    let (id, directory, mut args) = runtime().block_on(async {
        let mut app = app(&temp, "no-images");
        app.insert_text("参考画像を使ってください");
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.submit().await.unwrap();
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "failed");
        assert_eq!(app.conversation.draft(), "参考画像を使ってください");
        assert_eq!(app.conversation.images().len(), 2);
        assert!(!temp.path().join("sent-turn.txt").exists());
        let result = (
            app.conversation.id().to_string(),
            app.conversation.directory().to_path_buf(),
            app.conversation.settings().clone(),
        );
        app.conversation.close().await.unwrap();
        result
    });
    args.create = false;
    args.session = Some(id);
    args.prompt.clear();
    let conversation = Conversation::open(args, Some(temp.path().join("data"))).unwrap();
    assert_eq!(conversation.draft(), "参考画像を使ってください");
    assert_eq!(conversation.images().len(), 2);
    assert_eq!(conversation.directory(), directory);
}

#[test]
fn custom_prompt_keys_keep_focus_and_modal_precedence_and_send_actual_images() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.keys = crate::config::keys::PromptKeys {
            submit: vec!["F5".into()],
            paste: vec!["F6".into()],
        };
        app.keys.validate().unwrap();
        app.input_reader = fake_input;
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            135,
        )
        .unwrap();
        app.editor = editor("日本語の依頼");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines().join("\n"), "日本語の依頼\n");
        press(&mut app, KeyCode::Enter, KeyModifiers::ALT, &worker).await;
        assert!(
            !app.conversation.active(),
            "Replaced submission keys must not submit"
        );
        press(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL, &worker).await;
        assert!(
            app.input.is_none(),
            "Replaced paste keys must not access the clipboard"
        );
        for focus in [
            Focus::Preview,
            Focus::Nodes,
            Focus::Attachments,
            Focus::Conversation,
        ] {
            app.focus = focus;
            press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
            press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
            assert!(app.input.is_none() && !app.conversation.active());
        }
        app.focus = Focus::Prompt;
        app.help = true;
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        assert!(app.input.is_none());
        app.help = false;
        app.modal = Some(checkpoints::Modal::commands());
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        assert!(!app.conversation.active());
        app.modal = None;
        app.searching = true;
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        assert!(!app.conversation.active());
        app.searching = false;
        app.editor = editor("日本語の依頼");
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        let content = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images().len(), 2);
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "complete");
        assert!(app.conversation.draft().is_empty());
        for (index, expected) in [[255, 0, 0, 255], [0, 0, 255, 128]].iter().enumerate() {
            let received = image::open(temp.path().join(format!("received-image-1-{index}.png")))
                .unwrap()
                .to_rgba8();
            assert_eq!(received.get_pixel(0, 0).0, *expected);
        }
        app.conversation.close().await.unwrap();
    });
}
