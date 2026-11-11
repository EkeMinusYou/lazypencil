use super::*;

pub fn history_bottom(h: &mut Harness) -> Result {
    h.visible("fixture-model")?;
    h.paste("Original request")?;
    h.key(b"\r")?;
    h.wait("original turn saved", 15.0, |h| h.completed_turns(1))?;
    h.paste("Preserved draft")?;
    h.draft("Preserved draft")?;
    let id = state(h)["id"].as_str().unwrap().to_owned();
    let document_bytes = fs::read(&h.document)?;
    h.finish()?;
    let path = files(&h.root.join("data"), |name| {
        name.starts_with("turn-") && name.ends_with(".jsonl")
    })[0]
        .clone();
    let mut records = fs::read_to_string(&path)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let index = records
        .iter()
        .rposition(|value| value["type"] == "complete")
        .ok_or("Missing completed turn")?;
    let response = format!(
        "OLDEST_SAVED\n{}\nLATEST_RESTORED\n",
        "Saved response 日本語 abcdefghijklmnopqrstuvwxyz\n".repeat(6000)
    );
    records.insert(index, json!({"type":"update","notification":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":response}}}}));
    fs::write(
        &path,
        records
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>(),
    )?;
    let next = Harness::start(
        &h.binary,
        &h.root,
        &h.document,
        h.environment.clone(),
        "history-bottom",
        h.configuration.clone(),
    )?;
    *h = next;
    h.wait("native history replay started", 15.0, |h| {
        h.root.join("history-replay-started").exists()
    })?;
    h.visible("LATEST_RESTORED")?;
    h.pump_for(0.2)?;
    assert!(selected(h, &id));
    assert!(h.screen.text().contains("LATEST_RESTORED"));
    assert!(!h.screen.text().contains("REPLAYED_OLD"));
    assert!(!h.screen.text().contains("OLDEST_SAVED"));
    h.save_screen("history-bottom-loading-screen.txt")?;
    fs::write(h.root.join("history-replay-release"), b"release")?;
    h.visible("fixture-model")?;
    assert!(h.screen.text().contains("LATEST_RESTORED"));
    assert!(!h.screen.text().contains("REPLAYED_OLD"));
    h.draft("Preserved draft")?;
    assert_eq!(
        h.count("start"),
        1,
        "Restoring and connecting must not submit a request"
    );
    h.resize(100, 28)?;
    h.visible("LATEST_RESTORED")?;
    h.save_screen("history-bottom-ready-screen.txt")?;
    h.resize(145, 38)?;
    h.key(b"\t\x15")?;
    h.paste("Fresh request")?;
    h.key(b"\r")?;
    h.wait("fresh turn completes after restoring", 15.0, |h| {
        h.completed_turns(2)
    })?;
    h.visible("完了 1")?;
    assert_eq!(fs::read(&h.document)?, document_bytes);
    Ok(())
}

pub fn completion(h: &mut Harness) -> Result {
    h.wait("initial complete preview", 15.0, |h| {
        !h.frames.images.is_empty()
    })?;
    let original = h.frames.images.last().unwrap().clone();
    h.paste("/")?;
    h.visible("/ Commands")?;
    h.key(b"con")?;
    h.visible("/conversations")?;
    h.key(b"\t")?;
    h.draft("/conversations")?;
    h.hidden("/ Commands")?;
    assert!(!h.root.join("sent-turn.txt").exists());
    assert_eq!(h.frames.images.last(), Some(&original));
    h.key(b"\x15")?;
    h.paste("/att")?;
    h.key(b"\r")?;
    h.draft("/attach ")?;
    assert_eq!(state(h)["pending_attachments"], json!([]));
    h.save_screen("slash-completion-arguments-screen.txt")?;
    h.key(b"\x15")?;
    h.paste("/theme light")?;
    h.key(b"\r")?;
    h.visible("Use the command palette")?;
    h.draft("/theme light")?;
    assert_eq!(h.frames.images.last(), Some(&original));
    command_input(h, "Theme", "light")?;
    h.visible("UI theme: Light")?;
    h.draft("/theme light")?;
    h.key(b"\x15")?;
    h.paste("/help")?;
    h.key(b"\r")?;
    h.visible("Conversation commands")?;
    assert!(!h.screen.text().contains("Preview: pan"));
    h.key(b"\x1b")?;
    h.hidden("Conversation commands")?;
    h.resize(50, 16)?;
    h.paste("/")?;
    for _ in 0..11 {
        h.key(b"\x1b[B")?;
    }
    h.key(b"\t")?;
    h.draft("/help")?;
    h.save_screen("slash-completion-small-screen.txt")?;
    assert_eq!(h.count("start"), 0);
    assert!(!h.root.join("sent-turn.txt").exists());
    Ok(())
}

pub fn steering(h: &mut Harness) -> Result {
    h.paste("First request")?;
    h.key(b"\r")?;
    h.wait("mid-turn input available", 15.0, |h| {
        let text = h.screen.text();
        text.contains("Working…") && text.contains("steer")
    })?;
    h.paste("/queue Check the layout's spacing after this")?;
    h.key(b"\r")?;
    h.wait("explicit follow-up remains queued", 15.0, |h| {
        h.screen.text().contains("1 queued")
    })?;
    assert_eq!(h.count("complete"), 0);
    h.paste("Use English everywhere")?;
    h.key(b"\r")?;
    h.wait("steering and queued follow-up complete", 15.0, |h| {
        h.count("complete") == 2
    })?;
    let steers = fs::read_to_string(h.root.join("received-steering.jsonl"))?;
    let steers = steers
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0]["prompt"][0]["text"], "Use English everywhere");
    let prompts = fs::read_to_string(h.root.join("received-prompts.jsonl"))?;
    let prompts = prompts
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1]["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["text"] == "Check the layout's spacing after this")
    );
    h.paste("Unsent newer draft")?;
    h.draft("Unsent newer draft")?;
    h.save_screen("steering-complete-screen.txt")?;
    Ok(())
}

