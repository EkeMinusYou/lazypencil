//! Bounded terminal probing without a detached stdin reader or tmux configuration changes.

use crate::{Error, Result};
mod local;
use image::DynamicImage;
use local::{LocalImage, LocalTransfer};
use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    widgets::Widget,
};
#[cfg(unix)]
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions};
use ratatui_image::{
    FontSize, Resize,
    picker::{ProtocolType, cap_parser::Response},
    protocol::{Protocol, halfblocks::Halfblocks, kitty::Kitty, sixel::Sixel},
};
#[cfg(unix)]
use std::io::Write;
use std::sync::Arc;

/// A compact native upload, or the library encoder for negotiated/fallback paths.
#[derive(Clone)]
pub enum Encoded {
    Native(NativeImage),
    Protocol(Protocol),
}

impl Encoded {
    pub(super) fn native_bytes(&self) -> Option<usize> {
        match self {
            Self::Native(image) => {
                Some(image.upload.len() + image.raw_upload.as_ref().map_or(0, |raw| raw.len()))
            }
            Self::Protocol(_) => None,
        }
    }
    /// Doctor requests an upload receipt without changing normal preview traffic.
    pub(super) fn request_receipt(&mut self) -> bool {
        if let Self::Native(image) = self {
            image.upload = image.upload.replace("q=2", "q=0").into();
            true
        } else {
            false
        }
    }

    pub fn size(&self) -> Size {
        match self {
            Self::Native(image) => image.size,
            Self::Protocol(protocol) => protocol.size(),
        }
    }
}

pub struct NativeImage {
    id: u32,
    size: Size,
    pixels: [u32; 2],
    upload: Arc<str>,
    raw_upload: Option<Arc<Vec<u8>>>,
    inline_compression: bool,
    local: Option<Arc<LocalImage>>,
    transmitted: std::sync::atomic::AtomicBool,
}

impl Clone for NativeImage {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            size: self.size,
            pixels: self.pixels,
            upload: self.upload.clone(),
            raw_upload: self.raw_upload.clone(),
            inline_compression: self.inline_compression,
            local: self.local.clone(),
            // A retained frame must upload again into the inactive terminal slot.
            transmitted: false.into(),
        }
    }
}

impl NativeImage {
    fn new(
        image: DynamicImage,
        size: Size,
        id: u32,
        compressed: bool,
        local: Option<&Arc<LocalTransfer>>,
    ) -> Result<Self> {
        let pixels = [image.width(), image.height()];
        let rgba = image.into_rgba8();
        Self::new_shared(
            Arc::new(rgba.into_raw()),
            pixels,
            size,
            id,
            compressed,
            local,
        )
    }

