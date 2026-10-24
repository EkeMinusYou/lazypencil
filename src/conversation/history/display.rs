//! Streaming text pages: memory does not grow with a JSON string or record.

use super::{Budget, MAX_PAGE_BYTES, header, revision, transcript};
use crate::{Error, Result, document::hash, store::Store};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

fn invalid() -> Error {
    Error::new("INVALID_DOCUMENT", "Transcript record is not complete JSON")
}
fn io(error: std::io::Error) -> Error {
    Error::new("IO_ERROR", error.to_string())
}

trait Strings {
    fn begin(&mut self, path: &[Option<String>]);
    fn character(&mut self, value: char) -> Result<()>;
}

// Only small keys and metadata are collected. Even a 64 MiB text value is
// decoded a character at a time into the bounded output window.
struct Json<'a, 'b, R> {
    reader: BufReader<R>,
    look: Option<Option<u8>>,
    consumed: usize,
    budget: &'a Budget<'b>,
}
impl<'a, 'b, R: Read> Json<'a, 'b, R> {
    fn peek(&mut self) -> Result<Option<u8>> {
        if let Some(value) = self.look {
            return Ok(value);
        }
        let mut byte = [0];
        let value = loop {
            match self.reader.read(&mut byte) {
                Ok(0) => break None,
                Ok(_) => break Some(byte[0]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(io(error)),
            }
        };
        self.look = Some(value);
        Ok(value)
    }
    fn next(&mut self) -> Result<u8> {
        let value = self.peek()?.ok_or_else(invalid)?;
        self.look = None;
        self.consumed += 1;
        if self.consumed.is_multiple_of(4096) {
            self.budget.check()?;
        }
        Ok(value)
    }
    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.next()? == byte {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    fn whitespace(&mut self) -> Result<()> {
        while matches!(self.peek()?, Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.next()?;
        }
        Ok(())
    }
    fn hex(&mut self) -> Result<u32> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = char::from(self.next()?).to_digit(16).ok_or_else(invalid)?;
            value = value * 16 + digit;
        }
        Ok(value)
    }
    fn string(&mut self, mut emit: impl FnMut(char) -> Result<()>) -> Result<()> {
        self.expect(b'"')?;
        loop {
            let byte = self.next()?;
            let value = match byte {
                b'"' => return Ok(()),
                b'\\' => match self.next()? {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let first = self.hex()?;
                        let value = if (0xd800..=0xdbff).contains(&first) {
                            self.expect(b'\\')?;
                            self.expect(b'u')?;
                            let second = self.hex()?;
                            if !(0xdc00..=0xdfff).contains(&second) {
                                return Err(invalid());
                            }
                            0x10000 + ((first - 0xd800) << 10) + second - 0xdc00
                        } else {
                            first
                        };
                        char::from_u32(value).ok_or_else(invalid)?
                    }
                    _ => return Err(invalid()),
                },
                0..=0x1f => return Err(invalid()),
                0x20..=0x7f => char::from(byte),
                _ => {
                    let length = match byte {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err(invalid()),
                    };
                    let mut bytes = [byte, 0, 0, 0];
                    for item in &mut bytes[1..length] {
                        *item = self.next()?;
                    }
                    std::str::from_utf8(&bytes[..length])
                        .map_err(|_| invalid())?
                        .chars()
                        .next()
                        .ok_or_else(invalid)?
                }
            };
            emit(value)?;
        }
    }
    fn value(&mut self, path: &mut Vec<Option<String>>, visitor: &mut impl Strings) -> Result<()> {
        if path.len() > 128 {
            return Err(invalid());
        }
        self.whitespace()?;
        match self.peek()? {
            Some(b'"') => {
                visitor.begin(path);
                self.string(|c| visitor.character(c))?;
            }
            Some(b'{') => {
                self.next()?;
                self.whitespace()?;
                let mut keys = 0u32;
                if self.peek()? == Some(b'}') {
                    self.next()?;
                    return Ok(());
                }
                loop {
                    self.whitespace()?;
                    let mut key = Some(String::new());
                    self.string(|c| {
                        if key.as_ref().is_some_and(|s| s.len() + c.len_utf8() > 128) {
                            key = None;
                        }
                        if let Some(key) = &mut key {
                            key.push(c);
                        }
                        Ok(())
                    })?;
                    // Duplicate relevant keys have ambiguous display semantics.
                    // Keep the complete record available as raw JSON instead.
                    const KEYS: &[&str] = &[
                        "type",
                        "notification",
                        "update",
                        "sessionUpdate",
                        "content",
                        "text",
                        "error",
                        "message",
                        "title",
                        "status",
                        "authentication",
                        "source",
                        "turn",
                    ];
                    if let Some(index) = key
                        .as_deref()
                        .and_then(|key| KEYS.iter().position(|candidate| *candidate == key))
                    {
                        let bit = 1 << index;
                        if keys & bit != 0 {
                            return Err(invalid());
                        }
                        keys |= bit;
                    }
                    self.whitespace()?;
                    self.expect(b':')?;
                    path.push(key);
                    self.value(path, visitor)?;
                    path.pop();
                    self.whitespace()?;
                    match self.next()? {
                        b'}' => break,
                        b',' => {}
                        _ => return Err(invalid()),
                    }
                }
            }
            Some(b'[') => {
                self.next()?;
                self.whitespace()?;
                if self.peek()? == Some(b']') {
                    self.next()?;
                    return Ok(());
                }
                loop {
                    path.push(None);
                    self.value(path, visitor)?;
                    path.pop();
                    self.whitespace()?;
                    match self.next()? {
                        b']' => break,
                        b',' => {}
                        _ => return Err(invalid()),
                    }
                }
            }
            Some(b't' | b'f' | b'n') => {
                let word: &[u8] = match self.peek()? {
                    Some(b't') => b"true",
                    Some(b'f') => b"false",
                    _ => b"null",
                };
                for byte in word {
                    self.expect(*byte)?;
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                let mut number = Vec::new();
                while matches!(
                    self.peek()?,
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    if number.len() == 128 {
                        return Err(invalid());
                    }
                    number.push(self.next()?);
                }
                serde_json::from_slice::<serde_json::Number>(&number).map_err(|_| invalid())?;
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
    fn run(mut self, visitor: &mut impl Strings) -> Result<()> {
        self.budget.check()?;
        self.value(&mut Vec::new(), visitor)?;
        self.whitespace()?;
        if self.peek()?.is_some() {
            return Err(invalid());
        }
        self.budget.check()
    }
}

fn field(path: &[Option<String>]) -> Option<&'static str> {
    const FIELDS: &[(&str, &[&str])] = &[
        ("kind", &["type"]),
        ("update", &["notification", "update", "sessionUpdate"]),
        ("prompt", &["text"]),
        ("agent", &["notification", "update", "content", "text"]),
        ("error", &["error", "message"]),
        ("title", &["notification", "update", "title"]),
        ("status", &["notification", "update", "status"]),
        ("source", &["authentication", "source"]),
        ("turn", &["turn"]),
    ];
    FIELDS.iter().find_map(|(name, wanted)| {
        (path.len() == wanted.len()
            && path
                .iter()
                .zip(*wanted)
                .all(|(actual, wanted)| actual.as_deref() == Some(*wanted)))
        .then_some(*name)
    })
}
#[derive(Default)]
struct Metadata {
    fields: BTreeMap<&'static str, String>,
    present: BTreeSet<&'static str>,
    current: Option<&'static str>,
}
impl Strings for Metadata {
    fn begin(&mut self, path: &[Option<String>]) {
        self.current = field(path);
        if let Some(name) = self.current {
            self.present.insert(name);
            if matches!(name, "kind" | "update" | "status" | "source" | "turn") {
                self.fields.insert(name, String::new());
            }
        }
    }
    fn character(&mut self, value: char) -> Result<()> {
        if let Some(name) = self.current
            && let Some(text) = self.fields.get_mut(name)
        {
            if text.len() + value.len_utf8() <= 128 {
                text.push(value);
            } else {
                self.fields.remove(name);
            }
        }
        Ok(())
    }
}

struct Window {
    skip: u64,
    limit: usize,
    total: u64,
    full: bool,
    text: String,
}
impl Window {
    fn new(skip: u64, limit: usize) -> Self {
        Self {
            skip,
            limit,
            total: 0,
            full: false,
            text: String::new(),
        }
    }
    fn character(&mut self, value: char) -> Result<()> {
        if value.is_control() && !matches!(value, '\n' | '\t') {
            return Ok(());
        }
        let start = self.total;
        self.total += value.len_utf8() as u64;
        if start < self.skip && self.total > self.skip {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Display cursor splits a UTF-8 character",
            ));
        }
        if start >= self.skip && !self.full {
            if self.text.len() + value.len_utf8() <= self.limit {
                self.text.push(value);
            } else {
                self.full = true;
            }
        }
        Ok(())
    }
    fn write(&mut self, text: &str) -> Result<()> {
        for c in text.chars() {
            self.character(c)?;
        }
        Ok(())
    }
}
struct Text<'a> {
    field: &'static str,
    active: bool,
    window: &'a mut Window,
}
impl Strings for Text<'_> {
    fn begin(&mut self, path: &[Option<String>]) {
        self.active = field(path) == Some(self.field);
    }
    fn character(&mut self, value: char) -> Result<()> {
        if self.active {
            self.window.character(value)?;
        }
        Ok(())
    }
}

