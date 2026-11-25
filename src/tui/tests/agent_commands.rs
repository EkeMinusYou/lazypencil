async fn wait_for_agent_commands(app: &mut App, name: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(commands) = app.conversation.available_agent_commands()
                && commands.iter().any(|entry| entry.name == name)
            {
                assert!(!commands.iter().any(|entry| entry.name == "other-command"));
                break;
            }
            if let ConversationEvent::Message(value) = app.conversation.next_event().await.unwrap()
            {
                app.message(&value);
            }
        }
    })
    .await
    .unwrap();
}

fn agent_command_wire_prompts(temp: &TempDir) -> Vec<Value> {
    std::fs::read_to_string(temp.path().join("received-prompts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn agent_command_selection_sends_native_prefix_with_document_references_and_images() {
    for (agent, mode, command, prefix) in [
        (
            "codex",
            "agent-commands-codex",
            "$layout-review",
            "$layout-review",
        ),
        (
            "claude",
            "agent-commands-claude",
            "fixture:layout-review",
            "/fixture:layout-review",
        ),
    ] {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app_with_agent(&temp, mode, agent);
            checkpoint(&app, json!("#ff0000"), "reference-box");
            let worker = Worker::start(
                app.conversation.store().clone(),
                vec![],
                Backend::halfblocks(),
                170,
            )
            .unwrap();
            app.cells = Size::new(20, 12);
            app.request(&worker, false, false);
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            assert!(app.accept_preview(output));
            app.refresh_nodes(Some("box"));
            app.reference_selected_node().unwrap();

            app.editor = editor("/model");
            app.conversation.connect().unwrap();
            wait_for_agent_commands(&mut app, command).await;
            app.modal = Some(command_modal(commands::Action::AgentCommands));
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            let Some(checkpoints::Modal::AgentCommands(picker)) = &mut app.modal else {
                panic!()
            };
            picker.paste("review the layout");
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            assert_eq!(app.outbox.draft_command.as_deref(), Some(command));
            assert_eq!(app.editor.lines(), ["/model"]);
            assert!(!temp.path().join("sent-turn.txt").exists());
            assert_eq!(
                outbox::Outbox::load(app.conversation.directory())
                    .unwrap()
                    .draft_command
                    .as_deref(),
                Some(command)
            );

            app.accept_input(fake_clipboard(
                Path::new("unused"),
                None,
                &CancellationToken::new(),
            ));
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
            let prompts = agent_command_wire_prompts(&temp);
            let blocks = prompts[0]["prompt"].as_array().unwrap();
            let command = format!("{prefix} /model");
            if agent == "codex" {
                assert_eq!(blocks[0]["text"], command);
                assert!(
                    blocks[1]["text"]
                        .as_str()
                        .unwrap()
                        .contains("Document path:")
                );
                assert!(
                    blocks.iter().any(|block| block["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("Explicit design references")
                            && text.contains("box")))
                );
            } else {
                assert_eq!(blocks.len(), 3);
                assert!(blocks[..2].iter().all(|block| block["type"] == "image"));
                let text = blocks[2]["text"].as_str().unwrap();
                assert!(text.starts_with(&command));
                assert!(text.contains("Document path:"));
                assert!(text.contains("Explicit design references") && text.contains("box"));
            }
            assert_eq!(
                blocks
                    .iter()
                    .filter(|block| block["type"] == "image")
                    .count(),
                2
            );
            assert!(app.outbox.draft_command.is_none());
            app.conversation.close().await.unwrap();
        });
    }
}

#[test]
fn agent_command_selection_repairs_a_paused_queue_without_consuming_the_next_draft() {
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "agent-commands-queue");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            171,
        )
        .unwrap();
        app.conversation.connect().unwrap();
        wait_for_agent_commands(&mut app, "model").await;
        app.editor = editor("First turn");
        app.submit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let ConversationEvent::Message(value) =
                    app.conversation.next_event().await.unwrap()
                    && value["notification"]["update"]["content"]["text"] == "working"
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        app.editor = editor("Review this draft");
        app.modal = Some(command_modal(commands::Action::AgentCommands));
        app.activate_modal(&worker).unwrap();
        let Some(checkpoints::Modal::AgentCommands(picker)) = &mut app.modal else {
            panic!()
        };
        picker.paste("model");
        app.activate_modal(&worker).unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.submit().await.unwrap();
        assert_eq!(app.outbox.queued[0].agent_command.as_deref(), Some("model"));
        let images: Vec<_> = app.outbox.queued[0]
            .images
            .iter()
            .map(|asset| asset.id.clone())
            .collect();
        app.editor = editor("Keep this next draft");
        std::fs::write(temp.path().join("release-agent-command-turn"), b"ok").unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.send_queued().await.unwrap();
        let failure = complete(&mut app).await;
        assert_eq!(
            failure.last().unwrap()["error"]["code"],
            "AGENT_COMMAND_UNAVAILABLE"
        );
        assert_eq!(agent_command_wire_prompts(&temp).len(), 1);
        assert!(app.outbox.paused);

        app.modal = Some(command_modal(commands::Action::QueuedAgentCommand));
        app.activate_modal(&worker).unwrap();
        let Some(checkpoints::Modal::AgentCommands(picker)) = &mut app.modal else {
            panic!()
        };
        picker.paste("replacement");
        app.activate_modal(&worker).unwrap();
        assert_eq!(
            app.outbox.queued[0].agent_command.as_deref(),
            Some("replacement")
        );
        assert_eq!(app.outbox.queued[0].text, "Review this draft");
        assert_eq!(
            app.outbox.queued[0]
                .images
                .iter()
                .map(|asset| &asset.id)
                .collect::<Vec<_>>(),
            images.iter().collect::<Vec<_>>()
        );
        assert_eq!(app.editor.lines(), ["Keep this next draft"]);
        assert!(app.outbox.paused);
        app.execute_command(commands::Action::ResumeQueue, &[], &worker)
            .unwrap();
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        let prompts = agent_command_wire_prompts(&temp);
        assert_eq!(prompts.len(), 2);
        assert_eq!(
            prompts[1]["prompt"][0]["text"],
            "/replacement Review this draft"
        );
        assert_eq!(app.editor.lines(), ["Keep this next draft"]);
        app.conversation.close().await.unwrap();
    });
}
