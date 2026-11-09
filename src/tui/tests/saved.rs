// Included in tui::tests to preserve the existing test identities.

#[test]
fn saved_log_deletion_requires_review_rejects_stale_state_and_keeps_the_current_draft_and_images() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            171,
        )
        .unwrap();
        let document = std::fs::read(app.conversation.store().path()).unwrap();
        app.insert_text("削除する会話の下書き");
        app.save_draft().unwrap();
        let removed = app.conversation.id().to_string();
        let directory = app.conversation.directory().to_path_buf();
        app.select_conversation(saved::SessionChange::New, &worker)
            .await
            .unwrap();
        app.insert_text("現在の会話は保持");
        app.save_draft().unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        let current = app.conversation.id().to_string();
        let images = app.conversation.images().to_vec();
        app.open_saved(saved::Page::sessions(current.clone()), None)
            .unwrap();
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert!(app.document_task.is_none());
        assert!(
            saved_page(&app)
                .notice
                .as_deref()
                .unwrap()
                .contains("Select another")
        );
        let select = |app: &mut App| {
            if let Some(checkpoints::Modal::Saved(page)) = &mut app.modal {
                page.selected = page.data["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|entry| entry["id"] == removed)
                    .unwrap();
            }
        };
        select(&mut app);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        modal_review_layouts(&mut app, "delete-conversation", "Delete local conversation?");
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
        assert!(directory.exists());
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        std::fs::write(directory.join("new-log.jsonl"), b"changed after review").unwrap();
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        assert!(directory.exists());
        assert!(
            saved_page(&app)
                .notice
                .as_deref()
                .unwrap()
                .contains("REVISION_CONFLICT")
        );
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE, &worker).await;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.document_task.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        // An interrupt delivered after the atomic deletion must not restore
        // the stale review page or describe the deletion as cancelled.
        app.document_task.as_ref().unwrap().cancel.cancel();
        app.finish_document_result(result, &worker);
        assert!(!directory.exists());
        assert!(saved_page(&app).deletion_completed);
        assert!(
            !saved_page(&app).data["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == removed)
        );
        assert_eq!(app.conversation.id(), current);
        assert_eq!(app.editor.lines().join("\n"), "現在の会話は保持");
        assert_eq!(
            serde_json::to_value(app.conversation.images()).unwrap(),
            serde_json::to_value(images).unwrap()
        );
        assert!(!temp.path().join("sent-turn.txt").exists());
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            document
        );
    });
}

