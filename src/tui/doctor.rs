//! Automatic image checks using the same encoder and presenter as the TUI.
use super::{
    ImageProtocol,
    graphics::{Encoded, Presentation},
    terminal,
};
use crate::{Error, Result};
use ratatui::{
    layout::{Rect, Size},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::{Value, json};
use std::io::Write;
#[cfg(unix)]
use std::time::{Duration, Instant};

pub(crate) fn check_preview(png: &[u8], protocol: ImageProtocol) -> Result<Value> {
    let id = (uuid::Uuid::new_v4().as_u128() as u32 & 0x3fff_fffe).max(2);
    let mut screen =
        terminal::Screen::enter(protocol, id, false, crate::config::keys::Theme::Auto)?;
    let size = screen.terminal.size().map_err(terminal::io)?;
    if size.width < 24 || size.height < 12 {
        return Err(Error::new(
            "PREVIEW_UNAVAILABLE",
            "Terminal is too small for the automatic image test (24x12 minimum)",
        ));
    }
    let cells = Size::new(
        24.min(size.width.saturating_sub(4)),
        12.min(size.height.saturating_sub(7)),
    );
    let mut encoded = screen.picker.encode_preview(
        image::load_from_memory(png).map_err(|_| {
            Error::new(
                "RENDER_ERROR",
                "Cannot decode the test PNG for terminal encoding",
            )
        })?,
        cells,
        id,
    )?;
    let receipt = encoded.request_receipt();
    let native = screen.picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
        && !screen.picker.tmux_detected();
    let negotiated = format!("{:?}", screen.picker.protocol_type()).to_lowercase();
    let protocol = match &encoded {
        Encoded::Native(_) => "kitty",
        Encoded::Protocol(ratatui_image::protocol::Protocol::Kitty(_)) => "kitty",
        Encoded::Protocol(ratatui_image::protocol::Protocol::Sixel(_)) => "sixel",
        Encoded::Protocol(ratatui_image::protocol::Protocol::Halfblocks(_)) => "halfblocks",
        Encoded::Protocol(ratatui_image::protocol::Protocol::ITerm2(_)) => "iterm2",
    };
    let mut presentation = Presentation::default();
    presentation.rectangular(native);
    let mut area = Rect::default();
    screen.draw(|frame| {
        let outer = frame.area();
        frame.render_widget(
            Block::default()
                .title("lazypencil doctor · automatic preview check")
                .borders(Borders::ALL),
            outer,
        );
        frame.render_widget(
            Paragraph::new(
                "Rendering and transmitting a known test image. No confirmation is needed.",
            ),
            Rect::new(2, 1, outer.width.saturating_sub(4), 2),
        );
        area = Rect::new(
            (outer.width - cells.width) / 2,
            4,
            cells.width,
            cells.height,
        );
        presentation.begin();
        presentation.render(frame.buffer_mut(), area, &encoded);
        presentation.finish(frame.buffer_mut(), false);
    })?;
    let acknowledgement = if receipt {
        // Ask for a placement receipt as well as an upload receipt. The explicit
        // placement ID replaces the presenter's placement instead of adding one.
        let mut stdout = std::io::stdout();
        write!(
            stdout,
            "\x1b7\x1b[{};{}H\x1b_Ga=p,i={id},p=1,c={},r={},C=1,q=0;\x1b\\\x1b8",
            area.y + 1,
            area.x + 1,
            area.width,
            area.height
        )
        .map_err(terminal::io)?;
        stdout.flush().map_err(terminal::io)?;
        read_receipts(id)?
    } else {
        json!({"transport_status":"unconfirmed","upload_receipt":false,"placement_receipt":false})
    };
    let mut result = acknowledgement;
    result["protocol"] = json!(protocol);
    result["negotiated_protocol"] = json!(negotiated);
    result["encoded"] = json!(true);
    result["written"] = json!(true);
    result["physical_visibility"] = json!("not_verified");
    result["native_image_protocol"] = json!(protocol != "halfblocks");
    drop(screen);
    Ok(result)
}

#[cfg(unix)]
fn read_receipts(id: u32) -> Result<Value> {
    use std::os::fd::AsRawFd;
    let fd = std::io::stdin().as_raw_fd();
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut bytes = Vec::new();
    let mut upload = None;
    let mut placement = None;
    while Instant::now() < deadline && bytes.len() < 65536 {
        crate::cancel::check(Some(&crate::cancel::token()))?;
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 50) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(terminal::io(std::io::Error::last_os_error()));
        }
        if ready == 0 {
            continue;
        }
        let mut input = [0u8; 256];
        let count = unsafe { libc::read(fd, input.as_mut_ptr().cast(), input.len()) };
        if count <= 0 {
            break;
        }
        if input[..count as usize].contains(&3) {
            return Err(Error::new("CANCELLED", "Preview check cancelled"));
        }
        bytes.extend_from_slice(&input[..count as usize]);
        for (header, body) in replies(&bytes) {
            let mut reply_id = None;
            let mut reply_placement = None;
            for field in header.split(',') {
                if let Some(value) = field.strip_prefix("i=") {
                    reply_id = value.parse::<u32>().ok();
                }
                if let Some(value) = field.strip_prefix("p=") {
                    reply_placement = value.parse::<u32>().ok();
                }
            }
            if reply_id != Some(id) {
                continue;
            }
            if reply_placement == Some(1) {
                placement = Some(body == "OK");
            } else if reply_placement.is_none() {
                upload = Some(body == "OK");
            }
        }
        if upload.is_some() && placement.is_some() {
            break;
        }
    }
    Ok(
        json!({"transport_status":if upload==Some(false)||placement==Some(false) {"rejected"} else if upload==Some(true)&&placement==Some(true) {"accepted"} else {"unconfirmed"},"upload_receipt":upload,"placement_receipt":placement}),
    )
}

#[cfg(not(unix))]
fn read_receipts(_: u32) -> Result<Value> {
    Ok(json!({"transport_status":"unconfirmed","upload_receipt":false,"placement_receipt":false}))
}

#[cfg(unix)]
fn replies(bytes: &[u8]) -> impl Iterator<Item = (&str, &str)> {
    std::str::from_utf8(bytes)
        .unwrap_or("")
        .split("\x1b_G")
        .skip(1)
        .filter_map(|part| {
            let (reply, _) = part.split_once("\x1b\\")?;
            reply.split_once(';')
        })
}