fn scan(
    file: &mut File,
    start: u64,
    bytes: u64,
    visitor: &mut impl Strings,
    budget: &Budget<'_>,
) -> Result<()> {
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    Json {
        reader: BufReader::new(file.take(bytes)),
        look: None,
        consumed: 0,
        budget,
    }
    .run(visitor)
}

fn raw(
    file: &mut File,
    start: u64,
    bytes: u64,
    output: &mut Window,
    budget: &Budget<'_>,
) -> Result<()> {
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut reader = file.take(bytes);
    let mut buffer = [0; 65540];
    let mut carry = 0;
    loop {
        budget.check()?;
        let n = reader.read(&mut buffer[carry..65536 + carry]).map_err(io)?;
        let length = n + carry;
        let mut position = 0;
        while position < length {
            match std::str::from_utf8(&buffer[position..length]) {
                Ok(text) => {
                    output.write(text)?;
                    position = length;
                }
                Err(error) => {
                    output.write(
                        std::str::from_utf8(&buffer[position..position + error.valid_up_to()])
                            .expect("valid prefix"),
                    )?;
                    position += error.valid_up_to();
                    if let Some(n) = error.error_len() {
                        output.character('\u{fffd}')?;
                        position += n;
                    } else if n == 0 {
                        output.character('\u{fffd}')?;
                        position = length;
                    } else {
                        break;
                    }
                }
            }
        }
        carry = length - position;
        buffer.copy_within(position..length, 0);
        if n == 0 {
            return Ok(());
        }
    }
}

