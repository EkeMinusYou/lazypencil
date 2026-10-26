use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU16,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Screen {
    parser: vt100::Parser,
}
impl Screen {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            parser: vt100::Parser::new(
                NonZeroU16::new(height).unwrap(),
                NonZeroU16::new(width).unwrap(),
                0,
            ),
        }
    }
    pub fn resize(&mut self, width: u16, height: u16) {
        self.parser.set_size(
            NonZeroU16::new(height).unwrap(),
            NonZeroU16::new(width).unwrap(),
        );
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub fn text(&self) -> String {
        self.parser
            .screen()
            .rows(0, self.parser.screen().size().1.get())
            .map(|s| s.trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn cursor(&self) -> (u16, u16) {
        let (y, x) = self.parser.screen().cursor_position();
        (x, y)
    }
    pub fn contains_words(&self, first: &str, second: &str) -> bool {
        let text = self.text();
        text.match_indices(first).any(|(position, _)| {
            let suffix = &text[position + first.len()..];
            suffix.starts_with(char::is_whitespace) && suffix.trim_start().starts_with(second)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    palette: BTreeSet<[u8; 4]>,
}
impl Frame {
    pub fn new(width: usize, height: usize, rgba: Vec<u8>) -> Result<Self> {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || width * height > 16 * 1024 * 1024
            || rgba.len() != width * height * 4
        {
            return Err("Expected complete bounded RGBA geometry and pixels".into());
        }
        let palette = rgba
            .chunks_exact(4)
            .map(|p| p.try_into().unwrap())
            .collect();
        Ok(Self {
            width,
            height,
            rgba,
            palette,
        })
    }
    pub fn has_color(&self, color: [u8; 4]) -> bool {
        self.palette.contains(&color)
    }
}

#[derive(Default)]
pub struct KittyFrames {
    pub images: Vec<Frame>,
    pub compressed_frames: usize,
    pub deleted_resources: BTreeSet<u32>,
    pub pending: Vec<u8>,
    pub payload: Vec<u8>,
    dimensions: Option<(usize, usize)>,
    compressed: bool,
}
impl KittyFrames {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        let mut offset = 0;
        loop {
            let Some(start) = self.pending[offset..]
                .windows(3)
                .position(|s| s == b"\x1b_G")
                .map(|i| i + offset)
            else {
                let keep = self.pending.len().saturating_sub(2);
                self.pending.drain(..keep);
                return Ok(());
            };
            let Some(end) = self.pending[start + 3..]
                .windows(2)
                .position(|s| s == b"\x1b\\")
                .map(|i| i + start + 3)
            else {
                self.pending.drain(..start);
                if self.pending.len() > 8192 {
                    return Err("Oversized Kitty protocol packet".into());
                }
                return Ok(());
            };
            if end - start > 8192 {
                return Err("Oversized Kitty protocol packet".into());
            }
            let packet = &self.pending[start + 3..end];
            let split = packet.iter().position(|b| *b == b';');
            let parameters = std::str::from_utf8(&packet[..split.unwrap_or(packet.len())])?;
            let mut fields = BTreeMap::new();
            for field in parameters.split(',') {
                if let Some((key, value)) = field.split_once('=')
                    && fields.insert(key, value).is_some()
                {
                    return Err("Duplicate Kitty parameter".into());
                }
            }
            let action = fields.get("a").copied();
            if action == Some("d")
                && fields.get("d") == Some(&"I")
                && fields.get("q") == Some(&"2")
                && split.is_none()
            {
                self.deleted_resources.insert(
                    fields
                        .get("i")
                        .ok_or("Missing deleted Kitty resource ID")?
                        .parse()?,
                );
            }
            if action == Some("T") {
                if self.dimensions.is_some() {
                    return Err("A new Kitty frame interrupted an incomplete frame".into());
                }
                if fields.get("f") != Some(&"32") {
                    return Err("Expected RGBA Kitty pixels".into());
                }
                self.compressed = match fields.get("o").copied() {
                    None => false,
                    Some("z") => true,
                    _ => return Err("Unknown Kitty compression".into()),
                };
                let width = fields
                    .get("s")
                    .ok_or("Missing Kitty width")?
                    .parse::<usize>()?;
                let height = fields
                    .get("v")
                    .ok_or("Missing Kitty height")?
                    .parse::<usize>()?;
                if width == 0
                    || height == 0
                    || width > 8192
                    || height > 8192
                    || width * height > 16 * 1024 * 1024
                {
                    return Err("Invalid Kitty dimensions".into());
                }
                let retained = self
                    .images
                    .iter()
                    .try_fold(0usize, |bytes, frame| bytes.checked_add(frame.rgba.len()))
                    .ok_or("Overflowed retained Kitty pixels")?;
                const RETAINED_BYTES: usize = 64 * 1024 * 1024;
                if self.images.len() >= 4096
                    || retained > RETAINED_BYTES
                    || width * height * 4 > RETAINED_BYTES - retained
                {
                    return Err("Retained Kitty images exceed observer limits".into());
                }
                self.dimensions = Some((width, height));
                self.payload.clear();
            }
            if let Some((width, height)) = self.dimensions
                && (action.is_none() || action == Some("T"))
            {
                let expected = width * height * 4;
                if action.is_none() && fields.contains_key("o") {
                    return Err("Kitty compression must be declared in the first chunk".into());
                }
                let separator = split.ok_or("A Kitty image packet needs a payload separator")?;
                let decoded = STANDARD.decode(&packet[separator + 1..])?;
                // Incompressible ZLIB has a small encoding overhead. Bound both
                // the wire payload and the independently declared decoded size.
                let limit = if self.compressed {
                    expected + expected / 8 + 64
                } else {
                    expected
                };
                if decoded.len() > limit - self.payload.len() {
                    return Err("Kitty pixels exceed declared dimensions".into());
                }
                self.payload.extend_from_slice(&decoded);
                match fields.get("m").copied().unwrap_or("0") {
                    "1" => {}
                    "0" => {
                        let payload = std::mem::take(&mut self.payload);
                        let pixels = if self.compressed {
                            // Require one complete stream, its checksum, exact
                            // dimensions, and no trailing or concatenated data.
                            let mut decoder = flate2::Decompress::new(true);
                            let mut pixels = vec![0; expected + 1];
                            let status = decoder.decompress(
                                &payload,
                                &mut pixels,
                                flate2::FlushDecompress::Finish,
                            )?;
                            if status != flate2::Status::StreamEnd
                                || decoder.total_in() != payload.len() as u64
                                || decoder.total_out() != expected as u64
                            {
                                return Err("Incomplete or oversized Kitty ZLIB stream".into());
                            }
                            pixels.truncate(expected);
                            pixels
                        } else {
                            payload
                        };
                        if pixels.len() != expected {
                            return Err("Incomplete Kitty pixel data".into());
                        }
                        self.images.push(Frame::new(width, height, pixels)?);
                        self.compressed_frames += usize::from(self.compressed);
                        self.dimensions = None;
                        self.compressed = false;
                    }
                    _ => return Err("Invalid Kitty continuation marker".into()),
                }
            }
            offset = end + 2;
        }
    }
}

pub fn sixel_red_pixels(data: &[u8]) -> Result<(usize, usize)> {
    let dimensions = data
        .iter()
        .position(|b| *b == b'"')
        .ok_or("Missing Sixel raster dimensions")?;
    let mut end = dimensions + 1;
    while end < data.len() && (data[end].is_ascii_digit() || data[end] == b';') {
        end += 1;
    }
    let values = std::str::from_utf8(&data[dimensions + 1..end])?
        .split(';')
        .map(str::parse::<usize>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err("Invalid Sixel raster dimensions".into());
    }
    let (width, height) = (values[2], values[3]);
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("Oversized Sixel raster".into());
    }
    let mut pixels = vec![false; width * height];
    let mut colors = BTreeMap::new();
    let (mut color, mut x, mut y, mut index) = (0, 0, 0, 0);
    while index < data.len() {
        let mut symbol = data[index];
        if symbol == b'"' || symbol == b'#' {
            let mut end = index + 1;
            while end < data.len() && (data[end].is_ascii_digit() || data[end] == b';') {
                end += 1;
            }
            let values = std::str::from_utf8(&data[index + 1..end])?
                .split(';')
                .filter(|s| !s.is_empty())
                .map(str::parse::<usize>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if symbol == b'#' {
                color = *values.first().ok_or("Missing Sixel palette index")?;
                if values.len() == 5 {
                    if values[1] != 2 || values[2..].iter().any(|v| *v > 100) {
                        return Err("Expected RGB Sixel palette".into());
                    }
                    colors.insert(color, [values[2], values[3], values[4]]);
                }
            }
            index = end;
            continue;
        }
        let mut repeat = 1;
        if symbol == b'!' {
            let mut end = index + 1;
            while end < data.len() && data[end].is_ascii_digit() {
                end += 1;
            }
            repeat = std::str::from_utf8(&data[index + 1..end])?.parse()?;
            if repeat == 0 || repeat > width || end == data.len() {
                return Err("Invalid Sixel repeat".into());
            }
            symbol = data[end];
            index = end;
        }
        if (63..=126).contains(&symbol) {
            if x + repeat > width {
                return Err("Sixel band exceeds raster width".into());
            }
            for bit in 0..6 {
                if (symbol - 63) & (1 << bit) != 0 && y + bit < height {
                    let start = (y + bit) * width + x;
                    pixels[start..start + repeat].fill(colors.get(&color) == Some(&[100, 0, 0]));
                }
            }
            x += repeat;
        } else if symbol == b'$' {
            x = 0;
        } else if symbol == b'-' {
            x = 0;
            y += 6;
        }
        index += 1;
    }
    Ok((pixels.iter().filter(|v| **v).count(), width * height))
}
