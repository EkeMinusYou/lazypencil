use super::graphics::Backend;
use super::*;
use base64::Engine;
use clap::Parser;
use serde_json::json;
use tempfile::TempDir;

fn app(temp: &TempDir, mode: &str) -> App {
    let args = crate::cli::Arguments::try_parse_from([
        "lazypencil".into(),
        "tui".into(),
        temp.path().join("design.pen").into_os_string(),
        "--create".into(),
        "--adapter".into(),
        std::env::var_os("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
        format!(
            "--adapter-arg={}/tests/support/acp_fixture.py",
            env!("CARGO_MANIFEST_DIR")
        )
        .into(),
        format!("--adapter-arg={mode}").into(),
    ])
    .unwrap();
    let crate::cli::Command::Tui(args) = args.command else {
        unreachable!()
    };
    App::new(Conversation::open(args.prompt, Some(temp.path().join("data"))).unwrap())
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn fake_clipboard(_: &Path, path: Option<&Path>, _: &CancellationToken) -> Result<InputContent> {
    assert!(path.is_none());
    let mut images = vec![];
    for (name, rgba) in [
        ("red.png", [255, 0, 0, 255]),
        ("blue.png", [0, 0, 255, 128]),
    ] {
        let image = crate::image_input::from_rgba8(1, 1, &rgba)?;
        images.push(crate::clipboard::InputImage {
            name: name.into(),
            source_bytes: image.png.len(),
            data: base64::engine::general_purpose::STANDARD.encode(image.png),
        });
    }
    Ok(InputContent::Images { images })
}
async fn complete(app: &mut App) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = vec![];
        loop {
            match app.conversation.next_event().await.unwrap() {
                ConversationEvent::Message(value) => {
                    let done = matches!(value["type"].as_str(), Some("complete" | "failed"));
                    events.push(value);
                    if done {
                        return events;
                    }
                }
                ConversationEvent::Permission { .. } => panic!("Unexpected permission"),
            }
        }
    })
    .await
    .unwrap()
}

#[test]
fn control_v_attaches_ordered_rgba_images_and_two_turns_share_one_agent() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.input_reader = fake_clipboard;
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
                    KeyModifiers::NONE,
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
        app.accept_input(content);
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
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
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
fn permission_modal_can_answer_and_cancel_without_blocking_the_event_loop() {
    for cancel in [false, true] {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut app = app(
                &temp,
                if cancel {
                    "cancel-permission"
                } else {
                    "interactive-permission"
                },
            );
            let picker = Backend::halfblocks();
            let worker = Worker::start(
                app.conversation.store().clone(),
                vec![],
                picker.clone(),
                124,
            )
            .unwrap();
            app.insert_text("許可を確認する");
            app.submit().await.unwrap();
            loop {
                let event =
                    tokio::time::timeout(Duration::from_secs(5), app.conversation.next_event())
                        .await
                        .unwrap()
                        .unwrap();
                if let ConversationEvent::Permission { id, request } = event {
                    app.permissions.push_back(Permission {
                        id,
                        request,
                        selected: 0,
                        scroll: 0,
                    });
                    break;
                }
            }
            app.key(
                KeyEvent::new(
                    if cancel {
                        KeyCode::Char('c')
                    } else {
                        KeyCode::Enter
                    },
                    if cancel {
                        KeyModifiers::CONTROL
                    } else {
                        KeyModifiers::NONE
                    },
                ),
                &picker,
                &worker,
            )
            .await
            .unwrap();
            let events = complete(&mut app).await;
            assert_eq!(
                events.last().unwrap()["type"],
                if cancel { "failed" } else { "complete" },
                "{events:?}"
            );
            if cancel {
                assert_eq!(app.conversation.draft(), "許可を確認する");
            }
            app.conversation.close().await.unwrap();
        });
    }
}

#[test]
fn queued_completion_does_not_erase_the_next_draft() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            126,
        )
        .unwrap();
        app.insert_text("最初の依頼");
        app.submit().await.unwrap();
        loop {
            if let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap()
                && value["type"] == "turn_complete"
            {
                break;
            }
        }
        assert!(!app.conversation.active());
        app.sync_turn(&worker);
        app.insert_text("次の依頼は消えない");
        let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap() else {
            panic!("Expected completion")
        };
        assert_eq!(value["type"], "complete");
        app.message(&value);
        app.sync_turn(&worker);
        assert_eq!(app.editor.lines().join("\n"), "次の依頼は消えない");
        app.save_draft().unwrap();
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn maximum_draft_survives_restart() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    let text = "\n".repeat(1024 * 1024);
    app.conversation.set_draft(text.clone()).unwrap();
    let mut args = app.conversation.settings().clone();
    args.create = false;
    args.session = Some(app.conversation.id().to_string());
    drop(app);
    let conversation = Conversation::open(args, Some(temp.path().join("data"))).unwrap();
    assert_eq!(conversation.draft(), text);
}

fn checkpoint(app: &App, fill: Value, request: &str) -> Value {
    let store = app.conversation.store();
    store.transact(request, &store.read().unwrap().revision(), request, |_| {
        let document = crate::document::Document::parse(json!({"version":"2.21","children":[{"id":"box","type":"rectangle","width":80,"height":40,"fill":fill}]}).to_string())?;
        Ok((document, json!({})))
    }).unwrap()
}
async fn document_result(app: &mut App, worker: &Worker) {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        &mut app.document_task.as_mut().unwrap().receiver,
    )
    .await
    .unwrap()
    .unwrap();
    app.document_task.take();
    app.document_result(result, worker);
}
async fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers, worker: &Worker) {
    assert!(
        !app.key(
            KeyEvent::new(code, modifiers),
            &Backend::halfblocks(),
            worker
        )
        .await
        .unwrap()
    );
}

