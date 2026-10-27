use crate::util;
use anyhow::{Result, ensure};
use clap::Subcommand;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    Emoji {
        #[arg(long)]
        output: PathBuf,
    },
    Fonts {
        #[arg(long)]
        output: PathBuf,
    },
    Performance {
        #[arg(value_parser=["shadow","dashboard"])]
        scene: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Emoji { output } => emoji::prepare(
                &util::root().join("tests/fixtures/fonts/emoji-upstream.json"),
                &output,
            ),
            Self::Fonts { output } => fonts::write(&output),
            Self::Performance { scene, output_dir } => {
                util::print(&performance(&output_dir, &scene)?)
            }
        }
    }
}
pub fn performance(output_dir: &Path, scene: &str) -> Result<serde_json::Value> {
    ensure!(
        ["shadow", "dashboard"].contains(&scene),
        "Unknown performance scene"
    );
    fs::create_dir(output_dir)?;
    let source = util::root().join("tests/fixtures/performance");
    let document = source.join(format!("{scene}.pen"));
    let bytes = util::read(&document, util::MAX_FILE)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["version"] == "2.21",
        "Unexpected performance fixture version"
    );
    fs::write(output_dir.join(format!("{scene}.pen")), &bytes)?;
    let mut report = serde_json::json!({"document_sha256":util::digest(bytes)});
    if scene == "dashboard" {
        let asset = util::read(&source.join("avatar.png"), util::MAX_FILE)?;
        fs::write(output_dir.join("avatar.png"), &asset)?;
        report["asset_sha256"] = serde_json::json!(util::digest(asset));
    }
    Ok(report)
}

/// Keep the original dashboard intact for latency trials. This separate resource
/// fixture replaces its unpainted header frame with a script that returns the
/// same children, preserving the expanded node count and visible geometry.
pub fn performance_with_script(output_dir: &Path) -> Result<serde_json::Value> {
    performance_script(output_dir, false)
}

/// Keep the same dashboard and node identities, replacing only two generated
/// navigation backgrounds with an owned offline page and a software shader.
pub fn performance_with_dynamic(output_dir: &Path) -> Result<serde_json::Value> {
    performance_script(output_dir, true)
}

fn performance_script(output_dir: &Path, chromium: bool) -> Result<serde_json::Value> {
    let mut report = performance(output_dir, "dashboard")?;
    let path = output_dir.join("dashboard.pen");
    let mut document: serde_json::Value = util::json(&path)?;
    let header = &mut document["children"][0]["children"][0];
    ensure!(
        header["id"] == "header" && header["type"] == "frame" && header.get("fill").is_none(),
        "Resource fixture header changed"
    );
    let mut children = header
        .as_object_mut()
        .unwrap()
        .remove("children")
        .filter(serde_json::Value::is_array)
        .ok_or_else(|| anyhow::anyhow!("Missing header children"))?;
    if chromium {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let page = "<!doctype html><meta charset=utf-8><style>html,body{margin:0;width:100%;height:100%;background:#e0e7ff}div{width:0;height:100%;background:#6366f1}</style><div></div><script>document.querySelector('div').style.width='72%'</script>";
        let shader = "precision highp float;\n/** @resolution */ uniform vec2 resolution;\nvoid main(){vec2 uv=gl_FragCoord.xy/resolution;gl_FragColor=vec4(0.08+0.1*uv.x,0.25+0.3*uv.y,0.9,1.0);}\n";
        let nodes = children.as_array_mut().unwrap();
        let browser = nodes
            .iter_mut()
            .find(|node| node["id"] == "nav-0")
            .ok_or_else(|| anyhow::anyhow!("Missing browser resource background"))?;
        ensure!(
            browser["type"] == "rectangle",
            "Resource background changed"
        );
        browser["type"] = serde_json::json!("browser");
        browser.as_object_mut().unwrap().remove("fill");
        browser["url"] =
            serde_json::json!(format!("data:text/html;base64,{}", STANDARD.encode(page)));
        let background = nodes
            .iter_mut()
            .find(|node| node["id"] == "nav-1")
            .ok_or_else(|| anyhow::anyhow!("Missing shader resource background"))?;
        ensure!(
            background["type"] == "rectangle",
            "Resource background changed"
        );
        background["fill"] = serde_json::json!({"type":"shader","url":"resource-nav.glsl"});
        fs::write(output_dir.join("resource-nav.glsl"), shader)?;
        report["page_sha256"] = serde_json::json!(util::digest(page));
        report["shader_sha256"] = serde_json::json!(util::digest(shader));
    }
    let script = format!(
        "/**\n * @schema 2.21\n */\nreturn {};\n",
        serde_json::to_string(&children)?
    );
    header["type"] = serde_json::json!("script");
    header["scriptUri"] = serde_json::json!("resource-header.js");
    let bytes = util::encode(&document)?;
    fs::write(&path, &bytes)?;
    fs::write(output_dir.join("resource-header.js"), &script)?;
    report["static_document_sha256"] = report["document_sha256"].clone();
    report["document_sha256"] = serde_json::json!(util::digest(bytes));
    report["script_sha256"] = serde_json::json!(util::digest(script));
    report["scope"] = serde_json::json!(
        "Separate resource-only dashboard with the same header children generated by JavaScript; not an original latency trial"
    );
    if chromium {
        report["scope"] = serde_json::json!(
            "Separate Chromium resource dashboard: same node identities and geometry, one offline JavaScript page and one software shader; not an original latency trial or the static pixel reference"
        );
    }
    Ok(report)
}
pub mod emoji;
pub mod fonts;
pub mod oracles;
