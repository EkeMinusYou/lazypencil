//! Explicit clipboard/image input in a bounded child process, never a clipboard watcher.

use crate::{
    Error, Result,
    attachments::{Attachment, read_bounded},
    image_input,
};
use base64::Engine;
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
        if self.data.len() > image_input::MAX_SOURCE_BYTES.div_ceil(3) * 4
            || self.source_bytes > image_input::MAX_SOURCE_BYTES
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
        Attachment::from_bytes(directory, &bytes, &self.name)
    }
}

pub fn read_clipboard(executable: &Path, cancel: &CancellationToken) -> Result<InputContent> {
    read_worker(executable, None, cancel)
}
pub fn read_image(
    executable: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    read_worker(executable, Some(path), cancel)
}
fn read_worker(
    executable: &Path,
    path: Option<&Path>,
    cancel: &CancellationToken,
) -> Result<InputContent> {
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
            .take(MAX_WIRE as u64 + 1)
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
            if bytes.len() > MAX_WIRE {
                return Err(Error::new(
                    "ATTACHMENT_INVALID",
                    "Image input response exceeds its limit",
                ));
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                Error::new(
                    "CLIPBOARD_UNAVAILABLE",
                    "Image input worker returned invalid data",
                )
            })?;
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
            let content: InputContent = serde_json::from_value(value["content"].clone())
                .map_err(|_| Error::new("ATTACHMENT_INVALID", "Invalid image input response"))?;
            match &content {
                InputContent::Images { images } if images.is_empty() || images.len() > 8 => {
                    return Err(Error::new(
                        "ATTACHMENT_INVALID",
                        "An image input accepts 1..8 images",
                    ));
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

pub fn serve_worker() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    let result = if args.len() == 2 && args[0] == "--file" {
        file_images(&[PathBuf::from(&args[1])])
    } else if args.len() == 1 && args[0] == "--clipboard" {
        native_read()
    } else {
        Err(Error::new(
            "INVALID_ARGUMENT",
            "Invalid image input worker arguments",
        ))
    };
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

fn file_images(paths: &[PathBuf]) -> Result<InputContent> {
    if paths.is_empty() || paths.len() > 8 {
        return Err(Error::new(
            "ATTACHMENT_INVALID",
            "Paste at most eight image files",
        ));
    }
    let mut images = Vec::new();
    let mut original = 0usize;
    let mut normalized = 0usize;
    for path in paths {
        let bytes = read_bounded(path, image_input::MAX_SOURCE_BYTES)?;
        original += bytes.len();
        let image = image_input::normalize(&bytes, None)?;
        normalized += image.png.len();
        if original > crate::attachments::MAX_REQUEST_BYTES
            || normalized > crate::attachments::MAX_REQUEST_BYTES
        {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "Pasted images exceed 50 MiB",
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
fn native_read() -> Result<InputContent> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| unavailable(&error))?;
    match clipboard.get_image() {
        Ok(image) => {
            let image = image_input::from_rgba8(image.width, image.height, &image.bytes)?;
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
        Ok(files) if !files.is_empty() => return file_images(&files),
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
