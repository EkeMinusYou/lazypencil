use crate::{Error, Result};

pub const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    pub max_source_bytes: usize,
    pub max_output_bytes: usize,
    pub max_pixels: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: MAX_SOURCE_BYTES,
            max_output_bytes: MAX_SOURCE_BYTES,
            max_pixels: MAX_IMAGE_PIXELS,
        }
    }
}
impl DecodeLimits {
    pub fn validate(self) -> Result<()> {
        if !(1..=MAX_SOURCE_BYTES).contains(&self.max_source_bytes)
            || !(1..=MAX_SOURCE_BYTES).contains(&self.max_output_bytes)
            || !(1..=MAX_IMAGE_PIXELS).contains(&self.max_pixels)
        {
            return Err(invalid(
                "Image decode limits exceed the native byte or pixel bounds",
            ));
        }
        Ok(())
    }
}

/// An explicit send-image bounding box, never an instruction to upscale.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ImageBounds {
    pub width: u32,
    pub height: u32,
}

impl ImageBounds {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(invalid(
                "Image bounds must be positive and within 16 megapixels",
            ));
        }
        Ok(Self { width, height })
    }

    #[cfg(feature = "rendering")]
    fn fit(self, width: u32, height: u32) -> Result<(u32, u32)> {
        Self::new(self.width, self.height)?;
        if width <= self.width && height <= self.height {
            return Ok((width, height));
        }
        Ok(
            if u64::from(width) * u64::from(self.height)
                >= u64::from(height) * u64::from(self.width)
            {
                (
                    self.width,
                    (u64::from(height) * u64::from(self.width) / u64::from(width)).max(1) as u32,
                )
            } else {
                (
                    (u64::from(width) * u64::from(self.height) / u64::from(height)).max(1) as u32,
                    self.height,
                )
            },
        )
    }
}

impl std::str::FromStr for ImageBounds {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        let (width, height) = value
            .split_once('x')
            .ok_or_else(|| invalid("Use image bounds WIDTHxHEIGHT, such as 1024x768"))?;
        let parse = |text: &str| {
            text.parse::<u32>()
                .map_err(|_| invalid("Image bounds must contain integer dimensions"))
        };
        Self::new(parse(width)?, parse(height)?)
    }
}

impl std::fmt::Display for ImageBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

pub struct NormalizedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source_mime: &'static str,
    pub source_width: u32,
    pub source_height: u32,
}

#[cfg(feature = "rendering")]
pub fn from_rgba8(width: usize, height: usize, rgba: &[u8]) -> Result<NormalizedImage> {
    from_rgba8_with_limits(width, height, rgba, DecodeLimits::default())
}

#[cfg(feature = "rendering")]
pub fn from_rgba8_with_limits(
    width: usize,
    height: usize,
    rgba: &[u8],
    limits: DecodeLimits,
) -> Result<NormalizedImage> {
    limits.validate()?;
    if width == 0
        || height == 0
        || (width as u64).saturating_mul(height as u64) > limits.max_pixels
        || width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            != Some(rgba.len())
    {
        return Err(invalid(format!(
            "Clipboard pixels must be nonempty RGBA8 within {} pixels",
            limits.max_pixels
        )));
    }
    let info = skia_safe::ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        Some(skia_safe::ColorSpace::new_srgb()),
    );
    let image =
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(rgba), width * 4)
            .ok_or_else(|| invalid("Cannot allocate clipboard pixels"))?;
    let data = skia_safe::png_encoder::encode_image(None, &image, &Default::default())
        .ok_or_else(|| invalid("Cannot encode clipboard pixels"))?;
    if data.size() > limits.max_output_bytes {
        return Err(invalid(format!(
            "Clipboard PNG exceeds {} bytes",
            limits.max_output_bytes
        )));
    }
    Ok(NormalizedImage {
        png: data.as_bytes().into(),
        width: width as u32,
        height: height as u32,
        source_mime: "image/png",
        source_width: width as u32,
        source_height: height as u32,
    })
}

impl std::fmt::Debug for NormalizedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NormalizedImage")
            .field("png_bytes", &self.png.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("source_mime", &self.source_mime)
            .field("source_width", &self.source_width)
            .field("source_height", &self.source_height)
            .finish()
    }
}

pub fn normalize(bytes: &[u8], claimed_mime: Option<&str>) -> Result<NormalizedImage> {
    normalize_with_bounds(bytes, claimed_mime, None)
}

pub fn normalize_with_bounds(
    bytes: &[u8],
    claimed_mime: Option<&str>,
    bounds: Option<ImageBounds>,
) -> Result<NormalizedImage> {
    normalize_with_limits(bytes, claimed_mime, bounds, DecodeLimits::default())
}

pub fn normalize_with_limits(
    bytes: &[u8],
    claimed_mime: Option<&str>,
    bounds: Option<ImageBounds>,
    limits: DecodeLimits,
) -> Result<NormalizedImage> {
    limits.validate()?;
    if let Some(bounds) = bounds {
        ImageBounds::new(bounds.width, bounds.height)?;
    }
    if bytes.is_empty() || bytes.len() > limits.max_source_bytes {
        return Err(invalid(format!(
            "Image input must contain 1..{} bytes",
            limits.max_source_bytes
        )));
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(invalid(
            "Only static PNG, JPEG, and WebP input is supported",
        ));
    };
    if claimed_mime.is_some_and(|claimed| claimed != mime) {
        return Err(invalid(
            "The declared image MIME type does not match the content",
        ));
    }
    validate_container(bytes, mime)?;
    decode(bytes, mime, bounds, limits)
}

