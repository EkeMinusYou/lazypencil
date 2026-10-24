//! Data-only defaults from documentation comments preceding a declaration.

use super::{declarations::directive, invalid, numeric};
use crate::Result;
use serde_json::{Value, json};

pub(super) fn parse(documentation: &str, kind: &str) -> Result<Option<Value>> {
    let mut default = None;
    for line in documentation.lines() {
        let line = line.trim().trim_start_matches('*').trim();
        for (index, _) in line.match_indices("@default") {
            if index > 0
                && !line[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            {
                continue;
            }
            let tail = &line[index + "@default".len()..];
            if tail
                .chars()
                .next()
                .is_some_and(|value| !value.is_whitespace())
            {
                continue;
            }
            if default.is_some() {
                return Err(invalid("A uniform must have at most one @default"));
            }
            let raw = tail
                .split('@')
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches('*')
                .trim();
            if raw.len() > 64 * 1024 {
                return Err(invalid("Uniform @default exceeds 64 KiB"));
            }
            let value = if raw.starts_with('#') {
                if !directive(documentation, "color") || !matches!(kind, "vec3" | "vec4") {
                    return Err(invalid("Hex @default requires @color and vec3 or vec4"));
                }
                Value::String(raw.into())
            } else {
                let mut values = Vec::new();
                for value in raw.split(',') {
                    if values.len() == 4096 {
                        return Err(invalid("Uniform @default exceeds 4096 components"));
                    }
                    values.push(json!(decimal_prefix(value)?));
                }
                if values.len() == 1 {
                    values.pop().unwrap()
                } else {
                    Value::Array(values)
                }
            };
            let value = if kind == "bool" {
                match value.as_f64() {
                    Some(0.0) => Value::Bool(false),
                    Some(1.0) => Value::Bool(true),
                    _ => return Err(invalid("Boolean @default requires 0 or 1")),
                }
            } else {
                value
            };
            default = Some(
                numeric(kind, Some(&value))
                    .map_err(|_| invalid("Uniform @default does not match its numeric type"))?,
            );
        }
    }
    Ok(default)
}

fn decimal_prefix(value: &str) -> Result<f64> {
    // Native annotations use decimal-prefix parsing, including a leading sign
    // and exponent. For example, 0x2 has decimal prefix zero, not hexadecimal 2.
    let text = value.trim_start();
    let bytes = text.as_bytes();
    let mut cursor = usize::from(
        bytes
            .first()
            .is_some_and(|byte| matches!(byte, b'+' | b'-')),
    );
    let start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let mut digits = cursor - start;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        digits += cursor - start;
    }
    if digits == 0 {
        return Err(invalid("Uniform @default requires finite decimal numbers"));
    }
    if bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        let exponent = cursor;
        cursor += 1;
        if bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            cursor += 1;
        }
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            cursor = exponent;
        }
    }
    text[..cursor]
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid("Uniform @default requires finite decimal numbers"))
}
