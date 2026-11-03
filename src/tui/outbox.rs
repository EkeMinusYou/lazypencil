//! Durable queued prompts and delivery tracking for mid-turn input.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Prompt {
    pub text: String,
    pub selection: Option<Value>,
    #[serde(default)]
    pub references: Vec<crate::conversation::references::Reference>,
    #[serde(default)]
    pub image_ids: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SteeringPrompt {
    pub id: String,
    pub text: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Outbox {
    pub draft: Option<String>,
    pub queued: VecDeque<Prompt>,
    pub paused: bool,
    pub references: Vec<crate::conversation::references::Reference>,
    pub in_flight: Option<Prompt>,
    pub steering: Option<SteeringPrompt>,
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
                if let Some(input) = outbox.steering.take() {
                    // A crash may occur after delivery but before its receipt.
                    // Preserve the text for review without automatically replaying it.
                    outbox.queued.push_front(Prompt {
                        text: input.text,
                        selection: None,
                        references: vec![],
                        image_ids: vec![],
                    });
                    outbox.paused = true;
                }
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
                .map(|prompt| {
                    serde_json::to_vec(prompt).map_or(usize::MAX / 64, |bytes| bytes.len())
                })
                .sum::<usize>()
                + serde_json::to_vec(&prompt).map_or(usize::MAX / 64, |bytes| bytes.len())
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
    pub(super) fn queue_command(&mut self) -> Result<bool> {
        let line = self.editor.lines().join("\n");
        let Some(text) = line
            .strip_prefix("/queue")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        else {
            return Ok(false);
        };
        self.queue_prompt(text.trim_start().to_owned())?;
        self.set_status("Follow-up queued".into());
        Ok(true)
    }

    pub(super) fn queue_prompt(&mut self, text: String) -> Result<()> {
        self.outbox.push(Prompt {
            text,
            selection: None,
            references: self.outbox.references.clone(),
            image_ids: vec![],
        })?;
        self.outbox.references.clear();
        self.reference_rows.select(None);
        self.editor = editor("");
        self.outbox.draft = Some(String::new());
        self.outbox.save(self.conversation.directory())?;
        self.draft_changed = false;
        Ok(())
    }

    pub(super) fn steer_prompt(&mut self, text: String) -> Result<()> {
        let id = match self.conversation.steer(text.clone()) {
            Ok(id) => id,
            Err(_) if !self.conversation.can_steer() => {
                self.queue_prompt(text)?;
                self.set_status("The turn has finished accepting input; follow-up queued".into());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.outbox.steering = Some(SteeringPrompt { id, text });
        self.editor = editor("");
        self.outbox.draft = Some(String::new());
        self.outbox.save(self.conversation.directory())?;
        self.draft_changed = false;
        self.set_status("Sending mid-turn input…".into());
        Ok(())
    }

    pub(super) fn steering_result(&mut self, value: &Value) -> Result<()> {
        if !self
            .outbox
            .steering
            .as_ref()
            .is_some_and(|input| value["id"] == input.id)
        {
            return Ok(());
        }
        let input = self.outbox.steering.take().expect("pending steering");
        if value["outcome"] == "injected" {
            self.append(&format!("\nYou (mid-turn): {}\n", input.text));
            self.set_status("Mid-turn input delivered".into());
        } else {
            self.outbox.queued.push_front(Prompt {
                text: input.text,
                selection: None,
                references: vec![],
                image_ids: vec![],
            });
            if value["outcome"] == "promptRequired" {
                self.set_status("The turn could not accept input; queued for the next turn".into());
            } else {
                self.outbox.paused = true;
                self.append("\nMid-turn delivery was not confirmed. Input retained in the paused queue; review saved logs before resuming to avoid repeating it.\n");
                self.set_attention_status(
                    "Mid-turn delivery unconfirmed; queue paused for review".into(),
                );
            }
        }
        self.outbox.draft = Some(self.editor.lines().join("\n"));
        self.outbox.save(self.conversation.directory())
    }

    pub(super) fn restore_outbox(&mut self) {
        match Outbox::load(self.conversation.directory()) {
            Ok(outbox) => {
                if let Some(draft) = &outbox.draft
                    && (outbox.in_flight.is_some()
                        || !draft.is_empty()
                        || self.conversation.draft().is_empty())
                {
                    self.editor = editor(draft);
                }
                if !outbox.queued.is_empty() {
                    self.set_attention_status(
                        "Queued prompts restored; review delivery before /resume-queue".into(),
                    );
                }
                self.outbox = outbox;
                if self.outbox.in_flight.is_some() {
                    self.finish_reference_prompt(true);
                    self.set_attention_status(
                        "Interrupted reference prompt restored; sends only on submit".into(),
                    );
                    self.draft_changed = true;
                }
            }
            Err(error) => self.error(error),
        }
    }

    pub(super) async fn send_queued(&mut self) -> Result<()> {
        if self.conversation.active()
            || self.outbox.steering.is_some()
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
        // New follow-ups never inherit the active turn's images. An explicitly
        // retried failed reference request may retain its exact pending images.
        let image_ids = self
            .conversation
            .images()
            .iter()
            .map(|image| image.id.clone())
            .collect::<Vec<_>>();
        if prompt.image_ids != image_ids {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Review pending images before retrying: they must match the queued request; text-only follow-ups require no images",
            ));
        }
        let selection = crate::conversation::references::selection(&prompt.references)
            .or(prompt.selection.clone());
        self.conversation.set_draft(prompt.text.clone())?;
        if let Err(error) = self.conversation.start_send(selection).await {
            self.outbox.paused = true;
            self.outbox.save(self.conversation.directory())?;
            return Err(error);
        }
        self.outbox.queued.pop_front();
        self.outbox.in_flight = (!prompt.references.is_empty()).then(|| prompt.clone());
        self.turn_pending = true;
        self.turn_sequence = self.turn_sequence.wrapping_add(1);
        self.turn_started = Some(Instant::now());
        self.outbox.draft = Some(self.editor.lines().join("\n"));
        self.outbox.save(self.conversation.directory())?;
        self.append(&format!(
            "\nYou: {}{}\n[{} images]\n\n{:?}:\n",
            prompt.text,
            references::history(&prompt.references),
            prompt.image_ids.len(),
            self.conversation.settings().agent
        ));
        self.scroll = 0;
        self.clear_status();
        self.dirty = true;
        Ok(())
    }
}
