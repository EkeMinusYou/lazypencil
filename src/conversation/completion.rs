//! Complete a turn only after its transcript is durable; preserve drafts on failure.
use super::{Conversation, ConversationEvent, storage::Transcript};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::sync::oneshot;

pub(super) struct Completion {
    value: Value,
    result: Result<()>,
    document: Value,
    path: Option<PathBuf>,
    durability: Option<oneshot::Receiver<Result<()>>>,
}

impl Completion {
    fn transcript_failed(&mut self, error: Error) {
        let error = Error::new("AGENT_ERROR", "The turn transcript could not be completed")
            .details(json!({"transcript_error":error,"turn_error":self.result.as_ref().err()}));
        self.value["type"] = json!("failed");
        self.value["error"] = json!(error);
        self.result = Err(error);
    }
}

impl Conversation {
    pub(super) async fn settle_completion(&mut self) -> Result<()> {
        let Some(completion) = self.completion.as_mut() else {
            return Ok(());
        };
        // Keep the receiver owned across cancellation of a frontend select future.
        let durable = if let Some(receiver) = completion.durability.as_mut() {
            receiver.await.unwrap_or_else(|_| {
                Err(Error::new(
                    "IO_ERROR",
                    "Transcript durability worker stopped",
                ))
            })
        } else {
            Ok(())
        };
        let completion = self.completion.take().expect("pending completion");
        self.complete_turn(completion, durable)
    }
    pub(super) fn finish(&mut self, mut result: Result<()>) -> Result<()> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        if let Some(input) = self.pending_steering.take()
            && let Err(error) =
                self.message(json!({"type":"steering_result","id":input.id,"outcome":"unknown"}))
        {
            result = Err(error);
        }
        self.pending_permissions.clear();
        let usage = self
            .usage
            .as_mut()
            .map(|usage| usage.finish_turn())
            .transpose();
        let usage_result = usage.and_then(|records| {
            for record in records.into_iter().flatten() {
                self.message(record)?;
            }
            Ok(())
        });
        if let Err(error) = usage_result {
            result = Err(Error::new(
                "AGENT_ERROR",
                "The turn's design-resource audit could not be completed",
            )
            .details(json!({"audit_error":error,"turn_error":result.err()})));
        }
        if self.host.is_none() {
            self.usage.take();
        }
        let document = self.store.read();
        if result.is_ok()
            && let Err(error) = &document
        {
            result = Err(error.clone());
        }
        let actual = document
            .map(|document| json!({"revision":document.revision()}))
            .unwrap_or_else(|error| json!({"error":error}));
        let path = self
            .transcript
            .as_ref()
            .map(|transcript| transcript.path.clone());
        let value = match &result {
            Ok(()) => {
                json!({"type":"complete","session":self.id(),"turn":active.id,"native_session_id":self.session.state.native_session_id,"authentication":self.session.state.authentication,"document":actual,"before_revision":active.before,"transcript":path})
            }
            Err(error) => {
                json!({"type":"failed","session":self.id(),"turn":active.id,"native_session_id":self.session.state.native_session_id,"authentication":self.session.state.authentication,"error":error,"document":actual,"before_revision":active.before,"transcript":path})
            }
        };
        let mut completion = Completion {
            value,
            result,
            document: actual,
            path,
            durability: None,
        };
        match self.record(&completion.value) {
            Ok(value) => completion.value = value,
            Err(error) => completion.transcript_failed(error),
        }
        match self
            .transcript
            .as_ref()
            .map(Transcript::barrier)
            .transpose()
        {
            Ok(receiver) => completion.durability = receiver,
            Err(error) => completion.transcript_failed(error),
        }
        self.completion = Some(completion);
        Ok(())
    }
    fn complete_turn(&mut self, mut completion: Completion, durable: Result<()>) -> Result<()> {
        if let Err(error) = durable {
            completion.transcript_failed(error);
        }
        let consumed_draft = completion.result.is_ok().then(|| {
            (
                self.session.state.draft.clone(),
                self.session.state.pending_attachments.clone(),
                self.session.state.continuity_pending,
            )
        });
        if completion.result.is_ok() {
            self.session.state.draft.clear();
            self.session.state.pending_attachments.clear();
            self.session.state.continuity_pending = false;
        }
        if let Err(error) = self.session.save() {
            completion.value["type"] = json!("failed");
            completion.value["error"] = json!(error);
            completion.result = Err(error);
        }
        if completion.result.is_err()
            && let Some((draft, attachments, continuity_pending)) = consumed_draft
        {
            self.session.state.draft = draft;
            self.session.state.pending_attachments = attachments;
            self.session.state.continuity_pending = continuity_pending;
            if let Err(error) = self.session.save() {
                let error = Error::new(
                    "AGENT_ERROR",
                    "The failed turn's draft could not be restored",
                )
                .details(json!({"draft_restore_error":error,"turn_error":completion.result.err()}));
                completion.value["error"] = json!(error);
                completion.result = Err(error);
            }
            // A state-save failure supersedes the already durable model completion.
            let _ = self.record(&completion.value);
        }
        self.events
            .push_back(ConversationEvent::Message(completion.value));
        self.result = Some(completion.result.map_err(|error| {
            let cause = error.details.clone();
            error.details(
                json!({"session":self.id(),"document":completion.document,"transcript":completion.path,"cause":cause}),
            )
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ActiveTurn, storage::Transcript};
    use clap::Parser;
    use tokio_util::sync::CancellationToken;

    fn conversation(temp: &tempfile::TempDir) -> Conversation {
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
            "keep this draft".into(),
        ])
        .unwrap();
        let crate::cli::Command::Prompt(args) = arguments.command else {
            unreachable!()
        };
        let mut conversation = Conversation::open(args, Some(temp.path().join("data"))).unwrap();
        conversation.active = Some(ActiveTurn {
            id: uuid::Uuid::new_v4().to_string(),
            before: conversation.store.read().unwrap().revision(),
            content: None,
            cancel: CancellationToken::new(),
            denied: false,
        });
        conversation
    }

    #[tokio::test]
    async fn cancelled_completion_poll_keeps_the_barrier_and_draft_until_durable_success() {
        for failure in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut conversation = conversation(&temp);
            conversation.finish(Ok(())).unwrap();
            let (reply, receiver) = oneshot::channel();
            conversation.completion.as_mut().unwrap().durability = Some(receiver);
            tokio::select! {
                _ = conversation.settle_completion() => panic!("Undurable completion was published"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {},
            }
            assert!(conversation.active());
            assert!(conversation.result().is_none());
            assert_eq!(conversation.draft(), "keep this draft");
            reply
                .send(if failure {
                    Err(Error::new("IO_ERROR", "sync failed"))
                } else {
                    Ok(())
                })
                .unwrap();
            conversation.settle_completion().await.unwrap();
            assert!(!conversation.active());
            assert_eq!(conversation.result().unwrap().is_ok(), !failure);
            assert_eq!(
                conversation.draft(),
                if failure { "keep this draft" } else { "" }
            );
            assert_eq!(conversation.events.len(), 1);
        }
    }

    #[tokio::test]
    async fn permission_approval_is_not_delivered_when_its_transcript_cannot_be_written() {
        let temp = tempfile::tempdir().unwrap();
        let mut conversation = conversation(&temp);
        conversation.transcript = Some(Transcript::read_only(conversation.directory()));
        conversation
            .record(&json!({"type":"permission_response","option_id":"allow-once"}))
            .unwrap();
        let (reply, receiver) = oneshot::channel();
        let _ = conversation.deliver_permission(reply, Some("allow-once".into()));
        let delivered = tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
            .await
            .unwrap();
        assert!(
            !matches!(delivered, Ok(Some(_))),
            "An undurable approval must never reach the adapter"
        );
    }
}
