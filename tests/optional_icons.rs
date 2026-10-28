use lazypencil::{
    icons::{self, ListRequest},
    project::Project,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use tempfile::TempDir;
#[cfg(feature = "tui")]
#[path = "support/icon_reference.rs"]
mod reference;

fn phosphor_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icons/phosphor-core-2.1.1.tgz")
}
fn query(pack: &str) -> ListRequest {
    ListRequest {
        pack: Some(pack.into()),
        name: None,
        limit: 500,
        cursor: None,
    }
}
fn store(temp: &TempDir, node: Value) -> Store {
    let path = temp.path().join("design.pen");
    fs::write(
        &path,
        json!({"version":"2.21","children":[node]}).to_string(),
    )
    .unwrap();
    Store::open(path, Some(temp.path().join("data"))).unwrap()
}
fn node(library: &str, name: &str) -> Value {
    json!({"id":"icon","type":"icon","library":library,"icon":name,"width":96,"height":96,"fill":"#123456"})
}
fn resolved(project: &Project) -> Arc<icons::ResolvedIcon> {
    fn find(nodes: &[lazypencil::project::ResolvedNode]) -> Option<Arc<icons::ResolvedIcon>> {
        nodes.iter().find_map(|node| {
            node.icon
                .as_ref()
                .map(|icon| icon.as_ref().unwrap().clone())
                .or_else(|| find(&node.children))
        })
    }
    find(&project.resolve().unwrap().roots).unwrap()
}

#[test]
fn optional_packs_are_discoverable_without_implicit_download_and_preserve_original_licenses() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("absent");
    for pack in [
        "phosphor",
        "Material Symbols Outlined",
        "Material Symbols Rounded",
        "Material Symbols Sharp",
    ] {
        let page = icons::list(&data, &query(pack)).unwrap();
        assert_eq!(page["diagnostic"]["code"], "ASSET_NOT_FOUND");
        if pack.starts_with("Material") {
            assert_eq!(
                page["diagnostic"]["details"]["setup_command"],
                format!("lazypencil icons setup '{pack}'")
            );
        }
        assert_eq!(page["packs"].as_array().unwrap().len(), 6);
        let license = icons::license(pack).unwrap();
        let text = license["text"].as_str().unwrap();
        if pack == "phosphor" {
            assert!(text.contains("Copyright (c) 2023 Phosphor Icons"));
            assert!(text.contains("\r\n"));
        } else {
            assert!(text.contains("Version 2.0, January 2004"));
            assert_eq!(
                license["copyright"],
                "Copyright 2026 Google LLC.  All Rights Reserved."
            );
        }
    }
    assert!(!data.exists());
}

#[test]
fn phosphor_original_archive_reproduces_names_aliases_tags_and_pinned_geometry() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let result = icons::setup(&data, "phosphor", Some(&phosphor_source())).unwrap();
    assert_eq!(result["reused"], false);
    assert_eq!(
        icons::setup(&data, "phosphor", Some(&phosphor_source())).unwrap()["reused"],
        true
    );
    let mut request = query("phosphor");
    request.name = Some("HOUSE".into());
    let page = icons::list(&data, &request).unwrap();
    let names = page["icons"].as_array().unwrap();
    assert!(names.iter().any(|icon| icon["name"] == "house"));
    let alias = names.iter().find(|icon| icon["name"] == "House").unwrap();
    assert_eq!(alias["alias_of"], "house");
    assert!(alias["tags"].is_array());
    let mut request = query("phosphor");
    let mut names = std::collections::BTreeSet::new();
    loop {
        let page = icons::list(&data, &request).unwrap();
        assert_eq!(page["total"], 3060);
        for icon in page["icons"].as_array().unwrap() {
            assert!(names.insert(icon["name"].as_str().unwrap().to_string()));
            assert_eq!(icon["variable_weight"], false);
        }
        request.cursor = page["next_cursor"].as_str().map(str::to_owned);
        if request.cursor.is_none() {
            break;
        }
    }
    let path = Path::new(result["path"].as_str().unwrap());
    let data = fs::read(path.join("icons.json")).unwrap();
    assert_eq!(
        lazypencil::document::hash(&data),
        "9ee2953ad3d4416ad5c275c0eedbf891d9ec1e7a77cbe638d3016d3106780d5e"
    );
}

