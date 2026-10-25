//! Native stand-ins for compiler metadata, bounded commands and measurement fixtures.
use super::Result;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
pub fn response(args: &[String]) -> Result {
    let table: Value =
        serde_json::from_slice(&fs::read(args.first().ok_or("Missing response table")?)?)?;
    let key = serde_json::to_string(&args[1..])?;
    let record = table
        .get(&key)
        .ok_or_else(|| format!("Unexpected fixture invocation: {key}"))?;
    std::io::stdout().write_all(record["stdout"].as_str().unwrap_or_default().as_bytes())?;
    std::io::stderr().write_all(record["stderr"].as_str().unwrap_or_default().as_bytes())?;
    let code = record["status"].as_i64().unwrap_or(0);
    if code != 0 {
        std::process::exit(code as i32);
    }
    Ok(())
}
pub fn hold_tree(args: &[String]) -> Result {
    let mut child = Command::new(std::env::current_exe()?)
        .arg("hold-child")
        .stdin(Stdio::null())
        .spawn()?;
    super::write_json(
        Path::new(args.first().ok_or("Missing PID file")?),
        &serde_json::json!({"parent":std::process::id(),"child":child.id()}),
    )?;
    // A timeout must kill the whole group even when this parent exits on TERM.
    child.wait()?;
    Ok(())
}
pub fn hold_child() -> Result {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
pub fn render(args: &[String]) -> Result {
    let output = args
        .windows(2)
        .find(|w| w[0] == "--output")
        .ok_or("Missing render output")?;
    let dashboard = args.iter().any(|a| a.ends_with("dashboard.pen"));
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(1920u32.to_be_bytes());
    png.extend(1080u32.to_be_bytes());
    fs::write(&output[1], &png)?;
    let report = serde_json::json!({"width":1920,"height":1080,"node_count":1000,"diagnostics":[],"assets":if dashboard {serde_json::json!([{}])}else{serde_json::json!([])},"dynamic_artifacts":[],"script_executions":[],"png_hash":lazypencil_xtask::util::digest(png)});
    println!("{report}");
    Ok(())
}
