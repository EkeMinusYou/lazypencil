//! Explicit clipboard/image input in a bounded child process, never a clipboard watcher.

use crate::{
    Error, Result,
    attachments::{Attachment, AttachmentOptions, read_bounded},
    image_input,
};
use base64::Engine;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_WIRE: usize = 72 * 1024 * 1024;

fn wire_limit(limits: crate::config::attachments::AttachmentSettings) -> usize {
    let overhead = MAX_WIRE - crate::attachments::MAX_REQUEST_BYTES.div_ceil(3) * 4;
    let source = limits
        .max_total_source_bytes
        .min(limits.max_images.saturating_mul(limits.max_source_bytes));
    // A text clipboard can contain 1 MiB of characters escaped to six JSON bytes.
    (source.div_ceil(3) * 4 + overhead).max(7 * 1024 * 1024)
}

struct Process {
    child: std::process::Child,
    reader: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    Text { text: String },
    Images { images: Vec<InputImage> },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct InputImage {
    pub name: String,
    pub data: String,
    pub source_bytes: usize,
}

impl InputImage {
    pub fn store(self, directory: &Path) -> Result<Attachment> {
        self.store_with_bounds(directory, None)
    }

    pub fn store_with_bounds(
        self,
        directory: &Path,
        bounds: Option<image_input::ImageBounds>,
    ) -> Result<Attachment> {
        self.store_with_options(
            directory,
            AttachmentOptions {
                bounds,
                ..Default::default()
            },
        )
    }

    pub fn store_with_options(
        self,
        directory: &Path,
        options: AttachmentOptions,
    ) -> Result<Attachment> {
        options.limits.validate()?;
        if self.data.len() > options.limits.decode_limits().max_source_bytes.div_ceil(3) * 4
            || self.source_bytes > options.limits.decode_limits().max_source_bytes
        {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "Image worker output exceeds its limit",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(self.data)
            .map_err(|_| Error::new("ATTACHMENT_INVALID", "Invalid image worker data"))?;
        if bytes.len() != self.source_bytes {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "Image worker source size does not match its data",
            ));
        }
        Attachment::from_bytes_with_options(directory, &bytes, &self.name, options)
    }
}

pub fn read_clipboard(executable: &Path, cancel: &CancellationToken) -> Result<InputContent> {
    read_clipboard_with_bounds(executable, None, cancel)
}

pub fn read_clipboard_with_bounds(
    executable: &Path,
    bounds: Option<image_input::ImageBounds>,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_clipboard_with_options(
        executable,
        AttachmentOptions {
            bounds,
            ..Default::default()
        },
        cancel,
    )
}

pub fn read_clipboard_with_options(
    executable: &Path,
    options: AttachmentOptions,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_worker(executable, None, options, cancel)
}
pub fn read_image(
    executable: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_image_with_bounds(executable, path, None, cancel)
}

pub fn read_image_with_bounds(
    executable: &Path,
    path: &Path,
    bounds: Option<image_input::ImageBounds>,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_image_with_options(
        executable,
        path,
        AttachmentOptions {
            bounds,
            ..Default::default()
        },
        cancel,
    )
}