    fn new_shared(
        rgba: Arc<Vec<u8>>,
        pixels: [u32; 2],
        size: Size,
        id: u32,
        compressed: bool,
        local: Option<&Arc<LocalTransfer>>,
    ) -> Result<Self> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use image::{
            ImageEncoder,
            codecs::png::{CompressionType, FilterType, PngEncoder},
        };
        use std::fmt::Write as _;
        if let Some(transfer) = local
            && let Some(local) = transfer.store_raw(rgba.as_slice())
        {
            return Ok(Self {
                id,
                size,
                pixels,
                upload: format!(
                    "\x1b_Ga=t,f=32,i={id},s={},v={},q=2,m=0;",
                    pixels[0], pixels[1]
                )
                .into(),
                raw_upload: Some(rgba),
                inline_compression: compressed,
                local: Some(local),
                transmitted: false.into(),
            });
        }
        // Inline opaque uploads omit alpha losslessly. Local file transfers
        // keep RGBA directly, avoiding a second window-sized conversion buffer.
        let rgb = (local.is_none()
            && rgba
                .as_slice()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255))
        .then(|| {
            let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
            for pixel in rgba.as_slice().as_chunks::<4>().0 {
                rgb.extend_from_slice(&pixel[..3]);
            }
            rgb
        });
        let format = if !compressed {
            100
        } else if rgb.is_some() {
            24
        } else {
            32
        };
        let payload = if compressed {
            use std::io::Write as _;
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder
                .write_all(rgb.as_deref().unwrap_or(rgba.as_slice()))
                .and_then(|()| encoder.finish())
                .map_err(|error| {
                    Error::new(
                        "RENDER_ERROR",
                        format!("Cannot compress terminal pixels: {error}"),
                    )
                })?
        } else {
            let mut png = Vec::new();
            PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub)
                .write_image(
                    rgb.as_deref().unwrap_or(rgba.as_slice()),
                    pixels[0],
                    pixels[1],
                    if rgb.is_some() {
                        image::ExtendedColorType::Rgb8
                    } else {
                        image::ExtendedColorType::Rgba8
                    },
                )
                .map_err(|error| {
                    Error::new(
                        "RENDER_ERROR",
                        format!("Cannot encode terminal PNG: {error}"),
                    )
                })?;
            png
        };
        // Local terminals can read exact RGBA without decoding a compressed
        // viewport on their input thread. Keep the prepared inline fallback.
        let local = local.and_then(|transfer| {
            transfer.store(&payload, compressed.then_some(rgba.as_slice().len()))
        });
        drop(rgb);
        drop(rgba);
        // Encode pixels and packet chunks on the worker. Native placements need
        // no Unicode-placeholder grid, even when ZLIB was negotiated.
        let mut upload = String::new();
        let chunks = payload.chunks(3072);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            upload.push_str("\x1b_G");
            if index == 0 {
                let _ = write!(
                    upload,
                    "a=t,f={format},i={id},s={},v={},",
                    pixels[0], pixels[1]
                );
                if compressed {
                    upload.push_str("o=z,");
                }
            }
            let _ = write!(upload, "q=2,m={};", usize::from(index + 1 < count));
            STANDARD.encode_string(chunk, &mut upload);
            upload.push_str("\x1b\\");
        }
        Ok(Self {
            id,
            size,
            pixels,
            upload: upload.into(),
            raw_upload: None,
            inline_compression: compressed,
            local,
            transmitted: false.into(),
        })
    }

    fn upload(&self, target: u32) -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        if let Some(local) = &self.local
            && let Some(link) = local.link()
        {
            let path = link.path;
            if let Some(path_text) = path.to_str() {
                // The terminal owns this link. The cached original can be
                // evicted without racing the terminal's asynchronous file read.
                let mut control = self
                    .upload
                    .split_once(';')
                    .expect("Kitty upload control")
                    .0
                    .replacen(&format!("i={},", self.id), &format!("i={target},"), 1)
                    .replace("m=1", "m=0");
                if link.raw {
                    control = control.replace("o=z,", "");
                }
                return format!("{control},t=t;{}\x1b\\", STANDARD.encode(path_text));
            }
            let _ = std::fs::remove_file(path);
        }
        if let Some(raw) = &self.raw_upload {
            // Local-file failure must retain the negotiated compact inline
            // transport. Build it only on this exceptional path.
            let bitmap =
                image::RgbaImage::from_raw(self.pixels[0], self.pixels[1], (**raw).clone())
                    .unwrap();
            if let Ok(mut fallback) = Self::new(
                bitmap.into(),
                self.size,
                self.id,
                self.inline_compression,
                None,
            ) {
                if self.upload.contains("q=0") {
                    fallback.upload = fallback.upload.replace("q=2", "q=0").into();
                }
                return fallback.upload(target);
            }
            use std::fmt::Write as _;
            let control = self
                .upload
                .split_once(';')
                .expect("Raw Kitty upload control")
                .0
                .replacen(&format!("i={},", self.id), &format!("i={target},"), 1);
            let mut upload = String::new();
            let chunks = raw.chunks(3072);
            let count = chunks.len();
            for (index, chunk) in chunks.enumerate() {
                if index == 0 {
                    upload.push_str(
                        &control.replace("m=0", &format!("m={}", usize::from(count > 1))),
                    );
                    upload.push(';');
                } else {
                    let _ = write!(upload, "\x1b_Gq=2,m={};", usize::from(index + 1 < count));
                }
                STANDARD.encode_string(chunk, &mut upload);
                upload.push_str("\x1b\\");
            }
            return upload;
        }
        self.upload
            .replacen(&format!("i={},", self.id), &format!("i={target},"), 1)
    }
}

/// Bind a Kitty virtual placement to the same cell grid as its placeholders.
pub struct Image<'a>(pub &'a Protocol);

