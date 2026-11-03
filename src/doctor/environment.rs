use super::*;
use std::{
    ffi::OsStr,
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
};

fn version(value: &str) -> Option<[u32; 3]> {
    let mut result = [0; 3];
    let mut count = 0;
    for (index, part) in value.split('.').enumerate() {
        if index >= 3 || part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        result[index] = part.parse().ok()?;
        count += 1;
    }
    (count >= 2).then_some(result)
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    let mut bytes = [0u8; 64];
    let mut length = bytes.len();
    if unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length > bytes.len()
    {
        return None;
    }
    let value = std::str::from_utf8(&bytes[..length])
        .ok()?
        .trim_end_matches('\0');
    version(value)?;
    Some(value.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn os_version() -> Option<String> {
    None
}

fn platform_status(os: &str, architecture: &str, actual: Option<&str>, minimum: &str) -> Status {
    if os != "macos" || architecture != "aarch64" {
        return Status::Warning;
    }
    match actual.and_then(version).zip(version(minimum)) {
        Some((actual, minimum)) if actual < minimum => Status::Error,
        Some(_) => Status::Ok,
        None => Status::Warning,
    }
}

pub(super) fn platform(report: &mut Report, release: &Value) {
    let os = std::env::consts::OS;
    let architecture = std::env::consts::ARCH;
    let actual = os_version();
    let minimum = release["minimum_os"].as_str().unwrap_or("");
    let status = platform_status(os, architecture, actual.as_deref(), minimum);
    let message = match status {
        Status::Ok => "OS and CPU match the supported release environment".to_owned(),
        Status::Error => format!(
            "macOS {} is older than the required {minimum}",
            actual.as_deref().unwrap_or("unknown")
        ),
        _ if os != "macos" || architecture != "aarch64" => format!(
            "{os}/{architecture} is outside the supported macOS Apple Silicon release target"
        ),
        _ => "Cannot determine whether the macOS version meets the supported minimum".into(),
    };
    let hint = match status {
        Status::Error => Some(format!("Use macOS {minimum} or later on Apple Silicon.")),
        Status::Warning => Some("Check the supported environment in README.md; source/core functionality can differ from supported release builds.".into()),
        _ => None,
    };
    report.add("platform",status,message,hint,json!({"os":os,"architecture":architecture,"version":actual,"minimum_os":minimum,"supported_release_target":release["target"]}));
}

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
            if complete {
                font_loading(report, &snapshot);
            } else {
                report.add(
                    "fonts.loading",
                    Status::NotChecked,
                    "Font loading was not attempted because font inputs are incomplete",
                    None,
                    json!({"font_parsing":"not_checked"}),
                );
            }
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

#[cfg(feature = "rendering")]
fn font_loading(report: &mut Report, snapshot: &crate::config::UserSnapshot) {
    let loaded = (|| {
        let mut fonts = crate::text::Fonts::new(&snapshot.font_files)?;
        fonts.configure(&snapshot.settings.fonts)?;
        let inventory = fonts.inventory()?;
        snapshot.verify_unchanged()?;
        let families = inventory
            .iter()
            .map(|info| info.family.clone())
            .collect::<std::collections::BTreeSet<_>>();
        Ok::<_, Error>(
            json!({"font_parsing":"checked","loaded_faces":inventory.len(),"families":families,"default_family":fonts.default_family()}),
        )
    })();
    match loaded {
        Ok(details) => report.add("fonts.loading",Status::Ok,"Configured and bundled fonts loaded successfully",None,details),
        Err(error) => report.add("fonts.loading",Status::Error,"Configured font loading failed",
            Some("Replace invalid or unsupported font files, or fix fonts.default_family/fallbacks in the user configuration.".into()),json!({"font_parsing":"failed","error":error})),
    }
}

#[cfg(not(feature = "rendering"))]
fn font_loading(report: &mut Report, _: &crate::config::UserSnapshot) {
    report.add(
        "fonts.loading",
        Status::NotChecked,
        "This build cannot parse or load fonts without rendering support",
        None,
        json!({"font_parsing":"not_checked","rendering_compiled":false}),
    );
}

pub(super) fn terminal(report: &mut Report) {
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let term = std::env::var_os("TERM").unwrap_or_default();
    report.add("terminal", if tty && term != OsStr::new("dumb") {Status::Ok} else {Status::NotChecked},
        if tty {"Interactive terminal detected; image transport is checked separately"} else {"Not running in an interactive terminal; TUI image protocol is not checked"},
        (!tty || term == OsStr::new("dumb")).then(|| "Run lazypencil tui in an interactive terminal; see docs/tui.md for image protocol support.".into()),
        json!({"interactive":tty,"term":term.to_string_lossy(),"tmux":std::env::var_os("TMUX").is_some(),"image_protocol":"not_checked"}));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_minimum_uses_numeric_os_versions_and_reports_unknown_or_unsupported_hosts() {
        for (os, architecture, actual, expected) in [
            ("macos", "aarch64", Some("15.7.8"), Status::Error),
            ("macos", "aarch64", Some("15.7.9"), Status::Ok),
            ("macos", "aarch64", Some("15.8"), Status::Ok),
            ("macos", "aarch64", Some("26.0"), Status::Ok),
            ("macos", "aarch64", Some("15.7.9.1"), Status::Warning),
            ("macos", "aarch64", None, Status::Warning),
            ("macos", "x86_64", Some("26.0"), Status::Warning),
            ("linux", "aarch64", None, Status::Warning),
        ] {
            assert_eq!(
                serde_json::to_value(platform_status(os, architecture, actual, "15.7.9")).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        for invalid in ["", "15", "+15.7.9", "15..9", "15.7.x", "4294967296.0"] {
            assert!(version(invalid).is_none(), "{invalid}");
        }
    }
}
