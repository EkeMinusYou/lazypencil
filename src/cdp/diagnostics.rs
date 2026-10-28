//! Keep remote page content out of public acquisition failures.

use super::{Purpose, failed};
use crate::Error;
use serde_json::{Value, json};

pub(super) fn protocol_error(method: &str, error: &Value) -> Error {
    // Protocol messages and data may repeat request parameters or page content.
    // The command name and numeric protocol code identify the failing operation.
    failed(format!("CDP {method} failed")).details(json!({
        "stage":"browser_protocol", "cdp_method":method,
        "cdp_code":error.get("code").and_then(Value::as_i64)
    }))
}

pub(super) fn evaluation_error(purpose: Purpose, exception: &Value) -> Error {
    if purpose == Purpose::Page {
        // Even a first-line exception can contain cookies, query parameters or
        // local storage. A remote page can replace APIs used by our readiness code.
        return Error::new("PAGE_LOAD_FAILED", "Page JavaScript evaluation failed")
            .details(json!({"stage":"page_evaluation"}));
    }
    // Shader evaluation runs our own harness around authored local GLSL.
    // Preserve its bounded compiler diagnostics for the editing workflow.
    let description = exception["exception"]["description"]
        .as_str()
        .or_else(|| exception["text"].as_str())
        .unwrap_or("Page evaluation failed");
    Error::new(
        "PAGE_LOAD_FAILED",
        description
            .lines()
            .next()
            .unwrap_or("Page evaluation failed")
            .chars()
            .take(1024)
            .collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_and_protocol_failures_omit_remote_content_but_keep_shader_diagnostics() {
        let secret = "https://user:password@example.invalid/?token=private#session\u{1b}]52;c;data";
        for exception in [
            json!({"exception":{"description":secret},"text":secret,"url":secret}),
            json!({"text":secret}),
            json!({}),
        ] {
            let error = evaluation_error(Purpose::Page, &exception);
            assert_eq!(error.code, "PAGE_LOAD_FAILED");
            assert_eq!(error.message, "Page JavaScript evaluation failed");
            assert_eq!(error.details, Some(json!({"stage":"page_evaluation"})));
            assert!(!serde_json::to_string(&error).unwrap().contains(secret));
        }
        let error = protocol_error(
            "Page.navigate",
            &json!({"code":-32602,"message":secret,"data":secret}),
        );
        assert_eq!(error.code, "RUNTIME_UNAVAILABLE");
        assert_eq!(error.message, "CDP Page.navigate failed");
        assert_eq!(
            error.details,
            Some(
                json!({"stage":"browser_protocol","cdp_method":"Page.navigate","cdp_code":-32602})
            )
        );
        assert!(!serde_json::to_string(&error).unwrap().contains("password"));
        assert!(
            protocol_error("Page.navigate", &json!({"code":secret}))
                .details
                .unwrap()["cdp_code"]
                .is_null()
        );
        let shader = evaluation_error(
            Purpose::Shader,
            &json!({"exception":{"description":"GLSL compile failed at line 7\nprivate stack"}}),
        );
        assert_eq!(shader.message, "GLSL compile failed at line 7");
        let long = evaluation_error(Purpose::Shader, &json!({"text":"誤".repeat(2048)}));
        assert_eq!(long.message.chars().count(), 1024);
    }
}
