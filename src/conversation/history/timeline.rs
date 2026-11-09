//! Chronological, backwards pages for the live TUI's saved conversation display.

use super::*;

const PAGE_BYTES: usize = 32 * 1024;
type Boundary = (u64, Option<u64>);

struct Turn {
    id: String,
    revision: Option<String>,
}

pub(crate) struct Reader {
    store: Store,
    session: String,
    turns: Vec<Turn>,
    before: Option<Boundary>,
}

pub(crate) struct Page {
    pub text: String,
    pub reader: Option<Reader>,
}

impl Reader {
    pub fn start(
        store: Store,
        session: String,
        cutoff: std::time::SystemTime,
        cancel: &CancellationToken,
    ) -> Result<Page> {
        let budget = Budget {
            started: Instant::now(),
            cancel: Some(cancel),
        };
        let (directory, _) = super::session(&store, &session)?;
        let mut turns = Vec::new();
        let mut unavailable = 0;
        let mut legacy = false;
        for entry in std::fs::read_dir(&directory).map_err(|error| Error::io(&directory, error))? {
            budget.check()?;
            let entry = entry.map_err(|error| Error::io(&directory, error))?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_prefix("turn-"))
                .and_then(|name| name.strip_suffix(".jsonl"))
            else {
                continue;
            };
            if uuid(id).is_err() {
                continue;
            }
            let result = (|| {
                let mut file = transcript(&store, &session, id)?;
                let header = header_value(&mut file, &store, &session, id)?;
                let metadata = file
                    .metadata()
                    .map_err(|error| Error::io(&directory, error))?;
                let created_at = metadata.created().ok();
                if created_at.is_some_and(|time| time > cutoff) {
                    return Ok(None);
                }
                let created = created_at
                    .or_else(|| metadata.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos());
                let time = header["recorded_at_unix_ms"]
                    .as_u64()
                    .map(|ms| u128::from(ms) * 1_000_000)
                    .unwrap_or_else(|| {
                        legacy = true;
                        created
                    });
                if time
                    > cutoff
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                {
                    return Ok(None);
                }
                Ok::<_, Error>(Some((time, created, id.to_owned())))
            })();
            match result {
                Ok(Some(turn)) => turns.push(turn),
                Ok(None) => {}
                Err(_) => unavailable += 1,
            }
        }
        turns.sort();
        let reader = Self {
            store,
            session,
            turns: turns
                .into_iter()
                .map(|(_, _, id)| Turn { id, revision: None })
                .collect(),
            before: None,
        };
        let mut page = reader.previous(cancel)?;
        if unavailable > 0 {
            page.text.push_str(&format!(
                "\n[{unavailable} saved turns unavailable; inspect /logs]\n"
            ));
        }
        if legacy {
            page.text
                .push_str("\n[Legacy turns without timestamps use local file timestamp order]\n");
        }
        Ok(page)
    }

    pub fn previous(mut self, cancel: &CancellationToken) -> Result<Page> {
        let budget = Budget {
            started: Instant::now(),
            cancel: Some(cancel),
        };
        let mut chunks = std::collections::VecDeque::new();
        let mut remaining = PAGE_BYTES;
        while remaining >= 4 {
            budget.check()?;
            let Some(turn) = self.turns.last_mut() else {
                break;
            };
            let mut file = transcript(&self.store, &self.session, &turn.id)?;
            let current = revision(&mut file, &budget)?;
            if turn
                .revision
                .as_ref()
                .is_some_and(|saved| saved != &current)
            {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Saved conversation changed; reselect it to refresh history",
                ));
            }
            header(&mut file, &self.store, &self.session, &turn.id)?;
            let (text, before) = display::previous(&mut file, self.before, remaining, &budget)?;
            if revision(&mut file, &budget)? != current {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Saved conversation changed while reading history",
                ));
            }
            turn.revision = Some(current);
            remaining -= text.len();
            chunks.push_front(text);
            self.before = before;
            if before.is_none() {
                self.turns.pop();
            }
        }
        Ok(Page {
            text: chunks.into_iter().collect(),
            reader: (!self.turns.is_empty()).then_some(self),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::{Arguments, Command},
        conversation::Conversation,
    };
    use clap::Parser;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Conversation) {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("design.pen");
        let data = temp.path().join("data");
        Store::init(&file, Some(data.clone())).unwrap();
        let arguments = Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            file.into_os_string(),
        ])
        .unwrap();
        let Command::Prompt(args) = arguments.command else {
            unreachable!()
        };
        let conversation = Conversation::open(args, Some(data)).unwrap();
        (temp, conversation)
    }

    fn turn(conversation: &Conversation, id: &str, time: Option<u64>, text: &str) -> PathBuf {
        let path = conversation.directory().join(format!("turn-{id}.jsonl"));
        let records = [
            json!({"type":"start", "session":conversation.id(), "document":conversation.store().path(), "turn":id, "recorded_at_unix_ms":time}),
            json!({"type":"user_prompt", "text":text}),
            json!({"type":"permission_request", "id":"old-permission-must-not-be-replayed"}),
            json!({"type":"update", "notification":{"update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"回答です🙂"}}}}),
            json!({"type":"complete"}),
        ];
        let bytes = records
            .into_iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn start(conversation: &Conversation) -> Page {
        Reader::start(
            conversation.store().clone(),
            conversation.id().into(),
            std::time::SystemTime::now(),
            &CancellationToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn timeline_uses_recorded_chronology_instead_of_uuid_and_never_replays_events() {
        let (_temp, conversation) = fixture();
        let older = "ffffffff-ffff-4fff-8fff-ffffffffffff";
        let newer = "00000000-0000-4000-8000-000000000000";
        turn(&conversation, newer, Some(20), "Newer request");
        turn(&conversation, older, Some(10), "Older request");
        let page = start(&conversation);
        assert!(
            page.text.find("Older request").unwrap() < page.text.find("Newer request").unwrap()
        );
        assert_eq!(page.text.matches("回答です🙂").count(), 2);
        assert!(!page.text.contains("old-permission"));
        assert!(page.reader.is_none());
        assert!(!conversation.active());
    }

    #[test]
    fn timeline_pages_reconstruct_large_unicode_records_without_gaps_or_duplicates() {
        let (_temp, conversation) = fixture();
        let id = uuid::Uuid::new_v4().to_string();
        let request = "長い下書き🙂\n".repeat(10_000);
        turn(&conversation, &id, Some(10), &request);
        let mut page = start(&conversation);
        assert!(page.reader.is_some());
        assert!(page.text.len() <= PAGE_BYTES);
        assert!(page.text.ends_with("Request complete\n"));
        let mut combined = page.text;
        while let Some(reader) = page.reader {
            page = reader.previous(&CancellationToken::new()).unwrap();
            assert!(page.text.len() <= PAGE_BYTES);
            combined.insert_str(0, &page.text);
        }
        assert_eq!(
            combined,
            format!("\nTurn {id}\n\nYou:\n{request}\n\nAgent:\n回答です🙂\nRequest complete\n")
        );
    }

    #[test]
    fn timeline_rejects_changed_transcripts_between_pages_and_supports_cancellation() {
        let (_temp, conversation) = fixture();
        let path = turn(
            &conversation,
            &uuid::Uuid::new_v4().to_string(),
            Some(10),
            &"x".repeat(PAGE_BYTES * 3),
        );
        let page = start(&conversation);
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();
        assert_eq!(
            page.reader
                .unwrap()
                .previous(&CancellationToken::new())
                .err()
                .unwrap()
                .code,
            "REVISION_CONFLICT"
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            Reader::start(
                conversation.store().clone(),
                conversation.id().into(),
                std::time::SystemTime::now(),
                &cancel
            )
            .is_err()
        );
    }

    #[test]
    fn timeline_discloses_damaged_and_legacy_history_and_excludes_new_live_turns() {
        let (_temp, conversation) = fixture();
        turn(
            &conversation,
            &uuid::Uuid::new_v4().to_string(),
            None,
            "Legacy request",
        );
        let path = turn(
            &conversation,
            &uuid::Uuid::new_v4().to_string(),
            Some(10),
            "Damaged request",
        );
        std::fs::write(path, "broken").unwrap();
        let cutoff = std::time::SystemTime::now();
        turn(
            &conversation,
            &uuid::Uuid::new_v4().to_string(),
            Some(u64::MAX),
            "New live request",
        );
        let page = Reader::start(
            conversation.store().clone(),
            conversation.id().into(),
            cutoff,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(page.text.contains("Legacy request"));
        assert!(page.text.contains("1 saved turns unavailable"));
        assert!(page.text.contains("Legacy turns without timestamps"));
        assert!(!page.text.contains("New live request"));
    }
}
