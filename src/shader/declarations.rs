//! Bounded GLSL declaration scanning; compilation remains WebGL's responsibility.

use super::{Uniform, invalid};
use crate::Result;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    comment: bool,
    trailing: bool,
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
            if !token.trailing {
                pending.push('\n');
                pending.push_str(token.text);
            }
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
        let mut annotations = std::mem::take(&mut pending);
        let mut kind = code(&tokens, &mut cursor, &mut annotations)?;
        if matches!(kind, "lowp" | "mediump" | "highp") {
            kind = code(&tokens, &mut cursor, &mut annotations)?;
        }
        let mut declarations = Vec::new();
        loop {
            let name = code(&tokens, &mut cursor, &mut annotations)?;
            if !identifier(name) {
                return Err(invalid("Expected a GLSL uniform identifier"));
            }
            comments(&tokens, &mut cursor, &mut annotations);
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
                comments(&tokens, &mut cursor, &mut annotations);
            }
            declarations.push((name, array, length));
            match code(&tokens, &mut cursor, &mut annotations)? {
                "," => continue,
                ";" => break,
                _ => {
                    return Err(invalid(
                        "Expected a comma or semicolon in a uniform declaration",
                    ));
                }
            }
        }
        while tokens
            .get(cursor)
            .is_some_and(|token| token.comment && token.trailing)
        {
            annotations.push('\n');
            annotations.push_str(tokens[cursor].text);
            cursor += 1;
        }
        let resolution = directive(&annotations, "resolution");
        let time = directive(&annotations, "time");
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
            });
        }
    }
    Ok(result)
}

fn comments(tokens: &[Token<'_>], cursor: &mut usize, annotations: &mut String) {
    while tokens.get(*cursor).is_some_and(|token| token.comment) {
        annotations.push('\n');
        annotations.push_str(tokens[*cursor].text);
        *cursor += 1;
    }
}

fn code<'a>(tokens: &[Token<'a>], cursor: &mut usize, annotations: &mut String) -> Result<&'a str> {
    comments(tokens, cursor, annotations);
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

fn directive(comment: &str, name: &str) -> bool {
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
    let (mut cursor, mut line) = (0, 0);
    let mut last_code = None;
    let mut tokens = Vec::new();
    while cursor < bytes.len() {
        let start = cursor;
        if bytes[cursor].is_ascii_whitespace() {
            line += usize::from(bytes[cursor] == b'\n');
            cursor += 1;
            continue;
        }
        if bytes[cursor..].starts_with(b"//") {
            cursor = source[cursor..]
                .find('\n')
                .map_or(bytes.len(), |offset| cursor + offset);
            continue;
        }
        let comment = bytes[cursor..].starts_with(b"/*");
        let trailing = last_code == Some((line, true));
        if comment {
            cursor = source[cursor + 2..]
                .find("*/")
                .ok_or_else(|| invalid("Unclosed GLSL annotation comment"))?
                + cursor
                + 4;
            line += source[start..cursor]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
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
                line += 1;
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
        let text = if comment {
            &source[start + 2..cursor - 2]
        } else {
            &source[start..cursor]
        };
        tokens.push(Token {
            text,
            comment,
            trailing,
        });
        if !comment {
            last_code = Some((line, text == ";"));
        }
    }
    Ok(tokens)
}
