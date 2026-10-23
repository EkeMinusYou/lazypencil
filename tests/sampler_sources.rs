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
    for value in [
        "missing.png",
        "https://invalid.example/image.png",
        "file:forbidden.png",
    ] {
        for disabled in ["node", "parent", "properties"] {
            let (temp, store) = fixture();
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
    let (temp, store) = fixture();
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
            let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
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
            if detached {
                assert_eq!(rendered.png, originals[index]);
            } else {
                originals.push(rendered.png);
            }
        }
    }
}
