use crate::{Error, Result};
use crossterm::{
    cursor,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{Stdout, Write};

pub struct Screen {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub picker: super::graphics::Backend,
    pub startup_text: String,
    id: u32,
}
struct Restore {
    id: u32,
    tmux: bool,
}
impl Drop for Restore {
    fn drop(&mut self) {
        restore(self.id, self.tmux);
    }
}
impl Screen {
    pub fn camera_image_id(&self) -> u32 {
        self.id ^ 0x8000_0000
    }
    pub fn redraw(&mut self) -> Result<()> {
        // Terminal::clear queries the cursor and races EventStream's stdin reader.
        // A fullscreen resize also invalidates the back buffer without reading stdin.
        let area = self.terminal.size().map_err(io)?.into();
        self.terminal.resize(area).map_err(io)
    }
    pub fn enter(protocol: super::ImageProtocol, id: u32) -> Result<Self> {
        enable_raw_mode().map_err(io)?;
        let mut guard = Restore { id, tmux: false };
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )
        .map_err(io)?;
        let (picker, startup_text) = super::graphics::Backend::detect(protocol)?;
        guard.tmux = picker.tmux_detected();
        let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout())).map_err(io)?;
        std::mem::forget(guard);
        Ok(Self {
            terminal,
            picker,
            startup_text,
            id,
        })
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        restore(self.id, self.picker.tmux_detected());
    }
}
fn restore(id: u32, tmux: bool) {
    let mut stdout = std::io::stdout();
    let deletion = [id, id ^ 0x8000_0000]
        .map(|id| format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"))
        .concat();
    if tmux {
        let _ = write!(
            stdout,
            "\x1bPtmux;{}\x1b\\",
            deletion.replace('\x1b', "\x1b\x1b")
        );
    } else {
        let _ = stdout.write_all(deletion.as_bytes());
    }
    let _ = execute!(
        stdout,
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        cursor::Show
    );
    let _ = stdout.flush();
    let _ = disable_raw_mode();
}
pub fn io(error: std::io::Error) -> Error {
    Error::new("IO_ERROR", error.to_string())
}
