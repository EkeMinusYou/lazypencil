//! A bounded, cancellable startup check without a provider session or prompt.
use super::*;
use crate::agent::{AgentHost, auth::AuthReport};

pub(super) struct Check {
    pub cancel: CancellationToken,
    task: tokio::task::JoinHandle<Result<Option<AuthReport>>>,
}

impl Check {
    pub fn start(conversation: &Conversation) -> Result<Self> {
        let options = conversation.authentication_options()?;
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        Ok(Self {
            cancel,
            task: tokio::spawn(AgentHost::inspect_authentication(options, token)),
        })
    }

    pub async fn result(&mut self) -> Result<Option<AuthReport>> {
        (&mut self.task)
            .await
            .map_err(|_| Error::new("AGENT_ERROR", "Account check stopped unexpectedly"))?
    }

    pub async fn close(mut self) {
        self.cancel.cancel();
        let _ = self.result().await;
    }
}

impl Drop for Check {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
