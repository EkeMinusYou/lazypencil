// Included in tui::tests to share the native preview and key-routing fixtures.

fn preview_search_screen(app: &mut App) -> (String, Option<ratatui::layout::Position>) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
    terminal.draw(|frame| { app.draw(frame); }).unwrap();
    let cursor = terminal.backend().cursor_visible().then(|| terminal.get_cursor_position().unwrap());
    let text = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
    (text, cursor)
}

#[test]
fn preview_search_finds_full_text_names_and_ids_and_cycles_visible_matches() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let document = json!({"version":"2.21","children":[
            {"id":"page","type":"frame","layout":"none","width":1600,"height":600,"children":[
                {"id":"first","type":"text","textGrowth":"fixed-width-height","content":"Hello 日本語","width":180,"height":40},
                {"id":"second","type":"text","textGrowth":"fixed-width-height","x":1000,"content":"HELLO again","width":180,"height":40},
                {"id":"long","type":"text","textGrowth":"fixed-width-height","y":100,"content":format!("{}末尾の文字列", "あ".repeat(520)),"width":1000,"height":300},
                {"id":"named","name":"Hello shape","type":"rectangle","y":450,"width":80,"height":40},
                {"id":"hello-id","type":"rectangle","x":200,"y":450,"width":80,"height":40},
                {"id":"hidden","type":"text","textGrowth":"fixed-width-height","enabled":false,"content":"Hello","width":80,"height":40},
                {"id":"transparent","type":"frame","layout":"none","opacity":0,"width":100,"height":100,"children":[
                    {"id":"transparent-child","type":"text","textGrowth":"fixed-width-height","content":"Hello","width":80,"height":40}
                ]},
                {"id":"clip","type":"frame","layout":"none","clip":true,"width":100,"height":100,"children":[
                    {"id":"clipped","type":"text","textGrowth":"fixed-width-height","x":300,"content":"Hello","width":80,"height":40}
                ]}
            ]}
        ]}).to_string();
        let path = app.conversation.store().path().to_owned();
        std::fs::write(&path, &document).unwrap();
        app.insert_text("保持するプロンプト");
        let draft = app.editor.lines().to_vec();
        let picker = Backend::halfblocks();
        let worker = Worker::start(app.conversation.store().clone(), vec![], picker.clone(), 184).unwrap();
        app.cells = Size::new(80, 30);
        app.request(&worker, false, false);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        app.focus = Focus::Preview;
        app.nodes_collapsed = true;
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        assert!(app.preview_search.editing && app.has_modal());
        for c in "hello".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(app.focus, Focus::Preview);
        assert_eq!(app.selection.ids, ["first"]);
        assert_eq!(app.selected_node().unwrap().id, "first");
        assert!(!app.node_tree.rows().is_empty());
        assert!(app.view.zoom.is_some());
        let (screen, cursor) = preview_search_screen(&mut app);
        assert!(screen.contains("/hello") && screen.contains("1/4"), "{screen}");
        assert!(screen.contains("↑/↓ result") && screen.contains("Enter keep"));
        assert!(cursor.is_some());
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selection.ids, ["second"]);
        assert!(app.view.center[0] > 900.0);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(!app.preview_search.editing && !app.has_modal());
        assert!(preview_search_screen(&mut app).1.is_none());
        for expected in ["named", "hello-id", "first"] {
            press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
            assert_eq!(app.selection.ids, [expected]);
        }
        press(&mut app, KeyCode::Char('N'), KeyModifiers::SHIFT, &worker).await;
        assert_eq!(app.selection.ids, ["hello-id"]);
        assert!(preview_search_screen(&mut app).0.contains("4/4"));
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert!(app.preview_search.query.is_empty());
        assert_eq!(app.selection.ids, ["hello-id"]);

        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        for c in "日本語".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(app.selection.ids, ["first"]);
        assert!(preview_search_screen(&mut app).0.replace(' ', "").contains("/日本語"));
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.preview_search.query, "日本");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        app.paste_preview_search("末尾の文字列", &picker, &worker).unwrap();
        assert_eq!(app.selection.ids, ["long"]);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines(), draft);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), document);
        app.conversation.close().await.unwrap();
    });
}

#[test]
fn preview_search_routes_typing_handles_empty_results_and_refreshes_after_edits() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let path = app.conversation.store().path().to_owned();
        let document = json!({"version":"2.21","children":[
            {"id":"first","name":"First node","type":"text","textGrowth":"fixed-width-height","content":"hello world","width":200,"height":40},
            {"id":"second","type":"text","textGrowth":"fixed-width-height","x":400,"content":"hello world","width":200,"height":40}
        ]}).to_string();
        std::fs::write(&path, &document).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(app.conversation.store().clone(), vec![], picker.clone(), 185).unwrap();
        app.cells = Size::new(80, 30);
        app.request(&worker, false, false);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        app.focus = Focus::Preview;
        app.navigation.search_preview = vec!["F6".into()];
        app.navigation.next_search_match = vec!["b".into()];
        app.navigation.previous_search_match = vec!["Shift+b".into()];
        app.navigation.validate(&app.keys).unwrap();
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        assert!(!app.preview_search.editing);
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        let before = app.view;
        for c in "q?r pn".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(app.preview_search.query, "q?r pn");
        assert_eq!(app.view, before);
        assert!(!app.pointer.space && !app.help && !app.playback.active());
        assert!(preview_search_screen(&mut app).0.contains("No matches"));
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        app.paste_preview_search("hello world", &picker, &worker).unwrap();
        press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT, &worker).await;
        assert_eq!(app.selection.ids, ["second"]);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selection.ids, ["second"]);
        press(&mut app, KeyCode::Char('b'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.selection.ids, ["first"]);
        app.request(&worker, false, false);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(preview_search_screen(&mut app).0.contains("1/2"));
        std::fs::write(&path, document.replace("hello world", "updated text")).unwrap();
        app.request(&worker, false, true);
        app.accept_preview(tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap());
        assert!(preview_search_screen(&mut app).0.contains("No matches"));
        press(&mut app, KeyCode::Char('b'), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(1), KeyModifiers::NONE, &worker).await;
        assert!(app.help);
        let query = app.preview_search.query.clone();
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.preview_search.query, query);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE, &worker).await;
        app.pointer_focus(Focus::Nodes, &worker);
        assert!(!app.preview_search.editing);
        press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE, &worker).await;
        for c in "First".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE, &worker).await;
        }
        assert!(app.searching);
        assert_eq!(app.filter, "First");
        assert_eq!(app.selected_node().unwrap().id, "first");
        assert_eq!(app.preview_search.query, "hello world");
        app.conversation.close().await.unwrap();
    });
}
