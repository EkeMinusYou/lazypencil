// Included in tui::tests to preserve the existing test identities.

#[test]
fn previous_preview_survives_failed_updates_and_retries_until_recovery() {
    use ratatui::{Terminal, backend::TestBackend};
    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        let document = json!({"version":"2.21","children":[{
            "id":"page","type":"frame","width":180,"height":100,"children":[
                {"id":"ready","name":"Ready node","type":"rectangle","width":40,"height":30,"fill":"#FF0000"}
            ]}]}).to_string();
        std::fs::write(store.path(), &document).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(store.clone(), vec![], picker.clone(), 195).unwrap();
        app.cells = Size::new(70, 25);
        app.request(&worker, false, false);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        screen(&mut app);
        app.refresh_nodes(Some("ready"));
        app.selection.ids = vec!["ready".into()];
        app.insert_text("日本語の未送信プロンプト");
        let draft = app.editor.lines().to_vec();
        let conversation_log = app.log.clone();
        let previous = app.preview.as_ref().unwrap();
        let png = previous.image.png().unwrap().into_owned();
        let revision = previous.summary["revision"].clone();
        let view = previous.view;

        std::fs::write(store.path(), document.replace("2.21", "2.8")).unwrap();
        app.view.pan([15., 10.]);
        app.request(&worker, false, true);
        assert!(app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap()));
        assert!(app.preview_stale);
        assert_eq!(app.preview_error.as_ref().unwrap().code, "UNSUPPORTED_VERSION");
        assert_eq!(app.preview.as_ref().unwrap().image.png().unwrap().as_ref(), png);
        assert_eq!(app.preview.as_ref().unwrap().view, view);
        assert_eq!(app.selected_node().unwrap().id, "ready");
        assert_eq!(app.selection.ids, ["ready"]);
        assert!(!app.nodes_loading);
        let state = app.preview_state(&picker);
        assert_eq!(state["preview"]["stale"], true);
        assert_eq!(state["preview"]["updating"], false);
        assert_eq!(state["preview"]["displayed"]["revision"], revision);
        assert_ne!(state["preview"]["requested_view"], state["preview"]["displayed"]["view"]);
        let failed = screen(&mut app);
        assert!(failed.contains("Showing previous preview"));
        assert!(failed.contains("Nodes · Previous"));
        assert!(failed.contains("Ready node"));
        assert!(!failed.contains("Preview unavailable"));
        assert_eq!(app.open_preview().unwrap_err().code, "PREVIEW_UNAVAILABLE");
        assert!(!app.conversation.directory().join("previews").exists());

        app.set_status("Unrelated user action".into());
        app.dirty = false;
        app.request(&worker, true, false);
        assert!(!app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap()));
        assert!(!app.dirty);
        assert_eq!(app.status, "Unrelated user action");
        assert_eq!(app.preview.as_ref().unwrap().image.png().unwrap().as_ref(), png);

        app.request(&worker, false, true);
        assert!(app.preview_error.is_none());
        assert_eq!(app.preview_state(&picker)["preview"]["updating"], true);
        assert!(screen(&mut app).contains("Showing previous preview"));
        assert_eq!(app.open_preview().unwrap_err().code, "PREVIEW_UNAVAILABLE");
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(app.preview_stale);

        let repaired = document.replace("#FF0000", "#0000FF");
        std::fs::write(store.path(), &repaired).unwrap();
        app.request(&worker, false, true);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(!app.preview_stale);
        assert!(app.preview_error.is_none());
        assert!(app.status.is_empty());
        let state = app.preview_state(&picker);
        assert_eq!(state["preview"]["stale"], false);
        assert_eq!(state["preview"]["updating"], false);
        assert_ne!(state["preview"]["displayed"]["revision"], revision);
        assert_ne!(app.preview.as_ref().unwrap().image.png().unwrap().as_ref(), png);
        assert_eq!(app.selected_node().unwrap().id, "ready");
        assert_eq!(app.selection.ids, ["ready"]);
        assert_eq!(app.editor.lines(), draft);
        assert_eq!(app.log, conversation_log);
        let recovered = screen(&mut app);
        assert!(!recovered.contains("Showing previous preview"));
        assert!(!recovered.contains("Nodes · Previous"));
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), repaired);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn preview_missing_glyphs_display_initial_design_warn_and_recover_without_changing_copy() {
    use ratatui::{Terminal, backend::TestBackend};
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let document = json!({"version":"2.21","children":[{
            "id":"page","type":"frame","width":180,"height":100,"fill":"#FFFFFF","children":[
                {"id":"shape","type":"rectangle","x":120,"y":20,"width":20,"height":20,"fill":"#00FF00"},
                {"id":"label","type":"text","x":10,"y":10,"content":"日本語 \u{10ffff} English","fontSize":16,"width":100,"textGrowth":"fixed-width","stroke":"#000000","strokeWidth":0.5}
            ]}]}).to_string();
        let store = app.conversation.store().clone();
        std::fs::write(store.path(), &document).unwrap();
        let worker = Worker::start(store.clone(), vec![], Backend::halfblocks(), 194).unwrap();
        app.cells = Size::new(70, 25);
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        assert!(app.accept_preview(output));
        assert!(app.preview_error.is_none());
        let preview = app.preview.as_ref().unwrap();
        assert_eq!(preview.summary["provisional"], true);
        let diagnostics = preview.summary["diagnostics"].as_array().unwrap();
        let warnings = diagnostics.iter().filter(|item| item["code"] == "PREVIEW_MISSING_GLYPHS").collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1, "Repeated text measurement must not duplicate warnings");
        assert_eq!(warnings[0]["node_id"], "label");
        assert_eq!(warnings[0]["severity"], "warning");
        assert_eq!(warnings[0]["details"]["missing_characters"], json!(["\u{10ffff}"]));
        let label = preview.nodes.iter().find(|node| node.id == "label").unwrap();
        assert!(label.text.contains('\u{10ffff}'));
        assert!(label.diagnostic_count > 0);
        let pixels = image::load_from_memory(&preview.image.png().unwrap()).unwrap().into_rgba8();
        assert!(pixels.pixels().any(|pixel| pixel.0 == [0, 255, 0, 255]), "Unaffected shapes remain visible");
        let placeholder_png = preview.image.png().unwrap().into_owned();
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        let screen = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(screen.contains("Missing glyphs · provisional layout"));
        assert!(!screen.contains("Preview unavailable"));
        assert_eq!(app.open_preview().unwrap_err().code, "ASSET_NOT_FOUND");
        assert!(!app.conversation.directory().join("previews").exists());
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
        assert_eq!(crate::service::layout_snapshot(&store, &[], None, None).err().unwrap().code, "ASSET_NOT_FOUND");

        // The corrected document must paint exactly like the temporary squares.
        std::fs::write(store.path(), document.replace('\u{10ffff}', "□")).unwrap();
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        assert!(app.accept_preview(output));
        let preview = app.preview.as_ref().unwrap();
        assert_eq!(preview.summary["provisional"], false);
        assert!(preview.summary["diagnostics"].as_array().unwrap().iter().all(|item| item["code"] != "PREVIEW_MISSING_GLYPHS"));
        assert_eq!(preview.image.png().unwrap().as_ref(), placeholder_png);
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        let screen = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(!screen.contains("provisional layout"));
        assert!(crate::service::layout_snapshot(&store, &[], None, None).is_ok());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn collapsed_panels_expand_preview_skip_focus_and_preserve_editor_and_tree_state() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        std::fs::write(
            app.conversation.store().path(),
            json!({"version":"2.21","children":[
                {"id":"root","name":"Panel","type":"frame","width":80,"height":40,"children":[
                    {"id":"child","type":"rectangle","width":10,"height":10}
                ]}
            ]})
            .to_string(),
        )
        .unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            193,
        )
        .unwrap();
        app.cells = Size::new(51, 31);
        app.request(&worker, false, false);
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
        app.nodes.select(Some(0));
        app.navigate_node_tree(Nav::ExpandNode);
        assert_eq!(app.filtered_nodes().len(), 2);
        app.insert_text("日本語の下書き\n編集中");
        app.append("\nYou: Earlier prompt\n\nCodex:\nEarlier response\n");
        app.scroll = 7;
        let draft = app.editor.lines().to_vec();
        let cursor = app.editor.cursor();
        let log = app.log.clone();
        let view = app.view;
        for (key, nodes_collapsed, agent_collapsed, focus, preview_width) in [
            (KeyCode::F(2), true, false, Focus::Prompt, 73),
            (KeyCode::F(3), true, true, Focus::Preview, 98),
            (KeyCode::Char('n'), false, true, Focus::Nodes, 76),
            (KeyCode::Char('a'), false, false, Focus::Prompt, 51),
        ] {
            let modifiers = if matches!(key, KeyCode::Char(_)) {
                KeyModifiers::ALT
            } else {
                KeyModifiers::NONE
            };
            press(&mut app, key, modifiers, &worker).await;
            assert_eq!(
                (app.nodes_collapsed, app.agent_collapsed),
                (nodes_collapsed, agent_collapsed)
            );
            assert_eq!(app.focus, focus);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
            let mut cells = Size::default();
            terminal
                .draw(|frame| {
                    cells = app.draw(frame);
                })
                .unwrap();
            assert_eq!(cells, Size::new(preview_width, 32));
            let buffer = terminal.backend().buffer();
            let heading = (0..100)
                .map(|x| buffer[(x, 1)].symbol())
                .collect::<String>();
            assert_eq!(heading.contains("Nodes"), !nodes_collapsed);
            assert_eq!(heading.contains("Agent"), !agent_collapsed);
            assert!(!heading.contains("hide"));
            let footer = (0..100)
                .map(|x| buffer[(x, 35)].symbol())
                .collect::<String>();
            assert!(!footer.contains("F2") && !footer.contains("F3"));
            if agent_collapsed {
                assert!(footer.contains("Alt+A show Agent"));
            } else if nodes_collapsed {
                assert!(footer.contains("Alt+N show Nodes"));
            } else {
                assert!(footer.contains("Enter send") && footer.contains("Alt+Enter newline"));
            }
            for (code, modifiers) in [
                (KeyCode::Tab, KeyModifiers::NONE),
                (KeyCode::BackTab, KeyModifiers::SHIFT),
            ] {
                press(&mut app, code, modifiers, &worker).await;
                assert!(!nodes_collapsed || app.focus != Focus::Nodes);
                assert!(!agent_collapsed || !app.focus.in_conversation());
            }
            assert_eq!(app.focus, focus);
            assert_eq!(app.editor.lines(), draft);
            assert_eq!(app.editor.cursor(), cursor);
            assert_eq!(app.log, log);
            assert_eq!(app.scroll, 7);
            assert_eq!(app.selected_node().unwrap().id, "root");
            assert_eq!(app.filtered_nodes().len(), 2);
            assert_eq!(app.view.center, view.center);
            assert_eq!(app.view.zoom, view.zoom);
        }
        app.focus = Focus::Nodes;
        app.searching = true;
        app.filter = "Panel".into();
        app.search_nodes(Some("root"));
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE, &worker).await;
        assert!(!app.searching);
        assert_eq!(app.filter, "Panel");
        assert_eq!(app.focus, Focus::Preview);
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(3), KeyModifiers::NONE, &worker).await;
        app.searching = true;
        press(&mut app, KeyCode::F(3), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Prompt);
        assert!(!app.searching, "Expanding Agent must leave node search");
        press(&mut app, KeyCode::F(3), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT, &worker).await;
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert!(
            !app.agent_collapsed,
            "Adding a reference must reveal the input"
        );
        assert_eq!(app.focus, Focus::Prompt);
        assert_eq!(app.outbox.references.len(), 1);
        assert_eq!(app.editor.lines(), draft);
        assert!(!temp.path().join("sent-turn.txt").exists());
    });
}

