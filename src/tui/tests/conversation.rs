// Included in tui::tests to preserve the existing test identities.

#[test]
fn permission_modal_can_answer_and_cancel_without_blocking_the_event_loop() {
    for response in ["keyboard", "cancel", "mouse"] {
        let cancel = response == "cancel";
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
            app.keys = crate::config::keys::PromptKeys {
                submit: vec!["F5".into()],
                paste: vec!["F6".into()],
            };
            app.navigation = crate::config::keys::NavigationKeys {
                next_panel: vec!["F7".into()],
                commands: vec!["F8".into()],
                help: vec!["F9".into()],
                ..Default::default()
            };
            app.navigation.validate(&app.keys).unwrap();
            app.input_reader = fake_input;
            let worker = Worker::start(
                app.conversation.store().clone(),
                vec![],
                picker.clone(),
                124,
            )
            .unwrap();
            app.insert_text("許可を確認する");
            assert_eq!(app.agent_activity().content, "○ Idle");
            app.submit().await.unwrap();
            assert_eq!(app.agent_activity().content, "● Working");
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
            let footer = footer_tests::rows(&app, 80).pop().unwrap();
            assert!(footer.contains("Enter respond") && footer.contains("Esc deny"));
            assert!(!footer.contains("help") && !footer.contains("send"));
            press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
            press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
            for function in [7, 8, 9] {
                press(&mut app, KeyCode::F(function), KeyModifiers::NONE, &worker).await;
            }
            assert_eq!(app.focus, Focus::Prompt);
            assert!(!app.help && app.modal.is_none());
            assert!(app.input.is_none());
            assert_eq!(
                app.permissions.len(),
                1,
                "Prompt shortcuts cannot answer a permission"
            );
            assert_eq!(app.agent_activity().content, "! Blocked");
            let buffer = crate::tui::test_artifacts::snapshot(&mut app, "permission");
            assert!(buffer.content.iter().any(|cell| {
                cell.symbol() == "›" && cell.fg == theme::colors().running && cell.bg == theme::colors().selected_background
            }));
            let choice = app.pointer.area(mouse::Target::PermissionChoice(0)).unwrap();
            assert_eq!(buffer[(choice.right() - 1, choice.y)].bg, theme::colors().selected_background);
            assert_eq!(
                app.activity_line(Instant::now()).unwrap().spans[0].style.fg,
                app.agent_activity().style.fg
            );
            if response == "mouse" {
                use crossterm::event::{MouseButton, MouseEventKind as M};
                paint_mouse(&mut app, 130, 34);
                let choice = app.pointer.area(mouse::Target::PermissionChoice(0)).unwrap();
                for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
                    app.mouse_event(mouse_input(kind, (choice.x, choice.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
                    paint_mouse(&mut app, 130, 34);
                }
                assert_eq!(app.permissions.len(), 1, "Selecting a choice must not answer the request");
                let button = app.pointer.area(mouse::Target::Button(mouse::Action::Accept)).unwrap();
                for kind in [M::Down(MouseButton::Left), M::Up(MouseButton::Left)] {
                    app.mouse_event(mouse_input(kind, (button.x, button.y), KeyModifiers::NONE), &picker, &worker).await.unwrap();
                    paint_mouse(&mut app, 130, 34);
                }
                assert!(app.permissions.is_empty());
            } else {
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
            }
            assert_eq!(app.agent_activity().content, "● Working");
            let events = complete(&mut app).await;
            assert_eq!(app.agent_activity().content, "○ Idle");
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
fn tool_updates_keep_names_and_share_a_separate_block_from_prose() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    app.append("Inspecting the document.\n");
    for (id, title) in [
        ("read", "mcp.lazypencil.read_nodes"),
        ("render", "mcp.lazypencil.render"),
    ] {
        app.message(&json!({"type":"update","notification":{"update":{"sessionUpdate":"tool_call","toolCallId":id,"title":title,"status":"in_progress"}}}));
    }
    for id in ["render", "read"] {
        app.message(&json!({"type":"update","notification":{"update":{"sessionUpdate":"tool_call_update","toolCallId":id,"status":"completed"}}}));
    }
    app.append("\nThe document is ready.\n");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(180, 36)).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..36)
        .map(|y| {
            (0..180)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let first = rows
        .iter()
        .position(|line| line.contains("Inspecting the document."))
        .unwrap();
    let tools = rows
        .iter()
        .position(|line| line.contains("┌ Tools"))
        .unwrap();
    let read = rows
        .iter()
        .position(|line| line.contains("│ ✓ read nodes"))
        .unwrap();
    let render = rows
        .iter()
        .position(|line| line.contains("│ ✓ render"))
        .unwrap();
    let last = rows
        .iter()
        .position(|line| line.contains("The document is ready."))
        .unwrap();
    assert!(first + 1 < tools && tools < read && read < render && render + 1 < last);
    assert_eq!(
        rows.iter()
            .filter(|line| line.contains("read nodes"))
            .count(),
        1
    );
    assert!(
        !rows
            .iter()
            .any(|line| line.contains("● read nodes") || line.contains("✓ Tool"))
    );
    assert_eq!(app.tool_calls.len(), 2);
}

#[test]
fn tool_runs_stay_compact_and_refresh_visible_statuses() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "echo");
    app.append("Inspecting the document.\n");
    for index in 0..20 {
        app.message(&json!({"type":"update","notification":{"update":{
            "sessionUpdate":"tool_call", "toolCallId":index.to_string(),
            "title":format!("Call {index}"),
            "status":if index == 19 { "in_progress" } else { "completed" }
        }}}));
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(180, 36)).unwrap();
    let mut draw = |app: &mut App| {
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..36)
            .map(|y| {
                (0..180)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = draw(&mut app);
    for expected in [
        "Inspecting the document.",
        "│ … 17 earlier calls",
        "│ ✓ Call 17",
        "│ ✓ Call 18",
        "│ ● Call 19",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert_eq!(text.matches("Call ").count(), 3);
    app.message(&json!({"type":"update","notification":{"update":{
        "sessionUpdate":"tool_call_update", "toolCallId":"19", "status":"failed"
    }}}));
    let text = draw(&mut app);
    assert!(text.contains("│ ✕ Call 19"));
    assert!(!text.contains("│ ● Call 19"));
    assert_eq!(text.matches("earlier calls").count(), 1);
    assert_eq!(app.tool_calls.len(), 20);
    assert_eq!(app.log.matches("[tool_call]").count(), 20);
}

#[test]
fn activity_indicator_stays_above_the_prompt_when_history_scrolls() {
    fn row(
        terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
        needle: &str,
    ) -> Option<u16> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height).find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
    }
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        // Exact caret positions and unwrapped labels require this fixed pane geometry.
        app.panel_layout = crate::config::keys::PanelLayout {
            nodes: 22,
            preview: 48,
            agent: 30,
        };
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            192,
        )
        .unwrap();
        app.insert_text("Inspect the design");
        app.submit().await.unwrap();
        app.nodes_collapsed = true;
        let footer = footer_tests::rows(&app, 80);
        assert_eq!(footer.len(), 2);
        assert!(footer[0].trim().is_empty());
        assert!(footer[1].contains("Enter queue") && footer[1].contains("Ctrl+C cancel"));
        assert!(!footer[1].contains("show Nodes"));
        app.nodes_collapsed = false;
        app.agent_collapsed = true;
        app.focus = Focus::Preview;
        let footer = footer_tests::rows(&app, 80);
        assert_eq!(footer.len(), 2);
        assert!(footer[0].contains("Working…"));
        assert!(footer[1].contains("Ctrl+C cancel") && footer[1].contains("Alt+A show Agent"));
        app.agent_collapsed = false;
        app.focus = Focus::Prompt;
        app.append("Checking the document now.\n");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let response = row(&terminal, "Checking the document now.").unwrap();
        let activity = row(&terminal, "Working…").unwrap();
        assert_eq!(
            activity + 2,
            row(&terminal, "Prompt").unwrap(),
            "The indicator stays above Prompt with one blank row between them"
        );
        assert_eq!(row(&terminal, "● Working"), Some(1));
        assert!(
            activity > response,
            "Short conversations keep the indicator near the input"
        );
        for number in 0..45 {
            app.append(&format!("\nOutput line {number:02}\n"));
        }
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(row(&terminal, "Working…").unwrap(), activity);
        app.scroll = 5;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(
            row(&terminal, "Working…") == Some(activity),
            "Scrolling history must not hide the work indicator"
        );
        app.scroll = 0;
        for (width, height) in [(100, 24), (50, 16)] {
            terminal.backend_mut().resize(width, height);
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            assert_eq!(
                row(&terminal, "Working…").unwrap() + 2,
                row(&terminal, "Prompt").unwrap()
            );
        }
        complete(&mut app).await;
        app.sync_turn(&worker);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(row(&terminal, "Working…").is_none());
        terminal.backend_mut().resize(120, 36);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(row(&terminal, "Idle"), Some(1));
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn follow_up_queue_keeps_typing_live_and_sends_in_order_without_erasing_the_next_draft() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            188,
        )
        .unwrap();
        app.insert_text("First request");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(app.conversation.active());
        assert_eq!(app.editor.lines(), &[""]);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let caret = terminal.backend().cursor_position();
        assert_eq!(terminal.backend().buffer()[caret].symbol(), "A");
        assert_eq!(
            terminal.backend().buffer()[(caret.x + 1, caret.y)].symbol(),
            "d"
        );
        let started = app.turn_started.unwrap();
        assert_eq!(
            app.activity_line(started).unwrap().spans[0].style.fg,
            app.agent_activity().style.fg
        );
        let first = app.activity_line(started).unwrap().to_string();
        let later = app
            .activity_line(started + Duration::from_millis(1200))
            .unwrap()
            .to_string();
        assert!(first.contains("Working… 0s"));
        assert!(later.contains("Working… 1s"));
        assert_ne!(first.chars().next(), later.chars().next());
        for text in ["Second request", "Third request"] {
            for ch in text.chars() {
                press(&mut app, KeyCode::Char(ch), KeyModifiers::NONE, &worker).await;
            }
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            assert_eq!(app.editor.lines(), &[""]);
        }
        assert_eq!(app.outbox.queued.len(), 2);
        app.insert_text("Unsent next draft");
        app.save_draft().unwrap();
        assert_eq!(app.conversation.draft(), "First request");
        let restored = outbox::Outbox::load(app.conversation.directory()).unwrap();
        assert!(restored.paused);
        assert_eq!(restored.draft.as_deref(), Some("Unsent next draft"));
        assert_eq!(
            restored
                .queued
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["Second request", "Third request"]
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(
            terminal.backend().cursor_visible(),
            "Typing stays available while the agent works"
        );
        let buffer = terminal.backend().buffer();
        let rows = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let working = rows
            .iter()
            .position(|line| line.contains("Working…"))
            .unwrap();
        let queued = rows
            .iter()
            .position(|line| line.contains("2 queued"))
            .unwrap();
        let prompt = rows
            .iter()
            .position(|line| line.contains("✎ Prompt"))
            .unwrap();
        assert_eq!(queued, working + 2);
        assert!(rows[queued + 1].contains("1. Second request"));
        assert!(rows[queued + 2].contains("2. Third request"));
        assert_eq!(prompt, queued + 4);
        app.focus = Focus::Conversation;
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        assert_eq!(app.focus, Focus::Prompt);
        for (index, text) in ["Second request", "Third request"].into_iter().enumerate() {
            app.send_queued().await.unwrap();
            assert_eq!(app.conversation.draft(), text);
            assert_eq!(app.editor.lines(), &["Unsent next draft"]);
            assert_eq!(app.outbox.queued.len(), 1 - index);
            app.focus = Focus::Nodes;
            assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
            app.sync_turn(&worker);
            assert_eq!(
                app.focus,
                Focus::Nodes,
                "Completion does not steal document navigation focus"
            );
        }
        assert_eq!(app.conversation.draft(), "Unsent next draft");
        assert_eq!(app.editor.lines(), &["Unsent next draft"]);
        assert!(app.activity_line(Instant::now()).is_none());
        app.conversation.close().await.unwrap();
    });
}

