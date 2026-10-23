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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthEvidence {
    AdapterReport,
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
            status["account"]["plan"]
                .as_str()
                .filter(|plan| {
                    !plan.is_empty() && plan.len() <= 128 && !plan.chars().any(char::is_control)
                })
                .map(str::to_owned)
        } else {
            None
        };
        Self {
            source,
            evidence: AuthEvidence::AdapterReport,
            plan,
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
