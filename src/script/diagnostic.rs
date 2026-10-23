//! Bounded source locations from the pinned engine and the header parser.
use crate::Error;
use rquickjs::CaughtError;
use serde_json::json;

pub(super) const SOURCE_NAME: &str = "lazypencil-script.js";
const WRAPPER_LINES: usize = 3;
const MAX_TEXT: usize = 4096;
const EXCERPT_CHARS: usize = 240;

pub(super) fn javascript(error: CaughtError<'_>, source: &str, stage: &str) -> Error {
    // Read only the stack property for positions. A message can contain text
    // resembling a stack frame without identifying the throwing location.
    let stack = match &error {
        CaughtError::Exception(error) => error.stack(),
        _ => None,
    };
    let message = error.to_string().chars().take(MAX_TEXT).collect::<String>();
    let mut location = None;
    if let Some(stack) = stack {
        for (index, frame) in stack.lines().take(64).enumerate() {
            let Some(frame) = frame.trim().strip_prefix("at ") else {
                continue;
            };
            let frame = frame.strip_suffix(')').unwrap_or(frame);
            let Some((prefix, position)) = frame.rsplit_once(&format!("{SOURCE_NAME}:")) else {
                continue;
            };
            if !prefix.is_empty() && !prefix.ends_with('(') {
                continue;
            }
            let Some((line, column)) = position.split_once(':') else {
                continue;
            };
            let (Ok(line), Ok(column)) = (line.parse::<usize>(), column.parse::<usize>()) else {
                continue;
            };
            let Some(line) = line.checked_sub(WRAPPER_LINES).filter(|line| *line > 0) else {
                continue;
            };
            location = Some((
                line,
                column,
                if index == 0 { "source" } else { "call_site" },
            ));
            break;
        }
    }
    if stage == "compile"
        && let Some((line, _, _)) = location
        && let Some((last, (_, text))) = lines(source).enumerate().last()
        && line == last + 2
    {
        location = Some((last + 1, text.len() + 1, "end_of_source"));
    }
    let location = location.filter(|(line, column, _)| {
        lines(source)
            .nth(line - 1)
            .is_some_and(|(_, text)| *column > 0 && *column <= text.len() + 1)
    });
    attach(super::invalid(message), source, stage, location)
}

pub(super) fn header(error: Error, source: &str, offset: usize) -> Error {
    let (index, (start, text)) = lines(source)
        .enumerate()
        .take_while(|(_, (start, _))| *start <= offset)
        .last()
        .expect("the first line starts at zero");
    let column = offset.saturating_sub(start).min(text.len()) + 1;
    attach(
        error,
        source,
        "header",
        Some((index + 1, column, "declaration")),
    )
}

fn attach(
    mut error: Error,
    source: &str,
    stage: &str,
    location: Option<(usize, usize, &str)>,
) -> Error {
    let (line, column, kind) = location
        .map_or((None, None, "unavailable"), |(line, column, kind)| {
            (Some(line), Some(column), kind)
        });
    let mut excerpt = Vec::new();
    if let Some(line) = line {
        let start = line.saturating_sub(2);
        for (index, (_, text)) in lines(source).enumerate().skip(start).take(3) {
            let focus = if index + 1 == line {
                column.unwrap_or(1) - 1
            } else {
                0
            };
            let mut before = focus.min(text.len());
            while !text.is_char_boundary(before) {
                before -= 1;
            }
            let from = text[..before]
                .char_indices()
                .rev()
                .nth(79)
                .map_or(0, |(index, _)| index);
            let end = text[from..]
                .char_indices()
                .nth(EXCERPT_CHARS)
                .map_or(text.len(), |(index, _)| from + index);
            excerpt.push(
                json!({"line":index+1,"text":&text[from..end],"start_column":from+1,
                "truncated_start":from>0,"truncated_end":end<text.len()}),
            );
        }
    }
    let details = error.details.get_or_insert_with(|| json!({}));
    details["script_diagnostic"] = json!({"stage":stage,"line":line,"column":column,
        "column_unit":"utf8_byte","location_kind":kind,"source_excerpt":excerpt});
    error
}

/// ECMAScript line endings, retaining a final empty line for EOF diagnostics.
pub(super) fn lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = Some(0);
    std::iter::from_fn(move || {
        let start = offset?;
        let rest = &source[start..];
        if let Some(end) = rest.find(['\r', '\n', '\u{2028}', '\u{2029}']) {
            let separator = rest[end..].chars().next().expect("separator");
            let length = if rest[end..].starts_with("\r\n") {
                2
            } else {
                separator.len_utf8()
            };
            offset = Some(start + end + length);
            Some((start, &rest[..end]))
        } else {
            offset = None;
            Some((start, rest))
        }
    })
}

#[cfg(feature = "rendering")]
pub(super) fn context(
    mut error: Error,
    node: &crate::project::NodeAddress,
    path: &std::path::Path,
    hash: &str,
) -> Error {
    let details = error.details.get_or_insert_with(|| json!({}));
    details["node"] = json!(node);
    details["script_path"] = json!(path);
    details["source_hash"] = json!(hash);
    error
}
