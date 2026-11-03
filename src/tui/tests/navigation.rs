// Included in tui::tests to preserve the existing test identities.

#[cfg(unix)]
#[test]
fn http_prompt_preparation_keeps_navigation_and_control_c_responsive() {
    let temp = TempDir::new().unwrap();
    let server = http_image_reference::Server::new();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        std::fs::write(app.conversation.store().path(), json!({"version":"2.21","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":format!("http://{}/network/http-image/blocked.png",server.address)}}]}).to_string()).unwrap();
        app.insert_text("日本語の依頼を保持する");
        app.save_draft().unwrap();
        let draft = app.conversation.draft().to_owned();
        let picker = Backend::halfblocks();
        let worker = Worker::start(app.conversation.store().clone(), vec![], picker.clone(), 123).unwrap();
        app.submit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while server.network_counts().0 == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert!(app.conversation.active());
        assert!(!app.key(KeyEvent::new(KeyCode::Char('x'),KeyModifiers::NONE), &picker, &worker).await.unwrap());
        assert_eq!(app.editor.lines().join("\n"), "x");
        assert!(!app.key(KeyEvent::new(KeyCode::Tab,KeyModifiers::NONE), &picker, &worker).await.unwrap());
        assert_eq!(app.focus, Focus::Nodes);
        assert!(!app.key(KeyEvent::new(KeyCode::Char('c'),KeyModifiers::CONTROL), &picker, &worker).await.unwrap());
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["error"]["code"], "CANCELLED");
        assert_eq!(app.conversation.draft(), draft);
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

#[test]
fn emacs_prompt_keys_edit_unicode_lines_keep_yanks_bounded_and_preserve_undo() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            147,
        )
        .unwrap();
        let text = "前の行\n日本語 abc suffix";
        app.insert_text(text);
        press(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.cursor().0, 0);
        assert!(app.modal.is_none());
        press(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.cursor().0, 1);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL, &worker).await;
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL, &worker).await;
        }
        press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines(), ["前の行", " abc suffix"]);
        press(&mut app, KeyCode::Char('_'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines().join("\n"), text);
        press(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL, &worker).await;
        press(&mut app, KeyCode::Char('w'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines(), ["前の行", "日本語 abc "]);
        press(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines().join("\n"), text);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL, &worker).await;
        press(&mut app, KeyCode::Char('k'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines(), ["前の行", ""]);
        press(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines().join("\n"), text);
        app.save_draft().unwrap();
        assert_eq!(app.conversation.draft(), text);
        app.editor = editor(&"x".repeat(1024 * 1024));
        press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.cursor().1, 1024 * 1024 - 1);
        press(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL, &worker).await;
        press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL, &worker).await;
        assert!(app.editor.is_empty());
        app.insert_text("保持する");
        press(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL, &worker).await;
        assert_eq!(app.editor.lines(), ["保持する"]);
        assert_eq!(app.status, "Prompt exceeds 1 MiB");
        press(&mut app, KeyCode::Char('p'), KeyModifiers::ALT, &worker).await;
        assert!(matches!(
            app.modal,
            Some(checkpoints::Modal::Commands { .. })
        ));
        assert!(!temp.path().join("sent-turn.txt").exists());
    });
}

#[test]
fn panel_toggles_use_custom_bindings_and_palette_but_respect_dialogs() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            194,
        )
        .unwrap();
        app.navigation.toggle_nodes_panel = vec!["Alt+n".into()];
        app.navigation.toggle_agent_panel = vec!["Alt+a".into()];
        app.navigation.validate(&app.keys).unwrap();
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE, &worker).await;
        assert!(!app.nodes_collapsed);
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["n"]);
        press(&mut app, KeyCode::Char('n'), KeyModifiers::ALT, &worker).await;
        assert!(app.nodes_collapsed);
        app.show_help();
        press(&mut app, KeyCode::Char('a'), KeyModifiers::ALT, &worker).await;
        assert!(!app.agent_collapsed);
        assert!(
            app.help_text()
                .contains("Alt+N: collapse/expand Nodes; Alt+A: collapse/expand Agent")
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        for (label, nodes_collapsed, agent_collapsed) in [
            ("Toggle Agent panel", true, true),
            ("Toggle Nodes panel", false, true),
        ] {
            let previous = (app.nodes_collapsed, app.agent_collapsed);
            app.modal = Some(checkpoints::Modal::Commands {
                selected: checkpoints::COMMANDS
                    .iter()
                    .position(|item| *item == label)
                    .unwrap(),
            });
            press(&mut app, KeyCode::Char('n'), KeyModifiers::ALT, &worker).await;
            assert_eq!((app.nodes_collapsed, app.agent_collapsed), previous);
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
            assert!(app.modal.is_none());
            assert_eq!(
                (app.nodes_collapsed, app.agent_collapsed),
                (nodes_collapsed, agent_collapsed)
            );
        }
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 16)).unwrap();
        press(&mut app, KeyCode::Char('n'), KeyModifiers::ALT, &worker).await;
        let mut cells = Size::default();
        terminal
            .draw(|frame| {
                cells = app.draw(frame);
            })
            .unwrap();
        assert_eq!(cells, Size::new(48, 12));
        let buffer = terminal.backend().buffer();
        let footer = (0..50)
            .map(|x| buffer[(x, 15)].symbol())
            .collect::<String>();
        assert!(footer.contains("Alt+A show Agent") && footer.contains("? help"));
        assert_eq!(app.editor.lines(), ["n"]);
    });
}

