//! Prepare immutable send images off the event loop and adopt them atomically.
use super::*;
use crate::{
    attachments::{Attachment, AttachmentOptions},
    image_input::ImageBounds,
};

pub(super) fn input_bounds(
    value: Option<&str>,
    default: Option<ImageBounds>,
) -> Result<Option<ImageBounds>> {
    match value {
        None => Ok(default),
        Some("original") => Ok(None),
        Some(value) => value.parse().map(Some),
    }
}

pub(super) enum PreparedInput {
    Text(String),
    Images(Vec<Attachment>),
    Resized {
        index: usize,
        expected_id: String,
        image: Attachment,
    },
}

impl PreparedInput {
    fn discard(&self, directory: &Path, retained: &[Attachment]) {
        let images = match self {
            Self::Text(_) => return,
            Self::Images(images) => images.as_slice(),
            Self::Resized { image, .. } => std::slice::from_ref(image),
        };
        for image in images {
            // Keep shared and previously retained blobs even on cancellation.
            let _ = image.reclaim_files(directory, retained.iter());
        }
    }
}

pub(super) struct InputJob {
    pub(super) cancel: CancellationToken,
    pub(super) receiver: tokio::sync::oneshot::Receiver<Result<PreparedInput>>,
    thread: Option<std::thread::JoinHandle<()>>,
    directory: PathBuf,
    retained: Vec<Attachment>,
}
#[cfg(test)]
impl InputJob {
    pub(super) fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }
}
impl Drop for InputJob {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Ok(Ok(content)) = self.receiver.try_recv() {
            content.discard(&self.directory, &self.retained);
        }
    }
}

fn prepare(
    content: InputContent,
    directory: &Path,
    options: AttachmentOptions,
    pending: &[Attachment],
    retained: &[Attachment],
    cancel: &CancellationToken,
) -> Result<PreparedInput> {
    crate::cancel::check(Some(cancel))?;
    match content {
        InputContent::Text { text } => Ok(PreparedInput::Text(text)),
        InputContent::Images { images } => {
            options
                .limits
                .check_count(pending.len().saturating_add(images.len()))?;
            let mut prepared = Vec::new();
            let result =
                (|| {
                    for image in images {
                        crate::cancel::check(Some(cancel))?;
                        let limits = options.limits.remaining(pending.iter().chain(&prepared))?;
                        prepared.push(image.store_with_options(
                            directory,
                            AttachmentOptions { limits, ..options },
                        )?);
                    }
                    crate::cancel::check(Some(cancel))
                })();
            match result {
                Ok(()) => Ok(PreparedInput::Images(prepared)),
                Err(error) => {
                    PreparedInput::Images(prepared).discard(directory, retained);
                    Err(error)
                }
            }
        }
    }
}

