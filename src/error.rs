use serde::Serialize;
use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{code}: {message}{diagnostic}", diagnostic = self.diagnostic_text())]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Error {
    fn diagnostic_text(&self) -> String {
        use std::fmt::Write;
        let Some(details) = &self.details else {
            return String::new();
        };
        let Some(diagnostic) = details.get("script_diagnostic") else {
            return String::new();
        };
        let Some(line) = diagnostic["line"].as_u64() else {
            return String::new();
        };
        let path = details["script_path"].as_str().unwrap_or("<script>");
        let path = path
            .chars()
            .take(1024)
            .flat_map(char::escape_debug)
            .collect::<String>();
        let column = diagnostic["column"].as_u64().unwrap_or(1);
        let kind = diagnostic["location_kind"]
            .as_str()
            .filter(|kind| {
                matches!(
                    *kind,
                    "source" | "call_site" | "declaration" | "end_of_source"
                )
            })
            .unwrap_or("source");
        let mut output = format!("\n  --> {path}:{line}:{column} (UTF-8 byte column, {kind})");
        if let Some(excerpt) = diagnostic["source_excerpt"].as_array() {
            for item in excerpt.iter().take(3) {
                let number = item["line"].as_u64().unwrap_or(0);
                let text = item["text"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(240)
                    .flat_map(char::escape_debug)
                    .collect::<String>();
                let marker = if number == line { '>' } else { ' ' };
                let prefix = if item["truncated_start"] == true {
                    "…"
                } else {
                    ""
                };
                let suffix = if item["truncated_end"] == true {
                    "…"
                } else {
                    ""
                };
                let _ = write!(output, "\n{marker} {number:>4} | {prefix}{text}{suffix}");
            }
        }
        output
    }

    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn details(mut self, value: impl Into<serde_json::Value>) -> Self {
        self.details = Some(value.into());
        self
    }

    pub fn io(path: &Path, error: std::io::Error) -> Self {
        Self::new("IO_ERROR", format!("{}: {error}", path.display()))
    }

    pub fn exit_code(&self) -> u8 {
        match self.code {
            "INVALID_ARGUMENT" => 2,
            "REVISION_CONFLICT" | "REQUEST_ID_CONFLICT" => 3,
            "INVALID_DOCUMENT"
            | "UNSUPPORTED_VERSION"
            | "INVALID_OPERATION"
            | "UNSUPPORTED_FEATURE"
            | "ASSET_NOT_FOUND"
            | "ASSET_CONFLICT"
            | "FONT_LOCK_INVALID"
            | "FONT_LOCK_MISMATCH"
            | "REFERENCE_CYCLE"
            | "LAYOUT_UNRESOLVED"
            | "SNAPSHOT_UNAVAILABLE"
            | "ATTACHMENT_INVALID"
            | "PROJECT_NOT_TRUSTED"
            | "CLIPBOARD_UNAVAILABLE"
            | "CLIPBOARD_EMPTY"
            | "CLIPBOARD_TIMEOUT"
            | "ICON_INTEGRITY"
            | "ICON_UNAVAILABLE"
            | "RUNTIME_UNAVAILABLE"
            | "RUNTIME_INTEGRITY"
            | "SCRIPT_EXECUTION_FAILED"
            | "SHADER_COMPILE_FAILED"
            | "PAGE_LOAD_FAILED"
            | "RENDER_TIMEOUT" => 4,
            "RESOURCE_LIMIT" => 4,
            "RENDER_ERROR" => 4,
            "AGENT_UNAVAILABLE" | "AGENT_ERROR" => 5,
            "CANCELLED" => 130,
            _ => 1,
        }
    }
}
