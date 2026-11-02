use super::*;

pub(super) fn rows(app: &App, width: u16) -> Vec<String> {
    let height = app.footer_height();
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw_footer(frame, frame.area()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn footer_shows_only_contextual_actions_and_prefers_non_function_bindings() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        for (focus, expected) in [
            (
                Focus::Prompt,
                vec!["Enter send", "Alt+Enter newline", "F1 help"],
            ),
            (Focus::Nodes, vec!["/ search", "d diagnostics", "? help"]),
            (Focus::Preview, vec!["←/↓/↑/→ pan", "+/- zoom", "? help"]),
            (
                Focus::References,
                vec!["Delete remove", "Esc input", "? help"],
            ),
            (
                Focus::Attachments,
                vec!["Delete remove", "Esc input", "? help"],
            ),
            (
                Focus::Conversation,
                vec!["PgUp/PgDn scroll", "Esc input", "? help"],
            ),
        ] {
            app.focus = focus;
            let rendered = rows(&app, 100);
            assert_eq!(rendered.len(), 1);
            for hint in expected {
                assert!(
                    rendered[0].contains(hint),
                    "{focus:?}: {hint}: {}",
                    rendered[0]
                );
            }
            for omitted in [
                "F2",
                "F3",
                "hide Nodes",
                "hide Agent",
                "commands",
                "exits",
                "unbound",
            ] {
                assert!(!rendered[0].contains(omitted), "{focus:?}: {omitted}");
            }
        }
        app.focus = Focus::Prompt;
        app.keys.submit = vec!["F5".into(), "Ctrl+s".into()];
        app.navigation.help = vec!["F1".into(), "Alt+h".into()];
        app.navigation.validate(&app.keys).unwrap();
        let text = rows(&app, 100).join("\n");
        assert!(text.contains("Ctrl+S send") && text.contains("Alt+H help"));
        assert!(!text.contains("F1") && !text.contains("F5"));
        app.keys.submit = vec!["Alt+Enter".into(), "Ctrl+s".into()];
        assert!(rows(&app, 100)[0].contains("Shift+Enter newline"));
        app.focus = Focus::Nodes;
        app.searching = true;
        assert!(rows(&app, 100)[0].contains("Enter keep filter · Esc clear"));
        app.searching = false;
        app.navigation.help.clear();
        assert!(!rows(&app, 100)[0].contains("help"));
        app.navigation.search_nodes.clear();
        assert!(!rows(&app, 100)[0].contains("search"));
        app.help = true;
        assert_eq!(rows(&app, 100)[0].trim(), "Esc close");
    });
}

#[test]
fn footer_drops_whole_hints_preserves_help_and_recovers_collapsed_panels() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.focus = Focus::Preview;
        app.agent_collapsed = true;
        app.nodes_collapsed = true;
        let line = &rows(&app, 50)[0];
        assert!(line.contains("Alt+A show Agent"));
        assert!(line.ends_with("? help"));
        assert!(!line.contains("F3") && !line.contains("zoom"));
        app.agent_collapsed = false;
        assert!(rows(&app, 50)[0].contains("Alt+N show Nodes"));
        app.nodes_collapsed = false;
        app.keys.submit = vec!["Ctrl+Alt+Shift+Enter".into()];
        app.navigation.help = vec!["F1".into(), "Ctrl+Alt+Shift+F24".into()];
        app.focus = Focus::Prompt;
        let line = &rows(&app, 50)[0];
        assert!(line.ends_with("F1 help"));
        assert!(line.contains("Ctrl+Alt+Shift+Enter send"));
        assert!(
            !line.contains("newline"),
            "A partially fitting item must be omitted: {line}"
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(50, 16)).unwrap();
        let mut cells = Size::default();
        terminal
            .draw(|frame| {
                cells = app.draw(frame);
            })
            .unwrap();
        assert_eq!(cells.height, 12, "No blank notification row is reserved");
    });
}

#[test]
fn footer_notifications_expire_but_errors_survive_background_frames_until_recovery() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            197,
        )
        .unwrap();
        app.cells = Size::new(20, 8);
        app.set_status("Node reference added".into());
        assert_eq!(app.footer_height(), 2);
        let deadline = app.status_deadline.unwrap();
        app.expire_status(deadline - Duration::from_millis(1));
        assert_eq!(app.footer_height(), 2);
        app.expire_status(deadline);
        assert_eq!(app.footer_height(), 1);
        app.error(Error::new(
            "CLIPBOARD_UNAVAILABLE",
            "日本語の長いエラー詳細と復旧に必要な説明を履歴に保持します",
        ));
        let error = app.status.clone();
        app.expire_status(Instant::now() + Duration::from_secs(60));
        assert_eq!(app.status, error);
        let rendered = rows(&app, 50);
        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].starts_with("! CLIPBOARD_UNAVAILABLE"));
        assert!(rendered[0].ends_with('…'));
        assert!(rendered[1].ends_with("F1 help"));
        assert!(app.log.contains(&error));
        app.request(&worker, false, false);
        loop {
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            if output.generation == app.generation {
                app.accept_preview(output);
                break;
            }
        }
        assert_eq!(
            app.status, error,
            "An unrelated successful preview cannot erase an error"
        );
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("RENDER_ERROR", "Preview failure")),
        });
        app.request(&worker, false, true);
        assert!(app.status.contains("Preview failure"));
        loop {
            let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            if output.generation == app.generation {
                app.accept_preview(output);
                break;
            }
        }
        assert!(
            app.status.is_empty(),
            "Successful rendering resolves its own error"
        );
        app.set_attention_status("Queued prompts restored".into());
        app.expire_status(Instant::now() + Duration::from_secs(60));
        assert_eq!(app.footer_height(), 2);
        app.editor = editor(&"x".repeat(1024 * 1024));
        app.accept_input(Ok(InputContent::Text { text: "y".into() }));
        assert_eq!(app.status, "Prompt exceeds 1 MiB");
        assert!(app.status_kind.is_error());
    });
}