#[test]
fn checkpoint_palette_reviews_before_restoring_and_keeps_draft_and_images() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let first = checkpoint(&app, json!("#FF0000"), "first");
        checkpoint(&app, json!("#0000FF"), "second");
        app.insert_text("未送信の依頼");
        app.save_draft().unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        let images = app
            .conversation
            .images()
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            127,
        )
        .unwrap();
        press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::Confirm(_))));
        assert_eq!(
            app.conversation
                .store()
                .read()
                .unwrap()
                .node("box")
                .unwrap()["fill"],
            "#0000FF"
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::History(_))));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        assert!(app.modal.is_none());
        let store = app.conversation.store();
        assert_eq!(
            store.read().unwrap().node("box").unwrap()["fill"],
            "#FF0000"
        );
        let history = store.history(10, None).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(
            history[0].result["restored_checkpoint"],
            first["checkpoint_id"]
        );
        assert_eq!(app.editor.lines().join("\n"), "未送信の依頼");
        assert_eq!(app.conversation.draft(), "未送信の依頼");
        assert_eq!(
            app.conversation
                .images()
                .iter()
                .map(|image| image.id.clone())
                .collect::<Vec<_>>(),
            images
        );
        assert!(!temp.path().join("sent-turn.txt").exists());
    });
}

#[test]
fn checkpoint_review_rejects_new_document_or_dependency_changes() {
    for asset_change in [false, true] {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut app = app(&temp, "echo");
            let fill = if asset_change {
                std::fs::write(
                    temp.path().join("image.png"),
                    crate::image_input::from_rgba8(1, 1, &[255, 0, 0, 255])
                        .unwrap()
                        .png,
                )
                .unwrap();
                json!({"type":"image","url":"image.png"})
            } else {
                json!("#FF0000")
            };
            checkpoint(&app, fill.clone(), "first");
            checkpoint(&app, fill, "second");
            let worker = Worker::start(
                app.conversation.store().clone(),
                vec![],
                Backend::halfblocks(),
                128,
            )
            .unwrap();
            app.open_history(vec![None]).unwrap();
            document_result(&mut app, &worker).await;
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            let current = if asset_change {
                std::fs::write(
                    temp.path().join("image.png"),
                    crate::image_input::from_rgba8(1, 1, &[0, 0, 255, 255])
                        .unwrap()
                        .png,
                )
                .unwrap();
                app.conversation.store().read().unwrap().revision()
            } else {
                checkpoint(&app, json!("#00FF00"), "external")["revision"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            let count = app.conversation.store().history(10, None).unwrap().len();
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            document_result(&mut app, &worker).await;
            assert_eq!(app.conversation.store().read().unwrap().revision(), current);
            assert_eq!(
                app.conversation.store().history(10, None).unwrap().len(),
                count
            );
            let Some(checkpoints::Modal::History(page)) = &app.modal else {
                panic!("Failed restoration must return to the list")
            };
            assert!(page.notice.as_ref().unwrap().contains("REVISION_CONFLICT"));
            press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE, &worker).await;
            document_result(&mut app, &worker).await;
            let Some(checkpoints::Modal::History(page)) = &app.modal else {
                panic!("Expected fresh list")
            };
            assert_eq!(page.revision, current);
            assert!(page.notice.is_none());
        });
    }
}

#[cfg(unix)]
#[test]
fn checkpoint_restore_cancellation_keeps_the_ui_responsive_while_locked() {
    use fs2::FileExt;
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        checkpoint(&app, json!("#FF0000"), "first");
        checkpoint(&app, json!("#0000FF"), "second");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            129,
        )
        .unwrap();
        app.open_history(vec![None]).unwrap();
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(app.conversation.store().lock_file_path())
            .unwrap();
        lock.lock_exclusive().unwrap();
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        // Let the worker enter its lock wait, then exercise an actual TUI cancellation key.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let start = std::time::Instant::now();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL, &worker).await;
        document_result(&mut app, &worker).await;
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            app.conversation
                .store()
                .read()
                .unwrap()
                .node("box")
                .unwrap()["fill"],
            "#0000FF"
        );
        assert!(app.status.contains("CANCELLED"));
        drop(lock);
        assert_eq!(app.conversation.store().history(10, None).unwrap().len(), 2);
    });
}

#[test]
fn checkpoint_history_pages_cover_the_boundary_without_duplicates_or_missing_entries() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        for index in 1..=51 {
            checkpoint(
                &app,
                json!(format!("#{index:06X}")),
                &format!("checkpoint-{index}"),
            );
        }
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            130,
        )
        .unwrap();
        app.open_history(vec![None]).unwrap();
        document_result(&mut app, &worker).await;
        let Some(checkpoints::Modal::History(page)) = &app.modal else {
            panic!("Expected first page")
        };
        assert_eq!(page.entries.len(), 50);
        assert_eq!(page.entries.first().unwrap().sequence, 51);
        assert_eq!(page.entries.last().unwrap().sequence, 2);
        press(&mut app, KeyCode::PageDown, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        let Some(checkpoints::Modal::History(page)) = &app.modal else {
            panic!("Expected older page")
        };
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].sequence, 1);
        assert!(page.next_before.is_none());
        press(&mut app, KeyCode::PageUp, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        let Some(checkpoints::Modal::History(page)) = &app.modal else {
            panic!("Expected latest page")
        };
        assert_eq!(page.entries.len(), 50);
        assert_eq!(page.entries[0].sequence, 51);
        assert_eq!(page.cursors, vec![None]);
    });
}
