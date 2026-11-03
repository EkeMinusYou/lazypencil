use super::*;
use std::{
    ffi::OsStr,
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
};

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn executables(command: &Path) -> Vec<PathBuf> {
    let direct = command.is_absolute() || command.components().count() > 1;
    let directories = if direct {
        vec![PathBuf::new()]
    } else {
        std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default()
    };
    let mut found = Vec::new();
    for directory in directories {
        let candidate = directory.join(command);
        #[cfg(windows)]
        let candidates = {
            let mut paths = vec![candidate.clone()];
            if candidate.extension().is_none() {
                paths.push(candidate.with_extension("exe"));
                paths.push(candidate.with_extension("cmd"));
                paths.push(candidate.with_extension("bat"));
            }
            paths
        };
        #[cfg(not(windows))]
        let candidates = [candidate];
        for candidate in candidates {
            if executable(&candidate) {
                let path = if candidate.is_absolute() {
                    candidate
                } else {
                    let Ok(cwd) = std::env::current_dir() else {
                        continue;
                    };
                    cwd.join(candidate)
                };
                if !found.contains(&path) {
                    found.push(path);
                }
            }
        }
    }
    found
}

pub(super) fn needs_node(path: &Path) -> bool {
    use std::io::Read;
    let mut header = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(256).read_to_end(&mut header))
        .ok();
    let line = header
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    line.starts_with(b"#!")
        && std::str::from_utf8(line).is_ok_and(|line| {
            line.split_whitespace()
                .any(|word| word == "node" || word.ends_with("/node"))
        })
}

pub(super) fn directory(
    report: &mut Report,
    id: &str,
    selected: Result<PathBuf>,
) -> Option<PathBuf> {
    let path = match selected {
        Ok(path) => path,
        Err(error) => {
            report.add(
                id,
                Status::Error,
                "Cannot select the directory",
                Some(
                    "Check the corresponding LAZYPENCIL_DATA_DIR or LAZYPENCIL_LOCK_DIR override."
                        .into(),
                ),
                json!({"error_code":error.code}),
            );
            return None;
        }
    };
    let absolute = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir().ok()?.join(&path)
    };
    let mut ancestor = absolute.as_path();
    let issue = loop {
        match fs::metadata(ancestor) {
            Ok(metadata) if !metadata.is_dir() => {
                break Some("The path or an existing parent is not a directory");
            }
            Ok(metadata) => {
                #[cfg(unix)]
                let accessible = {
                    use std::os::unix::ffi::OsStrExt;
                    std::ffi::CString::new(ancestor.as_os_str().as_bytes()).is_ok_and(
                        |path| unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 },
                    )
                };
                #[cfg(not(unix))]
                let accessible = !metadata.permissions().readonly();
                let _ = metadata;
                break (!accessible).then_some(
                    "The directory or its existing parent does not allow writing/traversal",
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = ancestor.parent() else {
                    break Some("No existing parent directory is accessible");
                };
                ancestor = parent;
            }
            Err(_) => break Some("The directory cannot be inspected"),
        }
    };
    report.add(id, if issue.is_some() {Status::Error} else {Status::Ok},
        issue.unwrap_or(if absolute.is_dir() {"Existing directory is accessible"} else {"Directory is absent; its existing parent is accessible"}),
        issue.map(|_| "Check the directory path and permissions; doctor does not create or modify it.".into()),
        json!({"path":path,"exists":absolute.is_dir(),"existing_parent":ancestor,"write_test":"permissions_only"}));
    Some(path)
}

pub(super) fn fonts(report: &mut Report) {
    match crate::config::UserSnapshot::load() {
        Ok(snapshot) => {
            let complete = snapshot
                .require_fonts()
                .and_then(|()| snapshot.verify_unchanged())
                .is_ok();
            let summary = snapshot.summary();
            report.add("fonts", if complete {Status::Ok} else {Status::Error},
                if complete {"Configured font files are available"} else {"Configured font files or search directories are unavailable"},
                (!complete).then(|| "Fix fonts.files/fonts.search_paths in the user configuration, or restore the missing files.".into()),
                json!({"font_files":summary["font_files"],"diagnostics":summary["font_diagnostics"],"font_parsing":"not_checked"}));
        }
        Err(error) => report.add(
            "fonts",
            Status::Error,
            "Cannot inspect configured font files",
            Some(
                "Inspect fonts.files and fonts.search_paths with lazypencil config show --user."
                    .into(),
            ),
            json!({"error_code":error.code}),
        ),
    }
}

pub(super) fn terminal(report: &mut Report) {
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let term = std::env::var_os("TERM").unwrap_or_default();
    report.add("terminal", if tty && term != OsStr::new("dumb") {Status::Ok} else {Status::NotChecked},
        if tty {"Interactive terminal detected; image transport is checked separately"} else {"Not running in an interactive terminal; TUI image protocol is not checked"},
        (!tty || term == OsStr::new("dumb")).then(|| "Run lazypencil tui in an interactive terminal; see docs/tui.md for image protocol support.".into()),
        json!({"interactive":tty,"term":term.to_string_lossy(),"tmux":std::env::var_os("TMUX").is_some(),"image_protocol":"not_checked"}));
}
