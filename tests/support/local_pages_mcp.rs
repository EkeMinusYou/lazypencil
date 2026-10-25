//! The public MCP and CLI paths share captured local page inputs.

use super::*;
use base64::Engine;

#[path = "../../xtask/src/test_support/command_output.rs"]
#[allow(dead_code)]
mod command_output;

fn pixels(bytes: &[u8], color: skia_safe::Color) {
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((image.width(), image.height()), (32, 32));
    let raster = image.peek_pixels().unwrap();
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(raster.get_color((x, y)), color, "{x},{y}");
        }
    }
}

fn capture(client: &mut Client, document: &Value, refresh: bool) -> (Vec<u8>, Value) {
    let result=client.call("render",json!({"expected_revision":document["revision"],"expected_dependency_revision":document["dependency_revision"],"refresh":refresh}));
    assert_ne!(result["isError"], true, "{result}");
    let image = result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["type"] == "image")
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    (bytes, result["structuredContent"].clone())
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn real_mcp_and_cli_local_pages_declare_edit_pack_and_render_after_source_removal() {
    let mut records = Vec::new();
    for version in ["2.19", "2.21"] {
        let (temp, file, data) = fixture();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join("web")).unwrap();
        fs::write(temp.path().join("web/index.html"),"<!doctype html><link rel='icon' href='data:,'><link rel='stylesheet' href='style.css'><body></body>").unwrap();
        let css = temp.path().join("web/style.css");
        fs::write(&css, "html,body{margin:0;background:#00ff00}").unwrap();
        fs::write(&file,json!({"version":version,"children":[{"id":"page","type":"browser","width":32,"height":32,"url":"file:///original/site/index.html?literal=$mode#keep"}]}).to_string()).unwrap();
        let chromium =
            std::path::PathBuf::from(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
        let mut client = Client::start_with_chromium(&file, &data, Some(&chromium));
        let before = client.data("get_document", json!({}));
        let declared=client.data("apply_operations",json!({"request_id":"declare-local-pages","expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"],"operations":[{"op":"set_browser_resources","roots":[{"url":"file:///original/site/","path":"web"}]}]}));
        let saved = fs::read(&file).unwrap();
        let (first, first_report) = capture(&mut client, &declared, false);
        pixels(&first, skia_safe::Color::GREEN);
        let (cached, cached_report) = capture(&mut client, &declared, false);
        assert_eq!(first, cached);
        assert_eq!(
            first_report["dynamic_artifacts"][0]["acquisition"]["acquisition_id"],
            cached_report["dynamic_artifacts"][0]["acquisition"]["acquisition_id"]
        );
        fs::write(&css, "html,body{margin:0;background:#0000ff}").unwrap();
        let stale=client.call("render",json!({"expected_revision":declared["revision"],"expected_dependency_revision":declared["dependency_revision"]}));
        assert_eq!(
            stale["structuredContent"]["error"]["code"],
            "REVISION_CONFLICT"
        );
        assert!(
            stale["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["type"] != "image")
        );
        let current = client.data("get_document", json!({}));
        assert_eq!(current["revision"], declared["revision"]);
        assert_ne!(
            current["dependency_revision"],
            declared["dependency_revision"]
        );
        let (changed, changed_report) = capture(&mut client, &current, false);
        pixels(&changed, skia_safe::Color::BLUE);
        assert_ne!(
            first_report["dynamic_artifacts"][0]["acquisition"]["resources"]["revision"],
            changed_report["dynamic_artifacts"][0]["acquisition"]["resources"]["revision"]
        );
        assert_eq!(fs::read(&file).unwrap(), saved);
        let packed=client.data("apply_operations",json!({"request_id":"pack-local-pages","expected_revision":current["revision"],"expected_dependency_revision":current["dependency_revision"],"operations":[{"op":"pack_browser_resources"}]}));
        fs::remove_dir_all(temp.path().join("web")).unwrap();
        let packed_saved = fs::read(&file).unwrap();
        let (reloaded, reloaded_report) = capture(&mut client, &packed, true);
        pixels(&reloaded, skia_safe::Color::BLUE);
        assert_eq!(reloaded, changed);
        client.finish();
        let output = temp.path().join("rendered.png");
        let result = command_output::output(
            Command::new(env!("CARGO_BIN_EXE_lazypencil"))
                .env("LAZYPENCIL_CHROMIUM", &chromium)
                .arg("--json")
                .arg("--data-dir")
                .arg(&data)
                .arg("render")
                .arg(&file)
                .arg("--output")
                .arg(&output),
            Duration::from_secs(60),
            4 * 1024 * 1024,
        )
        .unwrap();
        assert!(
            !result.timed_out && result.status.success(),
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let cli: Value = serde_json::from_slice(&result.stdout).unwrap();
        let cli_png = fs::read(&output).unwrap();
        pixels(&cli_png, skia_safe::Color::BLUE);
        assert_eq!(cli_png, changed);
        assert_eq!(fs::read(&file).unwrap(), packed_saved);
        assert_eq!(
            cli["dynamic_artifacts"][0]["acquisition"]["local_files"]["fulfilled"],
            2
        );
        for (stage, bytes, report) in [
            ("first", first, first_report),
            ("changed", changed, changed_report),
            ("packed", reloaded, reloaded_report),
            ("cli", cli_png, cli),
        ] {
            let name = format!("codec-local-page-{version}-mcp-{stage}.png");
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                fs::create_dir_all(&directory).unwrap();
                fs::write(std::path::Path::new(&directory).join(&name), &bytes).unwrap();
            }
            records.push(json!({"document_version":version,"stage":stage,"name":name,"width":32,"height":32,"png_hash":lazypencil::document::hash(&bytes),"dynamic_artifacts":report["dynamic_artifacts"],"source_removed":matches!(stage,"packed"|"cli")}));
        }
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(
            std::path::Path::new(&directory).join("codec-local-pages-mcp.json"),
            serde_json::to_vec_pretty(
                &json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"images":records}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