impl App {
    pub(super) fn attach_startup_images(&mut self, paths: &[PathBuf]) -> Result<()> {
        let limits = self.conversation.attachment_limits()?;
        limits.check_count(self.pending_images().len().saturating_add(paths.len()))?;
        for path in paths {
            let options = AttachmentOptions {
                bounds: self.conversation.settings().image_bounds,
                limits,
            };
            let cancel = CancellationToken::new();
            let retained = self.image_references();
            let content = clipboard::read_image_with_options(
                self.conversation.executable(),
                path,
                options,
                &cancel,
            )?;
            let prepared = prepare(
                content,
                self.conversation.directory(),
                options,
                self.pending_images(),
                &retained,
                &cancel,
            )?;
            let PreparedInput::Images(images) = &prepared else {
                return Err(Error::new(
                    "ATTACHMENT_INVALID",
                    "Image file input did not return images",
                ));
            };
            let pending = self
                .pending_images()
                .iter()
                .cloned()
                .chain(images.iter().cloned())
                .collect();
            if let Err(error) = self.set_pending_images(pending) {
                if error.code != "IO_ERROR" {
                    prepared.discard(self.conversation.directory(), &retained);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn pending_images(&self) -> &[Attachment] {
        self.outbox
            .draft_images
            .as_deref()
            .unwrap_or_else(|| self.conversation.images())
    }

    pub(super) fn image_references(&self) -> Vec<Attachment> {
        self.conversation
            .image_references()
            .into_iter()
            .chain(self.pending_images().iter().cloned())
            .chain(
                self.outbox
                    .queued
                    .iter()
                    .chain(self.outbox.in_flight.iter())
                    .flat_map(|prompt| prompt.images.iter().cloned()),
            )
            .collect()
    }

    fn set_pending_images(&mut self, images: Vec<Attachment>) -> Result<()> {
        let previous = self.outbox.draft_images.replace(images);
        self.draft_changed = true;
        if let Err(error) = self.save_draft() {
            self.outbox.draft_images = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn remove_pending_image(&mut self, index: usize) -> Result<()> {
        let mut images = self.pending_images().to_vec();
        if index >= images.len() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "No attachment at this index",
            ));
        }
        let image = images.remove(index);
        self.set_pending_images(images)?;
        image.reclaim_files(
            self.conversation.directory(),
            self.image_references().iter(),
        )?;
        Ok(())
    }

    pub(super) fn clear_pending_images(&mut self) -> Result<()> {
        let images = self.pending_images().to_vec();
        self.set_pending_images(vec![])?;
        let retained = self.image_references();
        for image in images {
            image.reclaim_files(self.conversation.directory(), retained.iter())?;
        }
        Ok(())
    }

    fn replace_pending_image(
        &mut self,
        index: usize,
        expected_id: &str,
        image: Attachment,
    ) -> Result<()> {
        let mut images = self.pending_images().to_vec();
        let previous = images
            .get(index)
            .filter(|image| image.id == expected_id)
            .ok_or_else(|| {
                Error::new(
                    "REVISION_CONFLICT",
                    "The pending image changed during conversion",
                )
            })?;
        if previous.original.is_none()
            || previous.original != image.original
            || previous.source_bytes != image.source_bytes
        {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "A resize must retain the same captured original",
            ));
        }
        image.read(self.conversation.directory())?;
        let limits = self.conversation.attachment_limits()?;
        limits.check_request([&image])?;
        if image.bytes > previous.bytes {
            limits.check_request(
                images
                    .iter()
                    .enumerate()
                    .map(|(position, previous)| if position == index { &image } else { previous }),
            )?;
        }
        let previous = std::mem::replace(&mut images[index], image);
        self.set_pending_images(images)?;
        previous.reclaim_files(
            self.conversation.directory(),
            self.image_references().iter(),
        )?;
        Ok(())
    }

    fn start_image_job(
        &mut self,
        status: &str,
        work: impl FnOnce(&Path, &[Attachment], &CancellationToken) -> Result<PreparedInput>
        + Send
        + 'static,
    ) -> Result<()> {
        if self.input.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Finish or cancel the active image operation first",
            ));
        }
        let directory = self.conversation.directory().to_owned();
        let retained = self.image_references();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let owned = directory.clone();
        let references = retained.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("lazypencil-input".into())
            .spawn(move || {
                let result = work(&owned, &references, &token).and_then(|content| {
                    if token.is_cancelled() {
                        content.discard(&owned, &references);
                        Err(Error::new("CANCELLED", "Image input cancelled"))
                    } else {
                        Ok(content)
                    }
                });
                if let Err(Ok(content)) = sender.send(result) {
                    content.discard(&owned, &references);
                }
            })
            .map_err(terminal::io)?;
        self.input = Some(InputJob {
            cancel,
            receiver,
            thread: Some(thread),
            directory,
            retained,
        });
        self.set_attention_status(status.into());
        Ok(())
    }

    pub(super) fn start_input(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.start_input_with_bounds(path, self.conversation.settings().image_bounds)
    }

    pub(super) fn start_input_with_bounds(
        &mut self,
        path: Option<PathBuf>,
        bounds: Option<ImageBounds>,
    ) -> Result<()> {
        let executable = self.conversation.executable().to_owned();
        let reader = self.input_reader;
        let options = AttachmentOptions {
            bounds,
            limits: self.conversation.attachment_limits()?,
        };
        let pending = self.pending_images().to_vec();
        self.start_image_job(
            "Reading image input…",
            move |directory, retained, token| {
                prepare(
                    reader(&executable, path.as_deref(), options, token)?,
                    directory,
                    options,
                    &pending,
                    retained,
                    token,
                )
            },
        )
    }

    pub(super) fn start_resize(&mut self, index: usize, bounds: Option<ImageBounds>) -> Result<()> {
        let image = self
            .pending_images()
            .get(index)
            .cloned()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "No pending image at this index"))?;
        let original = image.original.as_ref().ok_or_else(|| {
            Error::new(
                "ATTACHMENT_ORIGINAL_UNAVAILABLE",
                "Reattach the source file to resize this older image",
            )
        })?;
        let bounds = bounds.unwrap_or(ImageBounds::new(original.width, original.height)?);
        let limits = self.conversation.attachment_limits()?;
        self.start_image_job(
            "Preparing send image…",
            move |directory, _retained, token| {
                crate::cancel::check(Some(token))?;
                let resized = image.resized_with_limits(directory, bounds, limits)?;
                Ok(PreparedInput::Resized {
                    index,
                    expected_id: image.id,
                    image: resized,
                })
            },
        )
    }

    pub(super) fn accept_prepared(&mut self, content: Result<PreparedInput>) -> bool {
        let content = match content {
            Ok(content) => content,
            Err(error) => {
                self.error(error);
                return false;
            }
        };
        let result = match &content {
            PreparedInput::Text(text) => {
                if self.insert_text(text) {
                    self.set_status("Pasted text".into());
                }
                return false;
            }
            PreparedInput::Images(images) => {
                let mut pending = self.pending_images().to_vec();
                pending.extend(images.iter().cloned());
                self.conversation
                    .attachment_limits()
                    .and_then(|limits| limits.check_request(&pending))
                    .and_then(|()| self.set_pending_images(pending))
            }
            PreparedInput::Resized {
                index,
                expected_id,
                image,
            } => self.replace_pending_image(*index, expected_id, image.clone()),
        };
        if let Err(error) = result {
            // An IO error can follow an atomic rename. Preserve candidate files
            // rather than delete data that a durable session may already reference.
            if error.code != "IO_ERROR" {
                content.discard(self.conversation.directory(), &self.image_references());
            }
            self.error(error);
            return false;
        }
        self.set_status(match content {
            PreparedInput::Resized { image, .. } => format!(
                "Send image: {}×{} · original retained",
                image.width, image.height
            ),
            _ => {
                let message = format!("{} images attached", self.pending_images().len());
                match self.navigation.preferred_label(Nav::PreviousSection, true) {
                    Some(key) => format!("{message} · {key} to review or remove"),
                    None => message,
                }
            }
        });
        true
    }

    #[cfg(test)]
    pub(super) fn accept_input(&mut self, content: Result<InputContent>) {
        let prepared = content.and_then(|content| {
            prepare(
                content,
                self.conversation.directory(),
                AttachmentOptions {
                    bounds: self.conversation.settings().image_bounds,
                    limits: self.conversation.attachment_limits()?,
                },
                self.pending_images(),
                &self.image_references(),
                &CancellationToken::new(),
            )
        });
        self.accept_prepared(prepared);
    }
}