impl Widget for Image<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        ratatui_image::Image::new(self.0).render(area, buffer);
        if matches!(self.0, Protocol::Kitty(_))
            && !area.is_empty()
            && let Some(cell) = buffer.cell_mut((area.x, area.y))
            && let Some((before, after)) = cell.symbol().split_once("a=T,U=1,")
        {
            // ratatui-image 11.1 leaves c/r unspecified. Kitty then chooses the
            // image's native pixel size, which can differ from the placeholder
            // grid after font/DPI changes or missing terminal pixel metrics.
            // Enrich its first transmission without adding a second placement
            // (unqualified a=p creates another, ambiguously selected placement).
            let size = self.0.size();
            let symbol = format!("{before}a=T,U=1,c={},r={},{after}", size.width, size.height);
            cell.set_symbol(&symbol);
        }
        if matches!(self.0, Protocol::Kitty(_))
            && crossterm::style::Colored::ansi_color_disabled_memoized()
        {
            // Kitty encodes image IDs in foreground RGB, even in a colorless UI.
            // Crossterm suppresses those SGR fields with NO_COLOR. Emit only this
            // protocol metadata explicitly, and restore the default foreground
            // after each cell so ordinary text still respects NO_COLOR.
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    if let Some(cell) = buffer.cell_mut((x, y))
                        && cell.symbol().contains('\u{10eeee}')
                        && let ratatui::style::Color::Rgb(r, g, b) = cell.fg
                    {
                        cell.set_symbol(&format!(
                            "\x1b[38;2;{r};{g};{b}m{}\x1b[39m",
                            cell.symbol()
                        ));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativePlacement {
    family: usize,
    image_id: u32,
    placement_id: u32,
    area: Rect,
    crop: Option<[u32; 4]>,
}

/// Present new Kitty frames in an inactive slot before replacing their cells.
#[derive(Default)]
pub(super) struct Presentation {
    slots: [Option<(u32, u32)>; 2],
    visible: Option<u32>,
    rectangular: bool,
    pending: [Option<String>; 2],
    desired: Option<NativePlacement>,
    shown: Vec<NativePlacement>,
    source_sizes: [Option<[u32; 2]>; 2],
    replay: bool,
    serial: u64,
}

impl Presentation {
    pub(super) fn serial(&self) -> u64 {
        self.serial
    }

    pub(super) fn rectangular(&mut self, enabled: bool) {
        self.rectangular = enabled;
    }

    pub(super) fn source_size(&self) -> Option<[u32; 2]> {
        (self.rectangular && self.slots[0].is_some() && self.pending[0].is_none())
            .then(|| self.source_sizes[0])
            .flatten()
    }

    pub(super) fn begin(&mut self) {
        self.desired = None;
    }

    pub(super) fn invalidate(&mut self) {
        self.replay = true;
    }

    pub(super) fn render(&mut self, buffer: &mut Buffer, area: Rect, image: &Encoded) {
        let image = match image {
            Encoded::Native(image) => image,
            Encoded::Protocol(protocol) => {
                Image(protocol).render(area, buffer);
                self.prepare(buffer, area, false);
                return;
            }
        };
        if area.is_empty() {
            return;
        }
        if !image
            .transmitted
            .swap(true, std::sync::atomic::Ordering::Relaxed)
            || self.slots[0].is_none_or(|(source, _)| source != image.id)
        {
            let slot = self.visible.map_or(0, |id| (id & 1) ^ 1);
            let target = (image.id & !1).max(2) | slot;
            self.slots[0] = Some((image.id, target));
            self.source_sizes[0] = Some(image.pixels);
            self.pending[0] = Some(image.upload(target));
        }
        if let Some((_, image_id)) = self.slots[0] {
            // Keep completed pixels at their original scale while a resized
            // viewport renders. Center the old frame and crop any overflow.
            let width = area.width.min(image.size.width);
            let height = area.height.min(image.size.height);
            let crop_width =
                (image.pixels[0] * u32::from(width) / u32::from(image.size.width)).max(1);
            let crop_height =
                (image.pixels[1] * u32::from(height) / u32::from(image.size.height)).max(1);
            self.desired = Some(NativePlacement {
                family: 0,
                image_id,
                placement_id: 1,
                area: Rect::new(
                    area.x + (area.width - width) / 2,
                    area.y + (area.height - height) / 2,
                    width,
                    height,
                ),
                crop: Some([
                    (image.pixels[0] - crop_width) / 2,
                    (image.pixels[1] - crop_height) / 2,
                    crop_width,
                    crop_height,
                ]),
            });
        }
        for position in area.positions() {
            if let Some(cell) = buffer.cell_mut(position) {
                let background = cell.bg;
                cell.reset();
                cell.set_bg(background);
            }
        }
    }

    pub(super) fn prepare(&mut self, buffer: &mut Buffer, area: Rect, camera: bool) {
        const PLACEHOLDER: char = '\u{10eeee}';
        let family = usize::from(camera);
        let transmission = area.positions().find_map(|position| {
            let cell = buffer.cell(position)?;
            let symbol = cell.symbol();
            if !symbol.contains("a=T,U=1,") {
                return None;
            }
            symbol
                .split(',')
                .find_map(|field| field.strip_prefix("i=")?.parse::<u32>().ok())
        });
        if let Some(source) = transmission {
            let slot = self.visible.map_or(0, |id| (id & 1) ^ 1);
            self.slots[family] = Some((source, (source & !1).max(2) | slot));
        }
        let Some((source, target)) = self.slots[family] else {
            return;
        };
        let [_, sr, sg, sb] = source.to_be_bytes();
        let [_, tr, tg, tb] = target.to_be_bytes();
        let mut visible = false;
        for position in area.positions() {
            let Some(cell) = buffer.cell_mut(position) else {
                continue;
            };
            if !cell.symbol().contains(PLACEHOLDER) {
                continue;
            }
            visible = true;
            if source != target {
                // The slot changes only the low ID bit. The high-byte diacritic
                // stays intact, including the temporary camera-frame marker.
                let symbol = cell
                    .symbol()
                    .replacen(&format!("i={source},"), &format!("i={target},"), 1)
                    .replace(
                        &format!("\x1b[38;2;{sr};{sg};{sb}m"),
                        &format!("\x1b[38;2;{tr};{tg};{tb}m"),
                    );
                cell.set_symbol(&symbol)
                    .set_fg(ratatui::style::Color::Rgb(tr, tg, tb));
            }
        }
        if visible && self.rectangular {
            if transmission.is_some() {
                let symbol = buffer[(area.x, area.y)].symbol();
                if let Some((upload, _)) = symbol.split_once(PLACEHOLDER) {
                    // Regular Kitty uploads have no virtual placement controls.
                    // Keep the packet chunks, compression, dimensions and ID.
                    if let Some((before, packet)) = upload.split_once("\x1b_G")
                        && let Some((control, payload)) = packet.split_once(';')
                    {
                        let dimension = |name: &str| {
                            control
                                .split(',')
                                .find_map(|field| field.strip_prefix(name)?.parse::<u32>().ok())
                        };
                        self.source_sizes[family] = dimension("s=")
                            .zip(dimension("v="))
                            .map(|(width, height)| [width, height]);
                        let control = control
                            .split(',')
                            .filter(|field| {
                                !field.starts_with("U=")
                                    && !field.starts_with("c=")
                                    && !field.starts_with("r=")
                            })
                            .map(|field| if field == "a=T" { "a=t" } else { field })
                            .collect::<Vec<_>>()
                            .join(",");
                        self.pending[family] = Some(format!("{before}\x1b_G{control};{payload}"));
                    }
                }
            }
            self.desired = Some(NativePlacement {
                family,
                image_id: target,
                placement_id: 1,
                area,
                crop: None,
            });
            for position in area.positions() {
                if let Some(cell) = buffer.cell_mut(position)
                    && cell.symbol().contains(PLACEHOLDER)
                {
                    cell.reset();
                }
            }
        } else if visible {
            self.visible = Some(target);
        }
    }

    pub(super) fn finish(&mut self, buffer: &mut Buffer, modal: bool) {
        if !self.rectangular {
            return;
        }
        self.finish_with_overlay(buffer, modal, None);
    }

    pub(super) fn finish_with_overlay(
        &mut self,
        buffer: &mut Buffer,
        modal: bool,
        overlay: Option<Rect>,
    ) {
        if !self.rectangular {
            return;
        }
        let desired = self
            .desired
            .filter(|_| !modal)
            .map(|placement| {
                let Some(overlay) = overlay.filter(|overlay| placement.area.intersects(*overlay))
                else {
                    return vec![placement];
                };
                let area = placement.area;
                let covered = area.intersection(overlay);
                let [sx, sy, sw, sh] = placement.crop.unwrap_or_else(|| {
                    let [width, height] = self.source_sizes[placement.family]
                        .expect("Native placement has source dimensions");
                    [0, 0, width, height]
                });
                let x = |column: u16| u32::from(column - area.x) * sw / u32::from(area.width);
                let y = |row: u16| u32::from(row - area.y) * sh / u32::from(area.height);
                [
                    Rect::new(area.x, area.y, area.width, covered.y - area.y),
                    Rect::new(
                        area.x,
                        covered.bottom(),
                        area.width,
                        area.bottom() - covered.bottom(),
                    ),
                    Rect::new(area.x, covered.y, covered.x - area.x, covered.height),
                    Rect::new(
                        covered.right(),
                        covered.y,
                        area.right() - covered.right(),
                        covered.height,
                    ),
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(index, part)| {
                    if part.is_empty() {
                        return None;
                    }
                    Some(NativePlacement {
                        area: part,
                        placement_id: index as u32 + 1,
                        crop: Some([
                            sx + x(part.x),
                            sy + y(part.y),
                            x(part.right()) - x(part.x),
                            y(part.bottom()) - y(part.y),
                        ]),
                        ..placement
                    })
                })
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut commands = String::new();
        let mut shown = Vec::new();
        for placement in desired {
            let upload = self.pending[placement.family].take();
            let replace = self.replay || upload.is_some() || !self.shown.contains(&placement);
            if let Some(upload) = upload {
                commands.push_str(&upload);
            }
            if replace {
                self.serial = self.serial.wrapping_add(1);
                use std::fmt::Write as _;
                let area = placement.area;
                let _ = write!(
                    commands,
                    "\x1b[{};{}H\x1b_Ga=p,i={},p={},c={},r={},C=1,q=2",
                    area.y + 1,
                    area.x + 1,
                    placement.image_id,
                    placement.placement_id,
                    area.width,
                    area.height
                );
                if let Some([x, y, width, height]) = placement.crop {
                    let _ = write!(commands, ",x={x},y={y},w={width},h={height}");
                }
                commands.push_str(";\x1b\\");
            }
            shown.push(placement);
            self.visible = Some(placement.image_id);
        }
        for previous in self.shown.iter().filter(|previous| {
            !shown.iter().any(|next| {
                (next.image_id, next.placement_id) == (previous.image_id, previous.placement_id)
            })
        }) {
            use std::fmt::Write as _;
            let _ = write!(
                commands,
                "\x1b_Ga=d,d=i,i={},p={},q=2;\x1b\\",
                previous.image_id, previous.placement_id
            );
        }
        self.shown = shown;
        self.replay = false;
        if !commands.is_empty() {
            let origin = buffer.area.as_position();
            if let Some(cell) = buffer.cell_mut(origin) {
                // Emit after modal composition, independently of image-cell
                // diffs. Save the backend's cursor and SGR state around I/O.
                cell.set_symbol(&format!("\x1b7{commands}\x1b8{}", cell.symbol()));
                cell.set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
                    std::num::NonZeroU16::MIN,
                ));
            }
        }
    }
}

#[derive(Clone)]
pub struct Backend {
    font: FontSize,
    protocol: ProtocolType,
    tmux: bool,
    kitty_compression: bool,
    local_transfer: Option<Arc<LocalTransfer>>,
    background: image::Rgba<u8>,
    background_known: bool,
}
impl Backend {
    #[cfg(test)]
    pub(super) fn native_for_test() -> Self {
        Self {
            protocol: ProtocolType::Kitty,
            kitty_compression: true,
            ..Self::halfblocks()
        }
    }
    pub fn halfblocks() -> Self {
        Self {
            font: FontSize::new(10, 20),
            protocol: ProtocolType::Halfblocks,
            tmux: false,
            kitty_compression: false,
            local_transfer: None,
            background: image::Rgba([248, 250, 252, 255]),
            background_known: false,
        }
    }
    pub fn detect(selected: super::ImageProtocol) -> Result<(Self, String)> {
        let mut backend = Self::halfblocks();
        backend.tmux = std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
            || std::env::var("TERM_PROGRAM").is_ok_and(|term| term == "tmux");
        if let Ok(window) = crossterm::terminal::window_size()
            && window.columns > 0
            && window.rows > 0
        {
            let width = window.width / window.columns;
            let height = window.height / window.rows;
            if (1..=256).contains(&width) && (1..=256).contains(&height) {
                backend.font = FontSize::new(width, height);
            }
        }
        let direct_kitty = !backend.tmux
            && std::env::var("TERM").as_deref() == Ok("xterm-kitty")
            && std::env::var_os("WEZTERM_EXECUTABLE").is_none()
            && std::env::var_os("KONSOLE_VERSION").is_none();
        if direct_kitty {
            backend.protocol = ProtocolType::Kitty;
            // Direct Kitty supports ZLIB without a capability query. Other
            // compatible terminals opt in only after a positive probe reply.
            backend.kitty_compression = true;
        }
        // Explicit protocols and a direct Kitty terminal need no capability query.
        // A late APC reply otherwise reaches Crossterm as ordinary prompt text.
        let capabilities = selected == super::ImageProtocol::Auto && !direct_kitty;
        // Query the background even for Kitty: compatible renderers do not
        // consistently composite transparent image pixels over terminal cells.
        let local_transfer = (!backend.tmux
            && std::env::var_os("SSH_CONNECTION").is_none()
            && std::env::var_os("SSH_TTY").is_none()
            && !matches!(
                selected,
                super::ImageProtocol::Halfblocks | super::ImageProtocol::Sixel
            ))
        .then(LocalTransfer::new)
        .and_then(std::result::Result::ok)
        .map(Arc::new);
        let (responses, startup, local_supported) =
            probe(backend.tmux, capabilities, local_transfer.as_deref())?;
        for response in &responses {
            if let Response::Background(r, g, b) = response {
                backend.background = image::Rgba([*r, *g, *b, 255]);
                backend.background_known = true;
            }
            if let Response::CellSize(Some((width, height))) = response
                && (1..=256).contains(width)
                && (1..=256).contains(height)
            {
                backend.font = FontSize::new(*width, *height);
            }
        }
        if capabilities && responses.contains(&Response::Sixel) {
            backend.protocol = ProtocolType::Sixel;
        }
        if capabilities
            && responses.contains(&Response::Kitty)
            && std::env::var_os("WEZTERM_EXECUTABLE").is_none()
            && std::env::var_os("KONSOLE_VERSION").is_none()
        {
            backend.protocol = ProtocolType::Kitty;
            backend.kitty_compression = responses.contains(&Response::KittyCompression);
        }
        backend.protocol = match selected {
            super::ImageProtocol::Kitty => ProtocolType::Kitty,
            super::ImageProtocol::Sixel => ProtocolType::Sixel,
            super::ImageProtocol::Halfblocks => ProtocolType::Halfblocks,
            super::ImageProtocol::Auto => backend.protocol,
        };
        if local_supported && backend.protocol == ProtocolType::Kitty {
            backend.local_transfer = local_transfer;
        }
        Ok((backend, startup))
    }
    pub fn font_size(&self) -> FontSize {
        self.font
    }
    pub fn protocol_type(&self) -> ProtocolType {
        self.protocol
    }
    pub fn tmux_detected(&self) -> bool {
        self.tmux
    }
    pub fn canvas_background(&self) -> image::Rgba<u8> {
        self.background
    }
    pub(super) fn with_canvas_background(mut self, background: image::Rgba<u8>) -> Self {
        self.background = background;
        self
    }
    pub fn terminal_background(&self) -> Option<[u8; 3]> {
        self.background_known.then_some([
            self.background[0],
            self.background[1],
            self.background[2],
        ])
    }
    pub fn encode_preview(&self, image: DynamicImage, size: Size, id: u32) -> Result<Encoded> {
        if self.protocol == ProtocolType::Kitty && !self.tmux {
            let image = self.resize(image, size)?;
            return NativeImage::new(
                image,
                size,
                id,
                self.kitty_compression,
                self.local_transfer.as_ref(),
            )
            .map(Encoded::Native);
        }
        self.encode(image, size, id).map(Encoded::Protocol)
    }

    pub(super) fn encode_preview_shared(
        &self,
        rgba: Arc<Vec<u8>>,
        pixels: [u32; 2],
        size: Size,
        id: u32,
    ) -> Result<Encoded> {
        let viewport = super::viewport::Viewport::new(self.font, size)?;
        if self.protocol == ProtocolType::Kitty && !self.tmux && pixels == viewport.raster {
            return NativeImage::new_shared(
                rgba,
                pixels,
                size,
                id,
                self.kitty_compression,
                self.local_transfer.as_ref(),
            )
            .map(Encoded::Native);
        }
        let rgba = Arc::try_unwrap(rgba).unwrap_or_else(|shared| (*shared).clone());
        let bitmap = image::RgbaImage::from_raw(pixels[0], pixels[1], rgba).unwrap();
        self.encode_preview(bitmap.into(), size, id)
    }

    fn resize(&self, mut image: DynamicImage, size: Size) -> Result<DynamicImage> {
        let [width, height] = super::viewport::Viewport::new(self.font, size)?.raster;
        if image.width() != width || image.height() != height {
            let resized = Resize::Fit(None).resize(
                &image,
                FontSize::new(1, 1),
                Size::new(width as u16, height as u16),
                Some(self.canvas_background()),
            );
            drop(image);
            image = resized;
        }
        Ok(image)
    }

    pub fn encode(&self, image: DynamicImage, size: Size, id: u32) -> Result<Protocol> {
        let viewport = super::viewport::Viewport::new(self.font, size)?;
        let image = self.resize(image, size)?;
        let result = match self.protocol {
            ProtocolType::Kitty => {
                Kitty::new(image, size, id, self.tmux, self.kitty_compression).map(Protocol::Kitty)
            }
            // Sixel has no general display-scaling operation. Keep the full
            // viewport with halfblocks rather than enlarging a bounded raster.
            ProtocolType::Sixel if viewport.scale == 1.0 => {
                Sixel::new(image, size, self.tmux).map(Protocol::Sixel)
            }
            _ => Halfblocks::new(image, size).map(Protocol::Halfblocks),
        };
        result.map_err(|error| {
            Error::new(
                "RENDER_ERROR",
                format!("Cannot encode terminal image: {error}"),
            )
        })
    }
}

#[cfg(unix)]
fn probe(
    tmux: bool,
    capabilities: bool,
    local: Option<&LocalTransfer>,
) -> Result<(Vec<Response>, String, bool)> {
    use std::{
        os::fd::AsRawFd,
        time::{Duration, Instant},
    };
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let original = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if original < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, original | libc::O_NONBLOCK) } < 0 {
        return Ok((vec![], String::new(), false));
    }
    struct Flags(i32, i32);
    impl Drop for Flags {
        fn drop(&mut self) {
            unsafe {
                libc::fcntl(self.0, libc::F_SETFL, self.1);
            }
        }
    }
    let _flags = Flags(fd, original);
    let mut query = probe_query(tmux, capabilities);
    let local_probe = local.and_then(|local| {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let mut file = tempfile::NamedTempFile::new_in(local.directory.path()).ok()?;
        file.write_all(&[0, 0, 0, 255]).ok()?;
        let path = file.path().to_str()?;
        query.push_str(&format!(
            "\x1b_Ga=q,t=t,f=32,s=1,v=1,i=2147483645;{}\x1b\\\x1b[6n",
            STANDARD.encode(path)
        ));
        Some(file)
    });
    let mut stdout = std::io::stdout();
    stdout
        .write_all(query.as_bytes())
        .map_err(super::terminal::io)?;
    stdout.flush().map_err(super::terminal::io)?;
    let deadline = Instant::now() + Duration::from_millis(350);
    let mut parser = Parser::new();
    let mut responses = vec![];
    let mut bytes = vec![];
    // Multiplexers may answer DSR before their asynchronous graphics reply.
    // Drain the existing bounded window instead of treating DSR as a barrier.
    while Instant::now() < deadline && bytes.len() < 65536 {
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut descriptor, 1, remaining.max(1)) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ready == 0 {
            break;
        }
        let mut input = [0u8; 256];
        let count = unsafe { libc::read(fd, input.as_mut_ptr().cast(), input.len()) };
        if count <= 0 {
            break;
        }
        let input = &input[..count as usize];
        bytes.extend_from_slice(input);
        for byte in input {
            responses.extend(parser.push(char::from(*byte)));
        }
    }
    // The dependency assumes 16-bit OSC components; terminals may use 1–4 hex
    // digits. Normalize these replies separately before choosing the background.
    responses.retain(|response| !matches!(response, Response::Background(..)));
    responses.extend(background_response(&bytes));
    let local_supported = local_probe.is_some() && local_transfer_response(&bytes);
    Ok((responses, startup_text(&bytes), local_supported))
}
#[cfg(not(unix))]
fn probe(_: bool, _: bool, _: Option<&LocalTransfer>) -> Result<(Vec<Response>, String, bool)> {
    Ok((vec![], String::new(), false))
}

