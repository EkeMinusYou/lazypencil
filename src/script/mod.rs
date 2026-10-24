//! Synchronous script execution through a bounded, disposable worker process.

mod diagnostic;
pub mod header;
#[cfg(feature = "rendering")]
mod layout;
mod worker;

#[cfg(feature = "rendering")]
pub use layout::compute;
pub use worker::ScriptRuntime;

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ENGINE: &str = "rquickjs/0.14.0;quickjs-ng/0.16.2;random/mulberry32-seed0-v1;source/function-v2;header/2.11-2.19-2.21-v1;inputs/native-defaults-v1;policy/v1";
pub const MAX_CODE_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_NODES: usize = 1000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRequest {
    pub code: String,
    pub width: f64,
    pub height: f64,
    pub inputs: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptOutput {
    pub nodes: Vec<Value>,
    pub reads_width: bool,
    pub reads_height: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Reply {
    Success {
        output: ScriptOutput,
    },
    Failure {
        message: String,
        timeout: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
    },
}

pub fn serve_worker() -> Result<()> {
    worker::serve()
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("SCRIPT_EXECUTION_FAILED", message)
}

fn validate_output(output: &[Value]) -> Result<()> {
    fn visit(node: &Value, count: &mut usize, depth: usize) -> Result<()> {
        *count += 1;
        if *count > MAX_NODES || depth > 64 {
            return Err(invalid("Script output exceeds 1000 nodes or 64 levels"));
        }
        let kind = node
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Every generated node must be an object with a type"))?;
        if !crate::document::NODE_TYPES.contains(&kind) {
            return Err(invalid("Script output contains an unknown node type"));
        }
        if kind == "script"
            && node
                .get("children")
                .and_then(Value::as_array)
                .is_some_and(|children| !children.is_empty())
        {
            return Err(invalid(
                "A generated script node cannot store its own children",
            ));
        }
        if node.get("reusable").is_some_and(|v| v != false) {
            return Err(invalid("Script output cannot define reusable components"));
        }
        if let Some(children) = node.get("children") {
            for child in children
                .as_array()
                .ok_or_else(|| invalid("Generated children must be an array"))?
            {
                visit(child, count, depth + 1)?;
            }
        }
        if kind == "ref" {
            overrides(node, count, depth + 1)?;
        }
        Ok(())
    }
    fn overrides(value: &Value, count: &mut usize, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(invalid("Script overrides exceed 64 levels"));
        }
        if let Some(descendants) = value.get("descendants") {
            for changes in descendants
                .as_object()
                .ok_or_else(|| invalid("Generated overrides must be objects"))?
                .values()
            {
                if changes.get("type").is_some() {
                    visit(changes, count, depth + 1)?;
                } else {
                    if !changes.is_object() {
                        return Err(invalid("Generated overrides must be objects"));
                    }
                    if let Some(children) = changes.get("children") {
                        for child in children
                            .as_array()
                            .ok_or_else(|| invalid("Generated children must be an array"))?
                        {
                            visit(child, count, depth + 1)?;
                        }
                    }
                    overrides(changes, count, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    let mut count = 0;
    for node in output {
        visit(node, &mut count, 0)?;
    }
    Ok(())
}
