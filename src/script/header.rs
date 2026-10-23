use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub schema: String,
    pub inputs: Vec<Input>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub name: String,
    #[serde(flatten)]
    pub kind: InputKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputKind {
    Number { min: Option<f64>, max: Option<f64> },
    String,
    Boolean,
    Color,
    Enum { options: Vec<String> },
    Ref,
}

impl Header {
    pub fn parse(code: &str) -> Result<Self> {
        if code.is_empty() || code.len() > super::MAX_CODE_BYTES {
            return Err(invalid("Script source must contain 1..1048576 bytes"));
        }
        let source = code;
        let code = code.trim_start_matches('\u{feff}').trim_start();
        let start = source.len() - code.len();
        let comment = code
            .strip_prefix("/*")
            .and_then(|s| s.split_once("*/").map(|(header, _)| header))
            .filter(|header| header.len() <= 64 * 1024)
            .ok_or_else(|| {
                super::diagnostic::header(
                    invalid("A script must start with a header comment containing @schema"),
                    source,
                    start,
                )
            })?;
        let mut schema = None;
        let mut inputs = Vec::new();
        let mut names = BTreeSet::new();
        for (offset, raw) in super::diagnostic::lines(comment) {
            let line = raw.trim().trim_start_matches('*').trim();
            let offset = start + 2 + offset + raw.find(line).unwrap_or(0);
            let located = |error| super::diagnostic::header(error, source, offset);
            if let Some(version) = line
                .strip_prefix("@schema")
                .filter(|rest| rest.starts_with(char::is_whitespace))
            {
                if schema
                    .replace((version.trim().to_string(), offset))
                    .is_some()
                {
                    return Err(located(invalid("Duplicate @schema declaration")));
                }
            } else if let Some(input) = line
                .strip_prefix("@input")
                .filter(|rest| rest.starts_with(char::is_whitespace))
            {
                let input = parse_input(input.trim()).map_err(located)?;
                if inputs.len() >= 256 || !names.insert(input.name.clone()) {
                    return Err(located(invalid(
                        "Input names must be unique, with at most 256 declarations",
                    )));
                }
                inputs.push(input);
            }
        }
        let (schema, offset) = schema.ok_or_else(|| {
            super::diagnostic::header(
                invalid("The script header is missing @schema"),
                source,
                start,
            )
        })?;
        if schema != crate::document::DOCUMENT_VERSION {
            return Err(super::diagnostic::header(
                Error::new(
                    "UNSUPPORTED_VERSION",
                    format!(
                        "Script schema {schema:?} is unsupported; supported schema is {}",
                        crate::document::DOCUMENT_VERSION
                    ),
                ),
                source,
                offset,
            ));
        }
        Ok(Self { schema, inputs })
    }

    /// Missing inputs without a declared default remain absent (undefined in JS).
    pub fn prepare(&self, values: &Map<String, Value>) -> Result<Map<String, Value>> {
        if values
            .keys()
            .any(|key| !self.inputs.iter().any(|input| &input.name == key))
        {
            return Err(invalid("Script inputs contain an undeclared name"));
        }
        let mut prepared = Map::new();
        for input in &self.inputs {
            if let Some(value) = values.get(&input.name).or(input.default.as_ref()) {
                prepared.insert(input.name.clone(), input.validate(value)?);
            }
        }
        Ok(prepared)
    }
}

impl Input {
    fn validate(&self, value: &Value) -> Result<Value> {
        let valid = match &self.kind {
            InputKind::Number { min, max } => {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        invalid(format!("Input {:?} must be a finite number", self.name))
                    })?;
                return Ok(Value::from(
                    value
                        .max(min.unwrap_or(f64::NEG_INFINITY))
                        .min(max.unwrap_or(f64::INFINITY)),
                ));
            }
            InputKind::String => value.is_string(),
            InputKind::Boolean => value.is_boolean(),
            InputKind::Color => value.as_str().is_some_and(|s| {
                matches!(s.len(), 4 | 5 | 7 | 9)
                    && s.starts_with('#')
                    && s[1..].bytes().all(|c| c.is_ascii_hexdigit())
            }),
            InputKind::Enum { options } => value
                .as_str()
                .is_some_and(|s| options.iter().any(|option| option == s)),
            InputKind::Ref => value.as_str().is_some_and(|s| {
                !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
            }),
        };
        if !valid {
            return Err(invalid(format!(
                "Input {:?} does not match its declared type or options",
                self.name
            )));
        }
        Ok(value.clone())
    }
}

