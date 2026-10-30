use super::graphics::Backend;
use super::*;
use base64::Engine;
use clap::Parser;
use serde_json::json;
use tempfile::TempDir;

#[path = "../../tests/support/fixture.rs"]
mod fixture_support;

#[cfg(unix)]
#[path = "../../tests/support/page_wait_reference.rs"]
mod http_image_reference;

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
        assert_eq!(app.editor.lines().join("\n"), draft);
        assert!(!app.key(KeyEvent::new(KeyCode::Tab,KeyModifiers::NONE), &picker, &worker).await.unwrap());
        assert_eq!(app.focus, Focus::Preview);
        assert!(!app.key(KeyEvent::new(KeyCode::Char('c'),KeyModifiers::CONTROL), &picker, &worker).await.unwrap());
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["error"]["code"], "CANCELLED");
        assert_eq!(app.conversation.draft(), draft);
        assert!(!temp.path().join("sent-turn.txt").exists());
        app.conversation.close().await.unwrap();
        worker.cancel();
    });
}

fn app(temp: &TempDir, mode: &str) -> App {
    let args = crate::cli::Arguments::try_parse_from([
        "lazypencil".into(),
        "tui".into(),
        temp.path().join("design.pen").into_os_string(),
        "--create".into(),
        "--adapter".into(),
        fixture_support::helper().into_os_string(),
        "--adapter-arg=acp".into(),
        format!("--adapter-arg={mode}").into(),
    ])
    .unwrap();
    let crate::cli::Command::Tui(args) = args.command else {
        unreachable!()
    };
    let harness = std::env::current_exe().unwrap();
    let executable = harness
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("lazypencil{}", std::env::consts::EXE_SUFFIX));
    assert!(
        executable.is_file(),
        "Build the matching lazypencil binary before running process-backed TUI unit tests: {}",
        executable.display()
    );
    App::new(
        Conversation::open_with_executable(args.prompt, Some(temp.path().join("data")), executable)
            .unwrap(),
    )
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
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
fn prompt_terminal_cursor_tracks_display_cells_scroll_resize_and_focus_for_imes() {
    use ratatui::{Terminal, backend::TestBackend, layout::Position};

    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        for (text, expected) in [
            ("ちょっと".to_owned(), Position::new(9, 30)),
            ("か\u{3099} 😀".to_owned(), Position::new(6, 30)),
            ("か\u{3099} 👩‍💻".to_owned(), Position::new(6, 30)),
            ("🇯🇵".to_owned(), Position::new(3, 30)),
            ("👨‍👩‍👧‍👦".to_owned(), Position::new(3, 30)),
            ("日\tA".to_owned(), Position::new(6, 30)),
            ("日".repeat(80), Position::new(98, 30)),
            ("一\n二\n三\n四\n五".to_owned(), Position::new(3, 32)),
        ] {
            app.editor = editor(&text);
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            assert!(terminal.backend().cursor_visible(), "{text:?}");
            assert_eq!(terminal.backend().cursor_position(), expected, "{text:?}");
            let cell = &terminal.backend().buffer()[expected];
            assert!(
                !cell.modifier.contains(Modifier::REVERSED),
                "Use the native cursor, not a second inverse cell"
            );
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| !cell.modifier.contains(Modifier::HIDDEN)),
                "The temporary caret marker must never reach the terminal"
            );
            assert_eq!(app.editor.lines().join("\n"), text);
        }
        let saved_editor = std::mem::replace(&mut app.editor, editor("abc"));
        app.editor.move_cursor(CursorMove::Head);
        app.editor.start_selection();
        app.editor.move_cursor(CursorMove::Forward);
        app.editor.move_cursor(CursorMove::Forward);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(3, 30));
        assert_eq!(terminal.backend().buffer()[(1, 30)].bg, Color::LightBlue);
        assert_eq!(terminal.backend().buffer()[(2, 30)].bg, Color::LightBlue);
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| !cell.modifier.contains(Modifier::HIDDEN))
        );
        app.editor = saved_editor;
        terminal.backend_mut().resize(50, 16);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(3, 12));
        app.editor = editor(&"日".repeat(80));
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(terminal.backend().cursor_position(), Position::new(48, 10));
        let draft = app.editor.lines().join("\n");
        app.help = true;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        app.help = false;
        app.focus = Focus::Preview;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        app.focus = Focus::Prompt;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(terminal.backend().cursor_visible());
        terminal.backend_mut().resize(49, 15);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert!(!terminal.backend().cursor_visible());
        assert_eq!(app.editor.lines().join("\n"), draft);
        assert!(!app.conversation.active());
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
        assert!(app.status.contains("playing"));
        app.playback_tick(&worker, now + Duration::from_secs(3)).unwrap();
        app.pause_playback();
        assert!(app.status.contains("t=2.000s"), "Status describes the complete image");
        assert!(app.status.contains("finishing requested frame"));
        let mut output = tokio::time::timeout(Duration::from_secs(15), worker.next()).await.unwrap();
        output.result.as_mut().unwrap().time_dependent = true;
        app.accept_preview(output);
        assert!(!app.playback.active());
        assert!(app.status.contains("t=3.000s"));
        assert!(!app.status.contains("finishing requested frame"));
        app.start_playback(5, now + Duration::from_secs(3)).unwrap();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL, &worker).await;
        assert!(!app.playback.active());
        assert_eq!(app.playback.deadline(), None);
        app.start_playback(10, now).unwrap();
        app.cells = Size::new(0, 0);
        press(&mut app, KeyCode::Char(']'), KeyModifiers::NONE, &worker).await;
        assert!(!app.playback.active());
        assert!((app.view.time - 3.1).abs() < 1e-10);
        app.editor = editor("/play 3");
        app.command(&worker).await.unwrap();
        assert_eq!(app.playback.fps(), Some(3));
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE, &worker).await;
        assert!(!app.playback.active());
        press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE, &worker).await;
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
        app.accept_preview(preview::Output {
            generation: app.generation,
            result: Err(Error::new("RENDER_ERROR", "Current frame failed")),
        });
        assert!(!app.playback.active());
        assert!(app.preview.is_none());
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
        press(&mut app, KeyCode::Char(']'), KeyModifiers::NONE, &worker).await;
        assert_eq!(app.view.time, 1.25);
        app.help = false;
        press(&mut app, KeyCode::Char(']'), KeyModifiers::NONE, &worker).await;
        assert!((app.view.time - 1.35).abs() < 1e-10);
        press(&mut app, KeyCode::Char('['), KeyModifiers::NONE, &worker).await;
        assert!((app.view.time - 1.25).abs() < 1e-10);
        app.view.set_time(0.01).unwrap();
        press(&mut app, KeyCode::Char('['), KeyModifiers::NONE, &worker).await;
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
            // Layout capture succeeds before the unsupported viewport fails.
            // Exercise both a first image and recovery after a previous image.
            worker.request(Request {
                generation,
                cells: Size::new(900, 1),
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
            let error = output
                .result
                .err()
                .expect("The viewport exceeds 8192 pixels");
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

fn fake_input(
    executable: &Path,
    path: Option<&Path>,
    _: crate::attachments::AttachmentOptions,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    fake_clipboard(executable, path, cancel)
}

fn fake_clipboard(_: &Path, path: Option<&Path>, _: &CancellationToken) -> Result<InputContent> {
    assert!(path.is_none());
    let mut images = vec![];
    for (name, rgba) in [
        ("red.png", [255, 0, 0, 255]),
        ("blue.png", [0, 0, 255, 128]),
    ] {
        let image = crate::image_input::from_rgba8(1, 1, &rgba)?;
        images.push(crate::clipboard::InputImage {
            name: name.into(),
            source_bytes: image.png.len(),
            data: base64::engine::general_purpose::STANDARD.encode(image.png),
        });
    }
    Ok(InputContent::Images { images })
}

#[test]
fn explicit_send_resize_preserves_the_draft_original_and_persisted_dimensions() {
    use crate::image_input::ImageBounds;
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.conversation
            .attach_file(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
            )
            .unwrap();
        let original = app.conversation.images()[0].clone();
        let bytes = original
            .read_original(app.conversation.directory())
            .unwrap();
        app.editor = editor("日本語の下書きを保持");
        app.start_resize(0, Some(ImageBounds::new(80, 80).unwrap()))
            .unwrap();
        assert_eq!(
            (
                app.conversation.images()[0].width,
                app.conversation.images()[0].height
            ),
            (320, 160)
        );
        assert!(
            app.start_resize(0, Some(ImageBounds::new(40, 40).unwrap()))
                .is_err()
        );
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        let resized = &app.conversation.images()[0];
        assert_eq!((resized.width, resized.height), (80, 40));
        assert_eq!(
            resized.read_original(app.conversation.directory()).unwrap(),
            bytes
        );
        assert!(super::images::label(resized, 0).contains("320×160 → 80×40"));
        assert_eq!(app.editor.lines().join("\n"), "日本語の下書きを保持");
        let saved: Value = serde_json::from_slice(
            &std::fs::read(app.conversation.directory().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["pending_attachments"][0]["width"], 80);
        assert_eq!(saved["pending_attachments"][0]["original"]["width"], 320);
        app.start_resize(0, None).unwrap();
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images()[0].id, original.id);
    });
}

