use anyhow::{Result, bail};
use std::{fs, io::Write, path::Path, process::Command, time::Duration};
#[path = "test_support/command_output.rs"]
mod command_output;
pub use command_output::Output;

pub fn output(command: &mut Command, timeout: Duration) -> Result<Output> {
    output_limited(command, timeout, 64 * 1024 * 1024)
}

pub fn output_limited(command: &mut Command, timeout: Duration, limit: u64) -> Result<Output> {
    clean_cargo_environment(command);
    Ok(command_output::output(command, timeout, limit)?)
}

pub fn output_observed(
    command: &mut Command,
    timeout: Duration,
    limit: u64,
    observer: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<Output> {
    clean_cargo_environment(command);
    Ok(command_output::output_observed(
        command,
        timeout,
        limit,
        Some(observer),
    )?)
}

fn clean_cargo_environment(command: &mut Command) {
    if matches!(
        Path::new(command.get_program())
            .file_name()
            .and_then(|s| s.to_str()),
        Some("cargo" | "cargo.exe")
    ) {
        // cargo run exports its package metadata. Build scripts can watch these
        // ambient values and invalidate otherwise identical dependency builds.
        // Preserve compiler flags, target paths and all other build settings.
        let metadata = std::env::vars_os()
            .map(|(name, _)| name)
            .chain(command.get_envs().map(|(name, _)| name.to_owned()))
            .filter(|name| {
                name.to_str().is_some_and(|name| {
                    name.starts_with("CARGO_PKG_")
                        || name.starts_with("CARGO_MANIFEST_")
                        || [
                            "CARGO_PRIMARY_PACKAGE",
                            "CARGO_CRATE_NAME",
                            "CARGO_BIN_NAME",
                        ]
                        .contains(&name)
                })
            })
            .collect::<Vec<_>>();
        for name in metadata {
            command.env_remove(name);
        }
    }
}

pub fn checked(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    let result = output(command, timeout)?;
    anyhow::ensure!(
        !result.timed_out,
        "Command exceeded {timeout:?}: {command:?}"
    );
    if !result.status.success() {
        bail!(
            "Command failed ({}) {command:?}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(result.stdout)
}

pub fn text(command: &mut Command) -> Result<String> {
    Ok(String::from_utf8(checked(
        command,
        Duration::from_secs(45),
    )?)?)
}

pub fn logged(command: &mut Command, timeout: Duration, log: &Path, append: bool) -> Result<()> {
    eprintln!("Running {command:?}");
    let result = output(command, timeout)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(log)?;
    for bytes in [&result.stdout, &result.stderr] {
        file.write_all(bytes)?;
        std::io::stdout().write_all(bytes)?;
    }
    eprintln!(
        "Command finished in {:.3}s: {}",
        result.elapsed.as_secs_f64(),
        result.status
    );
    anyhow::ensure!(
        !result.timed_out,
        "Command exceeded {timeout:?}: {command:?}; see {}",
        log.display()
    );
    anyhow::ensure!(
        result.status.success(),
        "Command failed ({}) {command:?}; see {}",
        result.status,
        log.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_exit_code_and_separate_output() {
        let result = output(Command::new("git").arg("--version"), Duration::from_secs(5)).unwrap();
        assert!(result.status.success());
        assert!(result.stdout.starts_with(b"git version "));
        let result = output(
            Command::new("git").arg("definitely-not-a-git-command"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
    }
}
