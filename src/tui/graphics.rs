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

/// Present new Kitty frames in an inactive slot before replacing their cells.
#[derive(Default)]
pub(super) struct Presentation {
    slots: [Option<(u32, u32)>; 2],
    visible: Option<u32>,
    rectangular: bool,
    pending: [Option<String>; 2],
    desired: Option<(usize, u32, Rect)>,
    shown: Option<(u32, Rect)>,
    replay: bool,
}

impl Presentation {
    pub(super) fn rectangular(&mut self, enabled: bool) {
        self.rectangular = enabled;
    }

    pub(super) fn begin(&mut self) {
        self.desired = None;
    }

    pub(super) fn invalidate(&mut self) {
        self.replay = true;
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
            self.desired = Some((family, target, area));
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
        let desired = self.desired.filter(|_| !modal);
        let mut commands = String::new();
        let mut shown = None;
        if let Some((family, id, area)) = desired {
            let upload = self.pending[family].take();
            let replace = self.replay || upload.is_some() || self.shown != Some((id, area));
            if let Some(upload) = upload {
                commands.push_str(&upload);
            }
            if replace {
                use std::fmt::Write as _;
                let _ = write!(
                    commands,
                    "\x1b[{};{}H\x1b_Ga=p,i={id},p=1,c={},r={},C=1,q=2;\x1b\\",
                    area.y + 1,
                    area.x + 1,
                    area.width,
                    area.height
                );
            }
            shown = Some((id, area));
            self.visible = Some(id);
        }
        if let Some((id, _)) = self.shown
            && shown.is_none_or(|(next, _)| next != id)
        {
            use std::fmt::Write as _;
            // Delete only the retired placement, keeping its pixels available
            // for slot reuse and restoration after a modal closes.
            let _ = write!(commands, "\x1b_Ga=d,d=i,i={id},p=1,q=2;\x1b\\");
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
    pub fn encode(&self, mut image: DynamicImage, size: Size, id: u32) -> Result<Protocol> {
        let width = u32::from(size.width) * u32::from(self.font.width);
        let height = u32::from(size.height) * u32::from(self.font.height);
        if image.width() != width || image.height() != height {
            let resized = Resize::Fit(None).resize(
                &image,
                self.font,
                size,
                Some(image::Rgba([248, 250, 252, 255])),
            );
            drop(image);
            image = resized;
        }
        let result = match self.protocol {
            ProtocolType::Kitty => {
                Kitty::new(image, size, id, self.tmux, self.kitty_compression).map(Protocol::Kitty)
            }
            ProtocolType::Sixel => Sixel::new(image, size, self.tmux).map(Protocol::Sixel),
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
