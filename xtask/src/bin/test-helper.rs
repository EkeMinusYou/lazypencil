#[path = "../test_support/mod.rs"]
mod test_support;

fn main() -> test_support::Result {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let explicit = args.first().is_some_and(|s| {
        [
            "acp",
            "agent-tree",
            "browser-tree",
            "sleep",
            "input-worker",
            "mark",
        ]
        .contains(&s.as_str())
    });
    if !explicit {
        let sidecar = std::env::current_exe()?.with_extension("fixture.json");
        if sidecar.is_file() {
            let mut fixed: Vec<String> = serde_json::from_slice(&std::fs::read(sidecar)?)?;
            fixed.extend(args);
            args = fixed;
        }
    }
    match args.first().map(String::as_str) {
        Some("acp") => test_support::acp::run(args.get(1).ok_or("Missing ACP fixture mode")?),
        Some("agent-tree") => test_support::process_tree::agent(&args[1..]),
        Some("browser-tree") => test_support::process_tree::browser(&args[1..]),
        Some("sleep") => {
            std::thread::sleep(std::time::Duration::from_secs(60));
            Ok(())
        }
        Some("input-worker") => test_support::process_tree::input_worker(&args[1..]),
        Some("mark") => {
            std::fs::write(args.get(1).ok_or("Missing marker path")?, b"started")?;
            Ok(())
        }
        _ => Err("Unknown deterministic test helper mode".into()),
    }
}
