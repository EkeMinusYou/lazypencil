//! Bounded terminal probing without a detached stdin reader or tmux configuration changes.

use crate::{Error, Result};
use image::DynamicImage;
use ratatui::layout::Size;
#[cfg(unix)]
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions};
use ratatui_image::{
    FontSize, Resize,
    picker::{ProtocolType, cap_parser::Response},
    protocol::{Protocol, halfblocks::Halfblocks, kitty::Kitty, sixel::Sixel},
};
#[cfg(unix)]
use std::io::Write;

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
            // Direct Kitty supports ZLIB. Keep other Kitty-compatible terminals
            // uncompressed without adding a late capability reply to stdin.
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
    pub fn encode(&self, image: DynamicImage, size: Size, id: u32) -> Result<Protocol> {
        let image = Resize::Fit(None).resize(
            &image,
            self.font,
            size,
            Some(image::Rgba([248, 250, 252, 255])),
        );
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
    let query = Parser::query(tmux, QueryStdioOptions::default());
    let mut stdout = std::io::stdout();
    stdout
        .write_all(query.as_bytes())
        .map_err(super::terminal::io)?;
    stdout.flush().map_err(super::terminal::io)?;
    let deadline = Instant::now() + Duration::from_millis(350);
    let mut parser = Parser::new();
    let mut responses = vec![];
    let mut bytes = vec![];
    'query: while Instant::now() < deadline && bytes.len() < 65536 {
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
        let mut complete = false;
        for byte in input {
            let values = parser.push(char::from(*byte));
            complete |= values.contains(&Response::Status);
            responses.extend(values);
        }
        if complete {
            break 'query;
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
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

    #[test]
    fn compressed_and_raw_kitty_transfers_preserve_identical_rgba_without_changing_placement() {
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 127],
            [0, 0, 255, 0],
            [18, 52, 86, 200],
        ];
        let source = image::RgbaImage::from_fn(64, 32, |x, y| {
            image::Rgba(colors[((x / 8 + y / 4) % 4) as usize])
        });
        let area = Rect::new(0, 0, 64, 32);
        let size = Size::new(64, 32);
        let mut lengths = Vec::new();
        for compressed in [false, true] {
            let backend = Backend {
                font: FontSize::new(1, 1),
                protocol: ProtocolType::Kitty,
                tmux: false,
                kitty_compression: compressed,
            };
            let protocol = backend
                .encode(DynamicImage::ImageRgba8(source.clone()), size, 77)
                .unwrap();
            assert_eq!(protocol.size(), size);
            let mut buffer = Buffer::empty(area);
            ratatui_image::Image::new(&protocol).render(area, &mut buffer);
            let mut wire = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    wire.push_str(buffer[(x, y)].symbol());
                }
            }
            assert_eq!(wire.contains("o=z,"), compressed);
            let mut decoder = observer::KittyFrames::default();
            for bytes in wire.as_bytes().chunks(137) {
                decoder.feed(bytes).unwrap();
            }
            assert_eq!(decoder.images.len(), 1);
            assert_eq!(decoder.compressed_frames, usize::from(compressed));
            assert_eq!(
                (decoder.images[0].width, decoder.images[0].height),
                (64, 32)
            );
            assert_eq!(&decoder.images[0].rgba, source.as_raw());
            lengths.push(wire.len());
        }
        assert!(
            lengths[1] < lengths[0],
            "Compressed transport must reduce bytes"
        );
    }
}
