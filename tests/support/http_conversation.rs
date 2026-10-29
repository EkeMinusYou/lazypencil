use super::*;
use clap::Parser;
use lazypencil::conversation::{Conversation, ConversationEvent};
use std::{fs, time::Instant};

#[path = "page_wait_reference.rs"]
mod reference;

fn conversation(temp: &TempDir, url: &str) -> Conversation {
    let file = temp.path().join("design.pen");
    fs::write(&file, json!({"version":"2.21","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":url}}]}).to_string()).unwrap();
    let args = lazypencil::cli::Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        file.into_os_string(),
        "--adapter".into(),
        helper().into_os_string(),
        "--adapter-arg=acp".into(),
        "--adapter-arg=echo".into(),
        "日本語の下書きを保持する".into(),
    ])
    .unwrap();
    let lazypencil::cli::Command::Prompt(args) = args.command else {
        unreachable!()
    };
    Conversation::open_with_executable(
        args,
        Some(temp.path().join("data")),
        env!("CARGO_BIN_EXE_lazypencil").into(),
    )
    .unwrap()
}

async fn blocked(server: &reference::Server) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while server.network_counts().0 == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Preparation never reached the actual HTTP server");
}

async fn disconnected(server: &reference::Server) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while server.closed_image_requests() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Cancelled preparation retained the HTTP connection");
}

#[test]
fn http_prompt_preparation_yields_preserves_draft_and_recovers_after_cancellation() {
    let temp = TempDir::new().unwrap();
    let server = reference::Server::new();
    let mut conversation = conversation(
        &temp,
        &format!("http://{}/network/http-image/blocked.png", server.address),
    );
    let before = fs::read(conversation.store().path()).unwrap();
    let draft = conversation.draft().to_owned();
    runtime().block_on(async {
        let started = Instant::now();
        conversation.start_send(None).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        blocked(&server).await;
        assert!(conversation.active());
        assert!(conversation.set_draft("replacement".into()).is_err());
        // Repeated frontend select losses must retain the same request, not
        // cancel it, launch another worker or consume the oneshot result.
        for _ in 0..3 {
            assert!(
                tokio::time::timeout(Duration::from_millis(20), conversation.next_event())
                    .await
                    .is_err()
            );
        }
        assert_eq!(server.network_counts().0, 1);
        conversation.cancel();
        let event = tokio::time::timeout(Duration::from_secs(10), conversation.next_event())
            .await
            .unwrap()
            .unwrap();
        let ConversationEvent::Message(value) = event else {
            panic!("Preparation must not request agent permissions");
        };
        assert_eq!(value["phase"], "preparation");
        assert_eq!(value["error"]["code"], "CANCELLED");
        assert!(!conversation.active());
        assert_eq!(conversation.draft(), draft);
        assert_eq!(fs::read(conversation.store().path()).unwrap(), before);
        assert!(!temp.path().join("sent-turn.txt").exists());
        disconnected(&server).await;

        // Reuse this local conversation after cancellation, and capture a real
        // successful HTTP image before submitting exactly one ordinary turn.
        let document = String::from_utf8(before)
            .unwrap()
            .replace("blocked.png", "yellow.png");
        fs::write(conversation.store().path(), document).unwrap();
        conversation.start_send(None).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let ConversationEvent::Message(value) = conversation.next_event().await.unwrap()
                    && matches!(value["type"].as_str(), Some("complete" | "failed"))
                {
                    break value;
                }
            }
        })
        .await;
        conversation.close().await.unwrap();
        let terminal = result.unwrap();
        assert_eq!(terminal["type"], "complete", "{terminal}");
        assert_eq!(conversation.draft(), "");
        assert!(temp.path().join("sent-turn.txt").exists());
    });
}

#[test]
fn http_prompt_preparation_close_and_drop_cancel_the_owned_capture() {
    for close in [true, false] {
        let temp = TempDir::new().unwrap();
        let server = reference::Server::new();
        let mut conversation = conversation(
            &temp,
            &format!("http://{}/network/http-image/blocked.png", server.address),
        );
        let before = fs::read(conversation.store().path()).unwrap();
        let state = conversation.directory().join("session.json");
        let draft = conversation.draft().to_owned();
        runtime().block_on(async {
            conversation.start_send(None).unwrap();
            blocked(&server).await;
            let started = Instant::now();
            if close {
                conversation.close().await.unwrap();
                assert!(!conversation.active());
            }
            drop(conversation);
            assert!(started.elapsed() < Duration::from_secs(10));
            disconnected(&server).await;
        });
        assert_eq!(fs::read(temp.path().join("design.pen")).unwrap(), before);
        let state: Value = serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
        assert_eq!(state["draft"], draft);
        assert!(!temp.path().join("sent-turn.txt").exists());
    }
}

#[test]
fn http_prompt_preparation_rejects_a_document_changed_before_submission() {
    let temp = TempDir::new().unwrap();
    let server = reference::Server::new();
    let mut conversation = conversation(
        &temp,
        &format!("http://{}/network/http-image/blocked.png", server.address),
    );
    fs::write(
        temp.path().join("lazypencil.toml"),
        "[runtime.browser]\ntimeout_ms=1000\n",
    )
    .unwrap();
    let draft = conversation.draft().to_owned();
    runtime().block_on(async {
        conversation.start_send(None).unwrap();
        blocked(&server).await;
        let revised = json!({"version":"2.21","children":[]}).to_string();
        fs::write(conversation.store().path(), &revised).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(10), conversation.next_event()).await;
        conversation.close().await.unwrap();
        let ConversationEvent::Message(value) = event.unwrap().unwrap() else {
            panic!("Stale prompt context must fail before authentication");
        };
        assert_eq!(value["error"]["code"], "REVISION_CONFLICT", "{value}");
        assert_eq!(conversation.draft(), draft);
        assert_eq!(
            fs::read_to_string(conversation.store().path()).unwrap(),
            revised
        );
        assert!(!temp.path().join("sent-turn.txt").exists());
        disconnected(&server).await;
    });
}