#[test]
fn cancelling_a_prepared_resize_reclaims_only_the_unadopted_send_variant() {
    use crate::image_input::ImageBounds;
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.conversation
            .attach_file(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
            )
            .unwrap();
        let original = app.conversation.images()[0].clone();
        let directory = app.conversation.directory().join("attachments");
        let names = || {
            std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        app.start_resize(0, Some(ImageBounds::new(80, 80).unwrap()))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !app.input.as_ref().unwrap().is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(names().len() > before.len());
        app.input.take();
        assert_eq!(names(), before);
        assert_eq!(app.conversation.images()[0].id, original.id);
        original
            .read_original(app.conversation.directory())
            .unwrap();
        original.read(app.conversation.directory()).unwrap();
    });
}

fn incomplete_image_batch(
    _: &Path,
    _: Option<&Path>,
    _: crate::attachments::AttachmentOptions,
    _: &CancellationToken,
) -> Result<InputContent> {
    assert_eq!(std::thread::current().name(), Some("lazypencil-input"));
    let image = crate::image_input::from_rgba8(1, 1, &[0, 255, 0, 255])?;
    Ok(InputContent::Images {
        images: vec![
            crate::clipboard::InputImage {
                name: "green.png".into(),
                source_bytes: image.png.len(),
                data: base64::engine::general_purpose::STANDARD.encode(image.png),
            },
            crate::clipboard::InputImage {
                name: "invalid.png".into(),
                source_bytes: 1,
                data: "invalid".into(),
            },
        ],
    })
}