fn parse_input(line: &str) -> Result<Input> {
    let (name, rest) = line
        .split_once(':')
        .ok_or_else(|| invalid("An @input needs a name and type separated by ':'"))?;
    let name = name.trim();
    if name.len() > 128
        || !name.chars().enumerate().all(|(index, c)| {
            c.is_ascii_alphabetic() || c == '_' || c == '$' || index > 0 && c.is_ascii_digit()
        })
        || name.is_empty()
    {
        return Err(invalid(
            "Input names must be ASCII identifiers of at most 128 bytes",
        ));
    }
    let rest = rest.trim();
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let kind = &rest[..end];
    let mut remaining = rest[end..].trim();
    let mut args = None;
    if let Some(after) = remaining.strip_prefix('(') {
        let end = closing_paren(after).ok_or_else(|| invalid("Unclosed input type arguments"))?;
        args = Some(&after[..end]);
        remaining = after[end + 1..].trim();
    }
    let kind = match (kind, args) {
        ("number", args) => {
            let mut min = None;
            let mut max = None;
            if let Some(args) = args {
                for arg in args.split(',') {
                    let (name, value) = arg.split_once('=').ok_or_else(|| {
                        invalid("Number arguments must be min=NUMBER or max=NUMBER")
                    })?;
                    let value = value
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| invalid("Number limits must be finite"))?;
                    let target = match name.trim() {
                        "min" => &mut min,
                        "max" => &mut max,
                        _ => return Err(invalid("Unknown number input argument")),
                    };
                    if target.replace(value).is_some() {
                        return Err(invalid("Duplicate number input limit"));
                    }
                }
            }
            if min.zip(max).is_some_and(|(min, max)| min > max) {
                return Err(invalid("Input minimum exceeds its maximum"));
            }
            InputKind::Number { min, max }
        }
        ("enum", Some(args)) => {
            let options: Vec<String> = serde_json::from_str(&format!("[{args}]"))
                .map_err(|_| invalid("Enum arguments must be quoted JSON strings"))?;
            if options.is_empty()
                || options.len() > 256
                || options.iter().collect::<BTreeSet<_>>().len() != options.len()
            {
                return Err(invalid("An enum requires 1..256 unique options"));
            }
            InputKind::Enum { options }
        }
        ("string", None) => InputKind::String,
        ("boolean", None) => InputKind::Boolean,
        ("color", None) => InputKind::Color,
        ("ref", None) => InputKind::Ref,
        _ => return Err(invalid("Unsupported input type or arguments")),
    };
    let default = if remaining.is_empty() {
        None
    } else {
        let raw = remaining
            .strip_prefix('=')
            .ok_or_else(|| invalid("Unexpected text after input type"))?
            .trim();
        Some(
            if matches!(kind, InputKind::Color) && raw.starts_with('#') {
                Value::String(raw.into())
            } else {
                serde_json::from_str(raw).map_err(|_| {
                    invalid("Input default must be a JSON value or a hexadecimal color")
                })?
            },
        )
    };
    let mut input = Input {
        name: name.into(),
        kind,
        default,
    };
    if let Some(value) = &input.default {
        input.default = Some(input.validate(value)?);
    }
    Ok(input)
}

fn closing_paren(args: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, c) in args.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
        }
        if c == ')' && !quoted {
            return Some(index);
        }
    }
    None
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("SCRIPT_EXECUTION_FAILED", message)
}