#[test]
fn conversation_region_defaults_to_prompt_and_keeps_history_and_attachments_accessible() {
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
            181,
        )
        .unwrap();
        let draft = "日本語の下書き\n編集中の本文";
        app.insert_text(draft);
        app.save_draft().unwrap();
        for (key, expected) in [
            (KeyCode::Tab, Focus::Nodes),
            (KeyCode::Tab, Focus::Preview),
            (KeyCode::Tab, Focus::Prompt),
            (KeyCode::BackTab, Focus::Preview),
            (KeyCode::BackTab, Focus::Nodes),
            (KeyCode::BackTab, Focus::Prompt),
        ] {
            press(
                &mut app,
                key,
                if key == KeyCode::BackTab {
                    KeyModifiers::SHIFT
                } else {
                    KeyModifiers::NONE
                },
                &worker,
            )
            .await;
            assert_eq!(app.focus, expected);
        }
        press(&mut app, KeyCode::Up, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::Conversation);
        press(&mut app, KeyCode::PageUp, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.scroll, 5);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Prompt);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let empty = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(empty.contains("Agent"));
        assert!(!empty.contains("Attachments"));
        let transcript = "\nYou: Keep this message\n[0 images]\n\nCodex:\nAgent response\n\n[in_progress] mcp.lazypencil.read_nodes\n\n[completed] Tool\n\nRequest complete\n";
        app.append(transcript);
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        let rendered = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        for content in ["Keep this message", "Codex", "Agent response", "read nodes", "Request complete"] {
            assert!(rendered.contains(content), "{content}");
        }
        assert!(!rendered.contains("[in_progress]"));
        assert!(!rendered.contains("[0 images]"));
        assert_eq!(app.log, transcript);
        assert!(app.history.is_some());
        app.append("\n**New response**\n");
        assert!(app.history.is_none());
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        let updated = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(updated.contains("New response"));
        assert!(!updated.contains("**New response**"));
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let attached = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(attached.contains("Attachments"));
        press(&mut app, KeyCode::Up, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::Attachments);
        assert_eq!(app.attachments.selected(), Some(0));
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.attachments.selected(), Some(1));
        press(&mut app, KeyCode::Up, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::Conversation);
        press(&mut app, KeyCode::Down, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::Attachments);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.conversation.images().len(), 1);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert!(app.conversation.images().is_empty());
        assert_eq!(app.focus, Focus::Prompt);
        press(&mut app, KeyCode::Up, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::Conversation);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Nodes);
        press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT, &worker).await;
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.editor.lines().join("\n"), draft);
        assert_eq!(app.conversation.draft(), draft);
        assert!(!app.conversation.active());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn agent_header_tracks_negotiated_model_and_hides_inactive_fast_mode() {
    let temp = TempDir::new().unwrap();
    let mut app = app(&temp, "settings-fast");
    assert!(app.header_line().to_string().contains("model: connecting"));
    assert!(!app.header_line().to_string().contains("Fast"));
    app.message(&json!({"type":"ready","response":{"configOptions":[
        {"id":"engine","category":"model","currentValue":"fixture-model"},
        {"id":"reasoning_effort","category":"thought_level","currentValue":"xhigh"},
        {"id":"fast-mode","currentValue":"on"}
    ]}}));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let top: String = (0..80)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect();
    assert!(top.contains("model: fixture-model"), "{top}");
    assert!(top.contains("effort: xhigh"), "{top}");
    assert!(top.contains("Fast"), "{top}");
    app.dirty = false;
    app.message(&json!({"type":"update","notification":{"update":{
        "sessionUpdate":"config_option_update","configOptions":[
            {"id":"model","currentValue":"fixture-model-2"},
            {"id":"reasoning_effort","currentValue":"high"},
            {"id":"fast-mode","currentValue":"off"}
        ]
    }}}));
    assert!(app.dirty);
    assert!(
        app.header_line()
            .to_string()
            .contains("model: fixture-model-2")
    );
    assert!(app.header_line().to_string().contains("effort: high"));
    assert!(!app.header_line().to_string().contains("Fast"));
    app.message(&json!({"type":"ready","response":{"configOptions":[
        {"id":"model","currentValue":"another-model"},
        {"id":"fast-mode","currentValue":true}
    ]}}));
    assert!(app.header_line().to_string().contains("Fast"));
    app.message(&json!({"type":"update","notification":{"update":{
        "sessionUpdate":"config_option_update","configOptions":[
            {"id":"model","currentValue":"standard-model"}
        ]
    }}}));
    assert!(
        app.header_line()
            .to_string()
            .contains("model: standard-model")
    );
    assert!(!app.header_line().to_string().contains("Fast"));
    assert!(!app.header_line().to_string().contains("effort: high"));
    runtime().block_on(async {
        app.update_agent_configuration(&json!([{"id":"fast-mode","currentValue":true}]));
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            186,
        )
        .unwrap();
        app.select_conversation(saved::SessionChange::New, &worker)
            .await
            .unwrap();
        assert!(app.header_line().to_string().contains("model: connecting"));
        assert!(!app.header_line().to_string().contains("Fast"));
        worker.cancel();
    });
}