#[test]
fn snapshots_share_validated_pack_bytes_and_still_reject_tampered_files() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp, node("phosphor", "House"));
    let installed = icons::setup(store.data_dir(), "phosphor", Some(&phosphor_source())).unwrap();
    let first = Project::load(&store).unwrap();
    let a = resolved(&first);
    assert_eq!(a.metadata()["canonical_name"], "house");
    let second = Project::load(&store).unwrap();
    let b = resolved(&second);
    assert!(Arc::ptr_eq(&a.pack, &b.pack));
    assert_eq!(first.dependencies().len(), 3);
    let path = Path::new(installed["path"].as_str().unwrap()).join("icons.json");
    fs::write(path, b"changed").unwrap();
    assert_eq!(
        first.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let third = Project::load(&store).unwrap();
    let scene = third.resolve().unwrap();
    assert_eq!(
        scene.roots[0]
            .icon
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
    assert_eq!(
        icons::setup(store.data_dir(), "phosphor", Some(&phosphor_source()))
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
}

#[test]
fn cli_accepts_the_offline_input_alias_and_reports_optional_pack_metadata() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "icons", "setup", "phosphor", "--input"])
        .arg(phosphor_source())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args([
            "--json", "icons", "list", "--pack", "phosphor", "--name", "HOUSE",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["icons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|icon| icon["name"] == "House")
    );
    for pack in [
        "Material Symbols Outlined",
        "Material Symbols Rounded",
        "Material Symbols Sharp",
    ] {
        let bad = temp.path().join("wrong.ttf");
        fs::write(&bad, b"not the pinned font").unwrap();
        assert_eq!(
            icons::setup(&data, pack, Some(&bad)).unwrap_err().code,
            "ICON_INTEGRITY"
        );
    }
}

#[cfg(feature = "rendering")]
#[test]
fn every_phosphor_canonical_and_alias_builds_its_original_filled_outline() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp, node("phosphor", "house"));
    icons::setup(store.data_dir(), "phosphor", Some(&phosphor_source())).unwrap();
    let mut request = query("phosphor");
    let mut nodes = Vec::new();
    loop {
        let page = icons::list(store.data_dir(), &request).unwrap();
        for icon in page["icons"].as_array().unwrap() {
            let mut n = node("phosphor", icon["name"].as_str().unwrap());
            n["id"] = json!(format!("i{}", nodes.len()));
            nodes.push(n);
        }
        request.cursor = page["next_cursor"].as_str().map(str::to_owned);
        if request.cursor.is_none() {
            break;
        }
    }
    fs::write(
        store.path(),
        json!({"version":"2.21","children":nodes}).to_string(),
    )
    .unwrap();
    for n in Project::load(&store).unwrap().resolve().unwrap().roots {
        let icon = n.icon.unwrap().unwrap();
        let path = icons::shape(&icon, skia_safe::Rect::from_wh(96.0, 96.0)).unwrap();
        assert!(!path.is_empty(), "{}", icon.name);
        assert!(path.compute_tight_bounds().is_finite(), "{}", icon.name);
    }
}

#[cfg(feature = "tui")]
#[test]
fn phosphor_holes_paints_and_fixed_weight_diagnostics_reach_the_real_renderer() {
    let temp = TempDir::new().unwrap();
    let mut icon = node("phosphor", "Circle");
    icon["fill"] = json!("#FF000080");
    let store = store(&temp, icon);
    icons::setup(store.data_dir(), "phosphor", Some(&phosphor_source())).unwrap();
    let (project, layout) = lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
    let rendered = lazypencil::render::png(&project, &layout, &Default::default()).unwrap();
    let image = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
    assert_eq!(image.get_pixel(48, 48).0[3], 0);
    assert!(image.pixels().any(|pixel| pixel.0 == [255, 0, 0, 128]));
    assert!(image.pixels().all(|pixel| pixel.0[3] <= 128));
    let mut icon = node("phosphor", "circle");
    icon["weight"] = json!(400);
    fs::write(
        store.path(),
        json!({"version":"2.21","children":[icon]}).to_string(),
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    assert_eq!(
        project.resolve().unwrap().roots[0]
            .icon
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .code,
        "UNSUPPORTED_FEATURE"
    );
}

#[cfg(feature = "tui")]
#[test]
#[ignore = "Requires the three exact official Material TTFs in LAZYPENCIL_TEST_MATERIAL_DIR"]
fn official_material_fonts_resolve_every_codepoint_and_continuous_weights_without_synthesis() {
    let sources = PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_MATERIAL_DIR").expect("Set LAZYPENCIL_TEST_MATERIAL_DIR"),
    );
    std::thread::scope(|scope| {
        let workers = ["Outlined", "Rounded", "Sharp"].map(|style| {
            let sources = &sources;
            (
                style,
                scope.spawn(move || verify_material_style(sources, style)),
            )
        });
        let failures = workers
            .into_iter()
            .filter_map(|(style, worker)| worker.join().err().map(|_| style))
            .collect::<Vec<_>>();
        assert!(failures.is_empty(), "Material styles failed: {failures:?}");
    });
}

