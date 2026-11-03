//! Local timing metadata without tool arguments, output bodies or credentials.

use crate::guidance::usage::UsageRequest;
use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) struct Timer {
    received: Instant,
    received_at_unix_ms: u64,
    call_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Measurements {
    call_id: String,
    received_at_unix_ms: u64,
    finished_at_unix_ms: u64,
    queue_us: u64,
    execution_us: u64,
    server_us: u64,
    is_error: bool,
}

impl Measurements {
    pub(crate) fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.call_id).is_ok()
            && self.queue_us.checked_add(self.execution_us) == Some(self.server_us)
    }
}

impl Timer {
    pub(super) fn new() -> Self {
        Self {
            received: Instant::now(),
            received_at_unix_ms: unix_ms(),
            call_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub(super) fn finish(
        &self,
        tool: &str,
        started: Option<Instant>,
        mut result: CallToolResult,
        usage: crate::Result<Option<UsageRequest>>,
    ) -> CallToolResult {
        let server_us = self.received.elapsed().as_micros() as u64;
        let queue_us = started
            .map(|started| started.duration_since(self.received).as_micros() as u64)
            .unwrap_or(server_us);
        let timing = Measurements {
            call_id: self.call_id.clone(),
            received_at_unix_ms: self.received_at_unix_ms,
            finished_at_unix_ms: unix_ms(),
            queue_us,
            execution_us: server_us - queue_us,
            server_us,
            is_error: result.is_error.unwrap_or(false),
        };
        // Keep completed edits successful if optional performance logging fails.
        let recorded = usage
            .and_then(|usage| usage.map_or(Ok(()), |usage| usage.record_timing(tool, &timing)));
        let meta = result.meta.get_or_insert_with(Default::default);
        meta.insert("lazypencil/timing".into(), json!(timing));
        if let Err(error) = recorded {
            meta.insert("lazypencil/timing_log_error".into(), json!(error.code));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_log_failure_preserves_the_tool_result_and_omits_error_messages() {
        let original = json!({"revision":"saved"});
        let result = Timer::new().finish(
            "apply_operations",
            Some(Instant::now()),
            CallToolResult::structured(original.clone()),
            Err(crate::Error::new("IO_ERROR", "PRIVATE_LOG_ERROR")),
        );
        assert_eq!(result.structured_content, Some(original));
        assert!(!result.is_error.unwrap_or(false));
        let meta = result.meta.unwrap();
        assert_eq!(meta["lazypencil/timing_log_error"], "IO_ERROR");
        assert!(
            !serde_json::to_string(&meta)
                .unwrap()
                .contains("PRIVATE_LOG_ERROR")
        );
    }
}