#[test]
fn diagnostics_switch_preserves_targets_and_reveals_hidden_nodes_after_refresh() {
    async fn rendered(app: &mut App, worker: &Worker) {
        loop {
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            if output.generation == app.generation {
                assert!(app.accept_preview(output));
                assert!(app.preview_error.is_none());
                break;
            }
        }
    }
    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                app.draw_nodes(frame, area);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let path = app.conversation.store().path().to_path_buf();
        let mut document = json!({"version":"2.21","children":[{
            "type":"frame","id":"root","name":"Root","width":100,"height":100,"layout":"none",
            "children":[
                {"type":"rectangle","id":"child-a","name":"Same name","x":95,"width":20,"height":10,"fill":"#00FF00"},
                {"type":"rectangle","id":"child-b","name":"Same name","x":95,"y":20,"width":20,"height":10,"fill":"#0000FF"}
            ]
        }]});
        std::fs::write(&path, document.to_string()).unwrap();
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 187).unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        app.focus = Focus::Nodes;
        app.filter = "root".into();
        app.refresh_nodes(Some("root"));
        assert_eq!(app.filtered_nodes().len(), 3);
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 1);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert!(app.diagnostics.active);
        assert!(app.diagnostics.selected_only);
        assert_eq!(app.visible_diagnostic_count(), 2);
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-a");
        assert!(screen(&mut app, 40, 26).contains("Subtree: Root"));
        press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-a");
        assert_eq!(app.selected_node().unwrap().id, "root");
        assert_eq!(app.filter, "root");
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-b");
        let text = screen(&mut app, 40, 26);
        let contents = text.replace('│', " ").split_whitespace().collect::<Vec<_>>().join(" ");
        for expected in ["Diagnostics", "warning: LAYOUT_OVERFLOW", "Same name", "child-b", "The child extends beyond its parent"] {
            assert!(contents.contains(expected), "Missing {expected}:\n{text}");
        }
        let top = screen(&mut app, 18, 16);
        for _ in 0..8 {
            press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL, &worker).await;
            screen(&mut app, 18, 16);
        }
        let bottom = screen(&mut app, 18, 16);
        assert_ne!(top, bottom, "Narrow diagnostic details must scroll");
        assert!(bottom.contains('}'), "Measured details remain reachable");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(!app.diagnostics.active);
        assert_eq!(app.filter, "root");
        assert_eq!(app.selected_node().unwrap().id, "root");
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(!app.diagnostics.active);
        assert!(app.filter.is_empty());
        assert_eq!(app.selected_node().unwrap().id, "child-b");
        assert_eq!(app.filtered_nodes().len(), 3);
        assert_eq!(app.view.center, [105.0, 25.0]);
        rendered(&mut app, &worker).await;

        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert!(app.diagnostics.selected_only);
        assert!(screen(&mut app, 40, 26).contains("Node: Same name"));
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-b");
        document["children"][0]["children"].as_array_mut().unwrap().reverse();
        std::fs::write(&path, document.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-b");
        assert_eq!(app.selected_node().unwrap().id, "child-b");
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        app.focus = Focus::Attachments;
        app.attachments.select(Some(0));
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        assert!(app.preview.as_ref().unwrap().attachment);
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-b");
        app.focus = Focus::Nodes;
        press(&mut app, KeyCode::Char('f'), KeyModifiers::NONE, &worker).await;
        assert!(app.diagnostics.active);
        rendered(&mut app, &worker).await;
        document["children"][0]["children"].as_array_mut().unwrap().remove(0);
        std::fs::write(&path, document.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert!(app.selected_node().is_none());
        assert!(app.selected_diagnostic().is_none());
        press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_diagnostic().unwrap()["node_id"], "child-a");
        document["children"][0]["children"][0]["x"] = json!(0);
        std::fs::write(&path, document.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert!(app.selected_diagnostic().is_none());
        assert!(screen(&mut app, 40, 20).contains("No layout diagnostics"));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(app.diagnostics.active);

        app.navigation.toggle_diagnostics = vec!["v".into()];
        app.navigation.validate(&app.keys).unwrap();
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert!(app.diagnostics.active);
        press(&mut app, KeyCode::Char('v'), KeyModifiers::NONE, &worker).await;
        assert!(!app.diagnostics.active);
        app.searching = true;
        press(&mut app, KeyCode::Char('v'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filter, "v");
        assert!(!app.diagnostics.active);
        app.searching = false;
        app.focus = Focus::Prompt;
        press(&mut app, KeyCode::Char('v'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), ["v"]);
        app.diagnostics.active = true;
        app.preview = None;
        assert!(screen(&mut app, 40, 20).contains("Loading diagnostics"));
        app.preview_error = Some(Error::new("RENDER_ERROR", "Fixture failure"));
        let unavailable = screen(&mut app, 40, 20);
        assert!(unavailable.contains("Diagnostics unavailable"));
        assert!(!unavailable.contains("No layout diagnostics"));
        assert!(!app.conversation.active());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn nodes_enter_expands_branches_and_fits_only_leaves() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        std::fs::write(
            app.conversation.store().path(),
            json!({"version":"2.21","children":[{
                "type":"frame","id":"root","width":160,"height":80,"layout":"none",
                "children":[
                    {"type":"rectangle","id":"leaf","x":20,"width":8,"height":8},
                    {"type":"frame","id":"empty","x":80,"width":20,"height":20,"children":[]}
                ]
            }]}).to_string(),
        ).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(), vec![], Backend::halfblocks(), 195,
        ).unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        assert!(app.accept_preview(output));
        assert!(app.preview_error.is_none());
        app.focus = Focus::Nodes;
        app.refresh_nodes(Some("root"));
        let view = app.view;
        let generation = app.generation;
        assert_eq!(app.filtered_nodes().len(), 1);

        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 3);
        assert_eq!(app.selected_node().unwrap().id, "root");
        assert_eq!(app.view, view);
        assert_eq!(app.generation, generation, "Expanding must not request a preview");
        assert!(app.selection.ids.is_empty());
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "leaf");
        assert_eq!(app.view, view);
        assert_eq!(app.generation, generation);
        assert!(app.selection.ids.is_empty());
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center, [24.0, 4.0]);
        assert!(app.view.zoom.is_some());
        assert_eq!(app.focus, Focus::Nodes);
        assert_eq!(app.selection.ids, ["leaf"]);

        app.refresh_nodes(Some("root"));
        press(&mut app, KeyCode::Char('f'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center, [80.0, 40.0]);
        assert_eq!(app.selected_node().unwrap().id, "root");
        assert_eq!(app.focus, Focus::Nodes);
        assert_eq!(app.selection.ids, ["root"]);
        app.refresh_nodes(Some("empty"));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center, [90.0, 10.0]);
        assert_eq!(app.selection.ids, ["empty"]);

        app.filter = "root".into();
        app.refresh_nodes(Some("root"));
        press(&mut app, KeyCode::Left, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 1);
        app.navigation.activate_node = vec!["e".into()];
        app.navigation.validate(&app.keys).unwrap();
        let view = app.view;
        let generation = app.generation;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 1);
        press(&mut app, KeyCode::Char('e'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 3);
        assert_eq!(app.selected_node().unwrap().id, "root");
        assert_eq!(app.view, view);
        assert_eq!(app.generation, generation);
        press(&mut app, KeyCode::Char('e'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "leaf");
        press(&mut app, KeyCode::Char('e'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center, [24.0, 4.0]);
        assert!(app.help_text().contains("Nodes: expand or enter a branch; fit a node without children"));
    });
}