fn other_session(h: &Harness, original: &Path) -> Option<(PathBuf, Value)> {
    let paths: Vec<_> = h.sessions().into_iter().filter(|p| p != original).collect();
    if paths.len() == 1 {
        read_json(&paths[0]).map(|v| (paths[0].clone(), v))
    } else {
        None
    }
}
fn selected(h: &Harness, id: &str) -> bool {
    h.screen
        .text()
        .lines()
        .next()
        .unwrap_or_default()
        .contains(&format!(" · {} · ", &id[..8]))
}
fn highlight(h: &mut Harness, target: &str, first: &str, second: &str) -> Result {
    let mut order = [first, second];
    order.sort();
    let text = h.screen.text();
    let current = order
        .iter()
        .position(|id| {
            text.lines()
                .any(|line| line.contains("▏›") && line.contains(*id))
        })
        .expect("Actual selected conversation in the modal list");
    let target_index = order.iter().position(|id| *id == target).unwrap();
    arrow(h, target_index as isize - current as isize)?;
    h.wait("target conversation highlighted", 15.0, |h| {
        h.screen
            .text()
            .lines()
            .any(|s| s.contains('›') && s.contains(target))
    })
}

pub fn selection(h: &mut Harness) -> Result<Value> {
    let first_draft = "最初の未送信の下書き\nq u N は本文";
    let second_draft = "二番目の未送信の下書き";
    h.paste(first_draft)?;
    h.draft(first_draft)?;
    let first_path = h.sessions()[0].clone();
    let first_id = state(h)["id"].as_str().unwrap().to_owned();
    let document = fs::read(&h.document)?;
    command(h, "New empty conversation")?;
    h.wait("new empty conversation selected", 15.0, |h| {
        other_session(h, &first_path).is_some_and(|(_, v)| {
            selected(h, v["id"].as_str().unwrap())
                && !h
                    .screen
                    .text()
                    .contains(first_draft.lines().next().unwrap())
        })
    })?;
    let (second_path, second) = other_session(h, &first_path).unwrap();
    let second_id = second["id"].as_str().unwrap().to_owned();
    h.paste(second_draft)?;
    h.wait("second conversation draft persisted", 15.0, |_| {
        read_json(&second_path).is_some_and(|v| v["draft"] == second_draft)
    })?;
    for (name, id, draft) in [
        ("first", &first_id, first_draft),
        ("second", &second_id, second_draft),
    ] {
        command(h, "Saved conversations")?;
        h.wait("both saved conversations listed", 15.0, |h| {
            h.screen.text().contains("Saved conversations")
                && h.screen.text().contains(&first_id)
                && h.screen.text().contains(&second_id)
        })?;
        highlight(h, id, &first_id, &second_id)?;
        h.key(b"u")?;
        h.wait("draft restored by session selection", 15.0, |h| {
            selected(h, id)
                && h.screen.text().contains(draft.lines().next().unwrap())
                && !h.screen.text().contains("Saved conversations")
        })?;
        h.save_screen(&format!("session-selection-{name}-screen.txt"))?;
    }
    assert_eq!(read_json(&first_path).unwrap()["draft"], first_draft);
    assert_eq!(read_json(&second_path).unwrap()["draft"], second_draft);
    assert!(!h.root.join("sent-turn.txt").exists());
    assert!(
        files(&h.root.join("data"), |s| s.starts_with("turn-")
            && s.ends_with(".jsonl"))
        .is_empty()
    );
    assert_eq!(fs::read(&h.document)?, document);
    fs::copy(first_path, h.root.join("session-selection-first.json"))?;
    fs::copy(second_path, h.root.join("session-selection-second.json"))?;
    Ok(
        json!({"source_commit":source_commit(),"case":h.case,"first_session":first_id,"second_session":second_id,"selected_on_exit":second_id,"first_draft_sha256":hash(first_draft),"second_draft_sha256":hash(second_draft),"document_sha256":hash(document),"turn_files":0}),
    )
}

