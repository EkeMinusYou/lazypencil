pub mod acp;
pub mod command;
pub mod process_tree;

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn write_json(path: impl AsRef<Path>, value: &impl serde::Serialize) -> Result {
    let path = path.as_ref();
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn append_json(path: impl AsRef<Path>, value: &serde_json::Value) -> Result {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut file, value)?;
    writeln!(file)?;
    Ok(())
}

pub fn spawn(args: &[String], detach: bool) -> Result<std::process::Child> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    if detach {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000200 | 0x00000008);
    }
    Ok(command.spawn()?)
}

pub fn wait_file(path: &Path, child: &mut std::process::Child, seconds: u64) -> Result {
    let end = Instant::now() + Duration::from_secs(seconds);
    while !path.is_file() {
        if child.try_wait()?.is_some() || Instant::now() >= end {
            return Err(format!("Fixture child did not publish {}", path.display()).into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
