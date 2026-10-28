//! Control only the private Kitty instance created by this disposable GHA test.

use super::raster;
use crate::{fixtures::oracles::Raster, process, util};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(super) fn installation() -> Result<Value> {
    let path = PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_KITTY_INSTALLATION")
            .context("Kitty installation evidence required")?,
    );
    let report: Value = serde_json::from_slice(&util::read(&path, 65536)?)?;
    ensure!(
        report["source_commit"] == std::env::var("GITHUB_SHA")?
            && report["version"] == crate::fixtures::kitty_ui::VERSION
            && report["archive_sha256"] == crate::fixtures::kitty_ui::SHA256
            && report["code_signature_verified"] == true,
        "Kitty installation belongs to another fixture/source"
    );
    for (name, variable) in [
        ("kitty", "LAZYPENCIL_TEST_KITTY"),
        ("kitten", "LAZYPENCIL_TEST_KITTEN"),
    ] {
        let path = PathBuf::from(std::env::var_os(variable).context("Kitty executable required")?)
            .canonicalize()?;
        ensure!(
            report[name].as_str() == path.to_str()
                && report[format!("{name}_binary_sha256")] == util::file_digest(&path)?,
            "Kitty fixture executable changed"
        );
    }
    Ok(report)
}

pub(super) struct Window {
    kitten: PathBuf,
    socket: PathBuf,
    root: PathBuf,
    id: u64,
    cells: [u32; 2],
    pixels: Option<[u32; 2]>,
}

impl Window {
    pub(super) fn launch(root: &Path, title: &str) -> Result<(Command, Self)> {
        installation()?;
        let kitty = PathBuf::from(
            std::env::var_os("LAZYPENCIL_TEST_KITTY").context("Pinned Kitty fixture required")?,
        )
        .canonicalize()?;
        let kitten = PathBuf::from(
            std::env::var_os("LAZYPENCIL_TEST_KITTEN").context("Pinned kitten fixture required")?,
        )
        .canonicalize()?;
        ensure!(
            kitty.parent() == kitten.parent(),
            "Kitty and kitten must share one owned app bundle"
        );
        util::chmod(root, 0o700)?;
        let socket = root.join("control.sock");
        ensure!(
            !socket.exists() && !socket.is_symlink(),
            "Kitty control socket must be new"
        );
        let config = root.join("kitty.conf");
        fs::write(
            &config,
            "font_family Menlo\nfont_size 11\ninitial_window_width 120c\ninitial_window_height 40c\nremember_window_size no\nwindow_padding_width 0\nbackground_opacity 1\nallow_remote_control socket-only\ncursor #ff00ff\ncursor_text_color #000000\ncursor_shape block\ncursor_blink_interval 0\nmacos_quit_when_last_window_closed yes\n",
        )?;
        let mut command = Command::new(kitty);
        command
            .arg("--config")
            .arg(config)
            .arg("--title")
            .arg(title)
            .arg("--listen-on")
            .arg(format!("unix:{}", socket.display()))
            .arg("--");
        Ok((
            command,
            Self {
                kitten,
                socket,
                root: root.into(),
                id: 0,
                cells: [0; 2],
                pixels: None,
            },
        ))
    }

    fn control(&self, operation: &str) -> Command {
        let mut command = Command::new(&self.kitten);
        command
            .args(["@", "--to"])
            .arg(format!("unix:{}", self.socket.display()))
            .arg(operation);
        command
    }

    fn matched(&self, operation: &str) -> Command {
        let mut command = self.control(operation);
        command.arg("--match").arg(format!("id:{}", self.id));
        command
    }

    fn checked(&self, command: &mut Command) -> Result<()> {
        let result = process::output_limited(command, Duration::from_secs(10), 65536)?;
        ensure!(
            !result.timed_out && result.status.success(),
            "Owned Kitty control failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    }

    pub(super) fn ready(&mut self) -> Result<bool> {
        let response =
            process::output_limited(&mut self.control("ls"), Duration::from_secs(2), 65536)?;
        if response.timed_out || !response.status.success() {
            return Ok(false);
        }
        let windows: Vec<Value> = serde_json::from_slice(&response.stdout)?;
        let windows = windows
            .iter()
            .flat_map(|w| w["tabs"].as_array().into_iter().flatten())
            .flat_map(|t| t["windows"].as_array().into_iter().flatten())
            .collect::<Vec<_>>();
        if windows.is_empty() {
            return Ok(false);
        }
        ensure!(windows.len() == 1, "Ambiguous owned Kitty window");
        let window = windows[0];
        self.id = window["id"]
            .as_u64()
            .filter(|id| *id > 0)
            .context("Invalid Kitty window ID")?;
        self.cells = ["columns", "lines"].map(|name| {
            window[name]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0)
        });
        if self.cells.contains(&0) {
            return Ok(false);
        }
        self.checked(&mut self.matched("focus-window"))?;
        Ok(true)
    }

    pub(super) fn key(&self, key: &str) -> Result<()> {
        let key = match key {
            "BackSpace" => "backspace",
            "Tab" => "tab",
            "Right" => "right",
            "Left" => "left",
            key => key,
        };
        self.checked(self.matched("send-key").arg(key))
    }

    pub(super) fn text(&self, text: &str) -> Result<()> {
        self.checked(self.matched("send-text").arg("--").arg(text))
    }

    pub(super) fn frame(&mut self) -> Result<Raster> {
        let path = self.root.join("latest.png");
        if path.exists() {
            fs::remove_file(&path)?;
        }
        self.checked(self.matched("screenshot").arg(&path))?;
        let frame = raster::decode_png(&util::read(&path, 20 * 1024 * 1024)?)?;
        self.pixels.get_or_insert([frame.width, frame.height]);
        Ok(frame)
    }

    pub(super) fn resize(&self, width: u32, height: u32) -> Result<()> {
        let pixels = self
            .pixels
            .context("Capture the original Kitty framebuffer before resize")?;
        let cells = [width, height].map(|v| u64::from(v));
        let width = (cells[0] * u64::from(self.cells[0]) / u64::from(pixels[0])).max(1);
        let height = (cells[1] * u64::from(self.cells[1]) / u64::from(pixels[1])).max(1);
        self.checked(self.matched("resize-os-window").args([
            "--unit",
            "cells",
            "--width",
            &width.to_string(),
            "--height",
            &height.to_string(),
        ]))
    }
}
