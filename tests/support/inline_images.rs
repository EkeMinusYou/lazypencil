use super::http_images::{apply, cli};
use super::*;
use base64::Engine;

fn data(bytes: &[u8], base64: bool) -> String {
    if base64 {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    } else {
        format!(
            "data:image/png,{}",
            bytes
                .iter()
                .map(|byte| format!("%{byte:02X}"))
                .collect::<String>()
        )
    }
}

fn assert_captured(project: &Project, origin: &Path, uri: &str, expected: &[u8]) {
    let path = project.resolve_image(origin, uri).unwrap();
    let asset = project.asset(&path).unwrap();
    assert_eq!(asset.bytes.as_ref(), expected);
    assert_eq!(asset.kind, "embedded_image");
    assert!(
        project
            .dependencies()
            .iter()
            .any(|dependency| dependency.path == path && dependency.kind == "embedded_image")
    );
    assert!(project.resolve_file(origin, uri).is_err());
    project.verify_unchanged().unwrap();
}

#[test]
fn embedded_image_paints_copy_and_detach_without_managed_files_in_both_formats() {
    let png = include_bytes!("../../examples/assets/crop-source.png");
    for version in ["2.19", "2.21"] {
        for base64 in [false, true] {
            let temp = TempDir::new().unwrap();
            fs::create_dir(temp.path().join(".git")).unwrap();
            let uri = data(png, base64);
            write(
                &temp.path().join("library.pen"),
                json!({"version":version,"children":[{"id":"tile","type":"frame","reusable":true,"width":100,"height":50,"fill":{"type":"image","url":uri}}]}),
            );
            let file = temp.path().join("main.pen");
            write(
                &file,
                json!({"version":version,"imports":{"ui":"library.pen"},"children":[{"id":"use","type":"ref","ref":"ui:tile"}]}),
            );
            fs::write(
                temp.path().join("lazypencil.toml"),
                "[runtime.browser]\nnetwork_policy='offline'\n",
            )
            .unwrap();
            let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
            assert_captured(&Project::load(&store).unwrap(), store.path(), &uri, png);
            let before = fs::read(&file).unwrap();
            let operations = json!([
                {"op":"import_component","source":{"library_alias":"ui","node_id":"tile"},"mode":"copy"},
                {"op":"delete","target":"use"},
                {"op":"remove_import","alias":"ui"}
            ]);
            apply(&store, operations.clone(), true);
            assert_eq!(fs::read(&file).unwrap(), before);
            assert!(!temp.path().join("assets").exists());
            let result = apply(&store, operations, false);
            let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
            assert!(!temp.path().join("assets").exists());
            fs::remove_file(temp.path().join("library.pen")).unwrap();
            apply(
                &store,
                json!([{"op":"detach_instance","target":target}]),
                false,
            );
            let project = Project::load(&store).unwrap();
            assert_captured(&project, store.path(), &uri, png);
            assert_eq!(
                project.resolve().unwrap().roots[0].properties["fill"]["url"],
                uri
            );
            assert!(!temp.path().join("assets").exists());
        }
    }
}

