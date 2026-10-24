//! Bounded GLSL declaration scanning; compilation remains WebGL's responsibility.

use super::{Uniform, invalid};
use crate::Result;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    comment: bool,
    documentation: bool,
}

pub(super) fn parse(source: &str) -> Result<Vec<Uniform>> {
    if source.len() > 1024 * 1024 {
        return Err(invalid("Shader source exceeds 1 MiB"));
    }
    let tokens = lex(source)?;
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut pending = String::new();
    let mut names = BTreeSet::new();
    let mut result = Vec::new();
    while cursor < tokens.len() {
        let token = tokens[cursor];
        if token.comment {
            pending = if token.documentation {
                token.text.into()
            } else {
                String::new()
            };
            cursor += 1;
            continue;
        }
        if token.text != "uniform" || depth != 0 {
            match token.text {
                "{" => depth += 1,
                "}" => depth = depth.saturating_sub(1),
                _ => {}
            }
            pending.clear();
            cursor += 1;
            continue;
        }
        cursor += 1;
        let documentation = std::mem::take(&mut pending);
        let mut kind = code(&tokens, &mut cursor)?;
        if matches!(kind, "lowp" | "mediump" | "highp") {
            kind = code(&tokens, &mut cursor)?;
        }
        let mut declarations = Vec::new();
        loop {
            let name = code(&tokens, &mut cursor)?;
            if !identifier(name) {
                return Err(invalid("Expected a GLSL uniform identifier"));
            }
            comments(&tokens, &mut cursor);
            let mut array = false;
            let mut length = None;
            if tokens.get(cursor).is_some_and(|token| token.text == "[") {
                array = true;
                cursor += 1;
                let start = cursor;
                while tokens.get(cursor).is_some_and(|token| token.text != "]") {
                    if matches!(tokens[cursor].text, ";" | "{" | "}" | "[") {
                        return Err(invalid("Invalid or multidimensional uniform array"));
                    }
                    cursor += 1;
                }
                if cursor == tokens.len() {
                    return Err(invalid("Unclosed uniform array dimension"));
                }
                let expression: Vec<_> = tokens[start..cursor]
                    .iter()
                    .filter(|token| !token.comment)
                    .collect();
                if expression.is_empty() {
                    return Err(invalid("Uniform arrays require a constant dimension"));
                }
                if expression.len() == 1 {
                    let value = expression[0].text;
                    length = if let Some(hex) = value
                        .strip_prefix("0x")
                        .or_else(|| value.strip_prefix("0X"))
                    {
                        usize::from_str_radix(hex, 16).ok()
                    } else if value.len() > 1 && value.starts_with('0') {
                        usize::from_str_radix(&value[1..], 8).ok()
                    } else {
                        value.parse::<usize>().ok()
                    };
                    if value.as_bytes()[0].is_ascii_digit() && length.is_none() {
                        return Err(invalid(
                            "Uniform array dimension exceeds the supported integer literal range",
                        ));
                    }
                    if length.is_some_and(|length| length == 0 || length > 256) {
                        return Err(invalid("Uniform arrays support 1–256 elements"));
                    }
                }
                cursor += 1;
                comments(&tokens, &mut cursor);
            }
            declarations.push((name, array, length));
            match code(&tokens, &mut cursor)? {
                "," => continue,
                ";" => break,
                _ => {
                    return Err(invalid(
                        "Expected a comma or semicolon in a uniform declaration",
                    ));
                }
            }
        }
        let resolution = directive(&documentation, "resolution");
        let time = directive(&documentation, "time");
        let default = super::defaults::parse(&documentation, kind)?;
        if time && default.is_some() {
            return Err(invalid("Built-in @time inputs do not support @default"));
        }
        if resolution && kind != "vec2" || time && kind != "float" || resolution && time {
            return Err(invalid("@resolution requires vec2; @time requires float"));
        }
        for (name, array, length) in declarations {
            if !names.insert(name) || result.len() >= 256 {
                return Err(invalid(
                    "Uniform names must be unique, with at most 256 declarations",
                ));
            }
            if array && (resolution || time) {
                return Err(invalid(
                    "Built-in @resolution and @time inputs must be scalar declarations",
                ));
            }
            result.push(Uniform {
                name: name.into(),
                kind: kind.into(),
                resolution,
                time,
                array,
                length,
                default: default.clone(),
            });
        }
    }
    Ok(result)
}

fn comments(tokens: &[Token<'_>], cursor: &mut usize) {
    while tokens.get(*cursor).is_some_and(|token| token.comment) {
        *cursor += 1;
    }
}

fn code<'a>(tokens: &[Token<'a>], cursor: &mut usize) -> Result<&'a str> {
    comments(tokens, cursor);
    let token = tokens
        .get(*cursor)
        .ok_or_else(|| invalid("Incomplete GLSL uniform declaration"))?;
    *cursor += 1;
    Ok(token.text)
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn directive(comment: &str, name: &str) -> bool {
    comment.match_indices('@').any(|(position, _)| {
        if position != 0
            && !comment[..position]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || c == '*')
        {
            return false;
        }
        let tail = &comment[position + 1..];
        tail.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            == Some(name)
    })
}

fn lex(source: &str) -> Result<Vec<Token<'_>>> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut tokens = Vec::new();
    while cursor < bytes.len() {
        let start = cursor;
        if bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        let line_comment = bytes[cursor..].starts_with(b"//");
        let block_comment = bytes[cursor..].starts_with(b"/*");
        let documentation = bytes[cursor..].starts_with(b"/**");
        let comment = line_comment || block_comment;
        if line_comment {
            cursor = source[cursor..]
                .find('\n')
                .map_or(bytes.len(), |offset| cursor + offset);
        } else if block_comment {
            cursor = source[cursor + 2..]
                .find("*/")
                .ok_or_else(|| invalid("Unclosed GLSL annotation comment"))?
                + cursor
                + 4;
        } else if bytes[cursor] == b'#' {
            // Macro bodies are not literal interface declarations. The actual
            // GLSL preprocessor still runs inside WebGL's compiler.
            loop {
                let end = source[cursor..]
                    .find('\n')
                    .map_or(bytes.len(), |offset| cursor + offset);
                let continued = source[cursor..end].trim_end_matches('\r').ends_with('\\');
                cursor = end;
                if cursor == bytes.len() || !continued {
                    break;
                }
                cursor += 1;
            }
        } else if bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_' {
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
        } else {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        if tokens.len() >= 262144 {
            return Err(invalid("Shader declaration token budget exceeded"));
        }
        let text = if block_comment {
            &source[start + 2..cursor - 2]
        } else if line_comment {
            &source[start + 2..cursor]
        } else {
            &source[start..cursor]
        };
        tokens.push(Token {
            text,
            comment,
            documentation,
        });
    }
    Ok(tokens)
}
