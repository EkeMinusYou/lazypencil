// Included in tui::tests to preserve the existing test identities.

#[test]
fn prompt_soft_wraps_typing_and_grows_without_changing_the_draft() {
    use ratatui::{Terminal, backend::TestBackend, layout::Position};

    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let mut terminal = Terminal::new(TestBackend::new(28, 20)).unwrap();
        let draw = |app: &mut App, terminal: &mut Terminal<TestBackend>| {
            app.pointer.begin();
            terminal
                .draw(|frame| app.draw_conversation(frame, frame.area()))
                .unwrap();
            let prompt = app
                .pointer
                .area(mouse::Target::Focus(Focus::Prompt))
                .unwrap();
            composer::prompt_block(true).inner(prompt)
        };
        let input = draw(&mut app, &mut terminal);
        assert_eq!(input.width, 20);
        let first_row = "abcdefghijABCDEFGHIJ";
        for ch in first_row.chars().chain(['K']) {
            app.editor
                .input(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            draw(&mut app, &mut terminal);
        }
        assert_eq!(
            terminal.backend().buffer()[(input.x, input.y)].symbol(),
            "a"
        );
        assert_eq!(
            terminal.backend().buffer()[(input.x, input.y + 1)].symbol(),
            "K"
        );
        assert_eq!(
            terminal.backend().cursor_position(),
            Position::new(input.x + 1, input.y + 1)
        );
        app.editor
            .input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        draw(&mut app, &mut terminal);
        assert_eq!(app.editor.cursor(), (0, 1));
        app.editor
            .input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.editor
            .input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        draw(&mut app, &mut terminal);
        assert!(
            terminal.backend().cursor_visible(),
            "A full row retains its caret"
        );
        assert_eq!(app.editor.lines(), [first_row]);

        let text = format!("{}終", "日".repeat(39));
        app.editor = editor("");
        app.insert_text(&text);
        let input = draw(&mut app, &mut terminal);
        assert_eq!(input.height, 4);
        assert_eq!(
            terminal.backend().buffer()[(input.x + 18, input.y + 3)].symbol(),
            "終"
        );
        assert_eq!(app.editor.scroll_offset(), (0, 0));
        app.save_draft().unwrap();
        assert_eq!(app.conversation.draft(), text);

        terminal.backend_mut().resize(48, 20);
        let input = draw(&mut app, &mut terminal);
        assert_eq!(input.height, 3);
        assert_eq!(app.editor.scroll_offset(), (0, 0));
        assert_eq!(app.editor.lines(), [text]);
        assert!(!app.conversation.active());
    });
}

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
            ("日".repeat(80), Position::new(80, 32)),
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
        assert_eq!(terminal.backend().cursor_position(), Position::new(43, 12));
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
        app.remove_pending_image(0).unwrap();
        assert_eq!(app.conversation.images()[0].display_name, "blue.png");
        app.clear_pending_images().unwrap();
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

fn pixel_input(pixel: [u8; 4]) -> Result<InputContent> {
    let image = crate::image_input::from_rgba8(2, 2, &pixel.repeat(4))?;
    Ok(InputContent::Images {
        images: vec![crate::clipboard::InputImage {
            name: "reference.png".into(),
            source_bytes: image.png.len(),
            data: base64::engine::general_purpose::STANDARD.encode(image.png),
        }],
    })
}

fn received_pixel(temp: &TempDir, turn: usize, pixel: [u8; 4], text: &str) {
    let received: Value = serde_json::from_slice(
        &std::fs::read(temp.path().join(format!("received-turn-{turn}.json"))).unwrap(),
    ).unwrap();
    assert_eq!(received["images"].as_array().unwrap().len(), 1);
    let path = temp.path().join(received["images"][0].as_str().unwrap());
    assert_eq!(image::open(path).unwrap().to_rgba8().get_pixel(0, 0).0, pixel);
    if !text.is_empty() {
        assert!(received["text"].as_array().unwrap().iter().any(|value| value == text));
    }
}

