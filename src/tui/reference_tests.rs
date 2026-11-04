use super::*;

async fn rendered(app: &mut App, worker: &Worker) {
    loop {
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        if output.generation == app.generation {
            assert!(app.accept_preview(output));
            assert!(app.preview_error.is_none(), "{:?}", app.preview_error);
            break;
        }
    }
}

fn select(app: &mut App, root: &str, path: &[&str]) {
    let id = app
        .preview
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|node| {
            node.target["address"]["node_id"] == root
                && node.target["address"]["instance_path"] == json!(path)
        })
        .unwrap()
        .id
        .clone();
    app.focus = Focus::Nodes;
    app.refresh_nodes(Some(&id));
}

fn references_received(temp: &TempDir, turn: usize) -> Vec<Value> {
    let received: Value = serde_json::from_slice(
        &std::fs::read(temp.path().join(format!("received-turn-{turn}.json"))).unwrap(),
    )
    .unwrap();
    received["text"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|block| {
            let text = block.as_str()?;
            let offset = text.find("{\"node_references\"")?;
            let selection: Value = serde_json::from_str(&text[offset..]).unwrap();
            Some(selection["node_references"].as_array().unwrap().clone())
        })
        .unwrap_or_default()
}

fn document() -> Value {
    json!({"version":"2.21","children":[
        {"type":"frame","id":"component","reusable":true,"layout":"none","width":100,"height":40,"children":[
            {"type":"text","id":"label","name":"Button label","content":"購入する","fontSize":12,"fill":"#000000"}
        ]},
        {"type":"ref","id":"instance-a","ref":"component","x":120},
        {"type":"ref","id":"instance-b","ref":"component","x":240}
    ]})
}

