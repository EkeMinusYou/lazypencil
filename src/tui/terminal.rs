use crate::{Error, Result};
use crossterm::{
    SynchronizedUpdate, cursor,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
/// Keep the terminal active while the picker hands off to the document workspace.
pub(super) struct Startup {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    guard: Restore,
}

struct Restore {
    id: Option<u32>,
    tmux: bool,
}
impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            restore(id, self.tmux);
        } else {
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen, cursor::Show);
            let _ = disable_raw_mode();
        }
    }
}
impl Startup {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().map_err(io)?;
        let guard = Restore {
            id: None,
            tmux: false,
        };
        execute!(std::io::stdout(), EnterAlternateScreen, cursor::Hide).map_err(io)?;
        let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout())).map_err(io)?;
        Ok(Self { terminal, guard })
    }

    pub fn draw(&mut self, draw: impl FnOnce(&mut ratatui::Frame)) -> Result<()> {
        draw_terminal(&mut self.terminal, draw)
    }

    pub fn activate(self, protocol: super::ImageProtocol, id: u32) -> Result<Screen> {
        let Self {
            terminal,
            mut guard,
        } = self;
        guard.id = Some(id);
        execute!(
            std::io::stdout(),
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(keyboard_flags(false))
        )
        .map_err(io)?;
        let (picker, startup_text) = super::graphics::Backend::detect(protocol)?;
        guard.tmux = picker.tmux_detected();
        std::mem::forget(guard);
        Ok(Screen {
            terminal,
            picker,
            startup_text,
            id,
            force_redraw: false,
            preview_keys: false,
        })
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
            // Update our existing stack entry atomically. Popping first exposes
            // legacy mode to multiplexers between the two commands.
            let mut stdout = std::io::stdout();
            write!(stdout, "\x1b[={}u", keyboard_flags(enabled).bits()).map_err(io)?;
            stdout.flush().map_err(io)?;
            self.preview_keys = enabled;
        }
        Ok(())
    }
    pub fn draw(&mut self, draw: impl FnOnce(&mut ratatui::Frame)) -> Result<()> {
        if self.force_redraw {
            // Invalidate the cached buffer without writing to the terminal or
            // querying the cursor while the input reader owns stdin.
            self.terminal.swap_buffers();
            for cell in &mut self.terminal.current_buffer_mut().content {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
            self.terminal.swap_buffers();
            self.force_redraw = false;
        }
        draw_terminal(&mut self.terminal, draw)
    }
    pub fn enter(protocol: super::ImageProtocol, id: u32) -> Result<Self> {
        Startup::enter()?.activate(protocol, id)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        restore(self.id, self.picker.tmux_detected());
    }
}
fn draw_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    draw: impl FnOnce(&mut ratatui::Frame),
) -> Result<()> {
    // Both the launcher and document workspace present each complete frame in
    // one update; modal removal must not expose a partially painted workspace.
    std::io::stdout()
        .sync_update(|_| terminal.draw(draw).map(|_| ()).map_err(io))
        .map_err(io)?
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
        LeaveAlternateScreen,
        cursor::Show
    );
    let _ = stdout.flush();
    let _ = disable_raw_mode();
}
pub fn io(error: std::io::Error) -> Error {
    Error::new("IO_ERROR", error.to_string())
}