fn formatted(
    file: &mut File,
    start: u64,
    bytes: u64,
    skip: u64,
    limit: usize,
    budget: &Budget<'_>,
) -> Result<(Window, bool)> {
    let mut metadata = Metadata::default();
    let valid = match scan(file, start, bytes, &mut metadata, budget) {
        Ok(()) => true,
        Err(error) if error.code == "INVALID_DOCUMENT" => false,
        Err(error) => return Err(error),
    };
    let get = |name| metadata.fields.get(name).map(String::as_str).unwrap_or("");
    let mut prefix = String::new();
    let mut suffix = "";
    let mut selected = None;
    let mut structured = valid;
    if valid {
        match (get("kind"), get("update")) {
            ("user_prompt", _) if metadata.present.contains("prompt") => {
                prefix = "\nYou:\n".into();
                suffix = "\n";
                selected = Some("prompt");
            }
            ("update", "agent_message_chunk") if metadata.present.contains("agent") => {
                selected = Some("agent");
            }
            ("update", "agent_thought_chunk") if metadata.present.contains("agent") => {
                prefix = "\nThinking:\n".into();
                selected = Some("agent");
            }
            ("update", "tool_call" | "tool_call_update") => {
                prefix = format!(
                    "\n[{}] ",
                    if get("status").is_empty() {
                        "running"
                    } else {
                        get("status")
                    }
                );
                if metadata.present.contains("title") {
                    selected = Some("title");
                } else {
                    prefix.push_str("Tool");
                }
                suffix = "\n";
            }
            ("failed", _) if metadata.present.contains("error") => {
                prefix = "\nRequest failed: ".into();
                selected = Some("error");
                suffix = "\n";
            }
            ("complete", _) => prefix = "\nRequest complete\n".into(),
            ("ready", _) => prefix = "\nAgent ready\n".into(),
            ("start", _) => prefix = format!("\nTurn {}\n", get("turn")),
            ("session_restarted", _) => {
                prefix = "\nNew native session (local context only).\n".into()
            }
            ("local_context", _) => prefix = "\nLocal history context supplied.\n".into(),
            ("auth_status", _) => prefix = format!("\nAuthentication: {}\n", get("source")),
            _ => structured = false,
        }
    }
    let mut output = Window::new(skip, limit);
    if structured {
        output.write(&prefix)?;
        if let Some(field) = selected {
            scan(
                file,
                start,
                bytes,
                &mut Text {
                    field,
                    active: false,
                    window: &mut output,
                },
                budget,
            )?;
        }
        output.write(suffix)?;
        // This bound also bounds the number of fixed-size pages per turn.
        if output.total > bytes.saturating_mul(2) {
            structured = false;
            output = Window::new(skip, limit);
        }
    }
    if !structured {
        raw(file, start, bytes, &mut output, budget)?;
    }
    if skip > output.total {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Display cursor exceeds its record",
        ));
    }
    Ok((output, !structured))
}