pub fn deletion(h: &mut Harness) -> Result<Value> {
    let archived_draft = "削除する保存会話の下書き";
    let current_draft = "現在の未送信下書きは保持🙂";
    h.paste(archived_draft)?;
    h.draft(archived_draft)?;
    let archived_path = h.sessions()[0].clone();
    let archived_id = state(h)["id"].as_str().unwrap().to_owned();
    let document = fs::read(&h.document)?;
    command(h, "New empty conversation")?;
    h.wait("new current conversation selected", 15.0, |h| {
        other_session(h, &archived_path)
            .is_some_and(|(_, v)| selected(h, v["id"].as_str().unwrap()))
    })?;
    let (current_path, current) = other_session(h, &archived_path).unwrap();
    let current_id = current["id"].as_str().unwrap().to_owned();
    h.paste(current_draft)?;
    h.wait("current draft persisted", 15.0, |_| {
        read_json(&current_path).is_some_and(|v| v["draft"] == current_draft)
    })?;
    command(h, "Saved conversations")?;
    h.wait("both conversations listed", 15.0, |h| {
        h.screen.text().contains("Saved conversations")
            && h.screen.text().contains(&archived_id)
            && h.screen.text().contains(&current_id)
    })?;
    h.key(b"d")?;
    h.visible("Select another conversation")?;
    highlight(h, &archived_id, &archived_id, &current_id)?;
    if h.case == "session-deletion-small" {
        h.resize(50, 16)?;
        h.wait("minimum terminal size", 15.0, |h| {
            h.screen.text().split('\n').count() == 16
        })?;
    }
    let review = |h: &Harness| {
        let text = h.screen.text();
        text.contains("Delete local conversation?")
            && text.contains(&archived_id)
            && text.contains("y Delete")
    };
    h.key(b"d")?;
    h.wait("review before confirmation", 15.0, review)?;
    h.save_screen("session-deletion-review-screen.txt")?;
    if h.case == "session-deletion-small" {
        h.key(b"\x1b[F")?;
        h.wait("scrolled review preserves UUID and controls", 15.0, |h| {
            review(h)
                && h.screen
                    .text()
                    .contains("Project assets and provider history")
                && h.screen.text().contains("kept.")
        })?;
        h.save_screen("session-deletion-scrolled-screen.txt")?;
        h.key(b"\x1b[H")?;
        h.wait("review returns to top", 15.0, |h| {
            review(h)
                && !h
                    .screen
                    .text()
                    .contains("Project assets and provider history")
        })?;
    }
    h.key(b"n")?;
    h.wait("deletion cancelled", 15.0, |h| {
        h.screen.text().contains("Saved conversations")
            && !h.screen.text().contains("Delete local conversation?")
    })?;
    assert!(archived_path.is_file());
    h.key(b"d")?;
    h.visible("Delete local conversation?")?;
    let mut changed = read_json(&archived_path).unwrap();
    changed["draft"] = format!("{} 外部で変更", changed["draft"].as_str().unwrap()).into();
    write_json(&archived_path, &changed)?;
    h.key(b"y")?;
    h.visible("Conversation changed")?;
    assert!(archived_path.is_file());
    h.save_screen("session-deletion-stale-screen.txt")?;
    h.key(b"n")?;
    h.wait("return from stale review", 15.0, |h| {
        h.screen.text().contains("Saved conversations")
            && !h.screen.text().contains("Delete local conversation?")
    })?;
    h.key(b"d")?;
    h.wait("fresh review", 15.0, |h| {
        review(h) && !h.screen.text().contains("Conversation changed")
    })?;
    h.key(b"y")?;
    h.wait("reviewed deletion committed", 15.0, |h| {
        !archived_path.parent().unwrap().exists()
            && h.screen.text().contains("Local deletion committed")
    })?;
    let current = read_json(&current_path).unwrap();
    assert_eq!(current["id"], current_id);
    assert_eq!(current["draft"], current_draft);
    assert!(current["native_session_id"].is_null() || current["native_session_id"] == "");
    assert!(!h.root.join("sent-turn.txt").exists());
    assert_eq!(fs::read(&h.document)?, document);
    // Include directories: deletion must not leave an unfinished trash transaction.
    fn no_trash(root: &Path) -> bool {
        fs::read_dir(root)
            .map(|entries| {
                entries.flatten().all(|e| {
                    !e.file_name()
                        .to_string_lossy()
                        .starts_with("conversation-trash-")
                        && (!e.path().is_dir() || no_trash(&e.path()))
                })
            })
            .unwrap_or(true)
    }
    assert!(no_trash(&h.root.join("data")));
    h.save_screen("session-deletion-complete-screen.txt")?;
    h.key(b"\x1b")?;
    h.hidden("Saved conversations")?;
    Ok(
        json!({"source_commit":source_commit(),"case":h.case,"deleted_session":archived_id,"current_session":current_id,"selected_on_exit":current_id,"current_rejected":true,"review_cancelled":true,"stale_rejected":true,"current_draft_sha256":hash(current_draft),"document_sha256":hash(document),"agent_started":false,"cleanup_complete":true,"minimum_terminal_review":h.case == "session-deletion-small"}),
    )
}