#[test]
fn saved_log_selection_preserves_drafts_images_and_native_context_and_closes_the_previous_connection()
 {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "selection");
        let store = app.conversation.store().clone();
        let document = std::fs::read(store.path()).unwrap();
        let first = app.conversation.id().to_string();
        let first_directory = app.conversation.directory().to_path_buf();
        app.insert_text("最初の会話の下書き");
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        let first_images = app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>();
        let worker = Worker::start(store.clone(), vec![], Backend::halfblocks(), 151).unwrap();
        app.modal = Some(command_modal(commands::Action::NewConversation));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        let second = app.conversation.id().to_string();
        assert_ne!(first, second);
        assert!(app.conversation.draft().is_empty() && app.conversation.images().is_empty());
        app.insert_text("Second unsent draft"); app.save_draft().unwrap();
        app.open_saved(saved::Page::sessions(second.clone()), None).unwrap();
        document_result(&mut app, &worker).await;
        let index = saved_page(&app).data["entries"].as_array().unwrap().iter().position(|entry| entry["id"] == first).unwrap();
        if let Some(checkpoints::Modal::Saved(page)) = &mut app.modal { page.selected = index; }
        press(&mut app, KeyCode::Char('u'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.conversation.id(), first);
        assert_eq!(app.editor.lines().join("\n"), "最初の会話の下書き");
        assert_eq!(app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>(), first_images);
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.submit().await.unwrap();
        assert!(app.select_conversation(saved::SessionChange::New, &worker).await.is_err());
        assert_eq!(app.conversation.id(), first);
        let events = complete(&mut app).await; assert_eq!(events.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        app.insert_text("First after turn");
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        app.save_draft().unwrap();
        assert!(std::fs::read_dir(&first_directory).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with("grant-")));
        app.agent.authentication = "subscription".into();
        app.select_conversation(saved::SessionChange::Existing(second.clone()), &worker).await.unwrap();
        assert_eq!(app.editor.lines().join("\n"), "Second unsent draft");
        assert_eq!(app.agent.authentication, "checking");
        assert!(app.conversation.images().is_empty() && app.permissions.is_empty());
        assert!(!std::fs::read_dir(&first_directory).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with("grant-")));
        app.submit().await.unwrap();
        let events = complete(&mut app).await; assert_eq!(events.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        app.insert_text("Second after turn"); app.save_draft().unwrap();
        app.select_conversation(saved::SessionChange::Existing(first.clone()), &worker).await.unwrap();
        assert_eq!(app.editor.lines().join("\n"), "First after turn");
        assert_eq!(app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>(), first_images);
        let methods = temp.path().join("received-session-methods.jsonl");
        let read = || std::fs::read_to_string(&methods).unwrap().lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(read().len(), 2, "Selecting an idle conversation must not start an adapter");
        app.submit().await.unwrap();
        let events = complete(&mut app).await; assert_eq!(events.last().unwrap()["type"], "complete");
        app.conversation.close().await.unwrap();
        let methods = read();
        assert_eq!(methods.iter().map(|value| value["method"].as_str().unwrap()).collect::<Vec<_>>(), ["session/new", "session/new", "session/load"]);
        let prompts = std::fs::read_to_string(temp.path().join("received-prompts.jsonl")).unwrap().lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(prompts.iter().map(|value| value["prompt"].as_array().unwrap().iter().filter(|block| block["type"] == "image").count()).collect::<Vec<_>>(), [2, 0, 2]);
        assert_eq!(crate::conversation::history::list(&store, Some(&first), 10, None, None).unwrap()["entries"].as_array().unwrap().len(), 2);
        assert_eq!(crate::conversation::history::list(&store, Some(&second), 10, None, None).unwrap()["entries"].as_array().unwrap().len(), 1);
        assert_eq!(std::fs::read(store.path()).unwrap(), document);
    });
}