#[cfg(unix)]
fn local_transfer_response(bytes: &[u8]) -> bool {
    bytes
        .windows(b"\x1b_Gi=2147483645;OK\x1b\\".len())
        .any(|part| part == b"\x1b_Gi=2147483645;OK\x1b\\")
}

#[cfg(unix)]
fn probe_query(tmux: bool, capabilities: bool) -> String {
    if capabilities {
        Parser::query(
            tmux,
            QueryStdioOptions {
                kitty_compression: true,
                terminal_background_color_osc: true,
                ..Default::default()
            },
        )
    } else {
        let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
        format!("{start}{escape}]11;?\x07{escape}[5n{end}")
    }
}

#[cfg(unix)]
fn background_response(bytes: &[u8]) -> Option<Response> {
    let prefix = b"\x1b]11;rgb:";
    bytes
        .windows(prefix.len())
        .enumerate()
        .find_map(|(index, window)| {
            if window != prefix {
                return None;
            }
            let color = &bytes[index + prefix.len()..];
            let end = color.iter().position(|byte| matches!(byte, 0x07 | 0x1b))?;
            if color[end] == 0x1b && !color[end..].starts_with(b"\x1b\\") {
                return None;
            }
            let color = std::str::from_utf8(&color[..end]).ok()?;
            let mut parts = color.split('/');
            let mut component = || {
                let part = parts.next()?;
                if !(1..=4).contains(&part.len())
                    || !part.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return None;
                }
                let value = u32::from_str_radix(part, 16).ok()?;
                Some((value * 255 / ((1 << (4 * part.len())) - 1)) as u8)
            };
            let (r, g, b) = (component()?, component()?, component()?);
            parts
                .next()
                .is_none()
                .then_some(Response::Background(r, g, b))
        })
}