#[cfg(feature = "rendering")]
fn decode(
    bytes: &[u8],
    mime: &'static str,
    bounds: Option<ImageBounds>,
    limits: DecodeLimits,
) -> Result<NormalizedImage> {
    use skia_safe::{Codec, ColorSpace, Data, ImageInfo};
    let mut codec = Codec::from_data(Data::new_copy(bytes))
        .ok_or_else(|| invalid("Cannot decode the image header"))?;
    let size = codec.dimensions();
    if size.width <= 0
        || size.height <= 0
        || size.width as u64 * size.height as u64 > limits.max_pixels
    {
        return Err(invalid(format!(
            "Image dimensions exceed {} decoded source pixels",
            limits.max_pixels
        )));
    }
    if codec.get_frame_count() > 1 {
        return Err(invalid("Animated images are not accepted"));
    }
    let info = ImageInfo::new_n32_premul(size, Some(ColorSpace::new_srgb()));
    let row_bytes = size.width as usize * 4;
    let mut pixels = vec![0u8; row_bytes * size.height as usize];
    let options = skia_safe::codec::Options {
        max_decode_memory: Some(128 * 1024 * 1024),
        ..Default::default()
    };
    let result = codec.get_pixels_with_options(&info, &mut pixels, row_bytes, Some(&options));
    if result != skia_safe::codec::Result::Success {
        return Err(invalid(format!("Image decoding failed: {result:?}")));
    }
    let image = skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), row_bytes)
        .ok_or_else(|| invalid("Cannot allocate the decoded image"))?;
    drop(pixels);
    let origin = codec.origin();
    let (source_width, source_height) = if origin.swaps_width_height() {
        (size.height, size.width)
    } else {
        (size.width, size.height)
    };
    let (width, height) = bounds
        .map_or(Ok((source_width as u32, source_height as u32)), |bounds| {
            bounds.fit(source_width as u32, source_height as u32)
        })?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or_else(|| invalid("Cannot allocate the normalized image"))?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    let resized = (width, height) != (source_width as u32, source_height as u32);
    if resized {
        canvas.scale((
            width as f32 / source_width as f32,
            height as f32 / source_height as f32,
        ));
    }
    canvas.concat(&origin.to_matrix((source_width, source_height)));
    if resized {
        let mut paint = skia_safe::Paint::default();
        paint.set_shader(image.to_shader(
            (skia_safe::TileMode::Clamp, skia_safe::TileMode::Clamp),
            skia_safe::SamplingOptions::from(skia_safe::FilterMode::Linear),
            None,
        ));
        canvas.draw_rect(
            skia_safe::Rect::from_wh(size.width as f32, size.height as f32),
            &paint,
        );
    } else {
        canvas.draw_image(&image, (0.0, 0.0), None);
    }
    let png =
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .ok_or_else(|| invalid("Cannot encode the normalized PNG"))?;
    if png.size() > limits.max_output_bytes {
        return Err(invalid(format!(
            "Normalized PNG exceeds {} bytes; choose a smaller explicit send size",
            limits.max_output_bytes
        )));
    }
    Ok(NormalizedImage {
        png: png.as_bytes().into(),
        width,
        height,
        source_mime: mime,
        source_width: source_width as u32,
        source_height: source_height as u32,
    })
}

#[cfg(not(feature = "rendering"))]
fn decode(
    _bytes: &[u8],
    _mime: &'static str,
    _bounds: Option<ImageBounds>,
    _limits: DecodeLimits,
) -> Result<NormalizedImage> {
    Err(Error::new(
        "UNSUPPORTED_FEATURE",
        "This binary cannot decode images; build with rendering support",
    ))
}

fn validate_container(bytes: &[u8], mime: &str) -> Result<()> {
    if mime == "image/png" {
        let mut offset = 8;
        let mut ended = false;
        while offset < bytes.len() {
            let header = bytes
                .get(offset..offset + 8)
                .ok_or_else(|| invalid("Truncated PNG chunk"))?;
            let length = u32::from_be_bytes(header[..4].try_into().expect("chunk length")) as usize;
            let end = offset
                .checked_add(length)
                .and_then(|n| n.checked_add(12))
                .filter(|n| *n <= bytes.len())
                .ok_or_else(|| invalid("Invalid PNG chunk length"))?;
            if &header[4..8] == b"acTL" {
                return Err(invalid("Animated PNG images are not accepted"));
            }
            offset = end;
            if &header[4..8] == b"IEND" {
                if length != 0 {
                    return Err(invalid("Invalid PNG end chunk"));
                }
                ended = true;
                break;
            }
        }
        if !ended {
            return Err(invalid("PNG is missing its end chunk"));
        }
    } else if mime == "image/webp" {
        let declared = u32::from_le_bytes(
            bytes
                .get(4..8)
                .ok_or_else(|| invalid("Truncated WebP header"))?
                .try_into()
                .expect("RIFF size"),
        ) as u64
            + 8;
        if declared != bytes.len() as u64 {
            return Err(invalid("WebP RIFF length does not match its input"));
        }
        let mut offset = 12;
        while offset < bytes.len() {
            let header = bytes
                .get(offset..offset + 8)
                .ok_or_else(|| invalid("Truncated WebP chunk"))?;
            let length =
                u32::from_le_bytes(header[4..8].try_into().expect("chunk length")) as usize;
            if matches!(&header[..4], b"ANIM" | b"ANMF") {
                return Err(invalid("Animated WebP images are not accepted"));
            }
            offset = offset
                .checked_add(8)
                .and_then(|n| n.checked_add(length))
                .and_then(|n| n.checked_add(length % 2))
                .filter(|n| *n <= bytes.len())
                .ok_or_else(|| invalid("Invalid WebP chunk length"))?;
        }
    } else if mime == "image/jpeg" && !bytes.ends_with(&[0xff, 0xd9]) {
        return Err(invalid("JPEG is missing its end marker"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("ATTACHMENT_INVALID", message)
}
