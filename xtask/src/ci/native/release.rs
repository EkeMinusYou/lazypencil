use super::*;
use crate::{fixtures, performance};

fn render(
    s: &Session,
    document: &Path,
    output: &Path,
    report_name: &str,
    expected: (u32, u32, usize, Option<u32>),
) -> Result<Value> {
    let (width, height, assets, nodes) = expected;
    let raw = s.capture(
        Command::new(s.executable("release"))
            .args(["--json", "render"])
            .arg(document)
            .arg("--output")
            .arg(output),
        300,
        report_name,
    )?;
    let report: Value = serde_json::from_slice(&raw)?;
    ensure!(
        report["width"] == width
            && report["height"] == height
            && nodes.is_none_or(|n| report["node_count"] == n),
        "Unexpected release geometry"
    );
    ensure!(
        report["diagnostics"] == json!([])
            && report["assets"]
                .as_array()
                .is_some_and(|a| a.len() == assets)
            && report["dynamic_artifacts"] == json!([])
            && report["script_executions"] == json!([]),
        "Unexpected release diagnostics or assets"
    );
    performance::png(output, &report, width, height)?;
    Ok(report)
}
fn find<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if value["id"] == id {
        return Some(value);
    }
    value
        .get_mut("children")?
        .as_array_mut()?
        .iter_mut()
        .find_map(|c| find(c, id))
}
pub fn compile(s: &Session, log: &str) -> Result<()> {
    s.cargo(&["build", "--locked", "--release"], 3600, log, false)
}

pub fn smoke(s: &Session) -> Result<()> {
    // Recheck Cargo inputs even when the parallel lane already compiled this
    // source. A warm build performs freshness checks without recompilation.
    compile(s, "lazypencil-release-check.log")?;
    let linkage_dir = s.temporary.join("release-linkage");
    let linkage = crate::evidence::linkage::collect(
        &s.executable("release"),
        crate::evidence::linkage::native_host()?,
        &linkage_dir,
    )?;
    util::publish(
        &linkage_dir.join("ci.json"),
        &util::encode(
            &json!({"source_commit":std::env::var("GITHUB_SHA")?,"binary_sha256":linkage["binary_sha256"],"scope":"Direct dynamic-load declarations for this CI release executable; no minimum-OS runtime acceptance."}),
        )?,
        false,
    )?;
    process::checked(
        Command::new(s.executable("release")).arg("--version"),
        Duration::from_secs(30),
    )?;
    render(
        s,
        &s.root.join("examples/mesh-study.pen"),
        &s.target.join("release-smoke.png"),
        "lazypencil-target/release-smoke.json",
        (1040, 900, 0, Some(26)),
    )?;
    let fixture = tempfile::tempdir_in(&s.target)?;
    fixtures::performance(&fixture.path().join("input"), "dashboard")?;
    render(
        s,
        &fixture.path().join("input/dashboard.pen"),
        &s.target.join("release-dashboard.png"),
        "lazypencil-target/release-dashboard.json",
        (1920, 1080, 1, Some(1000)),
    )?;
    let graphics = tempfile::tempdir_in(&s.target)?;
    for path in util::walk(&s.root.join("examples/assets"))? {
        let relative = path.strip_prefix(s.root.join("examples"))?;
        let output = graphics.path().join(relative);
        fs::create_dir_all(output.parent().unwrap())?;
        fs::copy(path, output)?;
    }
    let mut design: Value = util::json(&s.root.join("examples/graphics-study.pen"))?;
    render(
        s,
        &s.root.join("examples/graphics-study.pen"),
        &s.target.join("release-graphics.png"),
        "lazypencil-target/release-graphics.json",
        (1000, 760, 1, None),
    )?;
    let outline = find(&mut design, "outline").context("Missing outline fixture")?;
    ensure!(
        outline["underline"] == true,
        "Expected decorated text fixture"
    );
    outline["fill"] = json!([]);
    let mut hashes = Vec::new();
    for enabled in [false, true] {
        find(&mut design, "outline").unwrap()["underline"] = enabled.into();
        let document = graphics.path().join("strokes.pen");
        fs::write(&document, util::encode(&design)?)?;
        let report = render(
            s,
            &document,
            &graphics.path().join("strokes.png"),
            "release-strokes.json",
            (1000, 760, 1, None),
        )?;
        hashes.push(report["png_hash"].clone());
    }
    ensure!(
        hashes[0] != hashes[1],
        "Decoration-only stroke changes must produce visible output"
    );
    // Keep measurements isolated from every parallel test lane and Cargo startup.
    let report = performance::benchmark(&s.executable("release").canonicalize()?, 20, 30.0)?;
    util::publish(
        &s.target.join("release-benchmark.json"),
        &util::encode(&report)?,
        false,
    )?;
    println!("Release mesh, dashboard, decoration ink and 42 measured exports passed");
    Ok(())
}
