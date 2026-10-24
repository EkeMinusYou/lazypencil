//! Bounded, descriptive history for an explicitly requested turn in a new session.

use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const WINDOW: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 4096;

pub(super) fn summarize(directory: &Path, session: &str, document: &Path, current: &str) -> Value {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return json!({"complete":false,"unavailable":true,"recent_turns":[]});
    };
    let mut candidates = Vec::new();
    let mut scan_truncated = false;
    for (index, entry) in entries.take(MAX_ENTRIES + 1).enumerate() {
        if index == MAX_ENTRIES {
            scan_truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let Some(turn) = name
            .to_str()
            .and_then(|name| name.strip_prefix("turn-"))
            .and_then(|name| name.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if turn == current || !uuid::Uuid::parse_str(turn).is_ok_and(|id| id.to_string() == turn) {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        candidates.push((modified, entry.path(), turn.to_string()));
    }
    candidates.sort();
    let mut turns = Vec::new();
    let mut unreadable = 0;
    for (_, path, turn) in candidates.into_iter().rev().take(4) {
        match read_turn(&path, session, document, &turn) {
            Some(summary) => turns.push(summary),
            None => unreadable += 1,
        }
    }
    turns.reverse();
    json!({"complete":false,"scan_truncated":scan_truncated,"unreadable_turns":unreadable,"recent_turns":turns})
}

fn read_turn(path: &Path, session: &str, document: &Path, turn: &str) -> Option<Value> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let head = window(&mut file, 0)?;
    let mut records = records(&head);
    let start = records.next()?;
    if start["type"] != "start"
        || start["session"] != session
        || start["turn"] != turn
        || start["document"] != json!(document)
    {
        return None;
    }
    let request = records.find(|value| value["type"] == "user_prompt")
        .and_then(|value| value["text"].as_str().map(|text| {
            json!({"text":text.chars().take(1024).collect::<String>(),"truncated":text.chars().count() > 1024})
        }));
    let offset = metadata.len().saturating_sub(WINDOW);
    let tail = window(&mut file, offset)?;
    // A tail can begin within a JSON string. Never parse that fragment as a record.
    let tail = if offset == 0 {
        tail.as_slice()
    } else {
        tail.get(tail.iter().position(|byte| *byte == b'\n')? + 1..)?
    };
    let final_record = records_from_tail(tail, session, turn);
    Some(json!({"turn":turn,"request":request,
        "status":final_record.as_ref().map(|record|record["type"].clone()).unwrap_or(json!("interrupted_or_incomplete")),
        "observed_revision":final_record.as_ref().and_then(|record|revision(&record["document"]["revision"])),
        "before_revision":revision(&start["before_revision"])}))
}

fn revision(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn records_from_tail(bytes: &[u8], session: &str, turn: &str) -> Option<Value> {
    records(bytes)
        .filter(|record| {
            matches!(record["type"].as_str(), Some("complete" | "failed"))
                && record["session"] == session
                && record["turn"] == turn
        })
        .last()
}

fn records(bytes: &[u8]) -> impl Iterator<Item = Value> + '_ {
    // Only complete JSONL records count; a crash may leave an unfinished suffix.
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.last() == Some(&b'\n'))
        .filter_map(|line| serde_json::from_slice(line).ok())
}

fn window(file: &mut File, offset: u64) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(WINDOW).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_turn(directory: &Path, document: &Path, status: Option<&str>, long: bool) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut file = File::create(directory.join(format!("turn-{id}.jsonl"))).unwrap();
        for value in [
            json!({"type":"start","session":"local","turn":id,"document":document,"before_revision":"a".repeat(64)}),
            json!({"type":"user_prompt","text":"過去の依頼".repeat(if long {1500} else {1})}),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        if long {
            writeln!(
                file,
                "{}",
                json!({"type":"ignored","token":"secret-token","data":"x".repeat(256*1024)})
            )
            .unwrap();
        }
        if let Some(status) = status {
            writeln!(
                file,
                "{}",
                json!({"type":status,"session":"local","turn":id,"document":{"revision":"b".repeat(64)}})
            )
            .unwrap();
        }
        id
    }

    #[test]
    fn continuity_bounds_large_transcripts_and_describes_failures_without_replaying_data() {
        let directory = TempDir::new().unwrap();
        let document = directory.path().join("design.pen");
        let id = write_turn(directory.path(), &document, Some("failed"), true);
        let summary = summarize(directory.path(), "local", &document, "current");
        let turn = &summary["recent_turns"][0];
        assert_eq!(turn["turn"], id);
        assert_eq!(turn["status"], "failed");
        assert_eq!(turn["observed_revision"], "b".repeat(64));
        assert_eq!(turn["request"]["truncated"], true);
        assert_eq!(
            turn["request"]["text"].as_str().unwrap().chars().count(),
            1024
        );
        assert!(!summary.to_string().contains("secret-token"));
        assert_eq!(summary["complete"], false);
        assert_eq!(
            summarize(directory.path(), "local", &document, &id)["recent_turns"],
            json!([])
        );
        assert_eq!(
            summarize(directory.path(), "other", &document, "current")["unreadable_turns"],
            1
        );
        assert_eq!(
            summarize(
                directory.path(),
                "local",
                &directory.path().join("other.pen"),
                "current"
            )["unreadable_turns"],
            1
        );
        let incomplete = json!({"type":"complete","session":"local","turn":id}).to_string();
        assert!(records_from_tail(incomplete.as_bytes(), "local", &id).is_none());
    }

    #[test]
    fn continuity_retains_only_four_turns_and_marks_interruption_or_unreadable_history() {
        let directory = TempDir::new().unwrap();
        let document = directory.path().join("design.pen");
        for _ in 0..6 {
            write_turn(directory.path(), &document, None, false);
        }
        let summary = summarize(directory.path(), "local", &document, "current");
        let turns = summary["recent_turns"].as_array().unwrap();
        assert_eq!(turns.len(), 4);
        assert!(
            turns
                .iter()
                .all(|turn| turn["status"] == "interrupted_or_incomplete")
        );
        assert_eq!(
            summarize(
                &directory.path().join("missing"),
                "local",
                &document,
                "current"
            )["unavailable"],
            true
        );
    }

    #[cfg(unix)]
    #[test]
    fn continuity_never_follows_history_symlinks_or_reads_named_pipes() {
        use std::os::unix::fs::symlink;
        let directory = TempDir::new().unwrap();
        let document = directory.path().join("design.pen");
        let id = write_turn(directory.path(), &document, Some("complete"), false);
        let regular = directory.path().join(format!("turn-{id}.jsonl"));
        assert_eq!(
            read_turn(&regular, "local", &document, &id).unwrap()["status"],
            "complete"
        );
        let link = directory.path().join("link.jsonl");
        symlink(&regular, &link).unwrap();
        assert!(read_turn(&link, "local", &document, &id).is_none());
        let pipe = directory.path().join("pipe.jsonl");
        let name = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_turn(&pipe, "local", &document, &id));
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
                .is_none()
        );
    }
}