fn sampler_fixture(version: &str) -> (TempDir, Store, [Vec<u8>; 2], [String; 2]) {
    let (temp, store) = array_fixture();
    let bytes = [
        fs::read(temp.path().join("library/data/pixel.png")).unwrap(),
        fs::read(temp.path().join("library/data/dark.png")).unwrap(),
    ];
    let uris = [data(&bytes[0], true), data(&bytes[1], false)];
    let library = temp.path().join("library/data/images.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    document["variables"]["leaf"]["value"] =
        json!([{"value":uris[0]},{"theme":{"mode":"dark"},"value":uris[1]}]);
    write(&library, document);
    let library = temp.path().join("library/ui.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    document["children"][0]["clip"] = json!(true);
    for property in ["fill", "stroke", "effect"] {
        document["children"][0][property]["uniforms"]["image"][1] = json!(uris[1]);
    }
    write(&library, document);
    for path in [
        store.path().to_owned(),
        library,
        temp.path().join("library/data/images.lib.pen"),
    ] {
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["version"] = json!(version);
        write(&path, document);
    }
    fs::write(
        temp.path().join("lazypencil.toml"),
        "[runtime.browser]\nnetwork_policy='offline'\n",
    )
    .unwrap();
    (temp, store, bytes, uris)
}

#[test]
fn embedded_sampler_strings_keep_both_themes_and_original_data_urls_after_copy_and_recopy() {
    for version in ["2.19", "2.21"] {
        let (temp, store, bytes, uris) = sampler_fixture(version);
        let result = apply(&store, copy_operations(), false);
        let target = result["generated_ids"]["0"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let component = result["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        for (index, mode) in ["light", "dark"].into_iter().enumerate() {
            apply(
                &store,
                json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
                false,
            );
            let project = Project::load(&store).unwrap();
            let scene = project.resolve().unwrap();
            let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
            assert_eq!(node.children[0].properties["content"], uris[index]);
            assert_eq!(node.properties["fill"]["uniforms"]["image"][0], uris[index]);
            assert_captured(&project, store.path(), &uris[index], &bytes[index]);
        }
        let result = apply(
            &store,
            json!([
                {"op":"import_component","source":{"node_id":component},"mode":"copy","options":{"variable_prefix":"again_"}},
                {"op":"delete","target":target}
            ]),
            false,
        );
        let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
        apply(
            &store,
            json!([{"op":"set_theme","target":target,"set":{"mode":"dark"}}]),
            false,
        );
        let project = Project::load(&store).unwrap();
        let scene = project.resolve().unwrap();
        let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
        assert_eq!(node.children[0].properties["content"], uris[1]);
        assert_captured(&project, store.path(), &uris[1], &bytes[1]);
    }
}

#[test]
fn malformed_embedded_image_payloads_remain_inspectable_and_repairable_without_leaking_data() {
    for uri in [
        "data:image/png,private-invalid-payload",
        "data:text/html,private-invalid-payload",
        "data:image/png;bad-header,private-invalid-payload",
    ] {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("main.pen");
        write(
            &file,
            json!({"version":"2.21","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":uri}}]}),
        );
        let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
        let metadata = cli(&store, &["inspect", file.to_str().unwrap()]);
        assert_eq!(metadata["assets_complete"], false);
        assert_eq!(metadata["asset_diagnostics"][0]["kind"], "embedded_image");
        assert!(
            !metadata["asset_diagnostics"]
                .to_string()
                .contains("private-invalid-payload")
        );
        apply(
            &store,
            json!([{"op":"update","target":"image","set":{"fill":"#00ff00"}}]),
            false,
        );
        assert!(Project::load(&store).unwrap().assets_complete());
        assert!(!temp.path().join("assets").exists());
    }
}

#[cfg(feature = "rendering")]
fn raster(store: &Store) -> Vec<u8> {
    let (project, layout) = service::layout_snapshot(store, &[], None, None).unwrap();
    lazypencil::render::png(&project, &layout, &Default::default())
        .unwrap()
        .png
}

#[cfg(feature = "rendering")]
#[test]
fn embedded_images_match_local_image_pixels_after_the_original_file_is_removed() {
    let png = include_bytes!("../../examples/assets/crop-source.png");
    for version in ["2.19", "2.21"] {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("main.pen");
        let image = temp.path().join("image.png");
        fs::write(&image, png).unwrap();
        let mut document = json!({"version":version,"children":[{"id":"image","type":"rectangle","width":100,"height":50,"fill":{"type":"image","url":"image.png","mode":"contain"}}]});
        write(&file, document.clone());
        let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
        let expected = raster(&store);
        fs::remove_file(image).unwrap();
        for base64 in [false, true] {
            document["children"][0]["fill"]["url"] = json!(data(png, base64));
            write(&file, document.clone());
            assert_eq!(raster(&store), expected);
        }
        assert!(!temp.path().join("assets").exists());
    }
}

