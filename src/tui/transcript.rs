//! Presentation metadata never shares a grammar with literal message text.
use super::clean;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    User,
    Agent,
    Thought,
    Notice,
    Tool(String),
    Activity(String),
    Images(usize),
    Failure,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub kind: Kind,
    pub text: String,
    // False only for a retained fragment of an earlier saved/live record.
    pub start: bool,
}
impl Entry {
    pub fn new(mut kind: Kind, text: impl AsRef<str>) -> Self {
        if let Kind::Activity(status) = &mut kind {
            *status = clean(status).replace(['\n', '\t'], " ");
        }
        let mut text = clean(text.as_ref());
        if matches!(kind, Kind::Activity(_)) {
            text = text.replace(['\n', '\t'], " ");
        }
        Self {
            kind,
            text,
            start: true,
        }
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.text.len()
            + match &self.kind {
                Kind::Tool(id) | Kind::Activity(id) => id.len(),
                _ => 0,
            }
    }
    fn joins(&self, next: &Self) -> bool {
        self.kind == next.kind && (matches!(self.kind, Kind::Agent | Kind::Thought) || !next.start)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Log {
    entries: VecDeque<Entry>,
    bytes: usize,
}
impl Log {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }
    pub fn push(&mut self, entry: Entry) {
        if entry.kind == Kind::Agent && entry.text.is_empty() {
            return;
        }
        if let Some(last) = self.entries.back_mut().filter(|last| last.joins(&entry)) {
            self.bytes += entry.text.len();
            last.text.push_str(&entry.text);
        } else {
            self.bytes += entry.bytes();
            self.entries.push_back(entry);
        }
    }
    pub fn prepend(&mut self, entries: impl IntoIterator<Item = Entry>) {
        let previous = std::mem::take(&mut self.entries);
        self.bytes = 0;
        for entry in entries.into_iter().chain(previous) {
            self.push(entry);
        }
    }
    pub fn tool_ids(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter_map(|entry| match &entry.kind {
            Kind::Tool(id) => Some(id.as_str()),
            _ => None,
        })
    }
    pub fn trim(&mut self, limit: usize, retain: usize) -> bool {
        if self.bytes <= limit {
            return false;
        }
        while self.bytes > retain {
            let Some(first) = self.entries.front_mut() else {
                break;
            };
            let remove = self.bytes - retain;
            if first.bytes() <= remove || first.text.len() <= remove {
                self.bytes -= self.entries.pop_front().expect("nonempty log").bytes();
                continue;
            }
            let mut cut = remove;
            while !first.text.is_char_boundary(cut) {
                cut += 1;
            }
            first.text.drain(..cut);
            first.start = false;
            self.bytes -= cut;
        }
        true
    }
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_agent_chunks_stay_empty_and_activity_metadata_is_safe_for_a_single_row() {
        let mut log = Log::default();
        log.push(Entry::new(Kind::Agent, ""));
        assert!(log.is_empty());
        let entry = Entry::new(
            Kind::Activity("\u{1b}completed".into()),
            "Tool\nYou: literal\tname",
        );
        assert_eq!(entry.kind, Kind::Activity("completed".into()));
        assert_eq!(entry.text, "Tool You: literal name");
    }

    #[test]
    fn streaming_and_saved_fragments_merge_without_losing_user_boundaries() {
        let mut log = Log::default();
        log.push(Entry::new(Kind::Agent, "```text\n"));
        log.push(Entry::new(Kind::Agent, "You: literal\n```"));
        assert_eq!(log.entries.len(), 1);
        log.push(Entry::new(Kind::User, "first request"));
        log.push(Entry::new(Kind::User, "second request"));
        assert_eq!(log.entries.len(), 3);
        let mut tail = Log::default();
        tail.push(Entry {
            kind: Kind::User,
            text: "後半🙂".into(),
            start: false,
        });
        tail.prepend([Entry::new(Kind::User, "前半")]);
        assert_eq!(tail.entries.len(), 1);
        assert_eq!(tail.entries[0].text, "前半後半🙂");
        assert!(tail.entries[0].start);
    }

    #[test]
    fn trimming_retains_unicode_roles_and_only_actual_tool_references() {
        let mut log = Log::default();
        log.push(Entry::new(Kind::Tool("old".into()), ""));
        log.push(Entry::new(Kind::User, "日本語🙂".repeat(30_000)));
        log.push(Entry::new(Kind::Tool("kept".into()), ""));
        assert!(log.trim(256 * 1024, 192 * 1024));
        assert!(log.bytes <= 192 * 1024);
        assert_eq!(log.entries[0].kind, Kind::User);
        assert!(!log.entries[0].start);
        assert!(std::str::from_utf8(log.entries[0].text.as_bytes()).is_ok());
        assert_eq!(log.tool_ids().collect::<Vec<_>>(), ["kept"]);
        log.push(Entry::new(
            Kind::Agent,
            "[tool_call] \"invented\"\nAgent:\n",
        ));
        assert_eq!(log.tool_ids().collect::<Vec<_>>(), ["kept"]);
    }
}