#[test]
fn failed_background_image_batches_leave_existing_images_and_blobs_unchanged() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "echo");
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        let directory = app.conversation.directory().join("attachments");
        let names = || {
            std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        let ids = app
            .conversation
            .images()
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>();
        app.input_reader = incomplete_image_batch;
        app.start_input(None).unwrap();
        let content = tokio::time::timeout(
            Duration::from_secs(5),
            &mut app.input.as_mut().unwrap().receiver,
        )
        .await
        .unwrap()
        .unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert!(app.status.contains("ATTACHMENT_INVALID"));
        assert_eq!(names(), before);
        assert_eq!(
            app.conversation
                .images()
                .iter()
                .map(|image| image.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    });
}

async fn complete(app: &mut App) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = vec![];
        loop {
            match app.conversation.next_event().await.unwrap() {
                ConversationEvent::Message(value) => {
                    let done = matches!(value["type"].as_str(), Some("complete" | "failed"));
                    events.push(value);
                    if done {
                        return events;
                    }
                }
                ConversationEvent::Permission { .. } => panic!("Unexpected permission"),
            }
        }
    })
    .await
    .unwrap()
}

#[test]
fn control_v_attaches_ordered_rgba_images_and_two_turns_share_one_agent() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.input_reader = fake_input;
        let picker = Backend::halfblocks();
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            picker.clone(),
            123,
        )
        .unwrap();
        for character in "日本語の見出し\nhjkl+-q".chars() {
            app.key(
                KeyEvent::new(
                    if character == '\n' {
                        KeyCode::Enter
                    } else {
                        KeyCode::Char(character)
                    },
                    KeyModifiers::NONE,
                ),
                &picker,
                &worker,
            )
            .await
            .unwrap();
        }
        assert_eq!(app.editor.lines().join("\n"), "日本語の見出し\nhjkl+-q");
        app.key(
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        assert_eq!(app.conversation.images().len(), 0);
        let content = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images().len(), 2);
        // Removing a pending image never changes its source; re-paste preserves order.
        app.conversation.remove_image(0).unwrap();
        assert_eq!(app.conversation.images()[0].display_name, "blue.png");
        app.conversation.clear_images().unwrap();
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            &picker,
            &worker,
        )
        .await
        .unwrap();
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "complete");
        assert!(app.conversation.draft().is_empty());
        assert!(app.conversation.images().is_empty());
        for (index, expected) in [[255, 0, 0, 255], [0, 0, 255, 128]].iter().enumerate() {
            let image = image::open(temp.path().join(format!("received-image-1-{index}.png")))
                .unwrap()
                .to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, *expected);
        }
        let first: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("received-turn-1.json")).unwrap(),
        )
        .unwrap();
        assert!(
            first["text"]
                .as_array()
                .unwrap()
                .iter()
                .any(|text| text == "日本語の見出し\nhjkl+-q")
        );
        app.editor = editor("次の依頼");
        app.draft_changed = true;
        app.submit().await.unwrap();
        let events = complete(&mut app).await;
        assert!(
            !events.iter().any(|value| value["type"] == "ready"),
            "Second turn must reuse the live connection"
        );
        assert!(
            events
                .iter()
                .any(|value| value["notification"]["update"]["content"]["text"] == "完了 2\n")
        );
        app.conversation.close().await.unwrap();
        let second: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("received-turn-2.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(second["images"], json!([]));
        let logs = std::fs::read_dir(app.conversation.directory())
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl"))
                .then(|| std::fs::read_to_string(path).unwrap())
            })
            .collect::<String>();
        assert!(
            !logs.contains("iVBOR"),
            "Transcripts must omit image payloads"
        );
    });
}

