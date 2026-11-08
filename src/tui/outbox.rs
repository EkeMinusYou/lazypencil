//! Durable queued prompts and delivery tracking for mid-turn input.
use super::*;
use crate::attachments::Attachment;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Prompt {
    pub text: String,
    pub selection: Option<Value>,
    #[serde(default)]
    pub references: Vec<crate::conversation::references::Reference>,
    #[serde(default)]
    pub image_ids: Vec<String>,
    #[serde(default)]
    pub images: Vec<Attachment>,
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
    // None migrates older outboxes from the conversation's pending attachments.
    pub draft_images: Option<Vec<Attachment>>,
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
                        images: vec![],
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
        if prompt.text.trim().is_empty() && prompt.images.is_empty() {
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
        let name = line
            .split_once(char::is_whitespace)
            .map_or(line.as_str(), |(name, _)| name);
        if name != "/queue" {
            return Ok(false);
        }
        let (_, args) = commands::parse(&line)?;
        self.queue_prompt(args[0].clone())?;
        self.set_status("Follow-up queued".into());
        Ok(true)
    }

    pub(super) fn enqueue_prompt(&mut self, text: String) -> Result<()> {
        // The separate queue dialog preserves the prompt's draft attachments.
        self.enqueue_input(text, vec![])
    }

    fn enqueue_input(&mut self, text: String, images: Vec<Attachment>) -> Result<()> {
        if self.input.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for image input before queueing",
            ));
        }
        self.outbox.push(Prompt {
            text,
            selection: None,
            references: self.outbox.references.clone(),
            image_ids: images.iter().map(|image| image.id.clone()).collect(),
            images,
        })?;
        self.outbox.references.clear();
        self.reference_rows.select(None);
        self.draft_changed = true;
        Ok(())
    }

    pub(super) fn queue_prompt(&mut self, text: String) -> Result<()> {
        self.enqueue_input(text, self.pending_images().to_vec())?;
        self.outbox.draft_images = Some(vec![]);
        self.editor = editor("");
        self.save_draft()
    }

    pub(super) fn clear_queued_prompts(&mut self) -> Result<()> {
        let prompts = std::mem::take(&mut self.outbox.queued);
        let paused = self.outbox.paused;
        self.outbox.paused = false;
        if let Err(error) = self.outbox.save(self.conversation.directory()) {
            self.outbox.queued = prompts;
            self.outbox.paused = paused;
            return Err(error);
        }
        let retained = self.image_references();
        for image in prompts.iter().flat_map(|prompt| &prompt.images) {
            image.reclaim_files(self.conversation.directory(), retained.iter())?;
        }
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
            self.append(&format!(
                "\nYou (mid-turn): {}\n\n{:?}:\n",
                input.text,
                self.conversation.settings().agent
            ));
            self.set_status("Mid-turn input delivered".into());
        } else {
            self.outbox.queued.push_front(Prompt {
                text: input.text,
                selection: None,
                references: vec![],
                image_ids: vec![],
                images: vec![],
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
            Ok(mut outbox) => {
                let retained = self.conversation.image_references();
                for prompt in outbox.queued.iter_mut().chain(outbox.in_flight.iter_mut()) {
                    if prompt.images.is_empty() && !prompt.image_ids.is_empty() {
                        for id in &prompt.image_ids {
                            let Some(image) = retained.iter().find(|image| &image.id == id) else {
                                self.error(Error::new(
                                    "ATTACHMENT_INVALID",
                                    "A saved queued image is unavailable",
                                ));
                                return;
                            };
                            prompt.images.push(image.clone());
                        }
                    }
                }
                if outbox.draft_images.is_none()
                    && (outbox.in_flight.is_some()
                        || outbox.queued.iter().any(|prompt| !prompt.images.is_empty()))
                {
                    outbox.draft_images = Some(
                        self.conversation
                            .images()
                            .iter()
                            .filter(|image| {
                                !outbox
                                    .queued
                                    .iter()
                                    .chain(outbox.in_flight.iter())
                                    .any(|prompt| {
                                        prompt.images.iter().any(|captured| captured.id == image.id)
                                    })
                            })
                            .cloned()
                            .collect(),
                    );
                }
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
                    self.finish_prompt(true);
                    self.set_attention_status(
                        "Interrupted prompt restored; review saved logs before resubmitting".into(),
                    );
                    self.draft_changed = true;
                }
            }
            Err(error) => self.error(error),
        }
    }

    pub(super) async fn send_queued(&mut self) -> Result<()> {
        if self.conversation.active()
            || self.conversation.modes().pending().is_some()
            || self.outbox.in_flight.is_some()
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
        let selection = crate::conversation::references::selection(&prompt.references)
            .or(prompt.selection.clone());
        self.conversation
            .set_pending_input(prompt.text.clone(), prompt.images.clone())?;
        self.outbox.queued.pop_front();
        self.outbox.in_flight = Some(prompt.clone());
        self.outbox.draft = Some(self.editor.lines().join("\n"));
        if let Err(error) = self.outbox.save(self.conversation.directory()) {
            self.outbox.in_flight = None;
            self.outbox.queued.push_front(prompt);
            self.outbox.paused = true;
            return Err(error);
        }
        if let Err(error) = self.conversation.start_send(selection).await {
            self.outbox.paused = true;
            self.finish_prompt(true);
            self.draft_changed = true;
            self.save_draft()?;
            self.outbox.save(self.conversation.directory())?;
            return Err(error);
        }
        self.turn_pending = true;
        self.turn_sequence = self.turn_sequence.wrapping_add(1);
        self.turn_started = Some(Instant::now());
        self.outbox.draft = Some(self.editor.lines().join("\n"));
        self.outbox.save(self.conversation.directory())?;
        self.append(&format!(
            "\nYou: {}{}\n[{} images]\n\n{:?}:\n",
            prompt.text,
            references::history(&prompt.references),
            prompt.images.len(),
            self.conversation.settings().agent
        ));
        self.scroll = 0;
        self.clear_status();
        self.dirty = true;
        Ok(())
    }
}