pub fn read_image_with_options(
    executable: &Path,
    path: &Path,
    options: AttachmentOptions,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_worker(executable, Some(path), options, cancel)
}
fn read_worker(
    executable: &Path,
    path: Option<&Path>,
    options: AttachmentOptions,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    options.limits.validate()?;
    let max_wire = wire_limit(options.limits);
    if cancel.is_cancelled() {
        return Err(Error::new("CANCELLED", "Image input cancelled"));
    }
    let mut command = Command::new(executable);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .arg("__image_worker")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path {
        command.arg("--file").arg(path);
    } else {
        command.arg("--clipboard");
    }
    if let Some(bounds) = options.bounds {
        command.arg("--max-size").arg(bounds.to_string());
    }
    if options.limits != Default::default() {
        command.arg("--limits").arg(
            serde_json::to_string(&options.limits)
                .map_err(|error| Error::new("ATTACHMENT_INVALID", error.to_string()))?,
        );
    }
    let mut child = command.spawn().map_err(|error| {
        Error::new(
            "CLIPBOARD_UNAVAILABLE",
            format!("Cannot start image input worker: {error}"),
        )
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(max_wire as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let mut process = Process {
        child,
        reader: Some(reader),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        check_input_deadline(cancel, deadline)?;
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?
        {
            // A descendant may hold stdout after the direct child exits. Keep
            // cancellation and the whole request deadline live during EOF wait.
            let response_deadline = Instant::now() + Duration::from_secs(1);
            let bytes = loop {
                check_input_deadline(cancel, deadline)?;
                let Some(remaining) = response_deadline.checked_duration_since(Instant::now())
                else {
                    return Err(Error::new(
                        "CLIPBOARD_UNAVAILABLE",
                        "Image input worker closed without a response",
                    ));
                };
                match receiver.recv_timeout(remaining.min(Duration::from_millis(15))) {
                    Ok(bytes) => break bytes,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(Error::new(
                            "CLIPBOARD_UNAVAILABLE",
                            "Image input worker closed without a response",
                        ));
                    }
                }
            };
            if let Some(reader) = process.reader.take() {
                let _ = reader.join();
            }
            check_input_deadline(cancel, deadline)?;
            let bytes = bytes.map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
            if bytes.len() > max_wire {
                return Err(Error::new(
                    "ATTACHMENT_INVALID",
                    "Image input response exceeds its limit",
                ));
            }
            let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                Error::new(
                    "CLIPBOARD_UNAVAILABLE",
                    "Image input worker returned invalid data",
                )
            })?;
            drop(bytes);
            if let Some(error) = value.get("error") {
                let code = match error["code"].as_str() {
                    Some("ATTACHMENT_INVALID") => "ATTACHMENT_INVALID",
                    Some("CLIPBOARD_EMPTY") => "CLIPBOARD_EMPTY",
                    _ => "CLIPBOARD_UNAVAILABLE",
                };
                return Err(Error::new(
                    code,
                    error["message"]
                        .as_str()
                        .unwrap_or("Cannot read image input"),
                ));
            }
            if !status.success() {
                return Err(Error::new(
                    "CLIPBOARD_UNAVAILABLE",
                    "Image input worker terminated unexpectedly",
                ));
            }
            let content: InputContent = serde_json::from_value(
                value
                    .get_mut("content")
                    .map(Value::take)
                    .unwrap_or(Value::Null),
            )
            .map_err(|_| Error::new("ATTACHMENT_INVALID", "Invalid image input response"))?;
            match &content {
                InputContent::Images { images } => {
                    if images.is_empty() {
                        return Err(Error::new(
                            "ATTACHMENT_INVALID",
                            "No images in worker response",
                        ));
                    }
                    options.limits.check_count(images.len())?;
                }
                InputContent::Text { text } if text.len() > 1024 * 1024 => {
                    return Err(Error::new(
                        "ATTACHMENT_INVALID",
                        "Clipboard text exceeds 1 MiB",
                    ));
                }
                _ => {}
            }
            check_input_deadline(cancel, deadline)?;
            return Ok(content);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn check_input_deadline(cancel: &CancellationToken, deadline: Instant) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(Error::new("CANCELLED", "Image input cancelled"));
    }
    if Instant::now() >= deadline {
        return Err(Error::new(
            "CLIPBOARD_TIMEOUT",
            "Image input did not finish within five seconds",
        ));
    }
    Ok(())
}

#[derive(Parser)]
struct WorkerArguments {
    #[arg(
        long,
        allow_hyphen_values = true,
        conflicts_with = "clipboard",
        required_unless_present = "clipboard"
    )]
    file: Option<PathBuf>,
    #[arg(long, conflicts_with = "file", required_unless_present = "file")]
    clipboard: bool,
    #[arg(long)]
    max_size: Option<image_input::ImageBounds>,
    #[arg(long)]
    limits: Option<String>,
}

pub fn serve_worker() -> Result<()> {
    let result = (|| {
        let args = WorkerArguments::try_parse_from(std::env::args_os().skip(1))
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid image input worker arguments"))?;
        let limits: crate::config::attachments::AttachmentSettings = args
            .limits
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| Error::new("ATTACHMENT_INVALID", "Invalid image input limits"))?
            .unwrap_or_default();
        limits.validate()?;
        let options = AttachmentOptions {
            bounds: args.max_size,
            limits,
        };
        if let Some(file) = args.file {
            file_images(&[file], options)
        } else if args.clipboard {
            native_read(options)
        } else {
            Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid image input worker arguments",
            ))
        }
    })();
    let value = match result {
        Ok(content) => json!({"content":content}),
        Err(error) => json!({"error":error}),
    };
    serde_json::to_writer(std::io::stdout().lock(), &value)
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    std::io::stdout()
        .flush()
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))
}