#[test]
fn failed_image_turn_keeps_draft_for_restart_and_does_not_send_text_only() {
    let temp = TempDir::new().unwrap();
    let (id, directory, mut args) = runtime().block_on(async {
        let mut app = app(&temp, "no-images");
        app.insert_text("参考画像を使ってください");
        app.accept_input(fake_clipboard(
            Path::new("unused"),
            None,
            &CancellationToken::new(),
        ));
        app.submit().await.unwrap();
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "failed");
        assert_eq!(app.conversation.draft(), "参考画像を使ってください");
        assert_eq!(app.conversation.images().len(), 2);
        assert!(!temp.path().join("sent-turn.txt").exists());
        let result = (
            app.conversation.id().to_string(),
            app.conversation.directory().to_path_buf(),
            app.conversation.settings().clone(),
        );
        app.conversation.close().await.unwrap();
        result
    });
    args.create = false;
    args.session = Some(id);
    args.prompt.clear();
    let conversation = Conversation::open(args, Some(temp.path().join("data"))).unwrap();
    assert_eq!(conversation.draft(), "参考画像を使ってください");
    assert_eq!(conversation.images().len(), 2);
    assert_eq!(conversation.directory(), directory);
}

#[test]
fn permission_modal_can_answer_and_cancel_without_blocking_the_event_loop() {
    for cancel in [false, true] {
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
            app.submit().await.unwrap();
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
            let events = complete(&mut app).await;
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

fn checkpoint(app: &App, fill: Value, request: &str) -> Value {
    let store = app.conversation.store();
    store.transact(request, &store.read().unwrap().revision(), request, |_| {
        let document = crate::document::Document::parse(json!({"version":"2.21","children":[{"id":"box","type":"rectangle","width":80,"height":40,"fill":fill}]}).to_string())?;
        Ok((document, json!({})))
    }).unwrap()
}
async fn document_result(app: &mut App, worker: &Worker) {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        &mut app.document_task.as_mut().unwrap().receiver,
    )
    .await
    .unwrap()
    .unwrap();
    app.finish_document_result(result, worker);
}
async fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers, worker: &Worker) {
    assert!(
        !app.key(
            KeyEvent::new(code, modifiers),
            &Backend::halfblocks(),
            worker
        )
        .await
        .unwrap()
    );
}