#[test]
fn follow_up_images_are_captured_per_prompt_without_erasing_the_next_draft() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        let picker = Backend::halfblocks();
        let worker = Worker::start(app.conversation.store().clone(), vec![], picker.clone(), 529).unwrap();
        app.focus = Focus::Prompt;
        app.accept_input(pixel_input([255, 0, 0, 255]));
        app.insert_text("First red request");
        app.submit().await.unwrap();
        assert!(app.conversation.active());
        assert!(app.pending_images().is_empty());

        // Exercise the same explicit Ctrl+V path as the reported failure.
        app.input_reader = fake_input;
        press(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL, &worker).await;
        let result = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(result);
        assert_eq!(app.pending_images().len(), 2);
        let buffer = crate::tui::test_artifacts::snapshot(&mut app, "images-during-work");
        let screen = buffer.content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(screen.contains("Attachments") && screen.contains("blue.png"));
        app.remove_pending_image(0).unwrap();
        app.insert_text("Second blue request");
        app.submit().await.unwrap();
        assert_eq!(app.outbox.queued.len(), 1);
        assert!(app.pending_images().is_empty());

        // File input and an image-only follow-up must work with a queued prompt too.
        let path = temp.path().join("green.png");
        std::fs::write(&path, crate::image_input::from_rgba8(2, 2, &[0, 255, 0, 255].repeat(4)).unwrap().png).unwrap();
        app.input_reader = read_input;
        app.execute_command(commands::Action::Attach, &[path.to_str().unwrap().into()], &worker).unwrap();
        let result = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(result);
        assert_eq!(app.pending_images().len(), 1);
        app.submit().await.unwrap();
        assert_eq!(app.outbox.queued.len(), 2);
        assert!(app.outbox.queued[1].text.is_empty());
        app.queue_prompt("Fourth text-only request".into()).unwrap();

        app.accept_input(pixel_input([255, 255, 0, 255]));
        app.insert_text("Unsent yellow draft");
        let yellow = app.pending_images()[0].clone();
        for (turn, pixel, text) in [
            (1, [255, 0, 0, 255], "First red request"),
            (2, [0, 0, 255, 128], "Second blue request"),
            (3, [0, 255, 0, 255], ""),
        ] {
            if turn > 1 { app.send_queued().await.unwrap(); }
            assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
            received_pixel(&temp, turn, pixel, text);
            assert_eq!(app.editor.lines(), ["Unsent yellow draft"]);
            assert_eq!(app.pending_images()[0].id, yellow.id);
            yellow.read(app.conversation.directory()).unwrap();
        }
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        let fourth: Value = serde_json::from_slice(&std::fs::read(temp.path().join("received-turn-4.json")).unwrap()).unwrap();
        assert_eq!(fourth["images"], json!([]), "Text-only queued requests cannot inherit the next draft's image");
        assert!(fourth["text"].as_array().unwrap().iter().any(|text| text == "Fourth text-only request"));
        assert_eq!(app.editor.lines(), ["Unsent yellow draft"]);
        assert_eq!(app.pending_images()[0].id, yellow.id);
        assert!(app.outbox.queued.is_empty());
        assert_eq!(app.conversation.images()[0].id, yellow.id);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn interrupted_image_queue_retains_each_snapshot_and_accepts_images_while_paused() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.accept_input(pixel_input([255, 0, 0, 255]));
        app.insert_text("Interrupted red request");
        app.submit().await.unwrap();
        app.accept_input(pixel_input([0, 0, 255, 255]));
        app.insert_text("Queued blue request");
        app.submit().await.unwrap();
        app.accept_input(pixel_input([0, 255, 0, 255]));
        app.insert_text("Unsent green draft");
        app.save_draft().unwrap();
        let mut args = app.conversation.settings().clone();
        args.create = false;
        args.session = Some(app.conversation.id().into());
        let executable = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("lazypencil");
        // Leave the outbox's delivery marker intact, as when a frontend exits mid-turn.
        app.conversation.close().await.unwrap();
        drop(app);
        let conversation = Conversation::open_with_executable(args, Some(temp.path().join("data")), executable).unwrap();
        let mut app = App::new(conversation);
        assert!(!app.conversation.active());
        assert!(app.outbox.paused);
        assert_eq!(app.outbox.queued.len(), 2);
        assert_eq!(app.outbox.queued[0].text, "Interrupted red request");
        assert_eq!(app.outbox.queued[1].text, "Queued blue request");
        assert_eq!(app.editor.lines(), ["Unsent green draft"]);
        assert_eq!(app.pending_images().len(), 1);
        app.send_queued().await.unwrap();
        assert!(!app.conversation.active(), "Reopening cannot automatically resend interrupted work");

        // Removing/replacing the draft must retain files captured by paused requests.
        app.clear_pending_images().unwrap();
        app.accept_input(pixel_input([255, 0, 0, 255]));
        app.remove_pending_image(0).unwrap();
        app.accept_input(pixel_input([255, 255, 0, 255]));
        assert_eq!(app.pending_images().len(), 1, "Pasting is permitted while idle with a paused queue");
        for prompt in &app.outbox.queued {
            assert_eq!(prompt.images.len(), 1);
            prompt.images[0].read(app.conversation.directory()).unwrap();
            prompt.images[0].read_original(app.conversation.directory()).unwrap();
        }
        app.outbox.paused = false;
        for (turn, pixel, text) in [
            (1, [255, 0, 0, 255], "Interrupted red request"),
            (2, [0, 0, 255, 255], "Queued blue request"),
        ] {
            app.send_queued().await.unwrap();
            assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
            received_pixel(&temp, turn, pixel, text);
            assert_eq!(app.pending_images().len(), 1);
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn image_follow_ups_queue_instead_of_steering_and_survive_cancellation() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "steering-flood");
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 530).unwrap();
        app.insert_text("First request");
        app.submit().await.unwrap();
        steering_working(&mut app).await;
        app.accept_input(pixel_input([255, 0, 0, 255]));
        assert!(footer_tests::rows(&app, 120).last().unwrap().contains("queue"));
        app.insert_text("Image follow-up");
        app.submit().await.unwrap();
        assert!(app.outbox.steering.is_none());
        assert_eq!(app.outbox.queued.len(), 1);
        app.accept_input(pixel_input([0, 255, 0, 255]));
        app.insert_text("Newer draft");
        app.conversation.cancel();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "failed");
        app.sync_turn(&worker);
        assert!(app.outbox.paused);
        assert_eq!(app.outbox.queued.len(), 2);
        assert_eq!(app.outbox.queued[1].text, "Image follow-up");
        assert_eq!(app.outbox.queued[1].images.len(), 1);
        assert_eq!(app.editor.lines(), ["Newer draft"]);
        assert_eq!(app.pending_images().len(), 1);
        assert!(!temp.path().join("received-steering.jsonl").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn cancelling_while_an_image_is_being_prepared_keeps_failed_and_next_images_separate() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 531).unwrap();
        app.accept_input(pixel_input([255, 0, 0, 255]));
        app.insert_text("Cancelled image request");
        app.submit().await.unwrap();
        app.input_reader = fake_input;
        app.start_input(None).unwrap();
        app.conversation.cancel();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "failed");
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(app.outbox.queued[0].images.len(), 1);
        let result = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(result);
        assert_eq!(app.pending_images().len(), 2, "The failed request's image cannot be merged into the new paste");
        assert_eq!(app.pending_images()[0].display_name, "red.png");
        assert_eq!(app.pending_images()[1].display_name, "blue.png");
        assert!(app.outbox.paused);
        app.sync_turn(&worker);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn legacy_queued_image_ids_migrate_without_becoming_next_draft_images() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.accept_input(pixel_input([255, 0, 0, 255]));
        let id = app.pending_images()[0].id.clone();
        std::fs::write(app.conversation.directory().join("prompt-outbox.json"), json!({
            "draft":"New legacy draft", "paused":true,
            "queued":[{"text":"Legacy red request", "selection":null, "image_ids":[id]}]
        }).to_string()).unwrap();
        app.restore_outbox();
        assert!(app.pending_images().is_empty());
        assert_eq!(app.outbox.queued[0].images[0].id, id);
        app.accept_input(pixel_input([0, 0, 255, 255]));
        app.outbox.paused = false;
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        received_pixel(&temp, 1, [255, 0, 0, 255], "Legacy red request");
        assert_eq!(app.editor.lines(), ["New legacy draft"]);
        assert_eq!(app.pending_images().len(), 1);
        assert_ne!(app.pending_images()[0].id, id);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn clearing_an_image_queue_reclaims_only_blobs_unused_by_the_draft() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    app.accept_input(pixel_input([255, 0, 0, 255]));
    let red = app.pending_images()[0].clone();
    app.queue_prompt("Red follow-up".into()).unwrap();
    app.accept_input(pixel_input([0, 0, 255, 255]));
    let blue = app.pending_images()[0].clone();
    app.queue_prompt("Blue follow-up".into()).unwrap();
    app.accept_input(pixel_input([0, 0, 255, 255]));
    app.clear_queued_prompts().unwrap();
    assert!(app.outbox.queued.is_empty());
    assert!(red.read(app.conversation.directory()).is_err());
    assert!(red.read_original(app.conversation.directory()).is_err());
    blue.read(app.conversation.directory()).unwrap();
    blue.read_original(app.conversation.directory()).unwrap();
    app.clear_pending_images().unwrap();
    assert!(blue.read(app.conversation.directory()).is_err());
    assert!(blue.read_original(app.conversation.directory()).is_err());
}