#[cfg(unix)]
fn startup_text(bytes: &[u8]) -> String {
    let mut retained = vec![];
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b]11;") {
            // Consume complete or partial color replies, never their RGB payload
            // as prompt text. Keep ordinary UTF-8 typed around the probe intact.
            index += 5;
            while index < bytes.len() && !matches!(bytes[index], 0x07 | 0x1b) {
                index += 1;
            }
            if bytes.get(index) == Some(&0x07) {
                index += 1;
            } else if bytes[index..].starts_with(b"\x1b\\") {
                index += 2;
            }
            continue;
        } else if bytes[index..].starts_with(b"\x1b_G") {
            if let Some(end) = bytes[index + 3..]
                .windows(2)
                .position(|window| window == b"\x1b\\")
            {
                index += 3 + end + 2;
                continue;
            }
        } else if bytes[index..].starts_with(b"\x1b[")
            && let Some(end) = bytes[index + 2..]
                .iter()
                .position(|byte| (0x40..=0x7e).contains(byte))
        {
            let command = bytes[index + 2 + end];
            if matches!(command, b'c' | b't' | b'n' | b'R') {
                index += 3 + end;
                continue;
            }
        }
        retained.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&retained)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
#[path = "../../xtask/src/test_support/terminal.rs"]
#[allow(dead_code)]
mod observer;

#[cfg(test)]
#[path = "graphics_tests.rs"]
mod tests;