fn file_images(paths: &[PathBuf], options: AttachmentOptions) -> Result<InputContent> {
    options.limits.check_count(paths.len())?;
    if paths.is_empty() {
        return Err(Error::new("ATTACHMENT_INVALID", "No image files available"));
    }
    let mut images = Vec::new();
    let mut original = 0usize;
    let mut normalized = 0usize;
    for path in paths {
        let mut limits = options.limits;
        limits.max_total_source_bytes = limits.max_total_source_bytes.saturating_sub(original);
        limits.max_total_send_bytes = limits.max_total_send_bytes.saturating_sub(normalized);
        if limits.max_total_source_bytes == 0 || limits.max_total_send_bytes == 0 {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "The request image byte budget is exhausted",
            ));
        }
        let bytes = read_bounded(path, limits.decode_limits().max_source_bytes)?;
        original += bytes.len();
        let image = image_input::normalize_with_limits(
            &bytes,
            None,
            options.bounds,
            limits.decode_limits(),
        )?;
        normalized += image.png.len();
        if original > options.limits.max_total_source_bytes
            || normalized > options.limits.max_total_send_bytes
        {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "Pasted images exceed the configured total byte limit",
            ));
        }
        images.push(InputImage {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            // Validate/normalize in the worker, but preserve the encoded source
            // across the wire so attachment storage can retain its exact bytes.
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            source_bytes: bytes.len(),
        });
    }
    Ok(InputContent::Images { images })
}
fn native_read(options: AttachmentOptions) -> Result<InputContent> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| unavailable(&error))?;
    match clipboard.get_image() {
        Ok(image) => {
            let mut capture = options.limits.decode_limits();
            capture.max_output_bytes = capture.max_source_bytes;
            let image = image_input::from_rgba8_with_limits(
                image.width,
                image.height,
                &image.bytes,
                capture,
            )?;
            image_input::normalize_with_limits(
                &image.png,
                None,
                options.bounds,
                options.limits.decode_limits(),
            )?;
            let bytes = image.png.len();
            return Ok(InputContent::Images {
                images: vec![InputImage {
                    name: "clipboard.png".into(),
                    data: base64::engine::general_purpose::STANDARD.encode(image.png),
                    source_bytes: bytes,
                }],
            });
        }
        Err(arboard::Error::ContentNotAvailable) => {}
        Err(error) => return Err(unavailable(&error)),
    }
    match clipboard.get().file_list() {
        Ok(files) if !files.is_empty() => return file_images(&files, options),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => {}
        Err(error) => return Err(unavailable(&error)),
    }
    match clipboard.get_text() {
        Ok(text) if text.len() <= 1024 * 1024 && !text.is_empty() => {
            Ok(InputContent::Text { text })
        }
        Ok(text) if text.len() > 1024 * 1024 => Err(Error::new(
            "ATTACHMENT_INVALID",
            "Clipboard text exceeds 1 MiB",
        )),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Err(Error::new(
            "CLIPBOARD_EMPTY",
            "No image, image file, or text is available on this host's clipboard",
        )),
        Err(error) => Err(unavailable(&error)),
    }
}
fn unavailable(error: &arboard::Error) -> Error {
    Error::new(
        "CLIPBOARD_UNAVAILABLE",
        format!("Cannot read this host's clipboard: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_budget_tracks_selected_source_limits_without_disabling_text_paste() {
        use crate::config::attachments::AttachmentSettings;
        let defaults = AttachmentSettings::default();
        assert_eq!(wire_limit(defaults), MAX_WIRE);
        let small = AttachmentSettings {
            max_source_bytes: 1,
            max_total_source_bytes: 1,
            ..defaults
        };
        assert_eq!(wire_limit(small), 7 * 1024 * 1024);
        let large = AttachmentSettings {
            max_images: 64,
            max_total_source_bytes: 256 * 1024 * 1024,
            ..defaults
        };
        assert!(wire_limit(large) > MAX_WIRE);
        assert_eq!(
            wire_limit(AttachmentSettings {
                max_send_bytes: 1,
                max_total_send_bytes: 1,
                ..defaults
            }),
            MAX_WIRE
        );
    }
}