fn record_end(file: &mut File, start: u64, budget: &Budget<'_>) -> Result<u64> {
    file.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut reader = BufReader::new(file);
    let mut end = start;
    loop {
        budget.check()?;
        let buffer = reader.fill_buf().map_err(io)?;
        if buffer.is_empty() {
            return Ok(end);
        }
        let newline = buffer.iter().position(|b| *b == b'\n');
        let count = newline.map_or(buffer.len(), |n| n + 1);
        end += count as u64;
        if end > super::MAX_TRANSCRIPT {
            return Err(Error::new("AGENT_ERROR", "Saved transcript exceeds 64 MiB"));
        }
        reader.consume(count);
        if newline.is_some() {
            return Ok(end);
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    // Bump when formatting changes: `byte` addresses sanitized display bytes.
    version: u8,
    scope: String,
    revision: String,
    record: u64,
    byte: u64,
}
fn encode(value: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("cursor serialization"))
}

/// Formatted pages never deserialize complete text values or complete JSON records.
/// Unknown/malformed records remain visible as raw text; the raw API preserves
/// their exact bytes. Reading never starts or resumes an agent.
pub fn view(
    store: &Store,
    conversation: &str,
    turn: &str,
    cursor: Option<&str>,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    if !(4..=MAX_PAGE_BYTES).contains(&limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Display byte limit must be 4..65536",
        ));
    }
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    budget.check()?;
    let mut file = transcript(store, conversation, turn)?;
    let before = revision(&mut file, &budget)?;
    header(&mut file, store, conversation, turn)?;
    let scope = hash(
        &serde_json::to_vec(&(store.path(), conversation, turn)).expect("scope serialization"),
    );
    let mut position = if let Some(cursor) = cursor {
        if cursor.len() > 1024 {
            return Err(Error::new("INVALID_ARGUMENT", "Invalid display cursor"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid display cursor"))?;
        let value: Cursor = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid display cursor"))?;
        if value.version != 1 || value.scope != scope {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Display cursor belongs to another transcript",
            ));
        }
        if value.revision != before {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "Saved transcript changed; refresh its display",
            ));
        }
        value
    } else {
        Cursor {
            version: 1,
            scope,
            revision: before.clone(),
            record: 0,
            byte: 0,
        }
    };
    let current = encode(&position);
    let raw_offset = position.record;
    let length = file.metadata().map_err(io)?.len();
    if position.record > length {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Display cursor exceeds transcript length",
        ));
    }
    if position.record == length && position.byte != 0 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Display cursor exceeds its final record",
        ));
    }
    if position.record > 0 {
        file.seek(SeekFrom::Start(position.record - 1))
            .map_err(io)?;
        let mut byte = [0];
        file.read_exact(&mut byte).map_err(io)?;
        if byte[0] != b'\n' {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Display cursor is not at a record boundary",
            ));
        }
    }
    let mut text = String::new();
    let mut raw_records = 0;
    while position.record < length {
        budget.check()?;
        let end = record_end(&mut file, position.record, &budget)?;
        if end <= position.record {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "Saved transcript changed while finding its records",
            ));
        }
        let (window, raw) = formatted(
            &mut file,
            position.record,
            end - position.record,
            position.byte,
            limit - text.len(),
            &budget,
        )?;
        raw_records += usize::from(raw);
        let advanced = window.text.len() as u64;
        text.push_str(&window.text);
        if window.total > position.byte + advanced {
            position.byte += advanced;
            break;
        }
        position.record = end;
        position.byte = 0;
        if text.len() == limit {
            break;
        }
    }
    if revision(&mut file, &budget)? != before {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Saved transcript changed while displaying it",
        ));
    }
    Ok(
        json!({"session":conversation,"turn":turn,"revision":before,"text":text,"page_cursor":current,
        "next_cursor":(position.record < length).then(|| encode(&position)),"raw_offset":raw_offset,"raw_records":raw_records,
        "complete":position.record == length,"mode":"formatted"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{Conversation, storage::Transcript};
    use clap::Parser;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn conversation(temp: &TempDir) -> Conversation {
        let file = temp.path().join("design.pen");
        let data = temp.path().join("data");
        Store::init(&file, Some(data.clone())).unwrap();
        let args = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            file.into_os_string(),
        ])
        .unwrap();
        let crate::cli::Command::Prompt(args) = args.command else {
            unreachable!()
        };
        Conversation::open(args, Some(data)).unwrap()
    }
    fn turn(conversation: &Conversation) -> Transcript {
        let mut t = Transcript::create(conversation.directory()).unwrap();
        t.record(&json!({"type":"start","turn":t.id,"session":conversation.id(),"document":conversation.store().path()})).unwrap();
        t
    }
    fn budget() -> Budget<'static> {
        Budget {
            started: Instant::now(),
            cancel: None,
        }
    }

    #[test]
    fn formatted_pages_keep_all_requests_chunks_tools_and_failures_across_utf8_boundaries() {
        let temp = TempDir::new().unwrap();
        let c = conversation(&temp);
        let mut t = turn(&c);
        let expected = format!(
            "\nTurn {}\n\nYou:\n日本語\n🙂 \"quote\"\\slash\t]52;c;abc\nLine one\nつづき🙂\n[running] Read document\n[completed] Tool\nRequest failed: 失敗\n\nRequest complete\n",
            t.id
        );
        for record in [
            json!({"type":"user_prompt","text":"日本語\n🙂 \"quote\"\\slash\t\u{1b}]52;c;abc\u{7}"}),
            json!({"type":"update","notification":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Line one\n"}}}}),
            json!({"type":"update","notification":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"つづき🙂"}}}}),
            json!({"type":"update","notification":{"update":{"sessionUpdate":"tool_call","title":"Read document","status":"running"}}}),
            json!({"type":"update","notification":{"update":{"sessionUpdate":"tool_call_update","status":"completed"}}}),
            json!({"type":"failed","error":{"message":"失敗"}}),
            json!({"type":"complete"}),
        ] {
            t.record(&record).unwrap();
        }
        let before = std::fs::read(&t.path).unwrap();
        let metadata = std::fs::read(c.directory().join("session.json")).unwrap();
        let mut cursor = None;
        let mut output = String::new();
        let mut pages = 0;
        loop {
            let result = view(c.store(), c.id(), &t.id, cursor.as_deref(), 11, None).unwrap();
            let text = result["text"].as_str().unwrap();
            assert!(text.len() <= 11);
            assert_eq!(result["raw_records"], 0);
            output.push_str(text);
            pages += 1;
            let Some(next) = result["next_cursor"].as_str() else {
                assert_eq!(result["complete"], true);
                break;
            };
            assert_ne!(Some(next), cursor.as_deref());
            cursor = Some(next.to_string());
        }
        assert!(pages > 5);
        assert_eq!(output, expected);
        assert_eq!(std::fs::read(&t.path).unwrap(), before);
        assert_eq!(
            std::fs::read(c.directory().join("session.json")).unwrap(),
            metadata
        );
    }

    #[test]
    fn streaming_json_matches_serde_escapes_and_keeps_ambiguous_or_unfinished_records_raw() {
        for source in [
            r#"{"text":"\u65e5\u672c\u8a9e \ud83d\ude42 \"\\\/\b\f\n\r\t","type":"user_prompt"}"#,
            r#" { "noise": [null, true, false, -1.25e+2, {"text":"not the request"}], "text":"actual", "type":"user_prompt" } "#,
        ] {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(source.as_bytes()).unwrap();
            let reference: Value = serde_json::from_str(source).unwrap();
            let expected = format!(
                "\nYou:\n{}\n",
                reference["text"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .collect::<String>()
            );
            let (page, raw) = formatted(
                file.as_file_mut(),
                0,
                source.len() as u64,
                0,
                MAX_PAGE_BYTES,
                &budget(),
            )
            .unwrap();
            assert!(!raw);
            assert_eq!(page.text, expected);
        }
        for source in [
            r#"{"type":"user_prompt","text":"first","text":"second"}"#,
            r#"{"type":"user_prompt","text":"unfinished"#,
            r#"{"type":"user_prompt","text":"\ud800"}"#,
            r#"{"unknown":{"array":[1,2,3]}}"#,
        ] {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(source.as_bytes()).unwrap();
            let (page, raw) = formatted(
                file.as_file_mut(),
                0,
                source.len() as u64,
                0,
                MAX_PAGE_BYTES,
                &budget(),
            )
            .unwrap();
            assert!(raw);
            assert_eq!(page.text, source);
        }
        let mut file = NamedTempFile::new().unwrap();
        let mut bytes = vec![b'x'; 65535];
        bytes.extend_from_slice("日本語🙂".as_bytes());
        bytes.extend_from_slice(&[0xff, 0xe3, 0x81]);
        file.write_all(&bytes).unwrap();
        let mut output = Window::new(0, bytes.len() + 20);
        raw(
            file.as_file_mut(),
            0,
            bytes.len() as u64,
            &mut output,
            &budget(),
        )
        .unwrap();
        assert_eq!(output.text, String::from_utf8_lossy(&bytes));
    }

    #[test]
    fn huge_strings_and_keys_do_not_enter_metadata_and_late_pages_keep_the_complete_text() {
        let text = "日本語🙂".repeat(120_000);
        let key = "x".repeat(100_000);
        let source = json!({"type":"user_prompt","text":text,(key):"ignored"}).to_string();
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(source.as_bytes()).unwrap();
        let mut metadata = Metadata::default();
        scan(
            file.as_file_mut(),
            0,
            source.len() as u64,
            &mut metadata,
            &budget(),
        )
        .unwrap();
        assert!(metadata.present.contains("prompt"));
        assert!(!metadata.fields.contains_key("prompt"));
        assert!(metadata.fields.values().map(String::len).sum::<usize>() < 1024);
        let (page, raw) = formatted(
            file.as_file_mut(),
            0,
            source.len() as u64,
            6 + text.len() as u64 - 4,
            64,
            &budget(),
        )
        .unwrap();
        assert!(!raw);
        assert_eq!(page.text, "🙂\n");
    }

    #[test]
    fn display_cursors_reject_other_turns_replacements_and_invalid_boundaries() {
        let temp = TempDir::new().unwrap();
        let c = conversation(&temp);
        let mut t = turn(&c);
        t.record(&json!({"type":"user_prompt","text":"🙂".repeat(20_000)}))
            .unwrap();
        let first = view(c.store(), c.id(), &t.id, None, MAX_PAGE_BYTES, None).unwrap();
        let next = first["next_cursor"].as_str().unwrap();
        let other = turn(&c);
        assert_eq!(
            view(c.store(), c.id(), &other.id, Some(next), 64, None)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        let mut cursor: Cursor =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(next).unwrap()).unwrap();
        cursor.byte = 8;
        assert_eq!(
            view(c.store(), c.id(), &t.id, Some(&encode(&cursor)), 64, None)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        let original = std::fs::read(&t.path).unwrap();
        let mut changed = original.clone();
        let pos = changed.iter().rposition(|b| *b == 0x82).unwrap();
        changed[pos] = 0x83;
        let modified = t.file.metadata().unwrap().modified().unwrap();
        std::fs::write(&t.path, &changed).unwrap();
        t.file
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            view(c.store(), c.id(), &t.id, Some(next), 64, None)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            view(c.store(), c.id(), &t.id, None, 64, Some(&cancel))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    }

    #[test]
    fn cancellation_inside_a_large_record_is_not_downgraded_to_raw_display() {
        struct CancelRead {
            source: std::io::Cursor<Vec<u8>>,
            cancel: CancellationToken,
        }
        impl Read for CancelRead {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = self.source.read(output)?;
                if self.source.position() > 4096 {
                    self.cancel.cancel();
                }
                Ok(count)
            }
        }
        let cancel = CancellationToken::new();
        let source = json!({"text":"x".repeat(100_000),"type":"user_prompt"})
            .to_string()
            .into_bytes();
        let budget = Budget {
            started: Instant::now(),
            cancel: Some(&cancel),
        };
        let reader = CancelRead {
            source: std::io::Cursor::new(source),
            cancel: cancel.clone(),
        };
        let parser = Json {
            reader: BufReader::new(reader),
            look: None,
            consumed: 0,
            budget: &budget,
        };
        assert_eq!(
            parser.run(&mut Metadata::default()).unwrap_err().code,
            "CANCELLED"
        );
    }
}
