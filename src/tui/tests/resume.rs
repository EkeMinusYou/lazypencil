// Included in tui::tests to share the existing process fixtures.

async fn restored_history(app: &mut App) {
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        &mut app.saved_history_task.as_mut().unwrap().receiver,
    )
    .await
    .unwrap()
    .unwrap();
    app.accept_history(result);
}

#[test]
fn explicit_startup_prompt_replaces_the_restored_draft_without_sending_the_queue() {
    let temp = TempDir::new().unwrap();
    let mut original = app(&temp, "selection");
    original.insert_text("Previous draft");
    original.enqueue_prompt("Paused follow-up".into()).unwrap();
    original.save_draft().unwrap();
    let mut args = original.conversation.settings().clone();
    args.create = false;
    args.session = Some(original.conversation.id().into());
    args.prompt = "Explicit startup draft".into();
    let executable = original.conversation.executable().to_owned();
    drop(original);
    let conversation =
        Conversation::open_with_executable(args, Some(temp.path().join("data")), executable)
            .unwrap();
    let mut app = App::new(conversation);
    app.apply_startup_prompt("Explicit startup draft").unwrap();
    assert_eq!(app.editor.lines(), ["Explicit startup draft"]);
    assert_eq!(app.conversation.draft(), "Explicit startup draft");
    let outbox = outbox::Outbox::load(app.conversation.directory()).unwrap();
    assert_eq!(outbox.draft.as_deref(), Some("Explicit startup draft"));
    assert!(outbox.paused);
    assert_eq!(outbox.queued[0].text, "Paused follow-up");
    assert!(!temp.path().join("received-prompts.jsonl").exists());
}

#[test]
fn resumed_conversation_restores_history_draft_images_and_pauses_all_pending_delivery() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut original = app(&temp, "selection");
        let document = std::fs::read(original.conversation.store().path()).unwrap();
        saved_log(&original, 1, "以前の回答です");
        original.insert_text("再開後の下書き");
        original.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        original.save_draft().unwrap();
        original.enqueue_prompt("送信待ちの依頼".into()).unwrap();
        original.outbox.steering = Some(outbox::SteeringPrompt {
            id: "unconfirmed".into(),
            text: "未確認の追加指示".into(),
        });
        original
            .outbox
            .save(original.conversation.directory())
            .unwrap();
        original.conversation.remember_for_tui().unwrap();
        let id = original.conversation.id().to_owned();
        let images: Vec<_> = original
            .pending_images()
            .iter()
            .map(|image| image.id.clone())
            .collect();
        let mut args = original.conversation.settings().clone();
        args.create = false;
        drop(original);
        let (conversation, notice) =
            Conversation::open_tui(args, Some(temp.path().join("data")), false).unwrap();
        assert!(notice.unwrap().contains("Resumed previous conversation"));
        let mut resumed = App::new(conversation);
        assert_eq!(resumed.conversation.id(), id);
        assert_eq!(resumed.editor.lines().join("\n"), "再開後の下書き");
        assert_eq!(
            resumed
                .pending_images()
                .iter()
                .map(|image| image.id.clone())
                .collect::<Vec<_>>(),
            images
        );
        assert!(resumed.outbox.paused);
        assert_eq!(resumed.outbox.queued.len(), 2);
        restored_history(&mut resumed).await;
        assert!(resumed.saved_log.contains("以前の回答です"));
        assert!(resumed.permissions.is_empty());
        assert!(resumed.tool_calls.is_empty());
        assert!(!resumed.conversation.active());
        resumed.send_queued().await.unwrap();
        assert!(!resumed.conversation.active());
        assert!(!temp.path().join("received-session-methods.jsonl").exists());
        assert!(!temp.path().join("received-prompts.jsonl").exists());
        assert_eq!(
            std::fs::read(resumed.conversation.store().path()).unwrap(),
            document
        );
    });
}

#[test]
fn conversation_history_loads_older_pages_on_scroll_and_clears_on_new_selection() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "selection");
        let body = format!("OLDEST\n{}\nLATEST", "保存済みの回答🙂\n".repeat(6000));
        saved_log(&app, 1, &body);
        app.restore_history();
        restored_history(&mut app).await;
        assert!(app.saved_log.contains("LATEST"));
        assert!(!app.saved_log.contains("OLDEST"));
        app.append("\nLive message\n");
        let mut screen =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        screen
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        while app.saved_history.is_some() {
            app.scroll_history(true, u16::MAX);
            app.load_older_history();
            restored_history(&mut app).await;
            screen
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
        }
        assert!(app.saved_log.contains("OLDEST"));
        assert_eq!(app.saved_log.matches("LATEST").count(), 1);
        assert_eq!(app.log.matches("Live message").count(), 1);
        assert!(app.permissions.is_empty() && app.tool_calls.is_empty());
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            183,
        )
        .unwrap();
        app.select_conversation(saved::SessionChange::New, &worker)
            .await
            .unwrap();
        restored_history(&mut app).await;
        assert!(app.saved_log.is_empty());
        assert!(!app.log.contains("Live message"));
    });
}
