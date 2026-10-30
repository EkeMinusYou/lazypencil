use lazypencil::{operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;

#[path = "support/http_images.rs"]
mod http_images;

#[path = "support/inline_images.rs"]
mod inline_images;

#[path = "support/script_image_urls.rs"]
mod script_image_urls;

fn write(path: &Path, value: Value) {
    fs::write(path, value.to_string()).unwrap();
}

fn image(path: &Path, dark: bool) {
    #[cfg(feature = "rendering")]
    {
        use skia_safe::Color;
        let mut surface = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
        surface
            .canvas()
            .clear(if dark { Color::BLUE } else { Color::YELLOW });
        fs::write(
            path,
            skia_safe::png_encoder::encode_image(
                None,
                &surface.image_snapshot(),
                &Default::default(),
            )
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    }
    #[cfg(not(feature = "rendering"))]
    {
        use base64::Engine;
        let encoded = if dark {
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYPj/HwADAgH/5ncLrgAAAABJRU5ErkJggg=="
        } else {
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg=="
        };
        fs::write(
            path,
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();
    }
}

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir_all(temp.path().join("library/data")).unwrap();
    for dir in [
        temp.path().to_path_buf(),
        temp.path().join("library"),
        temp.path().join("library/data"),
    ] {
        image(
            &dir.join("pixel.png"),
            dir != temp.path().join("library/data"),
        );
        image(
            &dir.join("dark.png"),
            dir == temp.path().join("library/data"),
        );
    }
    write(
        &temp.path().join("library/data/images.lib.pen"),
        json!({"version":"2.21",
        "themes":{"mode":["light","dark"]},"variables":{"leaf":{"type":"string","value":[{"value":"pixel.png"},{"theme":{"mode":"dark"},"value":"dark.png"}]}},"children":[]}),
    );
    fs::write(temp.path().join("library/sample.glsl"),"precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D image;\nuniform float amount;\nuniform vec4 tone;\nvoid main(){gl_FragColor=texture2D(image,gl_FragCoord.xy/size)*tone*amount;}").unwrap();
    let shader = json!({"type":"shader","url":"sample.glsl","uniforms":{"image":"$alias","amount":"$amount","tone":"$tone"}});
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({"version":"2.21","imports":{"images":"data/images.lib.pen"},
        "variables":{"picture":{"type":"string","value":"$images:leaf"},"alias":{"type":"string","value":"$picture"},"amount":{"type":"number","value":1},"tone":{"type":"color","value":"#ffffff"}},
        "children":[{"id":"tile","type":"frame","reusable":true,"width":100,"height":50,"layout":"none",
            "fill":shader,"stroke":shader,"strokeWidth":1,"effect":shader,
            "children":[{"id":"label","type":"text","content":"$picture","y":25,"fontFamily":"Noto Sans JP","fontSize":10,"fill":"#000000"}]}]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"picture":{"type":"string","value":"wrong.png"}},"children":[{"id":"use","type":"ref","ref":"ui:tile"}]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn array_fixture() -> (TempDir, Store) {
    let (temp, store) = fixture();
    let library = temp.path().join("library/ui.lib.pen");
    let mut value: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    for property in ["fill", "stroke", "effect"] {
        value["children"][0][property]["uniforms"]["image"] = json!(["$alias", "pixel.png"]);
    }
    write(&library, value);
    let shader = temp.path().join("library/sample.glsl");
    let source = fs::read_to_string(&shader)
        .unwrap()
        .replace("sampler2D image;", "sampler2D image[2];")
        .replace("texture2D(image,gl_FragCoord.xy/size)", "mix(texture2D(image[0],gl_FragCoord.xy/size),texture2D(image[1],gl_FragCoord.xy/size),step(0.5,gl_FragCoord.x/size.x))");
    fs::write(&shader, source).unwrap();
    (temp, store)
}

fn absolute_fixture() -> (TempDir, Store, [String; 2]) {
    let (temp, store) = array_fixture();
    let paths = ["pixel.png", "dark.png"].map(|name| {
        temp.path()
            .join("library/data")
            .join(name)
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    });
    let source = temp.path().join("library/data/images.lib.pen");
    let mut value: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    value["variables"]["leaf"]["value"] = json!([
        {"value":paths[0]},
        {"theme":{"mode":"dark"},"value":paths[1]}
    ]);
    write(&source, value);
    (temp, store, paths)
}

fn file_url_fixture(version: &str) -> (TempDir, Store, [String; 2]) {
    let (temp, store, paths) = absolute_fixture();
    let urls = paths.map(|path| {
        let target = Path::new(&path).with_file_name(format!(
            "画像 % # ? {}",
            Path::new(&path).file_name().unwrap().to_str().unwrap()
        ));
        fs::rename(path, &target).unwrap();
        url::Url::from_file_path(target).unwrap().to_string()
    });
    let source = temp.path().join("library/data/images.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    document["variables"]["leaf"]["value"] = json!([
        {"value":urls[0]},
        {"theme":{"mode":"dark"},"value":urls[1]}
    ]);
    write(&source, document);
    for path in [
        source,
        temp.path().join("library/ui.lib.pen"),
        store.path().into(),
    ] {
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["version"] = json!(version);
        write(&path, document);
    }
    (temp, store, urls)
}

#[cfg(unix)]
const PROJECTIONS: &[&str] = &["aliases", "file_directory", "case"];

#[cfg(unix)]
fn projection_fixture(case: &str, version: &str) -> (TempDir, Store, [&'static str; 2]) {
    let (temp, store) = array_fixture();
    let data = temp.path().join("library/data");
    let strings = match case {
        "aliases" | "file_directory" => {
            let other = temp.path().join("library/other");
            fs::create_dir_all(other.join("deep")).unwrap();
            std::os::unix::fs::symlink(other.join("deep"), data.join("jump")).unwrap();
            if case == "aliases" {
                image(&other.join("pixel.png"), true);
                ["pixel.png", "jump/../pixel.png"]
            } else {
                fs::create_dir(data.join("tile.png")).unwrap();
                image(&data.join("tile.png/pixel.png"), false);
                image(&other.join("tile.png"), true);
                ["tile.png/pixel.png", "jump/../tile.png"]
            }
        }
        "case" => {
            // Case-insensitive filesystems already expose the second spelling.
            if !data.join("PIXEL.png").exists() {
                fs::hard_link(data.join("pixel.png"), data.join("PIXEL.png")).unwrap();
            }
            ["pixel.png", "PIXEL.png"]
        }
        _ => panic!("Unknown projection fixture"),
    };
    for name in [
        "main.pen",
        "library/ui.lib.pen",
        "library/data/images.lib.pen",
    ] {
        let path = temp.path().join(name);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["version"] = json!(version);
        if name.ends_with("images.lib.pen") {
            value["variables"]["leaf"]["value"] = json!([
                {"value":strings[0]},
                {"theme":{"mode":"dark"},"value":strings[1]}
            ]);
        }
        write(&path, value);
    }
    (temp, store, strings)
}

#[cfg(unix)]
fn recopy_sampler(source: &Store, component: &str, destination: &Path) -> (Store, String) {
    fs::create_dir_all(destination).unwrap();
    write(
        &destination.join("main.pen"),
        json!({"version":source.read().unwrap().version(),"imports":{"source":source.path()},"children":[]}),
    );
    let store = Store::open(destination.join("main.pen"), Some(destination.join("data"))).unwrap();
    let result = edit(
        &store,
        "recopy-projection",
        json!([
            {"op":"import_component","source":{"library_alias":"source","node_id":component},"mode":"copy","options":{"variable_prefix":"recopied_"}},
            {"op":"remove_import","alias":"source"}
        ]),
    );
    let id = result["generated_ids"]["0"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    (store, id)
}

#[cfg(unix)]
fn assert_copied_sampler(store: &Store, id: &str, text: &str, expected_hash: &str) {
    let project = Project::load(store).unwrap();
    let node = project
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == id)
        .unwrap();
    assert_eq!(node.children[0].properties["content"], text);
    for property in ["fill", "stroke", "effect"] {
        let uri = node.properties[property]["uniforms"]["image"][0]
            .as_str()
            .unwrap();
        let path = Path::new(uri);
        assert!(path.starts_with(store.path().parent().unwrap().join("assets/lazypencil")));
        assert_eq!(project.asset(path).unwrap().revision, expected_hash);
    }
}

fn edit(store: &Store, id: &str, operations: Value) -> Value {
    let metadata = service::document_metadata(store).unwrap();
    let request:EditRequest = serde_json::from_value(json!({"request_id":id,"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":operations})).unwrap();
    service::apply_request(store, &request).unwrap()
}

#[test]
fn sampler_aliases_keep_the_terminal_definition_origin_in_every_graphics_property() {
    let (temp, store) = fixture();
    for mode in ["light", "dark"] {
        edit(
            &store,
            mode,
            json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
        );
        let project = Project::load(&store).unwrap();
        let node = project.resolve().unwrap().roots.remove(0);
        let expected = temp
            .path()
            .join("library/data")
            .join(if mode == "light" {
                "pixel.png"
            } else {
                "dark.png"
            })
            .canonicalize()
            .unwrap();
        for property in ["fill", "stroke", "effect"] {
            assert_eq!(
                node.properties[property]["uniforms"]["image"],
                json!(expected)
            );
            assert_eq!(node.properties[property]["uniforms"]["amount"], 1);
            assert_eq!(node.properties[property]["uniforms"]["tone"], "#ffffff");
        }
        assert_eq!(
            node.children[0].properties["content"],
            if mode == "light" {
                "pixel.png"
            } else {
                "dark.png"
            }
        );
        let images = project
            .dependencies()
            .into_iter()
            .filter(|entry| entry.kind == "image")
            .collect::<Vec<_>>();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].path, expected);
    }
}

#[test]
fn sampler_array_elements_keep_individual_origins_through_detachment_and_themes() {
    let (temp, store) = array_fixture();
    for detached in [false, true] {
        if detached {
            edit(
                &store,
                "detach-array",
                json!([{"op":"detach_instance","target":"use"}]),
            );
        }
        for mode in ["light", "dark"] {
            edit(
                &store,
                &format!("array-{detached}-{mode}"),
                json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
            );
            let project = Project::load(&store).unwrap();
            let node = project.resolve().unwrap().roots.remove(0);
            let leaf = temp
                .path()
                .join("library/data")
                .join(if mode == "light" {
                    "pixel.png"
                } else {
                    "dark.png"
                })
                .canonicalize()
                .unwrap();
            let literal = temp
                .path()
                .join("library/pixel.png")
                .canonicalize()
                .unwrap();
            for property in ["fill", "stroke", "effect"] {
                let inputs = node.properties[property]["uniforms"]["image"]
                    .as_array()
                    .unwrap();
                for (input, expected) in inputs.iter().zip([&leaf, &literal]) {
                    assert_eq!(
                        project
                            .resolve_file(node.property_source(property), input.as_str().unwrap())
                            .unwrap(),
                        *expected
                    );
                }
            }
            let images = project
                .dependencies()
                .into_iter()
                .filter(|entry| entry.kind == "image")
                .map(|entry| entry.path)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(images, [leaf, literal].into_iter().collect());
            assert_eq!(
                node.children[0].properties["content"],
                if mode == "light" {
                    "pixel.png"
                } else {
                    "dark.png"
                }
            );
        }
    }
    let project = Project::load(&store).unwrap();
    fs::write(temp.path().join("library/pixel.png"), b"changed image").unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn detaching_sampler_variables_retains_their_shared_text_values_and_future_edits() {
    let (temp, store) = fixture();
    let source = temp.path().join("library/data/images.lib.pen");
    let before = fs::read(&source).unwrap();
    let project = Project::load(&store).unwrap();
    let original = project.resolve().unwrap().roots.remove(0).properties;
    edit(
        &store,
        "detach",
        json!([{"op":"detach_instance","target":"use"}]),
    );
    let document = store.read().unwrap();
    assert_eq!(
        document.node("use").unwrap()["fill"]["uniforms"]["image"],
        "$ui:alias"
    );
    assert_eq!(
        document.value()["variables"]["picture"]["value"],
        "wrong.png"
    );
    assert_eq!(fs::read(&source).unwrap(), before);
    let resolved = Project::load(&store)
        .unwrap()
        .resolve()
        .unwrap()
        .roots
        .remove(0);
    for property in ["fill", "stroke", "effect"] {
        assert_eq!(
            original[property]["uniforms"],
            resolved.properties[property]["uniforms"]
        );
        let before_shader = temp
            .path()
            .join("library")
            .join(original[property]["url"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        let after_shader = temp
            .path()
            .join(resolved.properties[property]["url"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert_eq!(before_shader, after_shader);
    }
    image(&temp.path().join("library/data/replacement.png"), true);
    let mut changed: Value = serde_json::from_slice(&before).unwrap();
    changed["variables"]["leaf"]["value"] = json!("replacement.png");
    write(&source, changed);
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let resolved = Project::load(&store)
        .unwrap()
        .resolve()
        .unwrap()
        .roots
        .remove(0);
    assert_eq!(
        resolved.children[0].properties["content"],
        "replacement.png"
    );
    assert_eq!(
        resolved.properties["fill"]["uniforms"]["image"],
        json!(
            temp.path()
                .join("library/data/replacement.png")
                .canonicalize()
                .unwrap()
        )
    );
}

#[test]
fn reserved_sampler_values_and_colors_do_not_become_file_paths() {
    for value in ["pencil:object", "pencil:background", "#112233", ""] {
        let (temp, store) = fixture();
        let source = temp.path().join("library/data/images.lib.pen");
        let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        document["variables"]["leaf"]["value"] = json!(value);
        write(&source, document);
        let node = Project::load(&store)
            .unwrap()
            .resolve()
            .unwrap()
            .roots
            .remove(0);
        assert_eq!(node.properties["fill"]["uniforms"]["image"], value);
    }
}

#[test]
fn inactive_sampler_variables_do_not_open_missing_or_nonlocal_images() {
    for (value, array) in [
        "missing.png",
        "https://invalid.example/image.png",
        "file:forbidden.png",
    ]
    .into_iter()
    .flat_map(|value| [false, true].map(move |array| (value, array)))
    {
        for disabled in ["node", "parent", "properties"] {
            let (temp, store) = if array { array_fixture() } else { fixture() };
            let source = temp.path().join("library/data/images.lib.pen");
            let mut data: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
            data["variables"]["leaf"]["value"] = json!(value);
            write(&source, data);
            let mut root: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
            let library = temp.path().join("library/ui.lib.pen");
            let mut definitions: Value =
                serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
            match disabled {
                "node" => root["children"][0]["enabled"] = json!(false),
                "parent" => {
                    let children = root["children"].take();
                    root["children"] =
                        json!([{"id":"hidden","type":"frame","enabled":false,"children":children}]);
                }
                _ => {
                    for property in ["fill", "stroke", "effect"] {
                        definitions["children"][0][property]["enabled"] = json!(false);
                    }
                }
            }
            root["children"].as_array_mut().unwrap().push(json!({"id":"marker","type":"rectangle","x":120,"width":10,"height":10,"fill":"#cccccc"}));
            write(store.path(), root.clone());
            write(&library, definitions.clone());
            let project = Project::load(&store).unwrap();
            project.resolve().unwrap();
            assert!(
                project
                    .dependencies()
                    .iter()
                    .all(|entry| entry.kind != "image")
            );
            #[cfg(feature = "rendering")]
            {
                let (render_project, layout) =
                    service::layout_snapshot(&store, &[], None, None).unwrap();
                let output =
                    lazypencil::render::png(&render_project, &layout, &Default::default()).unwrap();
                assert!(output.width >= 10 && !output.png.is_empty());
            }
            match disabled {
                "node" | "parent" => root["children"][0]["enabled"] = json!(true),
                _ => {
                    for property in ["fill", "stroke", "effect"] {
                        definitions["children"][0][property]["enabled"] = json!(true);
                    }
                }
            }
            write(store.path(), root);
            write(&library, definitions);
            assert!(
                Project::load(&store)
                    .and_then(|project| project.resolve())
                    .is_err()
            );
        }
    }
}

#[cfg(feature = "rendering")]
#[test]
#[ignore = "Requires verified headless Chromium and permission to launch its sandbox"]
fn real_webgl_sampler_variables_and_detachment_preserve_both_theme_images() {
    use lazypencil::{dynamic::DynamicRuntime, render};
    use skia_safe::{Color, Data, Image};
    for array in [false, true] {
        let (temp, store) = if array { array_fixture() } else { fixture() };
        let executable =
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM");
        let mut runtime = DynamicRuntime::new(temp.path())
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(executable);
        let mut originals = Vec::new();
        for detached in [false, true] {
            if detached {
                edit(
                    &store,
                    "detach",
                    json!([{"op":"detach_instance","target":"use"}]),
                );
            }
            for (index, mode) in ["light", "dark"].into_iter().enumerate() {
                edit(
                    &store,
                    &format!("theme-{detached}-{mode}"),
                    json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
                );
                let (project, snapshot) =
                    service::layout_snapshot(&store, &[], None, None).unwrap();
                let rendered = render::png_with_runtime(
                    &project,
                    &snapshot,
                    &Default::default(),
                    &mut runtime,
                    None,
                )
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
                if array {
                    assert_eq!(
                        image.peek_pixels().unwrap().get_color((75, 10)),
                        Color::BLUE
                    );
                }
                if detached {
                    assert_eq!(rendered.png, originals[index]);
                } else {
                    originals.push(rendered.png);
                }
            }
        }
    }
}

fn copy_operations() -> Value {
    json!([
        {"op":"import_component","source":{"library_alias":"ui","node_id":"tile"},"mode":"copy","options":{"variable_prefix":"copied_","themes":{"ui:images:mode":{"action":"reuse","target":"mode"}}}},
        {"op":"delete","target":"use"},
        {"op":"remove_import","alias":"ui"}
    ])
}

#[cfg(unix)]
#[test]
fn sampler_projection_collisions_keep_text_themes_and_future_edits_after_independent_recopy() {
    for version in ["2.19", "2.21"] {
        for case in PROJECTIONS {
            let (temp, store, strings) = projection_fixture(case, version);
            let before = fs::read(store.path()).unwrap();
            let original_library =
                fs::read(temp.path().join("library/data/images.lib.pen")).unwrap();
            let hashes = strings.map(|uri| {
                lazypencil::document::hash(
                    &fs::read(temp.path().join("library/data").join(uri)).unwrap(),
                )
            });
            let metadata = service::document_metadata(&store).unwrap();
            let preview: EditRequest = serde_json::from_value(json!({"request_id":"projection-preview","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"dry_run":true,"operations":copy_operations()})).unwrap();
            service::apply_request(&store, &preview).unwrap();
            assert_eq!(fs::read(store.path()).unwrap(), before);
            assert!(!temp.path().join("assets").exists());
            assert!(store.history(10, None).unwrap().is_empty());
            let mut invalid = copy_operations().as_array().unwrap().clone();
            invalid.push(json!({"op":"update","target":"missing","set":{"x":1}}));
            let failed: EditRequest = serde_json::from_value(json!({"request_id":"projection-failed-batch","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":invalid})).unwrap();
            assert!(service::apply_request(&store, &failed).is_err());
            assert_eq!(fs::read(store.path()).unwrap(), before);
            assert!(!temp.path().join("assets").exists());
            assert!(store.history(10, None).unwrap().is_empty());
            let result = edit(&store, "projection-copy", copy_operations());
            let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
            let component = result["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            assert_eq!(
                fs::read(temp.path().join("library/data/images.lib.pen")).unwrap(),
                original_library
            );
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            for (index, mode) in ["light", "dark"].into_iter().enumerate() {
                edit(
                    &store,
                    &format!("copied-{mode}"),
                    json!([{"op":"set_theme","target":id,"set":{"mode":mode}}]),
                );
                assert_copied_sampler(&store, id, strings[index], &hashes[index]);
            }
            let (recopied, id) = recopy_sampler(&store, component, &temp.path().join("recopied"));
            fs::remove_file(store.path()).unwrap();
            fs::remove_dir_all(temp.path().join("assets")).unwrap();
            for (index, mode) in ["light", "dark"].into_iter().enumerate() {
                edit(
                    &recopied,
                    &format!("recopied-{mode}"),
                    json!([{"op":"set_theme","target":id,"set":{"mode":mode}}]),
                );
                assert_copied_sampler(&recopied, &id, strings[index], &hashes[index]);
            }
            let replacement = recopied.path().parent().unwrap().join("replacement.png");
            image(&replacement, false);
            let hash = lazypencil::document::hash(&fs::read(replacement).unwrap());
            edit(
                &recopied,
                "projection-future",
                json!([
                    {"op":"set_variable","name":"replacement","definition":{"type":"string","value":"replacement.png"}},
                    {"op":"set_variable","name":"recopied_copied_leaf","definition":{"type":"string","value":"$replacement"}}
                ]),
            );
            // A future consumer-owned value no longer uses the captured origin.
            let project = Project::load(&recopied).unwrap();
            let node = project
                .resolve()
                .unwrap()
                .roots
                .into_iter()
                .find(|node| node.id() == id)
                .unwrap();
            assert_eq!(node.children[0].properties["content"], "replacement.png");
            for property in ["fill", "stroke", "effect"] {
                let path = Path::new(
                    node.properties[property]["uniforms"]["image"][0]
                        .as_str()
                        .unwrap(),
                );
                assert_eq!(project.asset(path).unwrap().revision, hash);
                assert_eq!(
                    path,
                    recopied.path().parent().unwrap().join("replacement.png")
                );
            }
        }
    }
}

#[test]
fn absolute_sampler_copy_preserves_text_themes_portability_and_future_edits() {
    let (temp, store, strings) = absolute_fixture();
    assert_portable_sampler_copy(temp, store, strings, false);
}

#[test]
fn file_url_sampler_copy_preserves_encoded_text_themes_portability_and_future_edits() {
    for version in ["2.19", "2.21"] {
        let (temp, store, strings) = file_url_fixture(version);
        assert_portable_sampler_copy(temp, store, strings, true);
    }
}

#[test]
fn file_url_samplers_keep_root_restrictions_and_dependency_conflicts() {
    let (temp, store, urls) = file_url_fixture("2.21");
    let project = Project::load(&store).unwrap();
    let image_path = url::Url::parse(&urls[0]).unwrap().to_file_path().unwrap();
    image(&image_path, true);
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let outside = TempDir::new().unwrap();
    let outside_image = outside.path().join("outside.png");
    image(&outside_image, false);
    let source = temp.path().join("library/data/images.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    document["variables"]["leaf"]["value"] =
        json!(url::Url::from_file_path(outside_image).unwrap().as_str());
    write(&source, document.clone());
    assert_eq!(Project::load(&store).err().unwrap().code, "ASSET_NOT_FOUND");
    for uri in [
        format!("{}?download=1", urls[0]),
        format!("{}#fragment", urls[0]),
        "file://remote.example/image.png".into(),
        "file://user:secret@localhost/image.png".into(),
        "file:relative.png".into(),
        "file:///bad%00name.png".into(),
        "file:///bad%ffname.png".into(),
    ] {
        document["variables"]["leaf"]["value"] = json!(uri);
        write(&source, document.clone());
        assert_eq!(
            Project::load(&store).err().unwrap().code,
            "ASSET_NOT_FOUND",
            "{uri}"
        );
    }
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn file_url_samplers_detect_same_byte_symlink_rebinding() {
    let (temp, store, urls) = file_url_fixture("2.21");
    let path = url::Url::parse(&urls[0]).unwrap().to_file_path().unwrap();
    let alias = path.with_file_name("alias.png");
    let replacement = path.with_file_name("same.png");
    fs::copy(&path, &replacement).unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let source = temp.path().join("library/data/images.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    document["variables"]["leaf"]["value"] =
        json!(url::Url::from_file_path(&alias).unwrap().as_str());
    write(&source, document);
    let project = Project::load(&store).unwrap();
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(replacement, alias).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

fn assert_portable_sampler_copy(
    temp: TempDir,
    store: Store,
    strings: [String; 2],
    file_urls: bool,
) {
    let original = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let preview: EditRequest = serde_json::from_value(json!({"request_id":"absolute-preview","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"dry_run":true,"operations":copy_operations()})).unwrap();
    service::apply_request(&store, &preview).unwrap();
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
    let hashes = strings.each_ref().map(|path| {
        let path = if file_urls {
            url::Url::parse(path).unwrap().to_file_path().unwrap()
        } else {
            path.into()
        };
        lazypencil::document::hash(&fs::read(path).unwrap())
    });
    let result = edit(&store, "absolute-copy", copy_operations());
    let target = result["generated_ids"]["0"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let component = result["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    // Move the document and all managed files to a different project root.
    // Its displayed strings still contain the original, now missing paths.
    let moved = TempDir::new().unwrap();
    fs::create_dir(moved.path().join(".git")).unwrap();
    fs::rename(temp.path().join("assets"), moved.path().join("assets")).unwrap();
    fs::rename(store.path(), moved.path().join("main.pen")).unwrap();
    let store = Store::open(
        moved.path().join("main.pen"),
        Some(moved.path().join("data")),
    )
    .unwrap();
    for (index, mode) in ["light", "dark"].into_iter().enumerate() {
        edit(
            &store,
            &format!("absolute-{mode}"),
            json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
        );
        let project = Project::load(&store).unwrap();
        let scene = project.resolve().unwrap();
        let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
        assert_eq!(node.children[0].properties["content"], strings[index]);
        for property in ["fill", "stroke", "effect"] {
            let path = Path::new(
                node.properties[property]["uniforms"]["image"][0]
                    .as_str()
                    .unwrap(),
            );
            assert!(
                path.starts_with(
                    moved
                        .path()
                        .canonicalize()
                        .unwrap()
                        .join("assets/lazypencil")
                )
            );
            assert_eq!(project.asset(path).unwrap().revision, hashes[index]);
        }
        assert!(
            project
                .dependencies()
                .iter()
                .all(|dependency| !dependency.path.starts_with(temp.path()))
        );
    }
    // Recopying must resolve the terminal binding rather than reopening the
    // missing absolute path preserved for text.
    let copied = edit(
        &store,
        "absolute-recopy",
        json!([
            {"op":"import_component","source":{"node_id":component},"mode":"copy","options":{"variable_prefix":"again_"}},
            {"op":"delete","target":target}
        ]),
    );
    let target = copied["generated_ids"]["0"]["id"].as_str().unwrap();
    let project = Project::load(&store).unwrap();
    assert!(
        project
            .resolve()
            .unwrap()
            .roots
            .iter()
            .any(|node| node.id() == target)
    );
    image(&moved.path().join("replacement.png"), true);
    let replacement = moved.path().join("replacement.png").canonicalize().unwrap();
    let replacement_text = if file_urls {
        url::Url::from_file_path(&replacement).unwrap().to_string()
    } else {
        replacement.to_str().unwrap().into()
    };
    edit(
        &store,
        "absolute-future",
        json!([{"op":"set_variable","name":"again_copied_leaf","definition":{"type":"string","value":replacement_text}}]),
    );
    let node = Project::load(&store)
        .unwrap()
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == target)
        .unwrap();
    assert_eq!(
        node.children[0].properties["content"],
        json!(replacement_text)
    );
    for property in ["fill", "stroke", "effect"] {
        assert_eq!(
            node.properties[property]["uniforms"]["image"][0],
            json!(replacement)
        );
    }
}

#[test]
fn sampler_image_bindings_reject_unknown_versions_and_unsafe_targets_before_opening_assets() {
    let (temp, store) = fixture();
    let original = store.read().unwrap().value().clone();
    for (binding, code) in [
        (json!({"version":2,"images":{}}), "UNSUPPORTED_VERSION"),
        (
            json!({"version":1,"images":{},"extra":true}),
            "INVALID_DOCUMENT",
        ),
        (json!({"version":1,"images":false}), "INVALID_DOCUMENT"),
        (
            json!({"version":1,"images":{"/old.png":12}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"images":{"$alias":"images/a.png"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"images":{"/old.png":"../outside.png"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"images":{"/old.png":"/outside.png"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"images":{"/old.png":"C:\\outside.png"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"images":{"/old.png":"images//a.png"}}),
            "INVALID_DOCUMENT",
        ),
    ] {
        let mut document = original.clone();
        document["metadata"] = json!({"lazypencil":{"sampler_images":binding}});
        write(store.path(), document);
        assert_eq!(Project::load(&store).err().unwrap().code, code);
    }
    let mut document = original;
    document["metadata"] = json!({"lazypencil":{"sampler_images":{"version":1,"images":{"/unused.png":"missing.png"}}}});
    write(store.path(), document);
    assert!(Project::load(&store).is_ok());
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn independent_sampler_copy_keeps_text_aliases_themes_and_future_consumer_edits() {
    let (temp, store) = array_fixture();
    let original = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let preview: EditRequest = serde_json::from_value(json!({"request_id":"preview-copy","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"dry_run":true,"operations":copy_operations()})).unwrap();
    let preview = service::apply_request(&store, &preview).unwrap();
    assert!(
        preview["generated_ids"]["0"]["assets"]
            .as_array()
            .unwrap()
            .len()
            >= 4
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
    let mut invalid = copy_operations().as_array().unwrap().clone();
    invalid.push(json!({"op":"update","target":"absent","set":{"x":1}}));
    let invalid:EditRequest = serde_json::from_value(json!({"request_id":"invalid-copy","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":invalid})).unwrap();
    assert!(service::apply_request(&store, &invalid).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(!temp.path().join("assets").exists());
    let copied = edit(&store, "copy-samplers", copy_operations());
    let target = copied["generated_ids"]["0"]["id"].as_str().unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let document = store.read().unwrap();
    assert_eq!(
        document.value()["variables"]["copied_picture"]["value"],
        "$copied_leaf"
    );
    assert_eq!(
        document.value()["variables"]["copied_alias"]["value"],
        "$copied_picture"
    );
    assert!(
        document.value()["imports"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value.as_str().unwrap().starts_with("assets/lazypencil/"))
    );
    for mode in ["light", "dark"] {
        edit(
            &store,
            &format!("copied-{mode}"),
            json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
        );
        let project = Project::load(&store).unwrap();
        let scene = project.resolve().unwrap();
        let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
        assert_eq!(
            node.children[0].properties["content"],
            if mode == "light" {
                "pixel.png"
            } else {
                "dark.png"
            }
        );
        for property in ["fill", "stroke", "effect"] {
            let uri = node.properties[property]["uniforms"]["image"][0]
                .as_str()
                .unwrap();
            let path = project
                .resolve_file(node.property_source(property), uri)
                .unwrap();
            assert!(path.starts_with(store.path().parent().unwrap().join("assets/lazypencil")));
            assert!(project.asset(&path).is_ok());
        }
    }
    image(&temp.path().join("replacement.png"), true);
    edit(
        &store,
        "future-alias",
        json!([
            {"op":"set_variable","name":"replacement","definition":{"type":"string","value":"replacement.png"}},
            {"op":"set_variable","name":"copied_leaf","definition":{"type":"string","value":"$replacement"}}
        ]),
    );
    let project = Project::load(&store).unwrap();
    let node = project
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == target)
        .unwrap();
    assert_eq!(node.children[0].properties["content"], "replacement.png");
    assert_eq!(
        node.properties["fill"]["uniforms"]["image"][0],
        json!(temp.path().join("replacement.png").canonicalize().unwrap())
    );
}

#[test]
fn independent_sampler_copy_preserves_parent_and_transit_directory_uris() {
    let (temp, store) = fixture();
    fs::create_dir(temp.path().join("library/transit")).unwrap();
    let source = temp.path().join("library/data/images.lib.pen");
    let mut data: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    fs::create_dir(temp.path().join("library/data/素材")).unwrap();
    fs::copy(
        temp.path().join("library/data/dark.png"),
        temp.path().join("library/data/素材/暗い.png"),
    )
    .unwrap();
    data["variables"]["leaf"]["value"] = json!([
        {"value":"../transit/../pixel.png"},
        {"theme":{"mode":"dark"},"value":"./素材/暗い.png"}
    ]);
    write(&source, data);
    let result = edit(&store, "parent-copy", copy_operations());
    let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let project = Project::load(&store).unwrap();
    let node = project
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == target)
        .unwrap();
    assert_eq!(
        node.children[0].properties["content"],
        "../transit/../pixel.png"
    );
    let input = node.properties["fill"]["uniforms"]["image"]
        .as_str()
        .unwrap();
    assert!(Path::new(input).canonicalize().unwrap().is_file());
    edit(
        &store,
        "unicode-theme",
        json!([{"op":"set_theme","target":target,"set":{"mode":"dark"}}]),
    );
    let dark = Project::load(&store).unwrap();
    let dark = dark
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == target)
        .unwrap();
    assert_eq!(dark.children[0].properties["content"], "./素材/暗い.png");
    assert!(
        Path::new(
            dark.properties["fill"]["uniforms"]["image"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap()
        .is_file()
    );
    let imports = store.read().unwrap().value()["imports"].clone();
    let library = imports
        .as_object()
        .unwrap()
        .values()
        .find_map(|uri| {
            let path = temp.path().join(uri.as_str().unwrap());
            let bytes = fs::read(&path).unwrap();
            bytes
                .windows(b"../transit/../pixel.png".len())
                .any(|bytes| bytes == b"../transit/../pixel.png")
                .then_some(path)
        })
        .unwrap();
    let modified = fs::read(&library).unwrap();
    fs::write(&library, b"changed proxy library").unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    fs::write(library, modified).unwrap();
}

#[cfg(feature = "rendering")]
#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn real_webgl_independent_sampler_copy_matches_pixels_after_original_sources_are_deleted() {
    use lazypencil::{dynamic::DynamicRuntime, render};
    let mut evidence = Vec::new();
    for kind in ["relative", "absolute", "file_url"] {
        let absolute = kind == "absolute";
        let (temp, store) = if kind == "file_url" {
            let (temp, store, _) = file_url_fixture("2.21");
            (temp, store)
        } else if absolute {
            let (temp, store, _) = absolute_fixture();
            (temp, store)
        } else {
            array_fixture()
        };
        let mut runtime = DynamicRuntime::new(temp.path())
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(
                std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
            );
        let mut originals = Vec::new();
        for mode in ["light", "dark"] {
            edit(
                &store,
                &format!("before-{mode}"),
                json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
            );
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            originals.push(
                render::png_with_runtime(
                    &project,
                    &snapshot,
                    &Default::default(),
                    &mut runtime,
                    None,
                )
                .unwrap()
                .png,
            );
        }
        let result = edit(&store, "copy-real", copy_operations());
        let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        for (index, mode) in ["light", "dark"].into_iter().enumerate() {
            edit(
                &store,
                &format!("after-{mode}"),
                json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
            );
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
            let rendered = render::png_with_runtime(
                &project,
                &snapshot,
                &Default::default(),
                &mut runtime,
                None,
            )
            .unwrap();
            assert_eq!(
                rendered.png, originals[index],
                "Copied {mode} sampler pixels changed (source={kind})"
            );
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let directory = Path::new(&directory);
                fs::create_dir_all(directory).unwrap();
                let label = if kind == "file_url" {
                    "file_url".into()
                } else {
                    absolute.to_string()
                };
                let before = format!("codec-sampler-copy-{label}-{mode}-before.png");
                let after = format!("codec-sampler-copy-{label}-{mode}-after.png");
                fs::write(directory.join(&before), &originals[index]).unwrap();
                fs::write(directory.join(&after), &rendered.png).unwrap();
                evidence.push(json!({"source_kind":kind,"absolute":absolute,"theme":mode,"before":before,"after":after,"width":rendered.width,"height":rendered.height,"png_hash":lazypencil::document::hash(&rendered.png)}));
            }
        }
    }
    inline_images::real_webgl_case(&mut evidence);
    let https_image = http_images::real_webgl_case(&mut evidence);
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(
            Path::new(&directory).join("codec-sampler-copy.json"),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":evidence,"https_image":https_image}).to_string(),
        )
        .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn mapped_sampler_images_cannot_escape_allowed_roots_through_symlinks() {
    let (temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    image(&outside.path().join("outside.png"), true);
    std::os::unix::fs::symlink(
        outside.path().join("outside.png"),
        temp.path().join("library/data/redirect.png"),
    )
    .unwrap();
    let source = temp.path().join("library/data/images.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    document["variables"]["leaf"]["value"] = json!("/original.png");
    document["metadata"] = json!({"lazypencil":{"sampler_images":{"version":1,"images":{"/original.png":"redirect.png"}}}});
    write(&source, document);
    assert_eq!(Project::load(&store).err().unwrap().code, "ASSET_NOT_FOUND");
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
}

#[cfg(all(unix, feature = "rendering"))]
fn projection_png(
    store: &Store,
    runtime: &mut lazypencil::dynamic::DynamicRuntime,
) -> lazypencil::render::RenderedImage {
    let (project, snapshot) = service::layout_snapshot(store, &[], None, None).unwrap();
    lazypencil::render::png_with_runtime(&project, &snapshot, &Default::default(), runtime, None)
        .unwrap()
}

#[cfg(all(unix, feature = "rendering"))]
#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn real_webgl_sampler_projection_collisions_keep_pixels_after_original_and_first_copy_removal() {
    use lazypencil::dynamic::DynamicRuntime;
    let mut evidence = Vec::new();
    for version in ["2.19", "2.21"] {
        for case in PROJECTIONS {
            let (temp, store, _) = projection_fixture(case, version);
            let mut runtime = DynamicRuntime::new(temp.path())
                .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
                .with_executable(
                    std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
                        .expect("Set LAZYPENCIL_TEST_CHROMIUM"),
                );
            let mut originals = Vec::new();
            for mode in ["light", "dark"] {
                edit(
                    &store,
                    &format!("original-{mode}"),
                    json!([{"op":"set_theme","target":"use","set":{"mode":mode}}]),
                );
                originals.push(projection_png(&store, &mut runtime));
            }
            let result = edit(&store, "real-projection-copy", copy_operations());
            let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
            let component = result["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            let mut copies = Vec::new();
            for (index, mode) in ["light", "dark"].into_iter().enumerate() {
                edit(
                    &store,
                    &format!("copied-{mode}"),
                    json!([{"op":"set_theme","target":id,"set":{"mode":mode}}]),
                );
                let copied = projection_png(&store, &mut runtime);
                assert_eq!(
                    copied.png, originals[index].png,
                    "{version} {case} {mode} first copy"
                );
                copies.push(copied);
            }
            let (recopied, id) = recopy_sampler(&store, component, &temp.path().join("recopied"));
            fs::remove_file(store.path()).unwrap();
            fs::remove_dir_all(temp.path().join("assets")).unwrap();
            for (index, mode) in ["light", "dark"].into_iter().enumerate() {
                edit(
                    &recopied,
                    &format!("recopied-{mode}"),
                    json!([{"op":"set_theme","target":id,"set":{"mode":mode}}]),
                );
                let image = projection_png(&recopied, &mut runtime);
                assert_eq!(
                    image.png, originals[index].png,
                    "{version} {case} {mode} independent recopy"
                );
                if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                    let directory = Path::new(&directory);
                    fs::create_dir_all(directory).unwrap();
                    let prefix = format!("codec-sampler-projection-{version}-{case}-{mode}");
                    let before = format!("{prefix}-before.png");
                    let copied = format!("{prefix}-copied.png");
                    let after = format!("{prefix}-recopied.png");
                    fs::write(directory.join(&before), &originals[index].png).unwrap();
                    fs::write(directory.join(&copied), &copies[index].png).unwrap();
                    fs::write(directory.join(&after), &image.png).unwrap();
                    evidence.push(json!({"document_version":version,"case":case,"theme":mode,"before":before,"copied":copied,"recopied":after,"width":image.width,"height":image.height,"png_hash":lazypencil::document::hash(&image.png)}));
                }
            }
        }
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(
            Path::new(&directory).join("codec-sampler-projection.json"),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":evidence}).to_string(),
        )
        .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn sampler_copy_rejects_a_redirected_nested_bundle_directory_without_touching_the_document() {
    for collision in [false, true] {
        let (temp, store) = if collision {
            let (temp, store, _) = projection_fixture("aliases", "2.21");
            (temp, store)
        } else {
            fixture()
        };
        let before = fs::read(store.path()).unwrap();
        let metadata = service::document_metadata(&store).unwrap();
        let preview:EditRequest=serde_json::from_value(json!({"request_id":"nested-preview","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"dry_run":true,"operations":copy_operations()})).unwrap();
        let result = service::apply_request(&store, &preview).unwrap();
        let index = result["generated_ids"]["0"]["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| {
                asset["uri"]
                    .as_str()
                    .unwrap()
                    .ends_with(".bundle/bundle.json")
            })
            .unwrap()["uri"]
            .as_str()
            .unwrap();
        let bundle = temp.path().join(index).parent().unwrap().to_path_buf();
        fs::create_dir_all(bundle.parent().unwrap()).unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &bundle).unwrap();
        let metadata = service::document_metadata(&store).unwrap();
        let request:EditRequest=serde_json::from_value(json!({"request_id":"nested-conflict","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":copy_operations()})).unwrap();
        assert_eq!(
            service::apply_request(&store, &request).unwrap_err().code,
            "ASSET_CONFLICT"
        );
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(store.history(10, None).unwrap().is_empty());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}

#[test]
fn legacy_consumer_copies_generate_legacy_libraries_without_rewriting_user_sources() {
    let (temp, store) = array_fixture();
    let source_path = temp.path().join("library/ui.lib.pen");
    let original_library = fs::read(&source_path).unwrap();
    let original = store.read().unwrap().text().replace("2.21", "2.19");
    fs::write(store.path(), original).unwrap();
    edit(&store, "legacy-copy", copy_operations());
    assert_eq!(fs::read(source_path).unwrap(), original_library);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let document = store.read().unwrap();
    assert_eq!(document.version(), "2.19");
    let imports = document.value()["imports"].as_object().unwrap();
    assert!(!imports.is_empty());
    for uri in imports.values() {
        let path = store.path().parent().unwrap().join(uri.as_str().unwrap());
        let library =
            lazypencil::document::Document::parse(fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(library.version(), "2.19");
    }
    assert!(
        !Project::load(&store)
            .unwrap()
            .resolve()
            .unwrap()
            .roots
            .is_empty()
    );
}
