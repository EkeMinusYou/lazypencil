//! Prepare immutable send images off the event loop and adopt them atomically.
use super::*;
use crate::{attachments::Attachment, image_input::ImageBounds};

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
    bounds: Option<ImageBounds>,
    retained: &[Attachment],
    cancel: &CancellationToken,
) -> Result<PreparedInput> {
    crate::cancel::check(Some(cancel))?;
    match content {
        InputContent::Text { text } => Ok(PreparedInput::Text(text)),
        InputContent::Images { images } => {
            if images.len() > crate::attachments::MAX_REQUEST_IMAGES {
                return Err(Error::new(
                    "ATTACHMENT_INVALID",
                    "A request accepts at most eight images",
                ));
            }
            let mut prepared = Vec::new();
            let result = (|| {
                for image in images {
                    crate::cancel::check(Some(cancel))?;
                    prepared.push(image.store_with_bounds(directory, bounds)?);
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
    fn start_image_job(
        &mut self,
        status: &str,
        work: impl FnOnce(&Path, &[Attachment], &CancellationToken) -> Result<PreparedInput>
        + Send
        + 'static,
    ) -> Result<()> {
        if self.conversation.active() || self.input.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Finish or cancel the active image operation or agent request first",
            ));
        }
        let directory = self.conversation.directory().to_owned();
        let retained = self.conversation.image_references();
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
        self.status = status.into();
        Ok(())
    }

    pub(super) fn start_input(&mut self, path: Option<PathBuf>) -> Result<()> {
        let executable = std::env::current_exe().map_err(terminal::io)?;
        let reader = self.input_reader;
        let bounds = self.conversation.settings().image_bounds;
        self.start_image_job(
            "Reading image input…",
            move |directory, retained, token| {
                prepare(
                    reader(&executable, path.as_deref(), bounds, token)?,
                    directory,
                    bounds,
                    retained,
                    token,
                )
            },
        )
    }

    pub(super) fn start_resize(&mut self, index: usize, bounds: Option<ImageBounds>) -> Result<()> {
        let image = self
            .conversation
            .images()
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
        self.start_image_job(
            "Preparing send image…",
            move |directory, _retained, token| {
                crate::cancel::check(Some(token))?;
                let resized = image.resized(directory, bounds)?;
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
                self.insert_text(text);
                self.status = "Pasted text".into();
                return false;
            }
            PreparedInput::Images(images) => self.conversation.add_images(images.clone()),
            PreparedInput::Resized {
                index,
                expected_id,
                image,
            } => self
                .conversation
                .replace_image(*index, expected_id, image.clone()),
        };
        if let Err(error) = result {
            // An IO error can follow an atomic rename. Preserve candidate files
            // rather than delete data that a durable session may already reference.
            if error.code != "IO_ERROR" {
                content.discard(
                    self.conversation.directory(),
                    &self.conversation.image_references(),
                );
            }
            self.error(error);
            return false;
        }
        self.status = match content {
            PreparedInput::Resized { image, .. } => format!(
                "Send image: {}×{} · original retained",
                image.width, image.height
            ),
            _ => format!(
                "{} images attached · Tab to Attachments to review or remove",
                self.conversation.images().len()
            ),
        };
        true
    }

    #[cfg(test)]
    pub(super) fn accept_input(&mut self, content: Result<InputContent>) {
        let prepared = content.and_then(|content| {
            prepare(
                content,
                self.conversation.directory(),
                self.conversation.settings().image_bounds,
                &self.conversation.image_references(),
                &CancellationToken::new(),
            )
        });
        self.accept_prepared(prepared);
    }
}

pub(super) fn label(image: &Attachment, index: usize) -> String {
    let size = image
        .original
        .as_ref()
        .filter(|original| (original.width, original.height) != (image.width, image.height))
        .map(|original| {
            format!(
                "{}×{} → {}×{}",
                original.width, original.height, image.width, image.height
            )
        })
        .unwrap_or_else(|| format!("{}×{}", image.width, image.height));
    format!(
        "[Image {}] {} · {size} · PNG",
        index + 1,
        clean(&image.display_name)
    )
}
