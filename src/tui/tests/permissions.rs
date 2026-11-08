// Included in tui::tests to share isolated process and terminal fixtures.
async fn permission_ready(app: &mut App) {
    app.conversation.connect().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = app.conversation.next_event().await.unwrap();
            if let ConversationEvent::Message(value) = event {
                app.message(&value);
                if value["type"] == "ready" {
                    break;
                }
                assert_ne!(value["type"], "connection_error", "{value}");
            }
        }
    })
    .await
    .unwrap();
}
async fn permission_changed(app: &mut App) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while app.conversation.modes().pending().is_some() {
            if let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap()
            {
                app.message(&value);
            }
        }
    })
    .await
    .unwrap();
    app.permission_progress();
}
#[test]
fn permission_picker_preserves_drafts_confirms_full_access_and_reports_actual_mode() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions");
        app.insert_text("Keep this draft");
        permission_ready(&mut app).await;
        assert_eq!(app.conversation.modes().available.len(), 3);
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(2)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        crate::tui::test_artifacts::snapshot(&mut app, "permission-full-confirm");
        assert!(app.conversation.modes().pending().is_none());
        assert!(!temp.path().join("received-mode-changes.jsonl").exists());
        app.permission_click(permissions::Hit::Decision(false))
            .unwrap();
        assert!(app.conversation.modes().pending().is_none());
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Decision(true))
            .unwrap();
        permission_changed(&mut app).await;
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("agent-full-access")
        );
        assert!(app.header_line().to_string().contains("agent-full-access"));
        assert_eq!(app.editor.lines().join("\n"), "Keep this draft");
        assert!(!temp.path().join("sent-turn.txt").exists());
        assert!(!temp.path().join("lazypencil.toml").exists());
        app.conversation.close().await.unwrap();
    });
}
#[test]
fn permission_picker_renders_wide_compact_and_minimum_terminal_sizes() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions");
        permission_ready(&mut app).await;
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        for (width, height) in [(150, 42), (80, 24), (50, 16)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let rendered = buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Agent permissions"));
            assert!(rendered.contains("[Continue]"));
            assert!(
                app.pointer
                    .area(mouse::Target::AgentPermissions(permissions::Hit::Row(1)))
                    .is_some()
            );
            crate::tui::test_artifacts::artifact(buffer, &format!("permissions-{width}"));
        }
        app.conversation.close().await.unwrap();
    });
}
#[test]
fn permission_failure_pauses_queue_and_keeps_draft_and_reported_mode() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions-unconfirmed");
        permission_ready(&mut app).await;
        app.insert_text("Draft after failure");
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        permission_changed(&mut app).await;
        assert!(app.conversation.modes().error.is_some());
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("read-only")
        );
        assert!(app.outbox.paused);
        assert_eq!(app.editor.lines().join("\n"), "Draft after failure");
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn permission_project_save_requires_review_and_trust_and_survives_first_prompt() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions");
        permission_ready(&mut app).await;
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Scope(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        crate::tui::test_artifacts::snapshot(&mut app, "permission-project-review");
        assert!(!temp.path().join("lazypencil.toml").exists());
        assert!(!temp.path().join("received-mode-changes.jsonl").exists());
        app.permission_click(permissions::Hit::Continue).unwrap();
        permission_changed(&mut app).await;
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("workspace-write")
        );
        assert!(app.conversation.modes().saved.is_some());
        assert!(
            std::fs::read_to_string(temp.path().join("lazypencil.toml"))
                .unwrap()
                .contains("workspace-write")
        );
        app.conversation
            .settings()
            .project_execution
            .as_ref()
            .unwrap()
            .verify(app.conversation.store())
            .unwrap();
        app.conversation
            .set_draft("Check the result".into())
            .unwrap();
        app.conversation.send(None).await.unwrap();
        complete(&mut app).await;
        assert!(app.conversation.result().unwrap().is_ok());
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("workspace-write")
        );
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn permission_pending_change_keeps_tool_approval_and_can_be_cancelled() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions-hold");
        permission_ready(&mut app).await;
        app.conversation
            .set_draft("Hold for approval".into())
            .unwrap();
        app.conversation.send(None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match app.conversation.next_event().await.unwrap() {
                    ConversationEvent::Permission { id, request } => {
                        app.permissions.push_back(Permission {
                            id,
                            request,
                            selected: 1,
                            scroll: 0,
                        });
                        break;
                    }
                    ConversationEvent::Message(value) => app.message(&value),
                }
            }
        })
        .await
        .unwrap();
        let permission_id = app.permissions.front().unwrap().id.clone();
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        crate::tui::test_artifacts::snapshot(&mut app, "permission-busy-confirm");
        assert!(app.conversation.modes().pending().is_none());
        app.permission_click(permissions::Hit::Decision(true))
            .unwrap();
        assert_eq!(app.conversation.modes().pending(), Some("workspace-write"));
        assert!(!temp.path().join("received-mode-changes.jsonl").exists());
        app.permission_click(permissions::Hit::Close).unwrap();
        assert_eq!(app.permissions.front().unwrap().id, permission_id);
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::CancelPending)
            .unwrap();
        assert!(app.conversation.modes().pending().is_none());
        assert_eq!(app.permissions.front().unwrap().id, permission_id);
        app.conversation
            .respond(&permission_id, Some("deny".into()), true)
            .unwrap();
        complete(&mut app).await;
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn permission_default_save_waits_off_input_path_and_survives_cancelled_event_polls() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions");
        permission_ready(&mut app).await;
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Scope(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        let configuration = app
            .conversation
            .store()
            .path()
            .parent()
            .unwrap()
            .join("lazypencil.toml");
        let lock = crate::store::Store::lock_configuration(&configuration).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !app.conversation.modes().saving() {
                if let ConversationEvent::Message(value) =
                    app.conversation.next_event().await.unwrap()
                {
                    app.message(&value);
                }
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(120), app.conversation.next_event())
                .await
                .is_err()
        );
        assert!(app.conversation.modes().saving());
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("workspace-write")
        );
        app.permission_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        app.insert_text("Input stays responsive while settings are locked");
        assert_eq!(
            app.editor.lines().join("\n"),
            "Input stays responsive while settings are locked"
        );
        drop(lock);
        permission_changed(&mut app).await;
        assert!(app.conversation.modes().saved.is_some());
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn permission_project_default_is_not_saved_or_trusted_when_native_mode_is_rejected() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "settings-permissions-reject");
        permission_ready(&mut app).await;
        app.open_agent_permissions();
        app.permission_click(permissions::Hit::Row(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Scope(1)).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        app.permission_click(permissions::Hit::Continue).unwrap();
        permission_changed(&mut app).await;
        assert!(app.conversation.modes().error.is_some());
        assert!(app.outbox.paused);
        assert!(!temp.path().join("lazypencil.toml").exists());
        assert!(app.conversation.settings().project_execution.is_none());
        assert_eq!(
            app.conversation.modes().current.as_deref(),
            Some("read-only")
        );
        app.conversation.close().await.unwrap();
    });
}