pub fn history(h: &mut Harness) -> Result<Value> {
    let draft = "未送信の下書き\nq n p v は本文";
    h.paste(draft)?;
    h.draft(draft)?;
    let metadata_path = h.sessions()[0].clone();
    let saved_metadata = fs::read(&metadata_path)?;
    let design = fs::read(&h.document)?;
    let session = state(h);
    let turn_id = "00000000-0000-0000-0000-000000000001";
    let history_path = metadata_path
        .parent()
        .unwrap()
        .join(format!("turn-{turn_id}.jsonl"));
    let body = format!(
        "PAGE BEGIN 日本語\n{}PAGE END 日本語\n",
        (0..2100)
            .map(|i| format!("row {i:04} · 保存された応答\n"))
            .collect::<String>()
    );
    let expected = format!("\nTurn {turn_id}\n\nYou:\n保存済みの依頼\n{body}\nRequest complete\n");
    assert!(expected.len() > 2 * 32768 && expected.len() < 3 * 32760);
    let mut remaining = expected.as_str();
    let mut pages = Vec::new();
    while !remaining.is_empty() {
        let mut count = remaining.len().min(32768);
        while !remaining.is_char_boundary(count) {
            count -= 1;
        }
        pages.push(
            remaining[..count]
                .lines()
                .filter(|line| {
                    let bytes = line.as_bytes();
                    bytes.starts_with(b"row ")
                        && bytes.len() >= 8
                        && bytes[4..8].iter().all(u8::is_ascii_digit)
                        && line.get(8..) == Some(" · 保存された応答")
                })
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        remaining = &remaining[count..];
    }
    assert_eq!(pages.len(), 3);
    assert!(pages.iter().all(|p| !p.is_empty()));
    let formatted = |h: &Harness, marker: &str| {
        let text = h.screen.text();
        text.contains("Saved log · formatted")
            && !text.contains("Reading saved logs")
            && text.contains(marker)
    };
    let records = [
        json!({"type":"start","session":session["id"],"turn":turn_id,"document":session["document"]}),
        json!({"type":"user_prompt","text":"保存済みの依頼"}),
        json!({"type":"update","notification":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":body}}}}),
        json!({"type":"complete","session":session["id"],"turn":turn_id}),
    ];
    let mut log = String::new();
    for mut record in records {
        record.sort_all_objects();
        log.push_str(&serde_json::to_string(&record)?);
        log.push('\n');
    }
    fs::write(&history_path, log)?;
    let original_hash = hash(fs::read(&history_path)?);
    let mut offsets = serde_json::Map::new();
    command(h, "Current conversation logs")?;
    h.wait("saved turn list", 15.0, |h| {
        h.screen.text().contains("Saved turns") && h.screen.text().contains(turn_id)
    })?;
    h.key(b"\r")?;
    h.wait("first saved page", 15.0, |h| {
        h.screen.text().contains("Saved log") && h.screen.text().contains("PAGE BEGIN 日本語")
    })?;
    snapshot(h, "formatted", &mut offsets)?;
    h.key(b"\x1b[F")?;
    h.wait("saved page end", 15.0, |h| {
        formatted(h, pages[0].last().unwrap())
    })?;
    h.key(b"\x1b[H")?;
    h.visible("PAGE BEGIN 日本語")?;
    h.key(b"v")?;
    h.wait("raw saved source", 15.0, |h| {
        h.screen.text().contains("raw JSONL") && h.screen.text().contains("\"document\"")
    })?;
    snapshot(h, "raw", &mut offsets)?;
    h.key(b"v")?;
    h.wait("same formatted page", 15.0, |h| {
        formatted(h, "PAGE BEGIN 日本語")
    })?;
    for page in &pages[1..] {
        h.key(b"n")?;
        h.wait("unique content in requested saved page", 15.0, |h| {
            formatted(h, &page[0])
        })?;
    }
    h.key(b"\x1b[F")?;
    h.wait("complete response without completion marker", 15.0, |h| {
        let text = h.screen.text();
        text.contains("PAGE END 日本語") && !text.contains("Request complete")
    })?;
    snapshot(h, "end", &mut offsets)?;
    h.key(b"p")?;
    h.wait("previous page", 15.0, |h| formatted(h, &pages[1][0]))?;
    let metadata = fs::metadata(&history_path)?;
    let changed = fs::read_to_string(&history_path)?.replacen("PAGE BEGIN", "PAGE BEGAN", 1);
    fs::write(&history_path, changed)?;
    fs::File::options()
        .write(true)
        .open(&history_path)?
        .set_times(
            fs::FileTimes::new()
                .set_accessed(metadata.accessed()?)
                .set_modified(metadata.modified()?),
        )?;
    h.key(b"n")?;
    h.visible("REVISION_CONFLICT")?;
    h.key(b"r")?;
    h.visible("PAGE BEGAN 日本語")?;
    snapshot(h, "refreshed", &mut offsets)?;
    h.key(b"\x7f")?;
    h.visible("Saved turns")?;
    h.key(b"s")?;
    h.visible("Saved conversations")?;
    h.key(b"c")?;
    h.visible("Saved turns")?;
    h.key(b"q")?;
    h.wait("saved logs closed", 15.0, |h| {
        !h.screen.text().contains("Saved turns") && !h.screen.text().contains("Saved conversations")
    })?;
    assert_eq!(state(h)["draft"], draft);
    no_agent(h);
    assert_eq!(fs::read(&metadata_path)?, saved_metadata);
    assert_eq!(fs::read(&h.document)?, design);
    assert_eq!(
        files(metadata_path.parent().unwrap(), |s| s.starts_with("turn-")
            && s.ends_with(".jsonl")),
        std::slice::from_ref(&history_path)
    );
    fs::copy(&history_path, h.root.join("saved-history-source.jsonl"))?;
    fs::write(h.root.join("saved-history-expected.txt"), &expected)?;
    fs::write(h.root.join("saved-history-session.json"), saved_metadata)?;
    fs::write(h.root.join("saved-history-design.pen"), design)?;
    Ok(
        json!({"source_commit":source_commit(),"case":h.case,"session":session["id"],"turn":turn_id,"screen_offsets":offsets,"original_sha256":original_hash,"refreshed_sha256":hash(fs::read(history_path)?),"expected_formatted_sha256":hash(expected),"draft_sha256":hash(draft),"display_pages":3,"agent_started":false,"turn_files":1}),
    )
}