fn saved_log(app: &App, number: usize, text: &str) -> PathBuf {
    let id = format!("00000000-0000-0000-0000-{number:012}");
    let path = app
        .conversation
        .directory()
        .join(format!("turn-{id}.jsonl"));
    let values = [
        json!({"type":"start","session":app.conversation.id(),"turn":id,"document":app.conversation.store().path()}),
        json!({"type":"user_prompt","text":"保存済みの依頼"}),
        json!({"type":"update","notification":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}}),
        json!({"type":"complete","session":app.conversation.id(),"turn":id}),
    ];
    std::fs::write(
        &path,
        values
            .into_iter()
            .map(|value| value.to_string() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    path
}
fn saved_page(app: &App) -> &saved::Page {
    let Some(checkpoints::Modal::Saved(page)) = &app.modal else {
        panic!("Expected saved logs")
    };
    page
}

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
        app.modal = Some(checkpoints::Modal::Commands { selected: 7 });
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
        app.authentication = "subscription".into();
        app.select_conversation(saved::SessionChange::Existing(second.clone()), &worker).await.unwrap();
        assert_eq!(app.editor.lines().join("\n"), "Second unsent draft");
        assert_eq!(app.authentication, "unknown");
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
        for _ in 0..5 { press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await; }
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 50);
        press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 5);
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        assert_eq!(saved_page(&app).data["entries"].as_array().unwrap().len(), 50);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await; document_result(&mut app, &worker).await;
        let first = saved_page(&app).data["text"].as_str().unwrap().to_string();
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
fn custom_prompt_keys_keep_focus_and_modal_precedence_and_send_actual_images() {
    let temp = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut app = app(&temp, "image-inspect");
        app.keys = crate::config::keys::PromptKeys {
            submit: vec!["F5".into()],
            paste: vec!["F6".into()],
        };
        app.keys.validate().unwrap();
        app.input_reader = fake_input;
        let worker = Worker::start(
            app.conversation.store().clone(),
            vec![],
            Backend::halfblocks(),
            135,
        )
        .unwrap();
        app.editor = editor("日本語の依頼");
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert_eq!(app.editor.lines().join("\n"), "日本語の依頼\n");
        press(&mut app, KeyCode::Enter, KeyModifiers::ALT, &worker).await;
        assert!(
            !app.conversation.active(),
            "Replaced submission keys must not submit"
        );
        press(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL, &worker).await;
        assert!(
            app.input.is_none(),
            "Replaced paste keys must not access the clipboard"
        );
        for focus in [
            Focus::Preview,
            Focus::Nodes,
            Focus::Attachments,
            Focus::Conversation,
        ] {
            app.focus = focus;
            press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
            press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
            assert!(app.input.is_none() && !app.conversation.active());
        }
        app.focus = Focus::Prompt;
        app.help = true;
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        assert!(app.input.is_none());
        app.help = false;
        app.modal = Some(checkpoints::Modal::commands());
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        assert!(!app.conversation.active());
        app.modal = None;
        app.searching = true;
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        assert!(!app.conversation.active());
        app.searching = false;
        app.editor = editor("日本語の依頼");
        press(&mut app, KeyCode::F(6), KeyModifiers::NONE, &worker).await;
        let content = (&mut app.input.as_mut().unwrap().receiver).await.unwrap();
        app.input.take();
        app.accept_prepared(content);
        assert_eq!(app.conversation.images().len(), 2);
        press(&mut app, KeyCode::F(5), KeyModifiers::NONE, &worker).await;
        let events = complete(&mut app).await;
        assert_eq!(events.last().unwrap()["type"], "complete");
        assert!(app.conversation.draft().is_empty());
        for (index, expected) in [[255, 0, 0, 255], [0, 0, 255, 128]].iter().enumerate() {
            let received = image::open(temp.path().join(format!("received-image-1-{index}.png")))
                .unwrap()
                .to_rgba8();
            assert_eq!(received.get_pixel(0, 0).0, *expected);
        }
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
        assert!(app.status.contains("Cached preview"));
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
        for character in "xadtzv".chars() {
            press(&mut app, KeyCode::Char(character), KeyModifiers::NONE, &worker).await;
        }
        assert_eq!(app.editor.lines().join("\n"), "xadtzv");
        assert!(!app.help && app.modal.is_none() && !app.conversation.active());
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
        press(&mut app, KeyCode::Char(']'), KeyModifiers::NONE, &worker).await;
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
        assert_eq!(app.focus, Focus::Attachments);
        assert_eq!(std::fs::read(store.path()).unwrap(), saved);
        assert!(app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &picker, &worker).await.unwrap());
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
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        document_result(&mut app, &worker).await;
        press(&mut app, KeyCode::Down, KeyModifiers::NONE, &worker).await;
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE, &worker).await;
        assert!(matches!(app.modal, Some(checkpoints::Modal::Confirm(_))));
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
