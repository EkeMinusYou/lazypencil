use lazypencil::{
    icons::{self, ListRequest},
    project::Project,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

fn archive() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icons/feather-icons-4.29.2.tgz")
}
fn request(pack: &str) -> ListRequest {
    ListRequest {
        pack: Some(pack.into()),
        name: None,
        limit: 500,
        cursor: None,
    }
}
fn fixture(temp: &TempDir, node: Value) -> Store {
    let path = temp.path().join("design.pen");
    std::fs::write(
        &path,
        json!({"version":"2.21","children":[node]}).to_string(),
    )
    .unwrap();
    Store::open(path, Some(temp.path().join("data"))).unwrap()
}

#[test]
fn bundled_catalogue_is_paged_searchable_alias_aware_and_never_installs_implicitly() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("unused");
    let mut query = request("lucide");
    query.limit = 173;
    let mut names = BTreeSet::new();
    loop {
        let page = icons::list(&data, &query).unwrap();
        assert_eq!(page["total"], 2154);
        for entry in page["icons"].as_array().unwrap() {
            assert!(names.insert(entry["name"].as_str().unwrap().to_string()));
            assert_eq!(entry["license"], "ISC AND MIT");
        }
        query.cursor = page["next_cursor"].as_str().map(str::to_string);
        if query.cursor.is_none() {
            break;
        }
    }
    assert_eq!(names.len(), 2154);
    query.name = Some("home".into());
    let page = icons::list(&data, &query).unwrap();
    let home = page["icons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|icon| icon["name"] == "home")
        .unwrap();
    assert_eq!(home["alias_of"], "house");
    query.name = Some("settings".into());
    query.limit = 1;
    let first = icons::list(&data, &query).unwrap();
    query.cursor = first["next_cursor"].as_str().map(str::to_string);
    assert!(query.cursor.is_some());
    query.name = Some("search".into());
    assert_eq!(
        icons::list(&data, &query).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
    let feather = icons::list(&data, &request("feather")).unwrap();
    assert_eq!(feather["diagnostic"]["code"], "ASSET_NOT_FOUND");
    assert!(feather["icons"].as_array().unwrap().is_empty());
    assert!(!data.exists());
    assert_eq!(
        icons::setup(&data, "lucide", None).unwrap()["bundled"],
        true
    );
    assert!(!data.exists());
}

#[test]
fn icon_pins_have_real_source_digests_and_complete_matching_licenses() {
    use base64::Engine;
    for descriptor in icons::catalogue() {
        for digest in [
            &descriptor.source_sha256,
            &descriptor.data_sha256,
            &descriptor.license_sha256,
        ] {
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        let integrity = descriptor.source_integrity.strip_prefix("sha512-").unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(integrity)
                .unwrap()
                .len(),
            64
        );
        let license = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources/icons")
                .join(&descriptor.id)
                .join("LICENSE"),
        )
        .unwrap();
        assert_eq!(
            lazypencil::document::hash(&license),
            descriptor.license_sha256
        );
    }
}

#[test]
fn explicit_feather_installation_verifies_source_data_license_and_reuses_atomically() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let wrong = temp.path().join("wrong.tgz");
    std::fs::write(&wrong, b"not the pinned package").unwrap();
    assert_eq!(
        icons::setup(&data, "feather", Some(&wrong))
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
    assert!(!data.join("icons/feather/4.29.2").exists());
    let first = icons::setup(&data, "feather", Some(&archive())).unwrap();
    assert_eq!(first["reused"], false);
    let second = icons::setup(&data, "feather", Some(&wrong)).unwrap();
    assert_eq!(second["reused"], true);
    let list = icons::list(&data, &request("feather")).unwrap();
    assert_eq!(list["total"], 287);
    let path = Path::new(first["path"].as_str().unwrap());
    assert!(
        std::fs::read_to_string(path.join("LICENSE"))
            .unwrap()
            .contains("Cole Bemis")
    );
    let original = std::fs::read(path.join("icons.json")).unwrap();
    std::fs::write(path.join("icons.json"), b"modified").unwrap();
    assert_eq!(
        icons::list(&data, &request("feather")).unwrap_err().code,
        "ICON_INTEGRITY"
    );
    assert_eq!(
        icons::setup(&data, "feather", Some(&archive()))
            .unwrap_err()
            .code,
        "ICON_INTEGRITY"
    );
    assert_eq!(std::fs::read(path.join("icons.json")).unwrap(), b"modified");
    std::fs::write(path.join("icons.json"), original).unwrap();
    std::fs::write(path.join("LICENSE"), b"stripped license").unwrap();
    assert_eq!(
        icons::list(&data, &request("feather")).unwrap_err().code,
        "ICON_INTEGRITY"
    );
}

