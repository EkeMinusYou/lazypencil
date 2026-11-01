//! Read-only authentication reporting. Credentials remain owned by the adapter.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Subscription,
    ApiKey,
    Gateway,
    External,
    NotAuthenticated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthReport {
    pub source: AuthSource,
    pub evidence: AuthEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Display-only identity; never serialize it into events or saved state.
    #[serde(skip)]
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthEvidence {
    AdapterReport,
}

fn display_text(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .filter(|value| {
            !value.trim().is_empty()
                && value.len() <= limit
                && !value.chars().any(|character| {
                    character.is_control()
                        || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
        })
        .map(|value| value.trim().to_owned())
}

impl AuthReport {
    pub fn from_notification(params: &Value) -> Self {
        let status = &params["authStatus"];
        let source = match status["kind"].as_str() {
            Some("account") => AuthSource::Subscription,
            Some("api_key") => AuthSource::ApiKey,
            Some("gateway") => AuthSource::Gateway,
            Some("external") => AuthSource::External,
            Some("none") => AuthSource::NotAuthenticated,
            _ => AuthSource::Unknown,
        };
        let plan = if source == AuthSource::Subscription {
            display_text(&status["account"]["plan"], 128)
        } else {
            None
        };
        Self {
            source,
            evidence: AuthEvidence::AdapterReport,
            plan,
            identity: (source == AuthSource::Subscription)
                .then(|| {
                    ["email", "name", "id"]
                        .into_iter()
                        .find_map(|key| display_text(&status["account"][key], 320))
                })
                .flatten(),
        }
    }

    pub fn display(&self) -> String {
        let source = match self.source {
            AuthSource::Subscription => "subscription",
            AuthSource::ApiKey => "API key",
            AuthSource::Gateway => "gateway",
            AuthSource::External => "external provider",
            AuthSource::NotAuthenticated => "not signed in",
            AuthSource::Unknown => "not reported",
        };
        let mut display = source.to_owned();
        if let Some(identity) = &self.identity {
            display = format!("{identity} · {source}");
        } else if self.source == AuthSource::Subscription {
            display.push_str(" · account not reported");
        }
        if let Some(plan) = &self.plan {
            display.push_str(&format!(" · {plan}"));
        }
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_identity_is_bounded_and_never_serialized() {
        let report = AuthReport::from_notification(
            &json!({"authStatus":{"kind":"account","account":{"email":"person@example.test","plan":"pro"},"accessToken":"private-token"}}),
        );
        assert!(report.display().contains("person@example.test"));
        let serialized = serde_json::to_string(&report).unwrap();
        assert!(
            !serialized.contains("person@example.test") && !serialized.contains("private-token")
        );
        let saved: AuthReport = serde_json::from_str(&serialized).unwrap();
        assert!(saved.identity.is_none());
        for email in [
            "\x1b]52;injection".into(),
            "person\u{202e}@example.test".into(),
            "x".repeat(321),
        ] {
            let report = AuthReport::from_notification(
                &json!({"authStatus":{"kind":"account","account":{"email":email}}}),
            );
            assert!(report.identity.is_none());
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuthPolicy {
    pub subscription: bool,
    pub require_report: bool,
}
impl AuthPolicy {
    pub fn check(self, report: Option<&AuthReport>) -> Result<()> {
        let source = report
            .map(|report| report.source)
            .unwrap_or(AuthSource::Unknown);
        let message = match source {
            AuthSource::Unknown if self.require_report => Some(
                "The adapter did not report a usable authentication route; unverified requests are blocked",
            ),
            AuthSource::NotAuthenticated => Some(
                "The runtime reports no authenticated account; sign in using the runtime's own login command",
            ),
            AuthSource::Subscription if !self.subscription => {
                Some("API access was requested but the runtime reports subscription authentication")
            }
            AuthSource::ApiKey | AuthSource::Gateway | AuthSource::External
                if self.subscription =>
            {
                Some(
                    "Subscription access was requested but the runtime reports an API key, gateway, or external provider",
                )
            }
            _ => None,
        };
        match message {
            Some(message) => Err(Error::new("AGENT_ERROR", message).details(json!({"reason":"auth_route_mismatch","requested_auth":if self.subscription {"subscription"} else {"api"},"authentication":report}))),
            None => Ok(()),
        }
    }
}

pub(crate) async fn wait_for_conflict(
    receiver: &mut watch::Receiver<Option<AuthReport>>,
    policy: AuthPolicy,
) -> Error {
    loop {
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        if let Err(error) = policy.check(receiver.borrow().as_ref()) {
            return error;
        }
    }
}