async fn steering_working(app: &mut App) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap() {
                app.message(&value);
                if value["type"] == "update" {
                    break;
                }
            }
        }
    }).await.unwrap();
    assert!(app.conversation.can_steer());
}

#[test]
fn steering_submit_delivers_during_work_and_queue_remains_explicit() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "steering-flood");
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 518).unwrap();
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        assert!(!app.conversation.images().is_empty());
        app.insert_text("First request");
        app.submit().await.unwrap();
        steering_working(&mut app).await;
        assert!(footer_tests::rows(&app, 120).last().unwrap().contains("steer"));
        let started = app.turn_started;

        app.insert_text("/queue After this, check the layout's spacing");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(app.outbox.queued[0].text, "After this, check the layout's spacing");
        app.insert_text("Actually, use English everywhere");
        app.submit().await.unwrap();
        assert!(app.outbox.steering.is_some());
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(app.turn_started, started);
        app.insert_text("Unsent newer draft");
        assert!(app.submit().await.is_err(), "A pending receipt must not silently turn another steer into a queued prompt");
        let events = complete(&mut app).await;
        let receipt = events.iter().position(|value| value["type"] == "steering_result").unwrap();
        let completion = events.iter().position(|value| value["type"] == "complete").unwrap();
        assert!(receipt < completion);
        for value in events { app.message(&value); }
        app.sync_turn(&worker);
        assert!(app.outbox.steering.is_none());
        assert_eq!(app.editor.lines().join("\n"), "Unsent newer draft");
        assert!(!app.outbox.paused);
        assert_eq!(app.outbox.queued.len(), 1);
        let prompts = std::fs::read_to_string(temp.path().join("received-prompts.jsonl")).unwrap();
        assert_eq!(prompts.lines().count(), 1, "Steering must not create a second prompt");
        let steers = std::fs::read_to_string(temp.path().join("received-steering.jsonl")).unwrap();
        let steer: Value = serde_json::from_str(steers.lines().next().unwrap()).unwrap();
        assert_eq!(steer["prompt"].as_array().unwrap().len(), 1, "Do not reattach active images or guidance");
        assert_eq!(steer["prompt"][0]["text"], "Actually, use English everywhere");
        let transcript = std::fs::read_dir(app.conversation.directory()).unwrap()
            .flatten().find(|entry| entry.path().extension().is_some_and(|extension| extension == "jsonl")).unwrap().path();
        let transcript = std::fs::read_to_string(transcript).unwrap();
        assert!(transcript.lines().map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|value| value["type"] == "user_prompt" && value["text"] == "Actually, use English everywhere"));
        app.send_queued().await.unwrap();
        complete(&mut app).await;
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn steering_queue_command_works_with_modified_submit_and_multiline_text() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.insert_text("/queue Keep the layout's spacing\nand check typography");
        app.submit().await.unwrap();
        assert!(!app.conversation.active());
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(app.outbox.queued[0].text, "Keep the layout's spacing\nand check typography");
        assert_eq!(app.editor.lines(), &[""]);
        app.insert_text("/queue");
        assert!(app.submit().await.is_err());
        assert_eq!(app.editor.lines(), &["/queue"]);
        assert_eq!(app.outbox.queued.len(), 1);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn steering_idle_race_queues_once_and_delivery_failure_pauses_without_erasing_drafts() {
    for mode in ["steering-idle", "steering-failure", "steering-unowned"] {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut app = app(&temp, mode);
            let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 519).unwrap();
            app.insert_text("First request");
            app.submit().await.unwrap();
            steering_working(&mut app).await;
            app.insert_text("Mid-turn request");
            app.submit().await.unwrap();
            app.insert_text("Newer draft");
            if mode == "steering-failure" {
                loop {
                    if let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap() {
                        let delivered = value["type"] == "steering_result";
                        app.message(&value);
                        if delivered { break; }
                    }
                }
            } else {
                for value in complete(&mut app).await { app.message(&value); }
                app.sync_turn(&worker);
            }
            let recovered = usize::from(mode == "steering-unowned");
            assert_eq!(app.outbox.queued.len(), 1 + recovered, "{mode}");
            if recovered > 0 { assert_eq!(app.outbox.queued[1].text, "First request"); }
            assert_eq!(app.outbox.queued[0].text, "Mid-turn request");
            assert_eq!(app.outbox.paused, mode != "steering-idle");
            assert_eq!(app.editor.lines().join("\n"), "Newer draft");
            if mode == "steering-idle" {
                app.send_queued().await.unwrap();
                complete(&mut app).await;
                assert!(app.outbox.queued.is_empty());
                let prompts = std::fs::read_to_string(temp.path().join("received-prompts.jsonl")).unwrap();
                assert_eq!(prompts.lines().count(), 2);
            }
            let _ = app.conversation.close().await;
        });
    }
}

