//! Preauthorization restricted to the bound document's registered MCP tools.

use agent_client_protocol::schema::v1::{PermissionOptionKind, RequestPermissionRequest};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct DocumentPermissions {
    session: Option<String>,
    tools: HashSet<String>,
    pending: HashMap<String, String>,
}
impl Default for DocumentPermissions {
    fn default() -> Self {
        Self::new()
    }
}
impl DocumentPermissions {
    pub fn new() -> Self {
        Self {
            session: None,
            tools: crate::mcp::tool_names().into_iter().collect(),
            pending: HashMap::new(),
        }
    }
    pub fn set_session(&mut self, session: &str) {
        self.session = Some(session.into());
        self.pending.clear();
    }

    pub fn observe(&mut self, notification: &Value) {
        if self.session.as_deref() != notification["sessionId"].as_str() {
            return;
        }
        let update = &notification["update"];
        let Some(id) = update["toolCallId"].as_str() else {
            return;
        };
        match update["sessionUpdate"].as_str() {
            Some("tool_call") => {
                self.pending.remove(id);
            }
            Some("tool_call_update") => {}
            _ => return,
        }
        if matches!(
            update["status"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            self.pending.remove(id);
            return;
        }
        // Codex's permission request may carry only an ID. Correlate it with the
        // adapter's explicit MCP marker and structured server/tool facts, never a title.
        if update["_meta"]["is_mcp_tool_call"] == true
            && update["rawInput"]["server"] == "lazypencil"
            && let Some(tool) = update["rawInput"]["tool"]
                .as_str()
                .filter(|tool| self.tools.contains(*tool))
            && self.pending.len() < 1024
        {
            self.pending.insert(id.into(), tool.into());
        } else if update.get("name").is_some() || update.get("rawInput").is_some() {
            self.pending.remove(id);
        }
    }

    pub fn allow_once(&self, request: &RequestPermissionRequest) -> Option<String> {
        if self.session.as_deref() != Some(request.session_id.0.as_ref()) {
            return None;
        }
        let named = request
            .tool_call
            .fields
            .name
            .as_deref()
            .and_then(|name| name.strip_prefix("mcp__lazypencil__"))
            .is_some_and(|name| self.tools.contains(name));
        let correlated = request
            .meta
            .as_ref()
            .and_then(|meta| meta.get("is_mcp_tool_approval"))
            .is_some_and(|value| value == true)
            && self
                .pending
                .contains_key(request.tool_call.tool_call_id.0.as_ref());
        if !named && !correlated {
            return None;
        }
        request
            .options
            .iter()
            .find(|option| matches!(option.kind, PermissionOptionKind::AllowOnce))
            .map(|option| option.option_id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(session: &str, name: Option<&str>, approval: bool) -> RequestPermissionRequest {
        serde_json::from_value(json!({"sessionId":session,"toolCall":{"toolCallId":"call-1","name":name,"title":"mcp__lazypencil__apply_operations"},
            "_meta":{"is_mcp_tool_approval":approval},"options":[{"optionId":"once","name":"Allow once","kind":"allow_once"}]})).unwrap()
    }

    #[test]
    fn names_are_bound_to_the_current_session_and_registry() {
        let mut policy = DocumentPermissions::new();
        policy.set_session("bound");
        assert_eq!(
            policy.allow_once(&request(
                "bound",
                Some("mcp__lazypencil__apply_operations"),
                false
            )),
            Some("once".into())
        );
        for request in [
            request("other", Some("mcp__lazypencil__apply_operations"), false),
            request("bound", Some("Bash"), false),
            request("bound", Some("mcp__lazypencil__not_registered"), false),
        ] {
            assert_eq!(policy.allow_once(&request), None);
        }
    }

    #[test]
    fn correlated_approvals_require_explicit_mcp_facts_and_expire_at_completion() {
        let mut policy = DocumentPermissions::new();
        policy.set_session("bound");
        let request = request("bound", None, true);
        let mut notification = json!({"sessionId":"bound","update":{"sessionUpdate":"tool_call","toolCallId":"call-1","status":"pending",
            "_meta":{"is_mcp_tool_call":true},"rawInput":{"server":"unrelated","tool":"apply_operations","arguments":{}}}});
        policy.observe(&notification);
        assert_eq!(policy.allow_once(&request), None);
        notification["update"]["rawInput"]["server"] = json!("lazypencil");
        policy.observe(&notification);
        assert_eq!(policy.allow_once(&request), Some("once".into()));
        policy.observe(&json!({"sessionId":"bound","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed"}}));
        assert_eq!(policy.allow_once(&request), None);
    }
}
