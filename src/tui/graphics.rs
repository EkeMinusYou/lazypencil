//! Bounded terminal probing without a detached stdin reader or tmux configuration changes.

use crate::{Error, Result};
use image::DynamicImage;
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

/// A compact native upload, or the library encoder for negotiated/fallback paths.
pub enum Encoded {
    Native(NativeImage),
    Protocol(Protocol),
}

impl Encoded {
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
    upload: String,
    transmitted: std::sync::atomic::AtomicBool,
}

impl NativeImage {
    fn new(image: DynamicImage, size: Size, id: u32) -> Result<Self> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use image::{
            ImageEncoder,
            codecs::png::{CompressionType, FilterType, PngEncoder},
        };
        use std::fmt::Write as _;
        let pixels = [image.width(), image.height()];
        let rgba = image.into_rgba8();
        let mut png = Vec::new();
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub)
            .write_image(&rgba, pixels[0], pixels[1], image::ExtendedColorType::Rgba8)
            .map_err(|error| {
                Error::new(
                    "RENDER_ERROR",
                    format!("Cannot encode terminal PNG: {error}"),
                )
            })?;
        // PNG is a required Kitty pixel format, including when compression was
        // not probed. Encode on the renderer thread, never during UI drawing.
        let mut upload = String::new();
        let chunks = png.chunks(3072);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            upload.push_str("\x1b_G");
            if index == 0 {
                let _ = write!(upload, "a=t,f=100,i={id},s={},v={},", pixels[0], pixels[1]);
            }
            let _ = write!(upload, "q=2,m={};", usize::from(index + 1 < count));
            STANDARD.encode_string(chunk, &mut upload);
            upload.push_str("\x1b\\");
        }
        Ok(Self {
            id,
            size,
            pixels,
            upload,
            transmitted: false.into(),
        })
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
}

/// Present new Kitty frames in an inactive slot before replacing their cells.
#[derive(Default)]
pub(super) struct Presentation {
    slots: [Option<(u32, u32)>; 2],
    visible: Option<u32>,
    rectangular: bool,
    pending: [Option<String>; 2],
    desired: Option<NativePlacement>,
    shown: Option<NativePlacement>,
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
        {
            let slot = self.visible.map_or(0, |id| (id & 1) ^ 1);
            let target = (image.id & !1).max(2) | slot;
            self.slots[0] = Some((image.id, target));
            self.source_sizes[0] = Some(image.pixels);
            self.pending[0] = Some(image.upload.replacen(
                &format!("i={},", image.id),
                &format!("i={target},"),
                1,
            ));
        }
        if let Some((_, image_id)) = self.slots[0] {
            self.desired = Some(NativePlacement {
                family: 0,
                image_id,
                placement_id: 1,
                area,
            });
        }
        for position in area.positions() {
            if let Some(cell) = buffer.cell_mut(position) {
                cell.reset();
                cell.set_bg(ratatui::style::Color::Rgb(248, 250, 252));
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
            });
            for position in area.positions() {
                if let Some(cell) = buffer.cell_mut(position)
                    && cell.symbol().contains(PLACEHOLDER)
                {
                    cell.reset();
                    cell.set_bg(ratatui::style::Color::Rgb(248, 250, 252));
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
        let desired = self.desired.filter(|_| !modal);
        let mut commands = String::new();
        let mut shown = None;
        if let Some(placement) = desired {
            let upload = self.pending[placement.family].take();
            let replace = self.replay || upload.is_some() || self.shown != Some(placement);
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
                commands.push_str(";\x1b\\");
            }
            shown = Some(placement);
            self.visible = Some(placement.image_id);
        }
        if let Some(previous) = self.shown
            && shown.is_none_or(|next| {
                (next.image_id, next.placement_id) != (previous.image_id, previous.placement_id)
            })
        {
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
}
impl Backend {
    pub fn halfblocks() -> Self {
        Self {
            font: FontSize::new(10, 20),
            protocol: ProtocolType::Halfblocks,
            tmux: false,
            kitty_compression: false,
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
        let mut startup = String::new();
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
        if selected == super::ImageProtocol::Auto && !direct_kitty {
            let (responses, input) = probe(backend.tmux)?;
            startup = input;
            for response in &responses {
                if let Response::CellSize(Some((width, height))) = response
                    && (1..=256).contains(width)
                    && (1..=256).contains(height)
                {
                    backend.font = FontSize::new(*width, *height);
                }
            }
            if responses.contains(&Response::Sixel) {
                backend.protocol = ProtocolType::Sixel;
            }
            if responses.contains(&Response::Kitty)
                && std::env::var_os("WEZTERM_EXECUTABLE").is_none()
                && std::env::var_os("KONSOLE_VERSION").is_none()
            {
                backend.protocol = ProtocolType::Kitty;
                backend.kitty_compression = responses.contains(&Response::KittyCompression);
            }
        }
        backend.protocol = match selected {
            super::ImageProtocol::Kitty => ProtocolType::Kitty,
            super::ImageProtocol::Sixel => ProtocolType::Sixel,
            super::ImageProtocol::Halfblocks => ProtocolType::Halfblocks,
            super::ImageProtocol::Auto => backend.protocol,
        };
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
    pub fn encode_preview(&self, image: DynamicImage, size: Size, id: u32) -> Result<Encoded> {
        if self.protocol == ProtocolType::Kitty && !self.tmux && !self.kitty_compression {
            let image = self.resize(image, size)?;
            return NativeImage::new(image, size, id).map(Encoded::Native);
        }
        self.encode(image, size, id).map(Encoded::Protocol)
    }

    fn resize(&self, mut image: DynamicImage, size: Size) -> Result<DynamicImage> {
        let [width, height] = super::viewport::Viewport::new(self.font, size)?.raster;
        if image.width() != width || image.height() != height {
            let resized = Resize::Fit(None).resize(
                &image,
                FontSize::new(1, 1),
                Size::new(width as u16, height as u16),
                Some(image::Rgba([248, 250, 252, 255])),
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
fn probe(tmux: bool) -> Result<(Vec<Response>, String)> {
    use std::{
        os::fd::AsRawFd,
        time::{Duration, Instant},
    };
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let original = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if original < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, original | libc::O_NONBLOCK) } < 0 {
        return Ok((vec![], String::new()));
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
    let query = Parser::query(
        tmux,
        QueryStdioOptions {
            kitty_compression: true,
            ..Default::default()
        },
    );
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
    Ok((responses, startup_text(&bytes)))
}
#[cfg(not(unix))]
fn probe(_: bool) -> Result<(Vec<Response>, String)> {
    Ok((vec![], String::new()))
}

#[cfg(unix)]
fn startup_text(bytes: &[u8]) -> String {
    let mut retained = vec![];
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b_G") {
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
            if matches!(command, b'c' | b't' | b'n') {
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