#[test]
fn explicit_references_preserve_drafts_instances_cursor_and_saved_history() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        // Exact caret positions and unwrapped labels require this fixed pane geometry.
        app.panel_layout = crate::config::keys::PanelLayout {
            nodes: 22,
            preview: 48,
            agent: 30,
        };
        let path = app.conversation.store().path().to_path_buf();
        let saved = document().to_string();
        std::fs::write(&path, &saved).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            191,
        )
        .unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        select(&mut app, "instance-a", &["label"]);
        app.insert_text("Selection alone is not a reference");
        app.submit().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        assert!(references_received(&temp, 1).is_empty());

        app.insert_text("このボタンの文言を変更したいです");
        select(&mut app, "instance-b", &["label"]);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.editor.lines(), ["このボタンの文言を変更したいです"]);
        assert!(!app.conversation.active());
        assert_eq!(
            app.outbox.references[0].target["address"],
            json!({"node_id":"instance-b","instance_path":["label"]})
        );
        assert_eq!(app.outbox.references[0].text_excerpt, "購入する");
        select(&mut app, "instance-b", &["label"]);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(
            app.outbox.references.len(),
            1,
            "Adding the same address updates its summary"
        );
        select(&mut app, "instance-a", &["label"]);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-b"
        );
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.outbox.references.len(), 2);
        press(&mut app, KeyCode::Up, KeyModifiers::ALT, &worker).await;
        assert_eq!(app.focus, Focus::References);
        press(&mut app, KeyCode::Delete, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.outbox.references.len(), 1);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-a"
        );
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;

        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.editor = editor("一行目\n二行目\n三行目");
        app.draft_changed = true;
        app.save_draft().unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 16)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(
            terminal.backend().cursor_visible(),
            "References and images must leave the IME caret visible at minimum size"
        );
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("References"));
        assert!(screen.contains("Button"));
        app.conversation.clear_images().unwrap();
        let mut args = app.conversation.settings().clone();
        args.create = false;
        args.session = Some(app.conversation.id().into());
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lazypencil");
        app.conversation.close().await.unwrap();
        drop(worker);
        drop(app);
        let conversation =
            Conversation::open_with_executable(args, Some(temp.path().join("data")), executable)
                .unwrap();
        let mut app = App::new(conversation);
        assert_eq!(app.editor.lines().join("\n"), "一行目\n二行目\n三行目");
        assert_eq!(app.outbox.references.len(), 1);
        assert!(!app.conversation.active());
        app.submit().await.unwrap();
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "complete");
        assert_eq!(
            references_received(&temp, 1)[0]["target"]["address"]["node_id"],
            "instance-a"
        );
        let start = events
            .iter()
            .find(|event| event["type"] == "start")
            .unwrap();
        let history = crate::conversation::history::view(
            app.conversation.store(),
            app.conversation.id(),
            start["turn"].as_str().unwrap(),
            None,
            65536,
            None,
        )
        .unwrap();
        assert!(
            history["text"]
                .as_str()
                .unwrap()
                .contains("References: Button label")
        );
        assert!(app.outbox.references.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn queued_references_are_frozen_and_missing_targets_preserve_the_next_draft() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        let path = app.conversation.store().path().to_path_buf();
        std::fs::write(&path, document().to_string()).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            192,
        )
        .unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        app.insert_text("First request");
        app.submit().await.unwrap();
        select(&mut app, "instance-b", &["label"]);
        app.reference_selected_node().unwrap();
        app.insert_text("Queued request");
        app.submit().await.unwrap();
        assert_eq!(app.outbox.queued.len(), 1);
        assert_eq!(
            app.outbox.queued[0].references[0].target["address"]["node_id"],
            "instance-b"
        );
        select(&mut app, "instance-a", &["label"]);
        app.reference_selected_node().unwrap();
        app.insert_text("Next unsent draft");
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        assert_eq!(
            references_received(&temp, 2)[0]["target"]["address"]["node_id"],
            "instance-b"
        );
        assert_eq!(app.editor.lines(), ["Next unsent draft"]);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-a"
        );

        let mut changed = document();
        changed["children"].as_array_mut().unwrap().remove(1);
        std::fs::write(&path, changed.to_string()).unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.submit().await.unwrap();
        select(&mut app, "instance-b", &["label"]);
        app.reference_selected_node().unwrap();
        app.insert_text("New draft while preparing");
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["phase"], "preparation");
        assert_eq!(events.last().unwrap()["error"]["code"], "NODE_NOT_FOUND");
        app.sync_turn(&worker);
        assert_eq!(app.editor.lines(), ["New draft while preparing"]);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-b"
        );
        assert_eq!(
            app.outbox.queued[0].references[0].target["address"]["node_id"],
            "instance-a"
        );
        assert!(app.outbox.paused);
        assert!(!temp.path().join("received-turn-3.json").exists());
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        assert_eq!(
            app.reference_available(&app.outbox.queued[0].references[0]),
            Some(false)
        );
        std::fs::write(&path, document().to_string()).unwrap();
        app.outbox.paused = false;
        app.send_queued().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        assert_eq!(
            references_received(&temp, 3)[0]["target"]["address"]["node_id"],
            "instance-a"
        );
        let retried: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("received-turn-3.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(retried["images"].as_array().unwrap().len(), 2);
        assert_eq!(app.editor.lines(), ["New draft while preparing"]);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-b"
        );
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn diagnostics_attach_the_selected_issue_without_changing_the_tree_target() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        std::fs::write(
            app.conversation.store().path(),
            json!({"version":"2.21","children":[{
                "type":"frame","id":"root","name":"Root","layout":"none","width":50,"height":30,
                "children":[{"type":"frame","id":"branch","name":"Branch","layout":"none","width":50,"height":30,
                    "children":[{"type":"text","id":"label","name":"Label","x":60,"width":80,"height":8,
                        "fontSize":20,"content":"Overflow","textGrowth":"fixed-width-height"}]}]
            },{
                "type":"frame","id":"unrelated","name":"Elsewhere","layout":"none","width":50,"height":30,
                "children":[{"type":"rectangle","id":"label-other","name":"Label","x":60,"width":20,"height":8}]
            }]})
            .to_string(),
        )
        .unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            195,
        )
        .unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        app.filter = "root".into();
        select(&mut app, "root", &[]);
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.filtered_nodes().len(), 1);
        assert_eq!(app.preview.as_ref().unwrap().summary["diagnostics"].as_array().unwrap().len(), 3);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(18, 16)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                app.draw_nodes(frame, area);
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(13, 1)].symbol(), "⚠");
        assert_eq!(terminal.backend().buffer()[(13, 1)].fg, theme::colors().running);
        assert_eq!(terminal.backend().buffer()[(16, 1)].symbol(), "]");
        app.insert_text("この診断を直してください");
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.visible_diagnostic_count(), 2);
        assert_eq!(app.selected_node().unwrap().id, "root");
        let first = app.selected_diagnostic().unwrap().clone();
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.editor.lines(), ["この診断を直してください"]);
        assert!(!app.conversation.active());
        assert_eq!(app.selected_node().unwrap().id, "root");
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "label"
        );
        assert_eq!(app.outbox.references[0].diagnostic.as_ref(), Some(&first));
        app.focus = Focus::Nodes;
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE, &worker).await;
        let second = app.selected_diagnostic().unwrap().clone();
        assert_ne!(first["code"], second["code"]);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        app.focus = Focus::Nodes;
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(
            app.outbox.references.len(),
            2,
            "The same issue is not attached twice"
        );
        select(&mut app, "label", &[]);
        app.diagnostics.active = false;
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(
            app.outbox.references.len(),
            3,
            "Node and diagnostic references remain distinct"
        );
        let restored = outbox::Outbox::load(app.conversation.directory()).unwrap();
        assert_eq!(restored.references[0].diagnostic.as_ref(), Some(&first));
        app.submit().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        let sent = references_received(&temp, 1);
        assert_eq!(sent[0]["diagnostic"], first);
        assert_eq!(sent[1]["diagnostic"], second);
        assert!(sent[2].get("diagnostic").is_none());
        let mut design: Value = serde_json::from_slice(&std::fs::read(app.conversation.store().path()).unwrap()).unwrap();
        design["children"][0]["reusable"] = json!(true);
        for (id, x) in [("instance-a", 100), ("instance-b", 200)] {
            design["children"].as_array_mut().unwrap().push(json!({"type":"ref","id":id,"ref":"root","x":x}));
        }
        std::fs::write(app.conversation.store().path(), design.to_string()).unwrap();
        app.filter.clear();
        app.request(&worker, false, true);
        rendered(&mut app, &worker).await;
        for id in ["instance-a", "instance-b"] {
            select(&mut app, id, &[]);
            press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
            press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE, &worker).await;
            assert_eq!(app.visible_diagnostic_count(), 2);
            for _ in 0..2 {
                let diagnostic = app.selected_diagnostic().unwrap();
                let node = app.preview.as_ref().unwrap().nodes.iter().find(|node| diagnostic["node_id"] == node.id).unwrap();
                assert_eq!(node.target["address"]["node_id"], id, "A subtree must not include diagnostics from another instance of the same component");
                press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
            }
            press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        }
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn interrupted_reference_requests_restore_the_draft_without_resending() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        std::fs::write(app.conversation.store().path(), document().to_string()).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            194,
        )
        .unwrap();
        app.cells = Size::new(20, 12);
        app.request(&worker, false, false);
        rendered(&mut app, &worker).await;
        select(&mut app, "instance-b", &["label"]);
        app.reference_selected_node().unwrap();
        app.insert_text("Interrupted reference request");
        app.submit().await.unwrap();
        assert!(app.outbox.in_flight.is_some());
        assert!(app.outbox.references.is_empty());
        let mut args = app.conversation.settings().clone();
        args.create = false;
        args.session = Some(app.conversation.id().into());
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lazypencil");
        app.conversation.close().await.unwrap();
        drop(worker);
        drop(app);
        let conversation =
            Conversation::open_with_executable(args, Some(temp.path().join("data")), executable)
                .unwrap();
        let mut app = App::new(conversation);
        assert_eq!(app.editor.lines(), ["Interrupted reference request"]);
        assert_eq!(app.outbox.references.len(), 1);
        assert_eq!(
            app.outbox.references[0].target["address"]["node_id"],
            "instance-b"
        );
        assert!(app.outbox.queued.is_empty());
        assert!(!app.conversation.active());
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn generated_references_validate_with_the_normal_script_pipeline_and_recover_on_failure() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        let script = temp.path().join("generated.js");
        std::fs::write(&script, "/** @schema 2.21 */ return [{type:'rectangle',id:'box',name:'Generated box',width:20,height:10,fill:'#00FF00'}];").unwrap();
        std::fs::write(app.conversation.store().path(), json!({"version":"2.21","children":[{"type":"script","id":"script","scriptUri":"generated.js","width":100,"height":100}]}).to_string()).unwrap();
        let worker = Worker::start(app.conversation.store().clone(), vec![], Backend::halfblocks(), 193).unwrap();
        app.cells = Size::new(20, 12);
        let executable = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("lazypencil");
        let mut script_runtime = crate::script::ScriptRuntime::new(executable);
        let (_, snapshot) = crate::service::layout_snapshot_with_runtime(app.conversation.store(), &[], None, None, &mut script_runtime).unwrap();
        let item = snapshot.items.iter().find(|item| item.record.generated_by.is_some()).unwrap();
        let reference = crate::conversation::references::Reference {
            diagnostic: None,
            target: crate::conversation::references::target(&item.node),
            name: "Generated box".into(), kind: "rectangle".into(), text_excerpt: String::new(),
            revision: snapshot.revision.clone(), dependency_revision: snapshot.dependency_revision.clone(),
        };
        app.outbox.references = vec![reference.clone()];
        app.insert_text("Describe the generated box");
        app.submit().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["type"], "complete");
        app.sync_turn(&worker);
        assert!(!references_received(&temp, 1)[0]["target"]["generated_by"].is_null());
        app.outbox.references = vec![reference];
        app.insert_text("Keep this failed draft");
        std::fs::write(&script, "/** @schema 2.21 */ return [];").unwrap();
        app.submit().await.unwrap();
        assert_eq!(complete(&mut app).await.last().unwrap()["error"]["code"], "NODE_NOT_FOUND");
        app.sync_turn(&worker);
        assert_eq!(app.editor.lines(), ["Keep this failed draft"]);
        assert_eq!(app.outbox.references.len(), 1);
        assert!(!temp.path().join("received-turn-2.json").exists());
        app.conversation.close().await.unwrap();
    });
}
