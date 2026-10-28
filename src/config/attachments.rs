//! User-owned image budgets shared by ingestion, workers, and prompt submission.
use super::{UserSource, user_config_path};
use crate::{Error, Result, attachments::Attachment, image_input};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_IMAGES: usize = 64;
pub const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub(super) const TEMPLATE: &str = "\n[attachments]\n# Personal image input/send budgets; no implicit resizing.\nmax_images = 8\nmax_source_bytes = 20971520\nmax_send_bytes = 20971520\nmax_total_source_bytes = 52428800\nmax_total_send_bytes = 52428800\nmax_source_pixels = 16777216\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AttachmentSettings {
    #[schemars(range(min = 1, max = 64))]
    pub max_images: usize,
    #[schemars(range(min = 1, max = 20971520))]
    pub max_source_bytes: usize,
    #[schemars(range(min = 1, max = 20971520))]
    pub max_send_bytes: usize,
    #[schemars(range(min = 1, max = 268435456))]
    pub max_total_source_bytes: usize,
    #[schemars(range(min = 1, max = 268435456))]
    pub max_total_send_bytes: usize,
    /// Decoded source pixels, before any explicitly requested downscale.
    #[schemars(range(min = 1, max = 16777216))]
    pub max_source_pixels: u64,
}

impl Default for AttachmentSettings {
    fn default() -> Self {
        Self {
            max_images: crate::attachments::MAX_REQUEST_IMAGES,
            max_source_bytes: image_input::MAX_SOURCE_BYTES,
            max_send_bytes: image_input::MAX_SOURCE_BYTES,
            max_total_source_bytes: crate::attachments::MAX_REQUEST_BYTES,
            max_total_send_bytes: crate::attachments::MAX_REQUEST_BYTES,
            max_source_pixels: image_input::MAX_IMAGE_PIXELS,
        }
    }
}

impl AttachmentSettings {
    pub fn validate(&self) -> Result<()> {
        if !(1..=MAX_IMAGES).contains(&self.max_images)
            || !(1..=image_input::MAX_SOURCE_BYTES).contains(&self.max_source_bytes)
            || !(1..=image_input::MAX_SOURCE_BYTES).contains(&self.max_send_bytes)
            || !(1..=MAX_TOTAL_BYTES).contains(&self.max_total_source_bytes)
            || !(1..=MAX_TOTAL_BYTES).contains(&self.max_total_send_bytes)
            || !(1..=image_input::MAX_IMAGE_PIXELS).contains(&self.max_source_pixels)
        {
            return Err(super::invalid(
                "Attachment limits require 1..64 images, per-image source/send bytes within 1..20 MiB, total source/send bytes within 1..256 MiB, and decoded source pixels within 1..16 Mi",
            ));
        }
        Ok(())
    }

    pub fn decode_limits(self) -> image_input::DecodeLimits {
        image_input::DecodeLimits {
            max_source_bytes: self.max_source_bytes.min(self.max_total_source_bytes),
            max_output_bytes: self.max_send_bytes.min(self.max_total_send_bytes),
            max_pixels: self.max_source_pixels,
        }
    }

    pub fn check_count(&self, count: usize) -> Result<()> {
        self.validate()?;
        if count > self.max_images {
            let message = if self.max_images == 8 {
                "A request accepts at most eight images".to_owned()
            } else {
                format!("A request accepts at most {} images", self.max_images)
            };
            return Err(Error::new("ATTACHMENT_INVALID", message).details(
                serde_json::json!({"limit":"max_images","maximum":self.max_images,"actual":count}),
            ));
        }
        Ok(())
    }

    pub fn check_request<'a>(
        &self,
        images: impl IntoIterator<Item = &'a Attachment>,
    ) -> Result<()> {
        let images = images.into_iter().collect::<Vec<_>>();
        self.check_count(images.len())?;
        let mut source_total = 0usize;
        let mut send_total = 0usize;
        for image in images {
            let source_bytes = self.check_source(image)?;
            check(
                "max_send_bytes",
                image.bytes as u64,
                self.max_send_bytes as u64,
            )?;
            source_total = source_total.saturating_add(source_bytes);
            send_total = send_total.saturating_add(image.bytes);
        }
        check(
            "max_total_source_bytes",
            source_total as u64,
            self.max_total_source_bytes as u64,
        )?;
        check(
            "max_total_send_bytes",
            send_total as u64,
            self.max_total_send_bytes as u64,
        )
    }
    pub fn check_source(&self, image: &Attachment) -> Result<usize> {
        self.validate()?;
        let bytes = image.source_size();
        let (width, height) = image
            .original
            .as_ref()
            .map_or((image.width, image.height), |source| {
                (source.width, source.height)
            });
        check(
            "max_source_bytes",
            bytes as u64,
            self.max_source_bytes as u64,
        )?;
        check(
            "max_total_source_bytes",
            bytes as u64,
            self.max_total_source_bytes as u64,
        )?;
        check(
            "max_source_pixels",
            u64::from(width) * u64::from(height),
            self.max_source_pixels,
        )?;
        Ok(bytes)
    }

    /// Budget for a new image, after validating all already selected images.
    pub fn remaining<'a>(self, images: impl IntoIterator<Item = &'a Attachment>) -> Result<Self> {
        let images = images.into_iter().collect::<Vec<_>>();
        self.check_request(images.iter().copied())?;
        let mut remaining = self;
        for image in images {
            remaining.max_total_source_bytes -= image.source_size();
            remaining.max_total_send_bytes -= image.bytes;
        }
        if remaining.max_total_source_bytes == 0 || remaining.max_total_send_bytes == 0 {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "The request image byte budget is exhausted",
            ));
        }
        Ok(remaining)
    }
}

fn check(name: &str, actual: u64, maximum: u64) -> Result<()> {
    if actual > maximum {
        return Err(Error::new(
            "ATTACHMENT_INVALID",
            format!("Image input exceeds {name}: {actual} > {maximum}"),
        )
        .details(serde_json::json!({"limit":name,"actual":actual,"maximum":maximum})));
    }
    Ok(())
}

pub fn load() -> Result<AttachmentSettings> {
    let source = UserSource::load(Some(user_config_path()?), true)?;
    source.verify()?;
    Ok(source.settings.attachments)
}

pub fn load_file(path: Option<PathBuf>) -> Result<AttachmentSettings> {
    let source = UserSource::load(path, false)?;
    source.verify()?;
    Ok(source.settings.attachments)
}
