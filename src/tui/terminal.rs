use crate::{Error, Result};
use crossterm::{
    SynchronizedUpdate, cursor,
    event::{
        DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend, buffer::CellDiffOption};
use std::io::{Stdout, Write};

pub struct Screen {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub picker: super::graphics::Backend,
    pub startup_text: String,
    id: u32,
    force_redraw: bool,
    preview_keys: bool,
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
    pub fn redraw(&mut self) {
        self.force_redraw = true;
    }
    pub fn preview_keys(&mut self, enabled: bool) -> Result<()> {
        if self.preview_keys != enabled {
            // Text-producing keys need both flags to report press/repeat/release
            // directly. Keep that mode scoped to navigation so IME commits in
            // Prompt and node search retain their UTF-8 representation.
            execute!(
                std::io::stdout(),
                PopKeyboardEnhancementFlags,
                PushKeyboardEnhancementFlags(keyboard_flags(enabled))
            )
            .map_err(io)?;
            self.preview_keys = enabled;
        }
        Ok(())
    }
    pub fn draw(&mut self, draw: impl FnOnce(&mut ratatui::Frame)) -> Result<()> {
        // Keep text and image-slot replacement in one display update. Never
        // erase the physical screen merely to invalidate the drawing cache.
        std::io::stdout()
            .sync_update(|_| {
                if self.force_redraw {
                    // Invalidate the previous buffer without emitting a clear or
                    // querying the cursor while EventStream owns stdin.
                    self.terminal.swap_buffers();
                    for cell in &mut self.terminal.current_buffer_mut().content {
                        cell.set_diff_option(CellDiffOption::AlwaysUpdate);
                    }
                    self.terminal.swap_buffers();
                    self.force_redraw = false;
                }
                self.terminal.draw(draw).map(|_| ()).map_err(io)
            })
            .map_err(io)?
    }
    pub fn enter(protocol: super::ImageProtocol, id: u32) -> Result<Self> {
        enable_raw_mode().map_err(io)?;
        let mut guard = Restore { id, tmux: false };
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange,
            PushKeyboardEnhancementFlags(keyboard_flags(false))
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
            force_redraw: false,
            preview_keys: false,
        })
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        restore(self.id, self.picker.tmux_detected());
    }
}
fn keyboard_flags(preview: bool) -> KeyboardEnhancementFlags {
    let base = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    if preview {
        base | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
    } else {
        base
    }
}

fn restore(id: u32, tmux: bool) {
    let mut stdout = std::io::stdout();
    let deletion = [id, id ^ 1, id ^ 0x8000_0000, id ^ 0x8000_0001]
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
        EndSynchronizedUpdate,
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableFocusChange,
        LeaveAlternateScreen,
        cursor::Show
    );
    let _ = stdout.flush();
    let _ = disable_raw_mode();
}
pub fn io(error: std::io::Error) -> Error {
    Error::new("IO_ERROR", error.to_string())
}