#[test]
fn panel_metadata_uses_global_header_and_contextual_preview_labels() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        std::fs::write(
            app.conversation.store().path(),
            json!({"version":"2.21","children":[
                {"type":"rectangle","id":"shape","width":80,"height":40,"fill":"#00FF00"}
            ]})
            .to_string(),
        )
        .unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            185,
        )
        .unwrap();
        app.cells = Size::new(20, 8);
        app.view.zoom = Some(1.0);
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        app.panel_layout = crate::config::keys::PanelLayout {
            nodes: 2,
            preview: 5,
            agent: 3,
        };
        app.agent.authentication = "fixture@example.test · subscription · team".into();
        app.message(&json!({"type":"ready","response":{"configOptions":[
            {"id":"model","currentValue":"gpt-6.1-sol"},
            {"id":"fast-mode","currentValue":"on"}
        ]}}));
        app.preview.as_mut().unwrap().summary["diagnostics"] = json!([{"severity":"warning"}]);
        app.view.zoom = Some(2.0);
        assert_eq!(
            app.preview_zoom_label().as_deref(),
            Some("100%"),
            "Pending zoom must not describe the old image"
        );
        assert_eq!(app.preview_timeline(), None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let region = |area: Rect| {
            let mut text = String::new();
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    text.push_str(terminal.backend().buffer()[(x, y)].symbol());
                }
                text.push('\n');
            }
            text.replace(' ', "")
        };
        let header = region(Rect::new(0, 0, 160, 1));
        assert!(header.contains("lazypencil") && header.contains(".pen"));
        for value in [
            "Codex",
            "gpt-6.1-sol",
            "fast",
            "fixture@example.test",
            &app.conversation.id()[..8],
            "subscription",
        ] {
            assert!(
                header.contains(value),
                "Missing agent metadata in the global header: {value}"
            );
        }
        assert!(!header.contains("Idle"));
        let nodes = region(Rect::new(0, 1, 32, 38));
        assert!(nodes.contains("1node") && nodes.contains("1diagnostic"));
        let preview = region(Rect::new(32, 1, 80, 38));
        assert!(preview.contains("Preview"));
        let preview_heading = region(Rect::new(32, 1, 80, 1));
        assert!(!preview_heading.contains('%'));
        assert!(region(Rect::new(32, 38, 80, 1)).contains("100%"));
        let zoom_cell = &terminal.backend().buffer()[(109, 38)];
        assert_eq!(zoom_cell.symbol(), "%");
        assert_eq!(zoom_cell.fg, theme::colors().secondary);
        assert!(!zoom_cell.modifier.contains(Modifier::BOLD));
        assert!(!preview.contains("Playback"));
        let agent = region(Rect::new(112, 1, 48, 38));
        assert!(agent.contains("Agent") && agent.contains("Idle"));
        assert!(!agent.contains("Codex"));
        for value in ["fixture@example.test", "subscription", &app.conversation.id()[..8]] {
            assert!(
                !agent.contains(value),
                "Metadata must not occupy the Agent history: {value}"
            );
        }
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].bg, theme::colors().canvas);
        assert_eq!(buffer[(3, 2)].fg, theme::colors().text);
        assert_eq!(buffer[(3, 2)].bg, theme::colors().canvas);
        assert_eq!(buffer[(114, 2)].symbol(), " ");
        assert_eq!(buffer[(114, 2)].fg, theme::colors().text);
        for (x, label, color) in [
            (2, "Nodes", theme::colors().nodes),
            (34, "Preview", theme::colors().preview),
            (114, "Agent", theme::colors().agent),
        ] {
            let start = (x..160)
                .find(|x| buffer[(*x, 1)].symbol() == label.get(..1).unwrap())
                .unwrap();
            assert_eq!(buffer[(start, 1)].fg, color);
        }
        crate::tui::test_artifacts::artifact(buffer, "workspace");
        let footer = region(Rect::new(0, 39, 160, 1));
        assert!(!footer.contains("node") && !footer.contains("100%") && !footer.contains("t="));
        assert!(footer.contains("Entersend") && footer.contains("F1help"));
        for (nodes, agent, width) in [(true, false, 110), (false, true, 126), (true, true, 158)] {
            app.nodes_collapsed = nodes;
            app.agent_collapsed = agent;
            let mut cells = Size::default();
            terminal.draw(|frame| cells = app.draw(frame)).unwrap();
            assert_eq!(cells.width, width);
        }
        app.nodes_collapsed = false;
        app.agent_collapsed = false;
        app.preview.as_mut().unwrap().time_dependent = true;
        terminal.backend_mut().resize(50, 16);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(32, 14)].symbol(), "%");
        assert_eq!(
            terminal.backend().buffer()[(32, 14)].fg,
            theme::colors().secondary
        );
        app.preview.as_mut().unwrap().time_dependent = false;
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.focus = Focus::Attachments;
        app.attachments.select(Some(0));
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        assert_eq!(app.preview.as_ref().unwrap().nodes.len(), 1);
        assert_eq!(
            app.preview.as_ref().unwrap().summary["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(app.preview_zoom_label(), None);
        assert_eq!(app.preview_timeline(), None);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn playback_preserves_input_pauses_on_seek_errors_and_static_output_and_drops_late_frames() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        std::fs::write(store.path(), json!({"version":"2.21","children":[{"type":"rectangle","id":"static","width":80,"height":40,"fill":"#00FF00"}]}).to_string()).unwrap();
        let saved = std::fs::read(store.path()).unwrap();
        let worker = Worker::start(store, vec![], Backend::halfblocks(), 135).unwrap();
        app.cells = Size::new(20, 8);
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        app.accept_preview(output);
        let now = Instant::now();
        assert!(app.start_playback(5, now).is_err());
        app.preview.as_mut().unwrap().time_dependent = true;
        app.start_playback(5, now).unwrap();
        let deadline = app.playback.deadline();
        for text in [
            "/play 0",
            "/play 11",
            "/play -1",
            "/play 1.5",
            "/play 1 2",
            "/time NaN",
        ] {
            app.editor = editor(text);
            assert_eq!(
                app.command(&worker).await.unwrap_err().code,
                "INVALID_ARGUMENT"
            );
            assert_eq!(app.playback.deadline(), deadline);
            assert_eq!(app.editor.lines(), &[text]);
        }
        app.editor = editor("kept draft");
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE, &worker).await;
        assert!(app.playback.active());
        assert_eq!(app.editor.lines(), &["kept draft "]);
        app.focus = Focus::Preview;
        app.help = true;
        app.playback_tick(&worker, now + Duration::from_secs(1))
            .unwrap();
        assert_eq!(app.generation, 1);
        assert_eq!(app.view.time, 0.0);
        app.help = false;
        // The elapsed clock advances while the renderer is deferred. No catch-up frames.
        app.playback_tick(&worker, now + Duration::from_secs(2))
            .unwrap();
        assert_eq!(app.generation, 2);
        assert_eq!(app.view.time, 2.0);
        let mut output = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        assert_eq!(output.result.as_ref().unwrap().view.time, 2.0);
        output.result.as_mut().unwrap().time_dependent = true;
        app.accept_preview(output);
        assert!(app.playback.active());
        assert_eq!(app.editor.lines(), &["kept draft "]);
        assert!(app.preview_timeline().unwrap().contains("playing"));
        app.playback_tick(&worker, now + Duration::from_secs(3)).unwrap();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 34)).unwrap();
        let mut playing_size = Size::default();
        terminal.draw(|frame| playing_size = app.draw(frame)).unwrap();
        app.pause_playback();
        assert!(app.preview_timeline().unwrap().contains("Playback 2.000s"), "Timeline describes the complete image");
        assert!(app.preview_timeline().unwrap().contains("finishing requested frame"));
        terminal.draw(|frame| assert_eq!(app.draw(frame), playing_size, "Pausing must not resize and cancel an in-flight retained page capture")).unwrap();
        let mut output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        output.result.as_mut().unwrap().time_dependent = true;
        app.accept_preview(output);
        assert!(!app.playback.active());
        assert!(app.preview_timeline().unwrap().contains("Playback 3.000s"));
        assert!(!app.preview_timeline().unwrap().contains("finishing requested frame"));
        terminal.draw(|frame| assert_eq!(app.draw(frame), playing_size)).unwrap();
        app.start_playback(5, now + Duration::from_secs(3)).unwrap();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL, &worker).await;
        assert!(!app.playback.active());
        assert_eq!(app.playback.deadline(), None);
        app.start_playback(10, now).unwrap();
        app.cells = Size::new(0, 0);
        press(&mut app, KeyCode::Char(']'), KeyModifiers::ALT, &worker).await;
        assert!(!app.playback.active());
        assert!((app.view.time - 3.1).abs() < 1e-10);
        app.editor = editor("/play 3");
        app.command(&worker).await.unwrap();
        assert_eq!(app.playback.fps(), Some(3));
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await;
        assert!(!app.playback.active());
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.playback.fps(), Some(5));
        app.editor = editor("/pause");
        app.command(&worker).await.unwrap();
        assert!(!app.playback.active());
        app.start_playback(5, now).unwrap();
        app.accept_preview(preview::Output {
            generation: app.generation - 1,
            result: Err(Error::new("RENDER_ERROR", "Superseded frame")),
        });
        assert!(app.playback.active());
        let previous_image = app.preview.as_ref().unwrap().image.identity();
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("RENDER_ERROR", "Current frame failed")),
        });
        assert!(!app.playback.active());
        assert!(app.preview_stale);
        assert_eq!(app.preview.as_ref().unwrap().image.identity(), previous_image);
        app.cells = Size::new(20, 8);
        app.request(&worker, false, false);
        let mut output = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        output.result.as_mut().unwrap().time_dependent = true;
        app.accept_preview(output);
        app.start_playback(5, now).unwrap();
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        app.accept_preview(output);
        assert!(
            !app.playback.active(),
            "A newly static document stops the timer"
        );
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            saved
        );
        assert!(!app.conversation.active());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn browser_only_previews_play_and_stop_when_the_completed_scene_becomes_static() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        std::fs::write(store.path(), json!({"version":"2.21","children":[{"type":"rectangle","id":"static","width":80,"height":40,"fill":"#00FF00"}]}).to_string()).unwrap();
        let saved = std::fs::read(store.path()).unwrap();
        let worker = Worker::start(store, vec![], Backend::halfblocks(), 137).unwrap();
        app.cells = Size::new(20,8);
        app.request(&worker, false, false);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(!app.preview.as_ref().unwrap().time_dependent);
        assert!(!app.preview.as_ref().unwrap().live_pages);
        assert!(app.start_playback(2, Instant::now()).is_err());
        // The real browser-only path is exercised by the page-playback PTY case.
        app.preview.as_mut().unwrap().live_pages = true;
        let now = Instant::now();
        app.start_playback(2, now).unwrap();
        app.editor = editor("kept page draft");
        app.playback_tick(&worker, now + Duration::from_secs(1)).unwrap();
        assert_eq!(app.view.time, 1.0);
        assert!(app.playback.active());
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        assert!(!output.result.as_ref().unwrap().live_pages);
        app.accept_preview(output);
        assert!(!app.playback.active());
        assert_eq!(app.playback.deadline(), None);
        assert_eq!(app.editor.lines(), &["kept page draft"]);
        assert!(!app.conversation.active());
        assert_eq!(std::fs::read(app.conversation.store().path()).unwrap(), saved);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn time_commands_validate_without_sending_or_saving_and_view_keys_keep_input_precedence() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        let saved = std::fs::read(store.path()).unwrap();
        let worker = Worker::start(store, vec![], Backend::halfblocks(), 134).unwrap();
        app.editor = editor("/time 1.25");
        app.command(&worker).await.unwrap();
        assert_eq!(app.view.time, 1.25);
        assert_eq!(app.conversation.draft(), "");
        assert!(!app.conversation.active());
        for text in [
            "/time NaN",
            "/time inf",
            "/time -1",
            "/time 1e999",
            "/time",
            "/time 1 2",
            "/time nope",
        ] {
            app.editor = editor(text);
            assert_eq!(
                app.command(&worker).await.unwrap_err().code,
                "INVALID_ARGUMENT"
            );
            assert_eq!(app.editor.lines(), &[text]);
            assert_eq!(app.view.time, 1.25);
        }
        app.editor = editor("");
        press(&mut app, KeyCode::Char(']'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), &["]"]);
        assert_eq!(app.view.time, 1.25);
        app.focus = Focus::Preview;
        app.help = true;
        press(&mut app, KeyCode::Char(']'), KeyModifiers::ALT, &worker).await;
        assert_eq!(app.view.time, 1.25);
        app.help = false;
        press(&mut app, KeyCode::Char(']'), KeyModifiers::ALT, &worker).await;
        assert!((app.view.time - 1.35).abs() < 1e-10);
        press(&mut app, KeyCode::Char('['), KeyModifiers::ALT, &worker).await;
        assert!((app.view.time - 1.25).abs() < 1e-10);
        app.view.set_time(0.01).unwrap();
        press(&mut app, KeyCode::Char('['), KeyModifiers::ALT, &worker).await;
        assert_eq!(app.view.time, 0.0);
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            saved
        );
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn periodic_preview_checks_retry_a_captured_layout_after_render_failure() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        let document = json!({"version":"2.21","children":[{
            "id":"red","type":"rectangle","width":80,"height":40,"fill":"#FF0000"
        }]})
        .to_string();
        std::fs::write(store.path(), &document).unwrap();
        let worker = Worker::start(store.clone(), vec![], Backend::halfblocks(), 130).unwrap();
        let mut successful_png = None;
        for generation in [1, 2] {
            // Layout capture succeeds before the empty viewport fails.
            // Exercise both a first image and recovery after a previous image.
            worker.request(Request {
                generation,
                cells: Size::new(0, 1),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            });
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            assert_eq!(output.generation, generation);
            let error = output.result.err().expect("The viewport has no width");
            assert_eq!(error.code, "RENDER_ERROR");

            let request = Request {
                generation,
                cells: Size::new(20, 8),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: true,
                attachment: None,
            };
            worker.request(request.clone());
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .expect("A periodic check must recover an unpublished image");
            assert_eq!(output.generation, generation);
            let preview = output.result.unwrap();
            assert_eq!(preview.nodes.len(), 1);
            assert_eq!(preview.nodes[0].id, "red");
            let png = preview.image.png().unwrap().into_owned();
            let pixels = image::load_from_memory(&png).unwrap().to_rgba8();
            assert!(pixels.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
            if let Some(previous) = &successful_png {
                assert_eq!(previous, &png);
            }
            successful_png = Some(png);
            // A current, published image still avoids periodic re-encoding.
            worker.request(request);
            assert!(
                tokio::time::timeout(Duration::from_millis(150), worker.next())
                    .await
                    .is_err()
            );
        }
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn initial_preview_loading_is_distinct_from_failure_and_explicit_retry() {
    use ratatui::{Terminal, backend::TestBackend};
    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let initial = screen(&mut app);
        let conversation_log = app.log.clone();
        assert!(initial.contains("Loading preview…"));
        assert!(initial.contains("Loading nodes…"));
        assert!(!initial.contains("Rendering errors"));
        assert!(!initial.contains("Preview unavailable"));
        crate::tui::test_artifacts::snapshot(&mut app, "preview-loading");
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("RENDER_ERROR", "Owned preview failure")),
        });
        let failed = screen(&mut app);
        assert!(failed.contains("Preview unavailable"));
        assert!(failed.contains("Nodes unavailable"));
        assert!(!failed.contains("Loading nodes…"));
        assert!(failed.contains("Owned preview failure"));
        assert!(!failed.contains("Loading preview…"));
        let buffer = crate::tui::test_artifacts::snapshot(&mut app, "preview-error");
        let heading = "Preview unavailable".chars().map(|c| c.to_string()).collect::<Vec<_>>();
        let cells = buffer.content.windows(heading.len()).find(|cells| {
            cells.iter().zip(&heading).all(|(cell, symbol)| cell.symbol() == symbol)
        }).unwrap();
        assert!(cells.iter().all(|cell| cell.fg == theme::colors().error && cell.bg == theme::colors().canvas));
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            173,
        )
        .unwrap();
        app.cells = Size::new(20, 8);
        app.request(&worker, false, true);
        // An obsolete failure must not replace the retry's loading state.
        app.accept_preview(preview::Output {
            generation: app.generation - 1,
            result: Err(Error::new("RENDER_ERROR", "Stale preview failure")),
        });
        let retry = screen(&mut app);
        assert!(retry.contains("Loading preview…"));
        assert!(retry.contains("Loading nodes…"));
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        assert!(app.preview.is_some());
        let loaded = screen(&mut app);
        assert!(!loaded.contains("Loading preview…"));
        assert!(!loaded.contains("Loading nodes…"));
        assert!(loaded.contains("No nodes"));
        assert!(!loaded.contains("Preview unavailable"));
        assert_eq!(app.log, conversation_log, "Preview errors do not belong in Agent history");
        assert!(!app.log.contains("Stale preview failure"));
        std::fs::write(app.conversation.store().path(), json!({"version":"2.21","children":[{
            "type":"rectangle","id":"ready","name":"Ready node","width":20,"height":10,"fill":"#00FF00"
        }]}).to_string()).unwrap();
        app.request(&worker, false, true);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        app.refresh_nodes(Some("ready"));
        app.request(&worker, false, true);
        let refreshing = screen(&mut app);
        assert!(refreshing.contains("Nodes · Loading…"));
        assert!(refreshing.contains("Ready node"));
        assert_eq!(app.selected_node().unwrap().id, "ready");
        app.accept_preview(preview::Output {
            generation: app.generation - 1,
            result: Err(Error::new("RENDER_ERROR", "Stale refresh failure")),
        });
        assert!(app.nodes_loading);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(!app.nodes_loading);
        app.request(&worker, true, false);
        assert!(!screen(&mut app).contains("Loading…"));
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("REVISION_CONFLICT", "Concurrent document update")),
        });
        assert_eq!(app.selected_node().unwrap().id, "ready");
        app.request(&worker, false, true);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert_eq!(app.selected_node().unwrap().id, "ready");
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("REVISION_CONFLICT", "Another document update")),
        });
        app.focus = Focus::Nodes;
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        app.request(&worker, false, true);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(app.selected_node().is_none());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn repeated_preview_errors_are_quiet_without_blocking_repair_or_explicit_retry() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        let worker = Worker::start(store.clone(), vec![], Backend::halfblocks(), 174).unwrap();
        app.cells = Size::new(20, 8);
        app.insert_text("日本語の未送信プロンプト");
        let mut previous_version = None;
        let mut has_completed_preview = false;
        for (version, check_only, changed) in [
            ("2.9", false, true),
            ("2.9", true, false),
            ("2.9", true, false),
            ("2.8", true, true),
            ("2.21", true, true),
            ("2.22", true, true),
            ("2.9", true, true),
            ("2.9", false, true),
        ] {
            let document = json!({"version":version,"children":[{
                "id":"shape","type":"rectangle","width":80,"height":40,"fill":"#FF0000"
            }]})
            .to_string();
            if previous_version != Some(version) {
                std::fs::write(store.path(), &document).unwrap();
                previous_version = Some(version);
            }
            app.request(&worker, check_only, !check_only);
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .expect("Failed previews must remain retryable");
            let log = app.log.clone();
            app.set_status("Unrelated user action".into());
            app.dirty = false;
            assert_eq!(app.accept_preview(output), changed);
            assert_eq!(app.dirty, changed);
            if !changed {
                assert_eq!(app.log, log);
                assert_eq!(app.status, "Unrelated user action");
            }
            if matches!(version, "2.21" | "2.22") {
                assert!(app.preview.is_some());
                assert!(app.preview_error.is_none());
                assert!(!app.preview_stale);
                has_completed_preview = true;
            } else {
                assert_eq!(app.preview.is_some(), has_completed_preview);
                assert_eq!(app.preview_stale, has_completed_preview);
                assert_eq!(
                    app.preview_error.as_ref().unwrap().code,
                    "UNSUPPORTED_VERSION"
                );
                assert!(
                    app.preview_error
                        .as_ref()
                        .unwrap()
                        .message
                        .contains("supported versions are 2.19, 2.21, 2.22")
                );
                assert!(!app.log.contains("supported versions are 2.19, 2.21, 2.22"));
            }
            assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
            assert_eq!(app.editor.lines(), &["日本語の未送信プロンプト"]);
            assert!(!app.conversation.active());
        }
        assert!(!app.log.contains("Cannot resolve version"));
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn sixel_camera_gestures_use_the_coalescing_worker_without_interim_encodes() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        std::fs::write(app.conversation.store().path(), json!({"version":"2.21","children":[
            {"type":"rectangle","id":"shape","x":24,"y":16,"width":120,"height":80,"fill":"#2563EB"}
        ]}).to_string()).unwrap();
        let saved = std::fs::read(app.conversation.store().path()).unwrap();
        let (picker, _) = Backend::detect(ImageProtocol::Sixel).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            179,
        )
        .unwrap();
        app.cells = Size::new(20, 8);
        app.view.zoom = Some(1.0);
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        assert!(app.preview.is_some());
        app.focus = Focus::Preview;
        for key in [
            KeyCode::Right,
            KeyCode::Char('+'),
            KeyCode::Down,
            KeyCode::Char('-'),
        ] {
            app.key(KeyEvent::new(key, KeyModifiers::NONE), &picker, &worker)
                .await
                .unwrap();
            assert!(
                app.camera.is_none(),
                "Sixel must not encode an interim frame on the input thread"
            );
        }
        let requested = app.view;
        let latest = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let output = worker.next().await;
                if output.generation == app.generation {
                    break output;
                }
            }
        })
        .await
        .unwrap();
        app.accept_preview(latest);
        let final_frame = app.preview.as_ref().unwrap();
        assert!(matches!(
            final_frame.protocol,
            graphics::Encoded::Protocol(ratatui_image::protocol::Protocol::Sixel(_))
        ));
        assert_eq!(final_frame.view.center, requested.center);
        assert_eq!(final_frame.view.zoom, requested.zoom);
        assert!(!final_frame.image.png().unwrap().is_empty());
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            saved
        );
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn cached_camera_feedback_keeps_exact_exports_and_generation_ownership_separate() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        std::fs::write(store.path(), json!({"version":"2.21","children":[
            {"type":"rectangle","id":"shape","x":24,"y":16,"width":120,"height":80,"fill":"#2563EB"}
        ]}).to_string()).unwrap();
        let saved = std::fs::read(store.path()).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(store, vec![], picker.clone(), 157).unwrap();
        app.cells = Size::new(20, 8);
        app.view = View {
            center: [100.0, 80.0],
            zoom: Some(1.0),
            time: 0.0,
        };
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        let exact = app
            .preview
            .as_ref()
            .unwrap()
            .image
            .png()
            .unwrap()
            .into_owned();
        let original_view = app.preview.as_ref().unwrap().view;
        app.focus = Focus::Preview;
        press(&mut app, KeyCode::Right, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.camera.as_ref().unwrap().generation, app.generation);
        assert!(app.status.is_empty());
        assert_eq!(
            app.preview.as_ref().unwrap().image.png().unwrap().as_ref(),
            exact
        );
        assert_eq!(
            app.preview.as_ref().unwrap().view.center,
            original_view.center
        );
        assert!(
            app.open_preview()
                .unwrap_err()
                .message
                .contains("complete image")
        );
        app.accept_preview(preview::Output {
            generation: app.generation - 1,
            result: Err(Error::new("RENDER_ERROR", "Superseded frame")),
        });
        assert!(app.camera.is_some());
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        assert!(app.camera.is_none());
        assert_eq!(app.preview.as_ref().unwrap().view.center, app.view.center);
        assert_ne!(
            app.preview.as_ref().unwrap().image.png().unwrap().as_ref(),
            exact
        );
        // Another camera gesture can be superseded by a time change or explicit refresh.
        press(&mut app, KeyCode::Left, KeyModifiers::NONE, &worker).await;
        assert!(app.camera.is_some());
        app.view.set_time(1.0).unwrap();
        app.request(&worker, false, false);
        app.camera_feedback(&picker).unwrap();
        assert!(app.camera.is_none());
        assert_eq!(
            std::fs::read(app.conversation.store().path()).unwrap(),
            saved
        );
        assert!(!app.conversation.active());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn custom_navigation_preserves_typing_search_modals_and_moves_the_actual_preview() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let store = app.conversation.store().clone();
        std::fs::write(store.path(), json!({"version":"2.21","children":[
            {"type":"rectangle","id":"first","name":"first","width":80,"height":40,"fill":"#00FF00"},
            {"type":"rectangle","id":"second","name":"second","x":100,"width":80,"height":40,"fill":"#0000FF"}
        ]}).to_string()).unwrap();
        let saved = std::fs::read(store.path()).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(store.clone(), vec![], picker.clone(), 155).unwrap();
        app.cells = Size::new(20, 8);
        app.request(&worker, false, false);
        let output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        app.accept_preview(output);
        assert_eq!(app.preview.as_ref().unwrap().nodes.len(), 2);
        app.navigation = crate::config::keys::NavigationKeys {
            next_panel: vec!["F6".into()], previous_panel: vec!["F7".into()],
            help: vec!["F8".into(), "t".into()], commands: vec!["F9".into()],
            quit: vec!["x".into()], pan_left: vec!["a".into()], pan_right: vec!["d".into()],
            pan_up: vec!["w".into()], pan_down: vec!["s".into()],
            zoom_in: vec!["z".into()], zoom_out: vec!["v".into()],
            fit_document: vec!["c".into()], next_item: vec!["n".into()],
            previous_item: vec!["p".into()], time_forward: vec![".".into()],
            time_backward: vec![",".into()], follow_output: vec!["F10".into()],
            ..Default::default()
        };
        app.navigation.validate(&app.keys).unwrap();
        app.editor = editor("");
        for character in "xadtzvhjkl".chars() {
            press(&mut app, KeyCode::Char(character), KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(app.editor.lines().join("\n"), "xadtzvhjkl");
        assert!(!app.help && app.modal.is_none() && !app.conversation.active());
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Nodes);
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Preview);
        let center = app.view.center;
        let zoom = app.view.zoom.unwrap();
        press(&mut app, KeyCode::Char('h'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center, center);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center[0], center[0] - 64.0 / f64::from(zoom));
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.zoom, Some((zoom * 1.25).min(16.0)));
        let zoomed = app.view.zoom;
        press(&mut app, KeyCode::Char('0'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.zoom, zoomed);
        press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.zoom, None);
        press(&mut app, KeyCode::Char(']'), KeyModifiers::ALT, &worker).await;
        assert_eq!(app.view.time, 0.0);
        press(&mut app, KeyCode::Char('.'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.time, 0.1);
        press(&mut app, KeyCode::Char(','), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.time, 0.0);
        press(&mut app, KeyCode::F(8), KeyModifiers::NONE, &worker).await;
        assert!(app.help);
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.center[0], center[0] - 64.0 / f64::from(zoom));
        press(&mut app, KeyCode::End, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.help_scroll, u16::MAX);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
        terminal.draw(|frame| { app.draw(frame); }).unwrap();
        assert!(app.help_scroll > 0 && app.help_scroll < u16::MAX);
        assert!(app.help_text().contains("a: Preview: pan left"));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(9), KeyModifiers::NONE, &worker).await;
        assert!(app.modal.is_some());
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE, &worker).await;
        assert!(app.modal.is_some());
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        app.focus = Focus::Nodes;
        app.nodes.select(Some(0));
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.nodes.selected(), Some(0));
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.nodes.selected(), Some(1));
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Nodes);
        assert_eq!(app.selected_node().unwrap().id, "second");
        assert_eq!(app.view.center, [140.0, 20.0]);
        assert!(app.camera.is_some());
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.nodes.selected(), Some(0));
        press(&mut app, KeyCode::Char('f'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Nodes);
        assert_eq!(app.selected_node().unwrap().id, "first");
        assert_eq!(app.view.center, [40.0, 20.0]);
        assert!(app.camera.is_some());
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('t'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE, &worker).await;
        assert!(app.searching && !app.help);
        assert_eq!(app.filter, "tx");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(!app.searching && app.filter.is_empty());
        app.focus = Focus::Conversation;
        app.scroll = 20;
        press(&mut app, KeyCode::End, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.scroll, 20);
        press(&mut app, KeyCode::F(10), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.scroll, 0);
        press(&mut app, KeyCode::F(7), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.focus, Focus::Preview);
        assert_eq!(std::fs::read(store.path()).unwrap(), saved);
        assert!(app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &picker, &worker).await.unwrap());
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn native_navigation_displays_only_exact_frames_without_a_settle_time_position_correction() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let document = json!({"version":"2.21","children":[{
            "id":"shape","type":"rectangle","width":160,"height":100,"fill":"#2563EB"
        }]})
        .to_string();
        std::fs::write(app.conversation.store().path(), &document).unwrap();
        app.insert_text("日本語の下書き");
        let (picker, _) = Backend::detect(ImageProtocol::Kitty).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            175,
        )
        .unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 36)).unwrap();
        app.presentation.rectangular(true);
        app.focus = Focus::Preview;
        terminal.draw(|frame| app.cells = app.draw(frame)).unwrap();
        app.view = View {
            center: [80.0, 50.0],
            zoom: Some(0.73),
            time: 0.0,
        };
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        for key in [
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Char('+'),
            KeyCode::Char('-'),
        ] {
            let previous_view = app.preview.as_ref().unwrap().view;
            let previous_png = app
                .preview
                .as_ref()
                .unwrap()
                .image
                .png()
                .unwrap()
                .into_owned();
            app.key(KeyEvent::new(key, KeyModifiers::NONE), &picker, &worker)
                .await
                .unwrap();
            let target = app.view;
            assert_ne!(target, previous_view);
            assert!(app.camera_navigation && app.camera.is_none() && app.camera_deadline.is_none());
            assert!(app.status.is_empty());
            assert!(app.open_preview().is_err());
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let commands = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                !commands.contains("_G"),
                "Pending input must keep the previous completed image without moving or scaling it"
            );
            assert_eq!(app.preview.as_ref().unwrap().view, previous_view);
            assert_eq!(
                app.preview.as_ref().unwrap().image.png().unwrap().as_ref(),
                previous_png
            );

            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            assert_eq!(output.result.as_ref().unwrap().view, target);
            let exact_png = output
                .result
                .as_ref()
                .unwrap()
                .image
                .png()
                .unwrap()
                .into_owned();
            assert!(app.accept_interactive_preview(output, &worker, &picker));
            assert_eq!(app.preview.as_ref().unwrap().view, target);
            assert!(app.camera.is_none());
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let commands = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(commands.contains("p=1,c="));
            assert!(!commands.contains(",p=2,"));
            let exact_image = image::load_from_memory(&exact_png).unwrap();
            assert!(
                commands.contains(&format!(
                    ",x=0,y=0,w={},h={};",
                    exact_image.width(),
                    exact_image.height()
                )),
                "A completed frame places the full image without a camera crop"
            );
            assert_eq!(
                app.preview.as_ref().unwrap().image.png().unwrap().as_ref(),
                exact_png
            );

            // Settling an already displayed target must not replace its pixels
            // or introduce a second placement with different geometry.
            app.finish_camera_gesture(&worker);
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let commands = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(!commands.contains("_G"));
            assert!(!app.camera_navigation && app.camera.is_none());
            assert_eq!(app.view, target);
            assert_eq!(
                app.preview.as_ref().unwrap().image.png().unwrap().as_ref(),
                exact_png
            );
        }
        assert_eq!(app.editor.lines(), &["日本語の下書き"]);
        assert_eq!(
            std::fs::read_to_string(app.conversation.store().path()).unwrap(),
            document
        );
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn progressive_navigation_refreshes_sources_without_rewinding_or_exporting_interim_views() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let path = app.conversation.store().path().to_path_buf();
        let document = json!({"version":"2.21","children":[
            {"type":"rectangle","id":"red","width":180,"height":90,"fill":"#FF0000"},
            {"type":"rectangle","id":"blue","x":400,"width":180,"height":90,"fill":"#0000FF"}
        ]})
        .to_string();
        std::fs::write(&path, &document).unwrap();
        app.insert_text("日本語の下書き");
        let (picker, _) = Backend::detect(ImageProtocol::Kitty).unwrap();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            210,
        )
        .unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 36)).unwrap();
        app.presentation.rectangular(true);
        app.focus = Focus::Preview;
        let mut cells = app.cells;
        terminal.draw(|frame| cells = app.draw(frame)).unwrap();
        app.cells = cells;
        app.view = View {
            center: [90.0, 45.0],
            zoom: Some(3.0),
            time: 0.0,
        };
        app.request(&worker, false, false);
        app.accept_preview(
            tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap(),
        );
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(app.presentation.source_size().is_some());
        app.key(
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        let epoch = app.generation;
        let first = tokio::time::timeout(Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        let rendered_view = first.result.as_ref().unwrap().view;
        for _ in 0..3 {
            app.key(
                KeyEvent::new_with_kind(
                    KeyCode::Char('l'),
                    KeyModifiers::NONE,
                    KeyEventKind::Repeat,
                ),
                &picker,
                &worker,
            )
            .await
            .unwrap();
            assert_eq!(app.generation, epoch);
            assert_ne!(app.preview.as_ref().unwrap().view, rendered_view);
        }
        let target = app.view;
        assert_ne!(rendered_view, target);
        assert!(app.accept_interactive_preview(first, &worker, &picker));
        assert_eq!(app.view, target);
        assert_eq!(app.preview.as_ref().unwrap().view, rendered_view);
        assert!(app.camera_navigation && app.camera.is_none());
        assert!(app.preview_generation.is_none());
        assert!(
            app.open_preview()
                .unwrap_err()
                .message
                .contains("complete image")
        );
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let commands = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            commands.find("a=t,").unwrap() < commands.find("a=p,").unwrap(),
            "Only a completed whole image may replace the previous viewport"
        );
        app.finish_camera_gesture(&worker);
        while app.camera_navigation {
            assert!(
                app.accept_interactive_preview(
                    tokio::time::timeout(Duration::from_secs(15), worker.next())
                        .await
                        .unwrap(),
                    &worker,
                    &picker,
                )
            );
            assert_eq!(app.view, target);
        }
        assert_eq!(app.preview.as_ref().unwrap().view, target);
        assert!(app.camera.is_none());
        assert_eq!(app.preview_generation, Some(app.generation));
        assert!(!app.accept_interactive_preview(
            preview::Output {
                generation: epoch,
                result: Err(Error::new("RENDER_ERROR", "Late navigation source"))
            },
            &worker,
            &picker
        ));
        assert_eq!(app.view, target);
        assert_eq!(app.editor.lines(), &["日本語の下書き"]);
        assert_eq!(std::fs::read_to_string(path).unwrap(), document);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn workspace_focus_uses_color_without_changing_borders() {
    use crate::config::keys::Theme;
    runtime().block_on(async {
        let temp = TempDir::new().unwrap();
        let mut app = app(&temp, "echo");
        app.append("You: Focus check\n\nCodex:\nResponse stays in the conversation\n");
        let area = Rect::new(0, 0, 130, 34);
        for (name, preset, background) in [
            ("light", Theme::Light, [250, 250, 250]),
            ("auto-light", Theme::Auto, [250, 250, 250]),
            ("monochrome-light", Theme::Monochrome, [250, 250, 250]),
            ("dark", Theme::Dark, [30, 30, 46]),
            ("monochrome-dark", Theme::Monochrome, [30, 30, 46]),
        ] {
            let _theme = theme::activate(preset, Some(background));
            assert_ne!(theme::colors().focus, theme::colors().frame);
            for focus in [
                Focus::Nodes,
                Focus::Preview,
                Focus::Conversation,
                Focus::Prompt,
            ] {
                app.focus = focus;
                let buffer = crate::tui::test_artifacts::snapshot(
                    &mut app,
                    &format!("focus-color-{name}-{focus:?}"),
                );
                let areas = workspace::areas(area, &app.panel_layout, false, false, 1);
                for (index, pane) in areas.panes.iter().enumerate() {
                    let active = match index {
                        0 => focus == Focus::Nodes,
                        1 => focus == Focus::Preview,
                        _ => focus.in_conversation(),
                    };
                    let corner = &buffer[(pane.x, pane.y)];
                    assert_eq!(corner.symbol(), "╭", "{preset:?}: {focus:?}, pane {index}");
                    assert_eq!(
                        corner.fg,
                        if active {
                            theme::colors().focus
                        } else {
                            theme::colors().frame
                        }
                    );
                }
                let prompt = app
                    .pointer
                    .area(mouse::Target::Focus(Focus::Prompt))
                    .unwrap();
                assert_eq!(buffer[(prompt.right() - 1, prompt.y)].symbol(), "─");
                assert_eq!(
                    buffer[(prompt.right() - 1, prompt.y)].fg,
                    if focus == Focus::Prompt {
                        theme::colors().focus
                    } else {
                        theme::colors().frame
                    }
                );
                let header = (0..area.width)
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>();
                assert_eq!(header.matches("Codex").count(), 1);
                let agent = areas.panes[2].inner(ratatui::layout::Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                let history = (agent.y..prompt.y)
                    .map(|y| {
                        (agent.x..agent.right())
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<String>();
                assert!(!history.contains("Codex"));
                assert!(history.contains("Focus check") && history.contains("Response stays"));
            }
        }
        app.conversation.close().await.unwrap();
    });
}