pub(super) fn dimensions(image: &Attachment) -> String {
    image
        .original
        .as_ref()
        .filter(|original| (original.width, original.height) != (image.width, image.height))
        .map(|original| {
            format!(
                "{}×{} → {}×{}",
                original.width, original.height, image.width, image.height
            )
        })
        .unwrap_or_else(|| format!("{}×{}", image.width, image.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn image_preparation_accounts_for_pending_images_but_keeps_text_paste_available() {
        let temp = tempfile::TempDir::new().unwrap();
        let original = Attachment::from_bytes(
            temp.path(),
            include_bytes!("../../examples/assets/crop-source.png"),
            "reference.png",
        )
        .unwrap();
        let pending = vec![original.clone()];
        let images = || InputContent::Images {
            images: vec![crate::clipboard::InputImage {
                name: "new.png".into(),
                source_bytes: 1,
                data: "invalid".into(),
            }],
        };
        let options = AttachmentOptions {
            limits: crate::config::attachments::AttachmentSettings {
                max_images: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let error = prepare(images(), temp.path(), options, &pending, &pending, &cancel)
            .err()
            .unwrap();
        assert_eq!(error.details.unwrap()["limit"], "max_images");
        let options = AttachmentOptions {
            limits: crate::config::attachments::AttachmentSettings {
                max_total_send_bytes: original.bytes,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            matches!(prepare(InputContent::Text { text: "日本語を貼り付け".into() }, temp.path(), options, &pending, &pending, &cancel).unwrap(), PreparedInput::Text(text) if text == "日本語を貼り付け")
        );
        assert_eq!(
            prepare(images(), temp.path(), options, &pending, &pending, &cancel)
                .err()
                .unwrap()
                .code,
            "ATTACHMENT_INVALID"
        );
        assert!(original.read_original(temp.path()).is_ok());

        // One candidate can fit, but the second must fail without retaining the
        // first candidate's newly persisted image or touching the existing one.
        let png = crate::image_input::from_rgba8(1, 1, &[255, 0, 0, 255])
            .unwrap()
            .png;
        // Source encoding and normalized send encoding can have different
        // lengths even for an unchanged one-pixel image.
        let send_bytes = crate::image_input::normalize(&png, None).unwrap().png.len();
        let options = AttachmentOptions {
            limits: crate::config::attachments::AttachmentSettings {
                max_total_send_bytes: send_bytes * 2 - 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let names = || {
            std::fs::read_dir(temp.path().join("attachments"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = names();
        let content = InputContent::Images {
            images: (0..2)
                .map(|_| crate::clipboard::InputImage {
                    name: "new.png".into(),
                    source_bytes: png.len(),
                    data: base64::engine::general_purpose::STANDARD.encode(&png),
                })
                .collect(),
        };
        assert!(prepare(content, temp.path(), options, &[], &pending, &cancel).is_err());
        assert_eq!(names(), before);
        assert_eq!(
            input_bounds(Some("original"), Some(ImageBounds::new(2, 2).unwrap())).unwrap(),
            None
        );
        assert_eq!(
            input_bounds(Some("10x20"), None).unwrap(),
            Some(ImageBounds::new(10, 20).unwrap())
        );
    }
}
