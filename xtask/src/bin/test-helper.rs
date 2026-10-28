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
            "fixture-command",
            "hold-tree",
            "hold-child",
            "render-fixture",
            "font-fixture",
            "png-inspect",
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
        Some("png-inspect") => {
            use std::io::Read;
            let path = args.get(1).ok_or("Missing PNG input")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(20 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            let frame = lazypencil_xtask::terminal::raster::decode_png(&bytes)?;
            serde_json::to_writer(
                std::io::stdout().lock(),
                &serde_json::json!({"width":frame.width,"height":frame.height,"first_pixel":frame.pixels[0]}),
            )?;
            Ok(())
        }
        Some("font-fixture") => {
            let destination = args.get(1).ok_or("Missing color-font output")?;
            let bytes = match args.get(2).map(String::as_str).unwrap_or("colr") {
                "colr" => lazypencil_xtask::fixtures::fonts::color::generate(),
                "sbix" => lazypencil_xtask::fixtures::fonts::color::bitmap::generate()?,
                "sbix-contours" => {
                    lazypencil_xtask::fixtures::fonts::color::bitmap::generate_with_silhouette(
                        true,
                    )?
                }
                _ => return Err("Unknown color-font fixture format".into()),
            };
            std::fs::write(destination, bytes)?;
            Ok(())
        }
        Some("fixture-command") => test_support::command::response(&args[1..]),
        Some("hold-tree") => test_support::command::hold_tree(&args[1..]),
        Some("hold-child") => test_support::command::hold_child(),
        Some("render-fixture") => test_support::command::render(&args[1..]),
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
