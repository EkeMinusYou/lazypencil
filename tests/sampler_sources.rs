use lazypencil::{operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;

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
    fs::write(
        path,
        if dark {
            &b"dark image fixture"[..]
        } else {
            &b"light image fixture"[..]
        },
    )
    .unwrap();
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
    fs::write(temp.path().join("library/sample.glsl"),"precision highp float;\n/* @resolution */ uniform vec2 size;\nuniform sampler2D image;\nuniform float amount;\nuniform vec4 tone;\nvoid main(){gl_FragColor=texture2D(image,gl_FragCoord.xy/size)*tone*amount;}").unwrap();
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
        let mut runtime = DynamicRuntime::new(temp.path()).with_executable(executable);
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
            assert!(path.starts_with(temp.path().join("assets/lazypencil")));
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
    data["variables"]["leaf"]["value"] = json!([
        {"value":"../transit/../pixel.png"},
        {"theme":{"mode":"dark"},"value":"./dark.png"}
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
    let (temp, store) = array_fixture();
    let mut runtime = DynamicRuntime::new(temp.path()).with_executable(
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
            render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
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
        let rendered =
            render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
                .unwrap();
        assert_eq!(
            rendered.png, originals[index],
            "Copied {mode} sampler pixels changed"
        );
    }
}

#[cfg(unix)]
#[test]
fn sampler_copy_rejects_a_redirected_nested_bundle_directory_without_touching_the_document() {
    let (temp, store) = fixture();
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