#[test]
fn saved_log_browser_pages_formats_refreshes_and_preserves_drafts_images_and_document() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.insert_text("未送信の依頼\n画像も保持"); app.save_draft().unwrap();
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        let images = app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>();
        let draft = app.conversation.draft().to_string();
        let document = std::fs::read(app.conversation.store().path()).unwrap();
        let metadata = std::fs::read(app.conversation.directory().join("session.json")).unwrap();
        let body = format!("BEGIN 日本語\n{}END 応答\n", (0..2500).map(|n| format!("line {n:04} · 保存された応答\n")).collect::<String>());
        let path = saved_log(&app, 1, &body);
        for number in 2..=55 { saved_log(&app, number, "別の応答"); }
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 139).unwrap();
        press(&mut app, KeyCode::Char('p'), KeyModifiers::ALT, &worker).await;
        for _ in 0..command_index(commands::Action::Logs) { press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await; }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 50);
        crate::tui::test_artifacts::snapshot(&mut app, "saved-turns");
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 5);
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 50);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        let first = saved_page(&app).data["text"].as_str().unwrap().to_string();
        crate::tui::test_artifacts::snapshot(&mut app, "saved-log");
        assert!(first.contains("BEGIN 日本語")); assert!(!first.contains("sessionUpdate"));
        press(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL, &worker).await;
        assert!(matches!(saved_page(&app).location, saved::Location::Text { raw: false, .. }));
        assert!(app.input.is_none());
        let mut screen = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        screen.draw(|frame| { app.draw(frame); }).unwrap();
        press(&mut app, KeyCode::End, KeyModifiers::NONE, &worker).await;
        let last_scroll = saved_page(&app).scroll; assert!(last_scroll > 0);
        press(&mut app, KeyCode::Up, KeyModifiers::NONE, &worker).await;
        assert!(saved_page(&app).scroll < last_scroll);
        press(&mut app, KeyCode::Char('v'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert!(matches!(saved_page(&app).location, saved::Location::Text { raw: true, .. }));
        assert!(saved_page(&app).data["text"].as_str().unwrap().contains("notification"));
        press(&mut app, KeyCode::Char('v'), KeyModifiers::NONE, &worker).await;
        assert_eq!(saved_page(&app).data["text"], first);
        let mut pages = vec![first];
        while saved_page(&app).data["next_cursor"].is_string() {
            press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
            pages.push(saved_page(&app).data["text"].as_str().unwrap().to_string());
        }
        assert!(pages.len() >= 2);
        assert_eq!(pages.concat(), format!("\nTurn 00000000-0000-0000-0000-000000000001\n\nYou:\n保存済みの依頼\n{body}\nRequest complete\n"));
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["text"], pages[pages.len() - 2]);
        let changed = std::fs::read_to_string(&path).unwrap().replacen("BEGIN", "BEGAN", 1);
        std::fs::write(&path, changed).unwrap();
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert!(saved_page(&app).notice.as_ref().unwrap().contains("REVISION_CONFLICT"));
        press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert!(saved_page(&app).data["text"].as_str().unwrap().contains("BEGAN 日本語"));
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE, &worker).await;
        assert!(matches!(saved_page(&app).location, saved::Location::Turns(_)));
        press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert!(matches!(saved_page(&app).location, saved::Location::Sessions));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(app.document_task.as_ref().unwrap().cancel.is_cancelled());
        document_result(&mut app, &worker).await;
        assert!(app.modal.is_none());
        assert_eq!(app.editor.lines().join("\n"), draft);
        assert_eq!(app.conversation.images().iter().map(|image| image.id.clone()).collect::<Vec<_>>(), images);
        assert_eq!(std::fs::read(app.conversation.store().path()).unwrap(), document);
        assert_eq!(std::fs::read(app.conversation.directory().join("session.json")).unwrap(), metadata);
        assert!(!app.conversation.active());
        assert!(serde_json::from_slice::<Value>(&metadata).unwrap()["native_session_id"].is_null());
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn saved_log_reads_preserve_permission_priority_and_emergency_cancel_stops_the_active_agent_too() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "cancel-permission");
        saved_log(&app, 1, "過去の応答");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            140,
        )
        .unwrap();
        app.insert_text("許可待ちでも履歴は読める");
        app.submit().await.unwrap();
        loop {
            if let ConversationEvent::Permission { id, request } =
                tokio::time::timeout(Duration::from_secs(5), app.conversation.next_event())
                    .await
                    .unwrap()
                    .unwrap()
            {
                app.permissions.push_back(Permission {
                    id,
                    request,
                    selected: 0,
                    scroll: 0,
                });
                break;
            }
        }
        app.open_saved(saved::Page::current(app.conversation.id().into()), None)
            .unwrap();
        document_result(&mut app, &worker).await;
        let selected = saved_page(&app).selected;
        for key in [KeyCode::Char('q'), KeyCode::Char('v'), KeyCode::F(1)] {
            press(&mut app, key, KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(saved_page(&app).selected, selected);
        assert_eq!(app.permissions.len(), 1);
        assert!(app.conversation.active());
        app.open_saved(
            saved::Page::current(app.conversation.id().into()),
            Some(Box::new(saved_page(&app).clone())),
        )
        .unwrap();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL, &worker).await;
        assert!(app.document_task.as_ref().unwrap().cancel.is_cancelled());
        assert!(app.permissions.is_empty());
        document_result(&mut app, &worker).await;
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "failed");
        app.sync_turn(&worker);
        assert_eq!(app.editor.lines().join("\n"), "許可待ちでも履歴は読める");
        app.conversation.close().await.unwrap();
    });
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
        press(&mut app, KeyCode::Char('p'), KeyModifiers::ALT, &worker).await;
        for _ in 0..command_index(commands::Action::Checkpoints) {
            press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        let buffer = crate::tui::test_artifacts::snapshot(&mut app, "checkpoints");
        assert!(buffer.content.iter().any(|cell| {
            cell.symbol() == "›" && cell.fg == theme::colors().preview && cell.bg == theme::colors().selected_background
        }));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::Confirm(_))));
        crate::tui::test_artifacts::snapshot(&mut app, "restore-review");
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
