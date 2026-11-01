//! Follow-up prompts stay separate from the active turn and survive restarts.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Prompt {
    pub text: String,
    pub selection: Option<Value>,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Outbox {
    pub draft: Option<String>,
    pub queued: VecDeque<Prompt>,
    pub paused: bool,
}

impl Outbox {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("prompt-outbox.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut outbox: Self = serde_json::from_slice(&bytes).map_err(|error| {
                    Error::new("IO_ERROR", format!("Cannot read prompt outbox: {error}"))
                })?;
                // Reopening a conversation must not submit prompts by itself.
                outbox.paused |= !outbox.queued.is_empty();
                Ok(outbox)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Error::io(&path, error)),
        }
    }

    pub fn save(&self, directory: &Path) -> Result<()> {
        let bytes =
            serde_json::to_vec(self).map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        crate::store::atomic_replace(&directory.join("prompt-outbox.json"), &bytes)
    }

    pub fn push(&mut self, prompt: Prompt) -> Result<()> {
        if prompt.text.trim().is_empty() {
            return Err(Error::new("INVALID_ARGUMENT", "Enter a follow-up prompt"));
        }
        if self.queued.len() >= 32
            || self
                .queued
                .iter()
                .map(|prompt| prompt.text.len())
                .sum::<usize>()
                + prompt.text.len()
                > 1024 * 1024
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The prompt queue is full (32 prompts or 1 MiB)",
            ));
        }
        self.queued.push_back(prompt);
        Ok(())
    }
}

impl App {
    pub(super) fn restore_outbox(&mut self) {
        match Outbox::load(self.conversation.directory()) {
            Ok(outbox) => {
                if let Some(draft) = &outbox.draft {
                    if !draft.is_empty() || self.conversation.draft().is_empty() {
                        self.editor = editor(draft);
                    }
                }
                if !outbox.queued.is_empty() {
                    self.status = "Queued prompts restored; /resume-queue sends them".into();
                }
                self.outbox = outbox;
            }
            Err(error) => self.error(error),
        }
    }

    pub(super) async fn send_queued(&mut self) -> Result<()> {
        if self.conversation.active()
            || self.outbox.paused
            || self.has_modal()
            || self.input.is_some()
            || self.document_task.is_some()
        {
            return Ok(());
        }
        let Some(prompt) = self.outbox.queued.front().cloned() else {
            return Ok(());
        };
        // Follow-ups queued during an active request contain text only. Never
        // accidentally resend the previous turn's retained attachments.
        if !self.conversation.images().is_empty() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Remove or send pending attachments before resuming text-only queued prompts",
            ));
        }
        self.conversation.set_draft(prompt.text.clone())?;
        if let Err(error) = self.conversation.start_send(prompt.selection).await {
            self.outbox.paused = true;
            self.outbox.save(self.conversation.directory())?;
            return Err(error);
        }
        self.outbox.queued.pop_front();
        self.turn_pending = true;
        self.turn_sequence = self.turn_sequence.wrapping_add(1);
        self.turn_started = Some(Instant::now());
        self.outbox.draft = Some(self.editor.lines().join("\n"));
        self.outbox.save(self.conversation.directory())?;
        self.append(&format!(
            "\nYou: {}\n[0 images]\n\n{:?}:\n",
            prompt.text,
            self.conversation.settings().agent
        ));
        self.scroll = 0;
        self.status.clear();
        self.dirty = true;
        Ok(())
    }
}
