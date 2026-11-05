use super::graphics::Backend;
use super::*;
use base64::Engine;
use clap::Parser;
use serde_json::json;
use tempfile::TempDir;

#[path = "reference_tests.rs"]
mod reference_tests;

#[path = "footer_tests.rs"]
mod footer_tests;

#[path = "../../tests/support/fixture.rs"]
mod fixture_support;

#[cfg(unix)]
#[path = "../../tests/support/page_wait_reference.rs"]
mod http_image_reference;

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
fn command_index(action: commands::Action) -> usize {
    commands::COMMANDS
        .iter()
        .position(|command| command.action == action)
        .unwrap()
}
fn command_modal(action: commands::Action) -> checkpoints::Modal {
    checkpoints::Modal::Commands {
        selected: command_index(action),
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
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

// Keep one test module so CI filters and shared process fixtures retain their identities.
include!("tests/preview.rs");
include!("tests/preview_search.rs");
include!("tests/conversation.rs");
include!("tests/images.rs");
include!("tests/navigation.rs");
include!("tests/mouse.rs");
include!("tests/saved.rs");

include!("tests/commands.rs");
