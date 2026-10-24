//! Bounded GLSL declaration scanning; compilation remains WebGL's responsibility.

use super::{Uniform, invalid};
use crate::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    comment: bool,
    documentation: bool,
}

#[derive(Clone)]
struct Declaration {
    name: String,
    kind: Arc<str>,
    array: bool,
    length: Option<usize>,
    documentation: Arc<str>,
}

type Structures = BTreeMap<String, Vec<Declaration>>;

pub(super) fn parse(source: &str) -> Result<Vec<Uniform>> {
    if source.len() > 1024 * 1024 {
        return Err(invalid("Shader source exceeds 1 MiB"));
    }
    parse_expanded(source)
}

pub(super) fn parse_expanded(source: &str) -> Result<Vec<Uniform>> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(invalid("Expanded shader exceeds 4 MiB"));
    }
    let tokens = lex(source)?;
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut pending = String::new();
    let mut names = BTreeSet::new();
    let mut structures = Structures::new();
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
        if token.text == "struct" && depth == 0 {
            cursor += 1;
            structure(&tokens, &mut cursor, &mut structures)?;
            pending.clear();
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
        let kind = kind(&tokens, &mut cursor, &mut structures, true)?;
        for declaration in declarations(&tokens, &mut cursor, &kind, &documentation)? {
            if !names.insert(declaration.name.clone()) || names.len() > 256 {
                return Err(invalid(
                    "Uniform names must be unique, with at most 256 declarations",
                ));
            }
            expand(declaration, &structures, Vec::new(), 0, &mut result)?;
        }
    }
    Ok(result)
}

fn kind(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    structures: &mut Structures,
    allow_structure: bool,
) -> Result<String> {
    let mut kind = code(tokens, cursor)?;
    if matches!(kind, "lowp" | "mediump" | "highp") {
        kind = code(tokens, cursor)?;
    }
    if !identifier(kind) {
        return Err(invalid("Expected a bounded GLSL uniform type name"));
    }
    if kind == "struct" {
        if !allow_structure {
            return Err(invalid(
                "Embedded structure definitions are invalid in GLSL ES 1.00",
            ));
        }
        structure(tokens, cursor, structures)
    } else {
        Ok(kind.into())
    }
}

fn structure(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    structures: &mut Structures,
) -> Result<String> {
    if structures.len() >= 256 {
        return Err(invalid("Shader structures exceed 256 type definitions"));
    }
    let token = code(tokens, cursor)?;
    let name = if token == "{" {
        format!("@anonymous-{}", *cursor)
    } else {
        if !identifier(token) || code(tokens, cursor)? != "{" {
            return Err(invalid("Expected a GLSL structure definition"));
        }
        token.into()
    };
    let mut fields = Vec::new();
    let mut names = BTreeSet::new();
    let mut pending = String::new();
    loop {
        let token = tokens
            .get(*cursor)
            .ok_or_else(|| invalid("Unclosed GLSL structure"))?;
        if token.comment {
            pending = if token.documentation {
                token.text.into()
            } else {
                String::new()
            };
            *cursor += 1;
            continue;
        }
        if token.text == "}" {
            *cursor += 1;
            break;
        }
        let kind = kind(tokens, cursor, structures, false)?;
        for field in declarations(tokens, cursor, &kind, &std::mem::take(&mut pending))? {
            if !names.insert(field.name.clone()) || fields.len() >= 256 {
                return Err(invalid(
                    "Structure fields must be unique, with at most 256 fields",
                ));
            }
            fields.push(field);
        }
    }
    if fields.is_empty() || structures.insert(name.clone(), fields).is_some() {
        return Err(invalid(
            "Structure types must be nonempty and uniquely defined",
        ));
    }
    Ok(name)
}

fn declarations(
    tokens: &[Token<'_>],
    cursor: &mut usize,
    kind: &str,
    documentation: &str,
) -> Result<Vec<Declaration>> {
    let mut result = Vec::new();
    let documentation: Arc<str> = documentation.into();
    let kind: Arc<str> = kind.into();
    loop {
        let name = code(tokens, cursor)?;
        if !identifier(name) {
            return Err(invalid("Expected a GLSL uniform identifier"));
        }
        comments(tokens, cursor);
        let mut array = false;
        let mut length = None;
        if tokens.get(*cursor).is_some_and(|token| token.text == "[") {
            array = true;
            *cursor += 1;
            let start = *cursor;
            while tokens.get(*cursor).is_some_and(|token| token.text != "]") {
                if matches!(tokens[*cursor].text, ";" | "{" | "}" | "[") {
                    return Err(invalid("Invalid or multidimensional uniform array"));
                }
                *cursor += 1;
            }
            if *cursor == tokens.len() {
                return Err(invalid("Unclosed uniform array dimension"));
            }
            let expression: Vec<_> = tokens[start..*cursor]
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
            *cursor += 1;
            comments(tokens, cursor);
        }
        result.push(Declaration {
            name: name.into(),
            kind: kind.clone(),
            array,
            length,
            documentation: documentation.clone(),
        });
        if result.len() > 256 {
            return Err(invalid("Too many grouped GLSL declarations"));
        }
        match code(tokens, cursor)? {
            "," => continue,
            ";" => break,
            _ => {
                return Err(invalid(
                    "Expected a comma or semicolon in a uniform declaration",
                ));
            }
        }
    }
    Ok(result)
}

fn expand(
    declaration: Declaration,
    structures: &Structures,
    mut indices: Vec<Option<usize>>,
    depth: usize,
    result: &mut Vec<Uniform>,
) -> Result<()> {
    if depth > 16 || declaration.name.len() > 4096 {
        return Err(invalid(
            "Uniform structure nesting exceeds 16 levels, contains a cycle, or has an overlong qualified name",
        ));
    }
    if let Some(fields) = structures.get(declaration.kind.as_ref()) {
        if directive(&declaration.documentation, "resolution")
            || directive(&declaration.documentation, "time")
            || directive(&declaration.documentation, "default")
        {
            return Err(invalid(
                "Uniform annotations must target typed structure fields, not an aggregate",
            ));
        }
        let prefix = if declaration.array {
            indices.push(declaration.length);
            format!("{}[]", declaration.name)
        } else {
            declaration.name
        };
        for field in fields {
            let mut field = field.clone();
            field.name = format!("{prefix}.{}", field.name);
            expand(field, structures, indices.clone(), depth + 1, result)?;
        }
        return Ok(());
    }
    let resolution = directive(&declaration.documentation, "resolution");
    let time = directive(&declaration.documentation, "time");
    let default = super::defaults::parse(&declaration.documentation, &declaration.kind)?;
    if time && default.is_some() {
        return Err(invalid("Built-in @time inputs do not support @default"));
    }
    if resolution && declaration.kind.as_ref() != "vec2"
        || time && declaration.kind.as_ref() != "float"
        || resolution && time
    {
        return Err(invalid("@resolution requires vec2; @time requires float"));
    }
    if (declaration.array || !indices.is_empty()) && (resolution || time) {
        return Err(invalid(
            "Built-in @resolution and @time inputs must be scalar declarations",
        ));
    }
    if result.len() >= 256 {
        return Err(invalid("Uniform expansion exceeds 256 typed leaf patterns"));
    }
    result.push(Uniform {
        name: declaration.name,
        kind: declaration.kind.to_string(),
        resolution,
        time,
        array: declaration.array,
        length: declaration.length,
        default,
        indices,
    });
    Ok(())
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
    if value.len() > 1024 {
        return false;
    }
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