#[test]
fn concurrent_icon_setup_converges_and_snapshots_detect_installation_and_file_changes() {
    let temp = TempDir::new().unwrap();
    let store = fixture(
        &temp,
        json!({"id":"icon","type":"icon","library":"feather","icon":"check","width":24,"height":24,"fill":"#000000"}),
    );
    let unavailable = Project::load(&store).unwrap();
    let scene = unavailable.resolve().unwrap();
    assert_eq!(
        scene.roots[0]
            .icon
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .code,
        "ASSET_NOT_FOUND"
    );
    let handles = (0..2)
        .map(|_| {
            let data = store.data_dir().to_path_buf();
            std::thread::spawn(move || icons::setup(&data, "feather", Some(&archive())).unwrap())
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| result["reused"] == false)
            .count(),
        1
    );
    assert_eq!(
        unavailable.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let project = Project::load(&store).unwrap();
    assert_eq!(project.dependencies().len(), 3);
    assert!(
        project
            .dependencies()
            .iter()
            .all(|dependency| dependency.kind == "icon_pack")
    );
    assert_eq!(
        project.resolve().unwrap().roots[0]
            .icon_metadata
            .as_ref()
            .unwrap()["version"],
        "4.29.2"
    );
    let saved = store.read().unwrap().revision();
    let folder = Path::new(results[0]["path"].as_str().unwrap());
    std::fs::write(folder.join("LICENSE"), b"changed").unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(store.read().unwrap().revision(), saved);
}

#[cfg(unix)]
#[test]
fn icon_file_symlink_retargeting_invalidates_live_and_cached_snapshots() {
    use std::{fs, os::unix::fs::symlink};
    for filename in ["manifest.json", "LICENSE", "icons.json"] {
        let temp = TempDir::new().unwrap();
        let store = fixture(
            &temp,
            json!({"id":"icon","type":"icon","library":"feather","icon":"circle","width":24,"height":24}),
        );
        let setup = icons::setup(store.data_dir(), "feather", Some(&archive())).unwrap();
        let requested = Path::new(setup["path"].as_str().unwrap()).join(filename);
        let original = temp.path().join("original");
        let replacement = temp.path().join("replacement");
        fs::copy(&requested, &original).unwrap();
        fs::copy(&requested, &replacement).unwrap();
        fs::remove_file(&requested).unwrap();
        symlink(&original, &requested).unwrap();
        let first = Project::load(&store).unwrap();
        let cached = Project::load(&store).unwrap();
        first.verify_unchanged().unwrap();
        cached.verify_unchanged().unwrap();

        fs::remove_file(&requested).unwrap();
        symlink(&replacement, &requested).unwrap();
        for project in [&first, &cached] {
            assert_eq!(
                project.verify_unchanged().unwrap_err().code,
                "REVISION_CONFLICT",
                "{filename}"
            );
        }
        let fresh = Project::load(&store).unwrap();
        fresh.verify_unchanged().unwrap();
        assert_ne!(fresh.dependency_revision(), first.dependency_revision());

        fs::remove_file(&requested).unwrap();
        symlink(&original, &requested).unwrap();
        first.verify_unchanged().unwrap();
        assert_eq!(
            fresh.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT"
        );
        fs::remove_file(&requested).unwrap();
        assert_eq!(
            first.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT"
        );
    }
}

#[cfg(unix)]
#[test]
fn replacing_a_pack_directory_symlink_invalidates_its_captured_file_targets() {
    use std::{fs, os::unix::fs::symlink};
    let temp = TempDir::new().unwrap();
    let store = fixture(
        &temp,
        json!({"id":"icon","type":"icon","library":"feather","icon":"circle","width":24,"height":24}),
    );
    let setup = icons::setup(store.data_dir(), "feather", Some(&archive())).unwrap();
    let requested = Path::new(setup["path"].as_str().unwrap());
    let original = temp.path().join("original-pack");
    let replacement = temp.path().join("replacement-pack");
    fs::rename(requested, &original).unwrap();
    fs::create_dir(&replacement).unwrap();
    for filename in ["manifest.json", "LICENSE", "icons.json"] {
        fs::copy(original.join(filename), replacement.join(filename)).unwrap();
    }
    symlink(&original, requested).unwrap();
    let project = Project::load(&store).unwrap();
    project.verify_unchanged().unwrap();
    fs::remove_file(requested).unwrap();
    symlink(&replacement, requested).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let fresh = Project::load(&store).unwrap();
    fresh.verify_unchanged().unwrap();
    assert_ne!(fresh.dependency_revision(), project.dependency_revision());
}

#[test]
fn cli_icons_search_and_offline_install_match_the_shared_service() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    for pack in ["lucide", "feather"] {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--data-dir")
            .arg(&data)
            .args(["--json", "icons", "license", pack])
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let license = value["text"].as_str().unwrap();
        assert!(license.contains("Cole Bemis"));
        assert!(license.contains("Permission"));
        assert_eq!(
            lazypencil::document::hash(license.as_bytes()),
            value["hash"]
        );
    }
    assert!(!data.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args([
            "--json", "icons", "list", "--name", "home", "--limit", "500",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut query = request("lucide");
    query.name = Some("home".into());
    assert_eq!(value, icons::list(&data, &query).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["icons", "setup", "feather", "--archive"])
        .arg(archive())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        icons::list(&data, &request("feather")).unwrap()["total"],
        287
    );
}

#[cfg(feature = "tui")]
fn render(
    store: &Store,
) -> (
    lazypencil::render::RenderedImage,
    lazypencil::layout::LayoutSnapshot,
) {
    let (project, layout) = lazypencil::service::layout_snapshot(store, &[], None, None).unwrap();
    let image = lazypencil::render::png(&project, &layout, &Default::default()).unwrap();
    (image, layout)
}
#[cfg(feature = "tui")]
fn rgba(image: &lazypencil::render::RenderedImage) -> image::RgbaImage {
    image::load_from_memory_with_format(&image.png, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

#[cfg(feature = "tui")]
#[test]
fn vector_icons_preserve_holes_aspect_ratio_gradient_and_transparent_overlaps() {
    let temp = TempDir::new().unwrap();
    let store = fixture(
        &temp,
        json!({"id":"icon","type":"icon","library":"lucide","icon":"circle","width":96,"height":48,"fill":{"type":"gradient","gradientType":"linear","rotation":90,"colors":[{"color":"#FF0000","position":0},{"color":"#0000FF","position":1}]}}),
    );
    let (image, layout) = render(&store);
    let bitmap = rgba(&image);
    assert_eq!((image.width, image.height), (96, 48));
    assert_eq!(
        bitmap.get_pixel(48, 24).0[3],
        0,
        "Holes must remain transparent"
    );
    assert_eq!(
        bitmap.get_pixel(4, 24).0[3],
        0,
        "Non-square nodes must fit without stretching"
    );
    assert!(bitmap.get_pixel(28, 24).0[3] > 200);
    assert!(bitmap.get_pixel(68, 24).0[3] > 200);
    assert_ne!(
        bitmap.get_pixel(28, 24).0,
        bitmap.get_pixel(68, 24).0,
        "Icon ink must use the real gradient paint"
    );
    assert_eq!(
        layout.items[0].record.icon.as_ref().unwrap()["canonical_name"],
        "circle"
    );
    let store = fixture(
        &temp,
        json!({"id":"icon","type":"icon","icon":"plus","width":48,"height":48,"fill":"#FF000080"}),
    );
    let (image, _) = render(&store);
    let bitmap = rgba(&image);
    assert_eq!(
        bitmap.get_pixel(24, 24).0[3],
        128,
        "Crossing primitive outlines must be one coverage mask"
    );
    assert_eq!(bitmap.get_pixel(24, 12).0[3], 128);
}

#[cfg(feature = "rendering")]
#[test]
fn every_bundled_and_installed_icon_builds_valid_nonempty_vector_geometry() {
    let temp = TempDir::new().unwrap();
    icons::setup(temp.path(), "feather", Some(&archive())).unwrap();
    let store = fixture(
        &temp,
        json!({"id":"placeholder","type":"rectangle","width":1,"height":1}),
    );
    for library in ["lucide", "feather"] {
        let mut query = request(library);
        let mut nodes = vec![];
        loop {
            let page = icons::list(temp.path(), &query).unwrap();
            for icon in page["icons"].as_array().unwrap() {
                nodes.push(json!({"id":format!("icon{}",nodes.len()),"type":"icon","library":library,"icon":icon["name"],"width":24,"height":24,"fill":"#000000"}));
            }
            query.cursor = page["next_cursor"].as_str().map(str::to_string);
            if query.cursor.is_none() {
                break;
            }
        }
        let document = lazypencil::document::Document::parse(
            json!({"version":"2.21","children":nodes}).to_string(),
        )
        .unwrap();
        let project =
            Project::from_document_with_data(store.path(), document, &[], Some(temp.path()))
                .unwrap();
        for node in project.resolve().unwrap().roots {
            let icon = node.icon.unwrap().unwrap();
            let path = icons::shape(&icon, skia_safe::Rect::from_wh(24.0, 24.0))
                .unwrap_or_else(|error| panic!("{}: {error}", icon.name));
            assert!(!path.is_empty(), "{}", icon.name);
            let bounds = path.compute_tight_bounds();
            assert!(bounds.is_finite(), "{}", icon.name);
            assert!(
                bounds.width() > 0.0 && bounds.height() > 0.0,
                "{}",
                icon.name
            );
        }
    }
}

#[cfg(feature = "rendering")]
#[test]
fn missing_icons_and_unsupported_weights_have_diagnostics_and_never_render_placeholders() {
    let temp = TempDir::new().unwrap();
    for (properties, code) in [
        (
            json!({"library":"feather","icon":"check"}),
            "ASSET_NOT_FOUND",
        ),
        (
            json!({"library":"lucide","icon":"does-not-exist"}),
            "ASSET_NOT_FOUND",
        ),
        (
            json!({"library":"lucide","icon":"check","weight":400}),
            "UNSUPPORTED_FEATURE",
        ),
    ] {
        let mut node = properties;
        node["id"] = json!("icon");
        node["type"] = json!("icon");
        node["width"] = json!(24);
        node["height"] = json!(24);
        node["fill"] = json!("#000000");
        let store = fixture(&temp, node);
        let (project, layout) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        assert!(
            layout.items[0]
                .record
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code)
        );
        assert_eq!(
            lazypencil::render::png(&project, &layout, &Default::default())
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(store.read().unwrap().node("icon").unwrap()["type"], "icon");
    }
}

#[cfg(feature = "tui")]
#[test]
fn generated_icons_capture_installed_pack_files_as_runtime_dependencies() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    icons::setup(&data, "feather", Some(&archive())).unwrap();
    std::fs::write(temp.path().join("draw.js"),"/** @schema 2.21 */ return [{type:'icon',library:'feather',icon:'check',width:pencil.width,height:pencil.height,fill:'#008000'}];").unwrap();
    let store = fixture(
        &temp,
        json!({"id":"script","type":"script","scriptUri":"draw.js","width":48,"height":48}),
    );
    let png = temp.path().join("render.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "render"])
        .arg(store.path())
        .arg("--output")
        .arg(&png)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["runtime_dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|dependency| dependency["kind"] == "icon_pack")
            .count(),
        3
    );
    assert_eq!(value["script_executions"].as_array().unwrap().len(), 1);
    let bitmap = image::open(png).unwrap().to_rgba8();
    assert!(bitmap.pixels().any(|pixel| pixel.0 == [0, 128, 0, 255]));
}

#[cfg(feature = "tui")]
#[test]
#[ignore = "Requires the pinned Chromium executable and the original pinned Lucide npm archive"]
fn native_icons_match_upstream_svg_rendered_by_a_real_browser() {
    use std::io::Read;
    let browser =
        std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM");
    let source = std::env::var_os("LAZYPENCIL_TEST_LUCIDE_ARCHIVE")
        .expect("Set LAZYPENCIL_TEST_LUCIDE_ARCHIVE");
    let source = std::fs::read(source).unwrap();
    assert_eq!(
        lazypencil::document::hash(&source),
        icons::catalogue()
            .iter()
            .find(|pack| pack.id == "lucide")
            .unwrap()
            .source_sha256
    );
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&source[..]));
    let mut originals = std::collections::BTreeMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().into_owned();
        if name.starts_with("package/icons")
            && name.extension().is_some_and(|extension| extension == "svg")
        {
            let mut svg = String::new();
            entry.read_to_string(&mut svg).unwrap();
            originals.insert(name.file_stem().unwrap().to_str().unwrap().to_string(), svg);
        }
    }
    let data: Value =
        serde_json::from_str(include_str!("../resources/icons/lucide/icons.json")).unwrap();
    let mut names = BTreeSet::from(["home".to_string(), "circle".to_string()]);
    for primitive in [
        "path", "line", "circle", "rect", "ellipse", "polyline", "polygon",
    ] {
        let (name, _) = data["icons"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, shapes)| {
                shapes
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|shape| shape[0] == primitive)
            })
            .unwrap();
        names.insert(name.clone());
    }
    let (name, _) = data["icons"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, shapes)| {
            shapes
                .as_array()
                .unwrap()
                .iter()
                .any(|shape| shape[1]["fill"] == "currentColor")
        })
        .unwrap();
    names.insert(name.clone());
    let temp = TempDir::new().unwrap();
    let mut dynamic =
        lazypencil::dynamic::DynamicRuntime::new(temp.path()).with_executable(browser);
    for name in names {
        let html = format!(
            "<style>html,body{{margin:0;background:white;color:#123456}}svg{{display:block;width:96px;height:48px}}</style>{}",
            originals[&name]
        );
        let uri = format!(
            "data:text/html;base64,{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, html)
        );
        let file = temp.path().join("design.pen");
        std::fs::write(&file,json!({"version":"2.21","children":[{"id":"icon","type":"icon","library":"lucide","icon":name,"width":96,"height":48,"fill":"#123456"},{"id":"reference","type":"browser","x":120,"width":96,"height":48,"url":uri}]}).to_string()).unwrap();
        let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
        let (project, layout) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        let result = lazypencil::render::png_with_runtime(
            &project,
            &layout,
            &lazypencil::render::RenderOptions {
                background: skia_safe::Color::WHITE,
                ..Default::default()
            },
            &mut dynamic,
            None,
        )
        .unwrap();
        let pixels = rgba(&result);
        let mut difference = 0u64;
        let mut ink = 0usize;
        for y in 0..48 {
            for x in 0..96 {
                let native = pixels.get_pixel(x, y).0;
                let expected = pixels.get_pixel(x + 120, y).0;
                difference += native
                    .iter()
                    .zip(expected)
                    .map(|(a, b)| u64::from(a.abs_diff(b)))
                    .sum::<u64>();
                ink += usize::from(native[0] < 200);
            }
        }
        assert!(ink > 10, "{name} unexpectedly has no ink");
        let average = difference as f64 / (96.0 * 48.0 * 4.0);
        assert!(
            average < 2.0,
            "{name} mean absolute channel difference {average} exceeds browser tolerance"
        );
    }
}