#[test]
fn an_outbox_write_failure_preserves_the_queued_images_and_the_next_draft() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.accept_input(pixel_input([255, 0, 0, 255]));
        app.queue_prompt("Red request".into()).unwrap();
        app.accept_input(pixel_input([0, 0, 255, 255]));
        app.insert_text("Unsent blue draft");
        app.save_draft().unwrap();
        let path = app.conversation.directory().join("prompt-outbox.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(app.send_queued().await.is_err());
        assert!(!app.conversation.active());
        assert!(app.outbox.paused && app.outbox.in_flight.is_none());
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(app.outbox.queued[0].images.len(), 1);
        assert_eq!(app.pending_images().len(), 1);
        assert_eq!(app.editor.lines(), ["Unsent blue draft"]);
        std::fs::remove_dir(&path).unwrap();
        app.outbox.paused = false;
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        received_pixel(&temp, 1, [255, 0, 0, 255], "Red request");
        assert_eq!(app.pending_images().len(), 1);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn startup_images_extend_the_restored_draft_without_changing_queued_snapshots() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "image-inspect");
    app.accept_input(pixel_input([255, 0, 0, 255]));
    app.queue_prompt("Queued red image".into()).unwrap();
    let red = app.outbox.queued[0].images[0].id.clone();
    app.accept_input(pixel_input([0, 0, 255, 255]));
    app.insert_text("Unsent blue draft");
    app.save_draft().unwrap();
    let mut args = app.conversation.settings().clone();
    args.create = false;
    args.session = Some(app.conversation.id().into());
    let executable = app.conversation.executable().to_owned();
    drop(app);
    let conversation = Conversation::open_with_executable(args, Some(temp.path().join("data")), executable).unwrap();
    let mut app = App::new(conversation);
    let path = temp.path().join("startup-green.png");
    std::fs::write(&path, crate::image_input::from_rgba8(2, 2, &[0, 255, 0, 255].repeat(4)).unwrap().png).unwrap();
    app.attach_startup_images(&[path]).unwrap();
    assert_eq!(app.pending_images().len(), 2);
    assert_eq!(app.pending_images()[1].display_name, "startup-green.png");
    assert_eq!(app.conversation.images().len(), 2);
    assert_eq!(app.editor.lines(), ["Unsent blue draft"]);
    assert_eq!(app.outbox.queued.len(), 1);
    assert_eq!(app.outbox.queued[0].images[0].id, red);
    assert!(app.outbox.paused);
}

#[test]
fn slash_queue_captures_draft_images_with_normal_and_modified_submit() {
    for modifiers in [KeyModifiers::NONE, KeyModifiers::CONTROL] {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut app = app(&temp, "echo");
            let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 532).unwrap();
            app.insert_text("First request");
            app.submit().await.unwrap();
            app.accept_input(pixel_input([255, 0, 0, 255]));
            app.insert_text("/queue Image follow-up");
            press(&mut app, KeyCode::Enter, modifiers, &worker).await;
            assert_eq!(app.outbox.queued.len(), 1);
            assert_eq!(app.outbox.queued[0].text, "Image follow-up");
            assert_eq!(app.outbox.queued[0].images.len(), 1);
            assert!(app.pending_images().is_empty());
            assert_eq!(app.editor.lines(), [""]);
            app.conversation.close().await.unwrap();
        });
    }
}