#[test]
fn steering_unconfirmed_delivery_survives_reopening_without_automatic_replay() {
    let temp = TempDir::new().unwrap();
    let mut outbox = outbox::Outbox {
        draft: Some("New draft".into()),
        steering: Some(outbox::SteeringPrompt { id: "pending".into(), text: "May already be delivered".into() }),
        ..Default::default()
    };
    outbox.save(temp.path()).unwrap();
    outbox = outbox::Outbox::load(temp.path()).unwrap();
    assert!(outbox.paused);
    assert!(outbox.steering.is_none());
    assert_eq!(outbox.queued.len(), 1);
    assert_eq!(outbox.queued[0].text, "May already be delivered");
    assert_eq!(outbox.draft.as_deref(), Some("New draft"));
}

#[test]
fn cancelling_a_turn_pauses_follow_ups_and_restores_input_focus() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            189,
        )
        .unwrap();
        app.insert_text("Cancelled request");
        app.submit().await.unwrap();
        app.insert_text("Queued follow-up");
        app.submit().await.unwrap();
        app.focus = Focus::Conversation;
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "failed");
        app.sync_turn(&worker);
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.editor.lines(), &[""]);
        assert_eq!(app.outbox.queued[0].text, "Cancelled request");
        assert_eq!(app.outbox.queued[1].text, "Queued follow-up");
        assert!(app.outbox.paused);
        app.send_queued().await.unwrap();
        assert!(!app.conversation.active());
        assert_eq!(app.outbox.queued.len(), 2);
        app.editor = editor("/clear-queue");
        app.command(&worker).await.unwrap();
        assert!(app.outbox.queued.is_empty());
        app.conversation.close().await.unwrap();
    });
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
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let ConversationEvent::Message(value) =
                    app.conversation.next_event().await.unwrap()
                {
                    assert!(
                        !matches!(value["type"].as_str(), Some("failed" | "connection_error")),
                        "Unexpected agent failure before completion: {value}"
                    );
                    if value["type"] == "turn_complete" {
                        break;
                    }
                }
            }
        })
        .await
        .expect("The fixture must reach turn_complete without waiting indefinitely");
        assert!(!app.conversation.active());
        app.sync_turn(&worker);
        app.insert_text("次の依頼は消えない");
        let ConversationEvent::Message(value) =
            tokio::time::timeout(Duration::from_secs(10), app.conversation.next_event())
                .await
                .expect("Completion must follow turn_complete")
                .unwrap()
        else {
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