#[cfg(feature = "rendering")]
#[test]
fn generated_embedded_images_survive_independent_script_copy_source_removal_and_recopy() {
    use lazypencil::{render, script::ScriptRuntime};
    let png = include_bytes!("../../examples/assets/crop-source.png");
    let uri = data(png, true);
    assert!(
        uri.len() > 4096,
        "Inline image bytes are not filesystem path names"
    );
    for version in ["2.19", "2.21"] {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join("library")).unwrap();
        let generated = json!([{"id":"image","type":"rectangle","width":32,"height":16,"fill":{"type":"image","url":uri}}]);
        fs::write(
            temp.path().join("library/generator.js"),
            format!("/** @schema {version} */\nreturn {generated};"),
        )
        .unwrap();
        write(
            &temp.path().join("library/library.pen"),
            json!({"version":version,"children":[{"id":"generator","type":"script","reusable":true,"width":32,"height":16,"scriptUri":"generator.js"}]}),
        );
        let file = temp.path().join("main.pen");
        write(
            &file,
            json!({"version":version,"imports":{"ui":"library/library.pen"},"children":[{"id":"use","type":"ref","ref":"ui:generator"}]}),
        );
        let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
        let render = || {
            let (project, layout) = service::layout_snapshot_with_runtime(
                &store,
                &[],
                None,
                None,
                &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
            )
            .unwrap();
            assert!(
                project
                    .runtime_dependencies()
                    .iter()
                    .any(|dependency| dependency.kind == "embedded_image")
            );
            let key = project.resolve_image(store.path(), &uri).unwrap();
            assert_eq!(project.asset(&key).unwrap().bytes.as_ref(), png);
            render::png(&project, &layout, &Default::default())
                .unwrap()
                .png
        };
        let expected = render();
        let result = apply(
            &store,
            json!([
                {"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy","options":{"script_assets":{"ui:generator":[{"kind":"image","uri":uri}]}}},
                {"op":"delete","target":"use"},
                {"op":"remove_import","alias":"ui"}
            ]),
            false,
        );
        let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
        let component = result["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        assert_eq!(render(), expected);
        apply(
            &store,
            json!([
                {"op":"import_component","source":{"node_id":component},"mode":"copy"},
                {"op":"delete","target":target}
            ]),
            false,
        );
        assert_eq!(render(), expected);
    }
}

#[cfg(feature = "rendering")]
pub(super) fn real_webgl_case(evidence: &mut Vec<Value>) {
    use lazypencil::{dynamic::DynamicRuntime, render};
    use skia_safe::{Color, Data, Image};
    let (temp, store, _bytes, _uris) = sampler_fixture("2.21");
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
    let mut originals = Vec::new();
    for mode in ["light", "dark"] {
        apply(
            &store,
            json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
            false,
        );
        let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered =
            render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
                .unwrap();
        let image = Image::from_encoded(Data::new_copy(&rendered.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            image.peek_pixels().unwrap().get_color((10, 10)),
            if mode == "light" {
                Color::YELLOW
            } else {
                Color::BLUE
            }
        );
        assert_eq!(
            image.peek_pixels().unwrap().get_color((75, 10)),
            Color::BLUE
        );
        originals.push(rendered.png);
    }
    let result = apply(&store, copy_operations(), false);
    let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    for (index, mode) in ["light", "dark"].into_iter().enumerate() {
        apply(
            &store,
            json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
            false,
        );
        let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered =
            render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
                .unwrap();
        assert_eq!(
            rendered.png, originals[index],
            "Embedded sampler copy changed {mode} pixels"
        );
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            let directory = Path::new(&directory);
            let before = format!("codec-sampler-copy-data-{mode}-before.png");
            let after = format!("codec-sampler-copy-data-{mode}-after.png");
            fs::write(directory.join(&before), &originals[index]).unwrap();
            fs::write(directory.join(&after), &rendered.png).unwrap();
            evidence.push(json!({"source_kind":"data","absolute":false,"theme":mode,"before":before,"after":after,"width":rendered.width,"height":rendered.height,"png_hash":lazypencil::document::hash(&rendered.png)}));
        }
    }
}