#[test]
fn nodes_tree_keeps_duplicate_names_distinct_through_search_refresh_and_attachments() {
    async fn rendered(app: &mut App, worker: &Worker) {
        loop {
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            if output.generation == app.generation {
                assert!(app.accept_preview(output));
                assert!(app.preview_error.is_none());
                break;
            }
        }
    }
    fn visible(app: &App) -> Vec<String> {
        let nodes = &app.preview.as_ref().unwrap().nodes;
        app.filtered_nodes()
            .iter()
            .map(|row| nodes[row.index].id.clone())
            .collect()
    }
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let path = app.conversation.store().path().to_path_buf();
        let mut document = json!({"version":"2.21","children":[{
            "type":"frame","id":"sidebar","name":"Sidebar","width":160,"height":80,"layout":"none",
            "children":[
                {"type":"frame","id":"room-a","name":"Room A","width":40,"height":40,"layout":"none","children":[
                    {"type":"rectangle","id":"icon-a","name":"Icon","width":8,"height":8,"fill":"#00FF00"}
                ]},
                {"type":"frame","id":"room-b","name":"Room B","x":80,"width":40,"height":40,"layout":"none","children":[
                    {"type":"rectangle","id":"icon-b","name":"Icon","width":8,"height":8,"fill":"#0000FF"}
                ]}
            ]
        }]});
        std::fs::write(&path, document.to_string()).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(app.conversation.store().clone(), vec![], picker, 182).unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        assert_eq!(visible(&app), ["sidebar"]);
        app.focus = Focus::Nodes;
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        let position = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| {
            let name = ["S", "i", "d", "e", "b", "a", "r"];
            terminal.backend().buffer().content.windows(name.len()).position(|cells| cells.iter().zip(name).all(|(cell, symbol)| cell.symbol() == symbol)).unwrap()
        };
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        let before_selection = position(&terminal);
        assert_eq!(before_selection % 120, 3, "Root labels should only reserve the branch marker and its separator");
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE, &worker).await;
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        assert_eq!(position(&terminal), before_selection, "Starting selection must not shift the tree");
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar", "room-a", "room-b"]);
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar"]);
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar", "room-a", "room-b"]);
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "room-a");
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "icon-a");
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "room-a");
        assert_eq!(visible(&app), ["sidebar", "room-a", "room-b"]);
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar"]);
        assert_eq!(app.selected_node().unwrap().id, "sidebar");
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar"]);
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        for character in "Icon".chars() {
            press(&mut app, KeyCode::Char(character), KeyModifiers::NONE, &worker).await;
        }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(visible(&app), ["sidebar", "room-a", "icon-a", "room-b", "icon-b"]);
        assert_eq!(app.selected_node().unwrap().id, "icon-a");
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "icon-b");
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "room-b");
        assert_eq!(visible(&app), ["sidebar", "room-a", "icon-a", "room-b"]);
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "icon-b");
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "icon-b");
        assert_eq!(visible(&app), ["sidebar", "room-a", "room-b", "icon-b"]);
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        assert_eq!(app.selected_node().unwrap().target["address"]["node_id"], "icon-b");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Nodes);
        rendered(&mut app, &worker).await;
        let old_row = app.nodes.selected();
        document["children"][0]["children"].as_array_mut().unwrap().reverse();
        std::fs::write(&path, document.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "icon-b");
        assert_ne!(app.nodes.selected(), old_row);
        assert_eq!(visible(&app), ["sidebar", "room-b", "icon-b", "room-a"]);
        app.accept_input(fake_clipboard(Path::new("unused"), None, &CancellationToken::new()));
        app.focus = Focus::Attachments;
        app.attachments.select(Some(0));
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        assert!(app.preview.as_ref().unwrap().attachment);
        assert_eq!(app.selected_node().unwrap().id, "icon-b");
        app.focus = Focus::Nodes;
        document["children"][0]["children"][0]["children"] = json!([]);
        std::fs::write(&path, document.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert!(app.selected_node().is_none(), "Deleting a target must never select another node by row number");
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "room-b");
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selected_node().unwrap().id, "sidebar");
        let instances = json!({"version":"2.21","children":[
            {"type":"frame","id":"component","name":"Card","reusable":true,"layout":"none","width":24,"height":24,"children":[
                {"type":"rectangle","id":"glyph","name":"Icon","width":8,"height":8,"fill":"#FF0000"}
            ]},
            {"type":"ref","id":"instance-a","name":"Card A","ref":"component","x":40},
            {"type":"ref","id":"instance-b","name":"Card B","ref":"component","x":80}
        ]});
        std::fs::write(&path, instances.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        app.filter = "glyph".into();
        app.search_nodes(None);
        let preview = app.preview.as_ref().unwrap();
        let repeated = preview.nodes.iter().enumerate().filter(|(_, node)| {
            node.target["source_id"] == "glyph"
                && node.target["address"]["instance_path"].as_array().is_some_and(|path| !path.is_empty())
        }).collect::<Vec<_>>();
        assert_eq!(repeated.len(), 2);
        assert_ne!(repeated[0].1.id, repeated[1].1.id);
        for (index, node) in &repeated {
            let parent = &preview.nodes[node.parent.unwrap()];
            assert!(parent.children.contains(index));
            assert_eq!(parent.target["address"]["node_id"], node.target["address"]["node_id"]);
            let parent_path = parent.target["address"]["instance_path"].as_array().unwrap();
            let child_path = node.target["address"]["instance_path"].as_array().unwrap();
            assert_eq!(child_path.len(), parent_path.len() + 1);
            assert_eq!(&child_path[..parent_path.len()], parent_path);
        }
        let second = repeated[1].0;
        let expected = repeated[1].1.target.clone();
        let row = app.filtered_nodes().iter().position(|row| row.index == second).unwrap();
        app.nodes.select(Some(row));
        assert_eq!(app.selected_node().unwrap().target, expected);
        let pages = json!({"version":"2.21","children":(0..40).map(|index| json!({
            "type":"rectangle","id":format!("page-{index}"),"name":format!("Item {index:02}"),"x":index*10,"width":8,"height":8,"fill":"#0000FF"
        })).collect::<Vec<_>>()});
        app.filter.clear();
        std::fs::write(&path, pages.to_string()).unwrap();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        app.nodes.select(Some(0));
        for (key, expected) in [('d',15),('d',30),('d',39),('u',24),('u',9),('u',0)] {
            press(&mut app, KeyCode::Char(key), KeyModifiers::CONTROL, &worker).await;
            assert_eq!(app.nodes.selected(), Some(expected));
            assert_eq!(app.selected_node().unwrap().id, format!("page-{expected}"));
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn held_hjkl_pan_accepts_repeat_events_and_stops_on_release_without_affecting_typing() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let picker = Backend::halfblocks();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            190,
        )
        .unwrap();
        app.cells = Size::new(20, 8);
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        app.focus = Focus::Preview;
        for (letter, delta) in [
            ('h', [-64.0, 0.0]),
            ('j', [0.0, 64.0]),
            ('k', [0.0, -64.0]),
            ('l', [64.0, 0.0]),
        ] {
            let mut expected = app.view;
            for kind in std::iter::once(KeyEventKind::Press)
                .chain(std::iter::repeat_n(KeyEventKind::Repeat, 8))
            {
                app.key(
                    KeyEvent::new_with_kind(KeyCode::Char(letter), KeyModifiers::NONE, kind),
                    &picker,
                    &worker,
                )
                .await
                .unwrap();
                expected.pan(delta);
                assert_eq!(app.view.center, expected.center);
            }
            app.key(
                KeyEvent::new_with_kind(
                    KeyCode::Char(letter),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                ),
                &picker,
                &worker,
            )
            .await
            .unwrap();
            assert_eq!(app.view.center, expected.center);
            for _ in 0..8 {
                app.key(
                    KeyEvent::new(KeyCode::Char(letter), KeyModifiers::NONE),
                    &picker,
                    &worker,
                )
                .await
                .unwrap();
                expected.pan(delta);
            }
            assert_eq!(app.view.center, expected.center);
        }
        app.focus = Focus::Prompt;
        app.key(
            KeyEvent::new_with_kind(KeyCode::Char('h'), KeyModifiers::NONE, KeyEventKind::Repeat),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        assert_eq!(app.editor.lines(), &["h"]);
        assert!(!app.conversation.active());
    });
}