#[cfg(feature = "tui")]
fn verify_material_style(sources: &Path, style: &str) {
    // Each style mutates and corrupts only its own installed font copy. The
    // pinned upstream directory is immutable and shared read-only by workers.
    let temp = TempDir::new().unwrap();
    let store = store(&temp, node("lucide", "circle"));
    let library = format!("Material Symbols {style}");
    let source = sources.join(format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf"));
    let setup = icons::setup(store.data_dir(), &library, Some(&source)).unwrap();
    let mut request = query(&library);
    let mut nodes = Vec::new();
    loop {
        let page = icons::list(store.data_dir(), &request).unwrap();
        assert_eq!(page["total"], 4304);
        for entry in page["icons"].as_array().unwrap() {
            assert_eq!(entry["weight_range"], json!([100, 700]));
            assert_eq!(entry["default_weight"], 400);
            let mut n = node(&library, entry["name"].as_str().unwrap());
            n["id"] = json!(format!("i{}", nodes.len()));
            nodes.push(n);
        }
        request.cursor = page["next_cursor"].as_str().map(str::to_owned);
        if request.cursor.is_none() {
            break;
        }
    }
    fs::write(
        store.path(),
        json!({"version":"2.21","children":nodes}).to_string(),
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    assert_eq!(project.dependencies().len(), 4);
    for n in project.resolve().unwrap().roots {
        let icon = n.icon.unwrap().unwrap();
        assert_eq!(icon.weight, Some(400.0));
        let path = icons::shape(&icon, skia_safe::Rect::from_wh(24.0, 24.0))
            .unwrap_or_else(|e| panic!("{library} {}: {e}", icon.name));
        assert!(!path.is_empty(), "{library} {}", icon.name);
        assert!(path.compute_tight_bounds().is_finite());
    }
    let mut hashes = Vec::new();
    let mut ink = Vec::new();
    for weight in [
        100.0, 200.0, 300.0, 350.5, 400.0, 450.0, 500.0, 600.0, 700.0, 100.0,
    ] {
        fs::write(store.path(),json!({"version":"2.21","themes":{"mode":["light","dark"]},"variables":{"thickness":{"type":"number","value":[{"value":weight,"theme":{"mode":"light"}},{"value":700,"theme":{"mode":"dark"}}]}},"children":[{"id":"component","type":"frame","reusable":true,"enabled":false,"children":[{"id":"symbol","type":"icon","library":library,"icon":"home","weight":"$thickness","width":96,"height":96,"fill":"#123456"}]},{"id":"instance","type":"ref","ref":"component","enabled":true,"theme":{"mode":"light"}}]}).to_string()).unwrap();
        let (project, layout) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        assert!(
            layout
                .items
                .iter()
                .all(|item| item.record.diagnostics.is_empty())
        );
        let rendered = lazypencil::render::png(&project, &layout, &Default::default()).unwrap();
        let image = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
        ink.push(image.pixels().filter(|p| p.0[3] > 128).count());
        hashes.push(lazypencil::document::hash(&rendered.png));
        assert!(
            layout.items.iter().any(|item| item
                .record
                .icon
                .as_ref()
                .is_some_and(|icon| icon["weight"] == weight)),
            "{weight}"
        );
    }
    assert_eq!(hashes[0], hashes[9]);
    assert!(ink[8] > ink[0] * 2, "{library}: {ink:?}");
    assert_ne!(hashes[3], hashes[4]);
    let mut design: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    design["children"][1]["theme"] = json!({"mode":"dark"});
    fs::write(store.path(), design.to_string()).unwrap();
    let (project, layout) = lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
    let rendered = lazypencil::render::png(&project, &layout, &Default::default()).unwrap();
    assert_eq!(lazypencil::document::hash(&rendered.png), hashes[8]);
    let project = Project::load(&store).unwrap();
    let captured = resolved(&project);
    let again = Project::load(&store).unwrap();
    let shared = resolved(&again);
    assert!(Arc::ptr_eq(&captured.pack, &shared.pack));
    let font = Path::new(setup["path"].as_str().unwrap()).join("font.ttf");
    let mut bytes = fs::read(&font).unwrap();
    bytes[0] ^= 1;
    fs::write(&font, bytes).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        icons::list(store.data_dir(), &query(&library))
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
    assert_eq!(
        icons::setup(store.data_dir(), &library, Some(&source))
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
}

#[cfg(feature = "tui")]
#[test]
#[ignore = "Requires pinned Chromium and original Material sources in LAZYPENCIL_TEST_MATERIAL_DIR"]
fn optional_icon_paths_match_original_svg_and_font_rendering_in_chromium() {
    let sources = PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_MATERIAL_DIR").expect("Set LAZYPENCIL_TEST_MATERIAL_DIR"),
    );
    let browser =
        std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM");
    let server = reference::Server::new(&sources, &phosphor_source());
    let temp = TempDir::new().unwrap();
    let store = store(&temp, node("phosphor", "house"));
    icons::setup(store.data_dir(), "phosphor", Some(&phosphor_source())).unwrap();
    for style in ["Outlined", "Rounded", "Sharp"] {
        icons::setup(
            store.data_dir(),
            &format!("Material Symbols {style}"),
            Some(&sources.join(format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf"))),
        )
        .unwrap();
    }
    let mut cases = Vec::new();
    for name in [
        "house",
        "circle",
        "gear",
        "arrow-right",
        "chart-bar",
        "check",
        "heart",
    ] {
        cases.push((
            "phosphor".to_string(),
            name.to_string(),
            None,
            format!("phosphor/{name}"),
            48,
        ));
    }
    for style in ["Outlined", "Rounded", "Sharp"] {
        for weight in [100.0, 350.5, 700.0] {
            cases.push((
                format!("Material Symbols {style}"),
                "settings".to_string(),
                Some(weight),
                format!("{style}/{weight}"),
                96,
            ));
        }
    }
    let mut runtime = lazypencil::dynamic::DynamicRuntime::new(store.data_dir())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(browser);
    for (library, name, weight, route, height) in cases {
        let mut icon = node(&library, &name);
        icon["height"] = json!(height);
        if let Some(weight) = weight {
            icon["weight"] = json!(weight);
        }
        let url = format!("http://{}/{route}", server.address);
        fs::write(store.path(),json!({"version":"2.21","children":[icon,{"id":"reference","type":"browser","x":120,"width":96,"height":height,"url":url}]}).to_string()).unwrap();
        let (project, layout) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered = lazypencil::render::png_with_runtime(
            &project,
            &layout,
            &lazypencil::render::RenderOptions {
                background: skia_safe::Color::WHITE,
                ..Default::default()
            },
            &mut runtime,
            None,
        )
        .unwrap();
        let pixels = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
        let mut difference = 0u64;
        let mut ink = 0;
        for y in 0..height {
            for x in 0..96 {
                let native = pixels.get_pixel(x, y).0;
                let original = pixels.get_pixel(x + 120, y).0;
                difference += native
                    .iter()
                    .zip(original)
                    .map(|(a, b)| u64::from(a.abs_diff(b)))
                    .sum::<u64>();
                ink += usize::from(native[0] < 200);
            }
        }
        let average = difference as f64 / (96.0 * f64::from(height) * 4.0);
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_ICON_OUTPUT_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            let stem = format!(
                "{}-{}-{}",
                library.replace(' ', "-"),
                name,
                weight.unwrap_or(0.0)
            );
            fs::write(directory.join(format!("{stem}.png")), &rendered.png).unwrap();
            fs::write(directory.join(format!("{stem}.json")),serde_json::to_vec_pretty(&json!({"mean_difference":average,"layout":layout.records(),"artifacts":rendered.dynamic_artifacts})).unwrap()).unwrap();
        }
        assert!(ink > 10, "{library} {name} {weight:?} has no ink");
        assert!(
            average < 2.0,
            "{library} {name} {weight:?}: mean difference {average}"
        );
        eprintln!("{library} {name} {weight:?}: mean difference {average:.6}");
    }
}
