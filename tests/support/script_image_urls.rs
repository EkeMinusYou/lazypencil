use super::http_images::{Server, apply, cli, cli_result};
use super::*;

const SHADER: &str = "precision highp float;uniform sampler2D tex;void main(){gl_FragColor=texture2D(tex,vec2(0.5));}";

fn fixture(version: &str, server: Option<&Server>) -> (TempDir, Store, String, String, String) {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let library = project.join("library");
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir(&library).unwrap();
    let image = library.join("image #1.png");
    fs::write(
        &image,
        include_bytes!("../../examples/assets/crop-source.png"),
    )
    .unwrap();
    let uri = match server {
        Some(server) => format!(
            "http://{}/network/http-image/redirect.png?variant=original#keep",
            server.address
        ),
        None => url::Url::from_file_path(&image).unwrap().to_string(),
    };
    let output = json!([{"id":"image","type":"rectangle","width":40,"height":20,"fill":{"type":"image","url":uri}}]);
    let code = format!("/** @schema {version} */\nreturn {output};");
    let source = library.join("generator #1.js");
    fs::write(&source, &code).unwrap();
    let script_uri = url::Url::from_file_path(&source).unwrap().to_string();
    let shader = library.join("sample #1.glsl");
    fs::write(&shader, SHADER).unwrap();
    let shader_uri = url::Url::from_file_path(&shader).unwrap().to_string();
    write(
        &library.join("library.pen"),
        json!({"version":version,"children":[{"id":"generator","type":"script","reusable":true,"width":40,"height":20,"scriptUri":script_uri}]}),
    );
    let file = project.join("main.pen");
    write(
        &file,
        json!({"version":version,"imports":{"ui":"library/library.pen"},"children":[{"id":"use","type":"ref","ref":"ui:generator"}]}),
    );
    let store = Store::open(&file, Some(temp.path().join("data"))).unwrap();
    (temp, store, uri, shader_uri, code)
}

fn copy(uri: &str, shader: &str) -> Value {
    json!([
        {"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy","options":{"script_assets":{"ui:generator":[{"kind":"image","uri":uri},{"kind":"shader","uri":shader}]}}},
        {"op":"delete","target":"use"},
        {"op":"remove_import","alias":"ui"}
    ])
}

#[cfg(feature = "rendering")]
fn raster(store: &Store) -> Vec<u8> {
    let output = TempDir::new().unwrap();
    let path = output.path().join("preview.png");
    cli(
        store,
        &[
            "render",
            store.path().to_str().unwrap(),
            "--output",
            path.to_str().unwrap(),
        ],
    );
    fs::read(path).unwrap()
}

#[test]
fn script_image_url_keys_keep_code_and_captured_bytes_after_source_removal_move_and_recopy() {
    for version in ["2.19", "2.21"] {
        for http in [false, true] {
            let server = http.then(Server::new);
            let (temp, store, uri, shader_uri, code) = fixture(version, server.as_ref());
            #[cfg(feature = "rendering")]
            let before = raster(&store);
            let original = fs::read(store.path()).unwrap();
            apply(&store, copy(&uri, &shader_uri), true);
            assert_eq!(fs::read(store.path()).unwrap(), original);
            assert!(!temp.path().join("project/assets").exists());
            let copied = apply(&store, copy(&uri, &shader_uri), false);
            let target = copied["generated_ids"]["0"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let component = copied["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let document = store.read().unwrap();
            let definition = document.node(&component).unwrap();
            let asset = definition["metadata"]["lazypencil"]["script_bindings"]["assets"]
                [format!("image:{uri}")]
            .as_str()
            .unwrap();
            let project = Project::load(&store).unwrap();
            let image = project.resolve_file(store.path(), asset).unwrap();
            assert!(
                project
                    .asset(&image)
                    .unwrap()
                    .bytes
                    .starts_with(b"\x89PNG\r\n\x1a\n")
            );
            assert_eq!(
                fs::read_to_string(
                    project
                        .resolve_file(store.path(), definition["scriptUri"].as_str().unwrap())
                        .unwrap()
                )
                .unwrap(),
                code
            );
            let shader = definition["metadata"]["lazypencil"]["script_bindings"]["assets"]
                [format!("shader:{shader_uri}")]
            .as_str()
            .unwrap();
            assert_eq!(
                fs::read_to_string(project.resolve_file(store.path(), shader).unwrap()).unwrap(),
                SHADER
            );
            drop(project);
            fs::remove_dir_all(temp.path().join("project/library")).unwrap();
            drop(server);
            drop(store);
            let moved = temp.path().join("relocated");
            fs::rename(temp.path().join("project"), &moved).unwrap();
            fs::write(
                moved.join("lazypencil.toml"),
                "[runtime.browser]\nnetwork_policy='offline'\n",
            )
            .unwrap();
            let store =
                Store::open(moved.join("main.pen"), Some(temp.path().join("data"))).unwrap();
            #[cfg(feature = "rendering")]
            assert_eq!(raster(&store), before);
            let assets = serde_json::Map::from_iter([(
                component.clone(),
                json!([{"kind":"image","uri":uri},{"kind":"shader","uri":shader_uri}]),
            )]);
            apply(
                &store,
                json!([
                    {"op":"import_component","source":{"node_id":component},"mode":"copy","options":{"script_assets":assets}},
                    {"op":"delete","target":target}
                ]),
                false,
            );
            let project = Project::load(&store).unwrap();
            assert!(
                project
                    .dependencies()
                    .iter()
                    .all(|dependency| dependency.kind != "remote_image")
            );
            project.verify_unchanged().unwrap();
            #[cfg(feature = "rendering")]
            assert_eq!(raster(&store), before);
        }
    }
}

#[test]
fn imported_generator_url_aliases_stay_in_their_own_captured_namespace() {
    for version in ["2.19", "2.21"] {
        let server = Server::new();
        let (temp, store, uri, _, _) = fixture(version, Some(&server));
        let library = temp.path().join("project/library");
        let path = library.join("library.pen");
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["imports"] = json!({"nested":"nested.pen"});
        write(&path, document);
        fs::write(library.join("generator #1.js"), format!("/** @schema {version} */\nreturn [{{id:'nested',type:'ref',ref:'nested:generator'}}];")).unwrap();
        let output = json!([{"id":"image","type":"rectangle","width":40,"height":20,"fill":{"type":"image","url":uri}}]);
        fs::write(
            library.join("nested.js"),
            format!("/** @schema {version} */\nreturn {output};"),
        )
        .unwrap();
        write(
            &library.join("nested.pen"),
            json!({"version":version,"children":[
                {"id":"generator","type":"script","reusable":true,"width":40,"height":20,"scriptUri":"nested.js"},
                {"id":"declared","type":"rectangle","reusable":true,"width":40,"height":20,"fill":{"type":"image","url":uri}}
            ]}),
        );
        #[cfg(feature = "rendering")]
        let before = raster(&store);
        let copied = apply(
            &store,
            json!([
                {"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy"},
                {"op":"delete","target":"use"},
                {"op":"remove_import","alias":"ui"}
            ]),
            false,
        );
        let target = copied["generated_ids"]["0"]["id"].as_str().unwrap();
        let component = copied["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        let document = store.read().unwrap();
        let bindings =
            &document.node(component).unwrap()["metadata"]["lazypencil"]["script_bindings"];
        let key = format!("image:{uri}");
        let active = bindings["script_namespace"].as_str().unwrap_or("");
        assert!(
            bindings["script_scopes"][active]["assets"]
                .get(&key)
                .is_none()
        );
        assert!(
            bindings["script_scopes"]
                .as_object()
                .unwrap()
                .values()
                .any(|scope| scope["assets"].get(&key).is_some())
        );
        fs::remove_dir_all(library).unwrap();
        drop(server);
        fs::write(
            temp.path().join("project/lazypencil.toml"),
            "[runtime.browser]\nnetwork_policy='offline'\n",
        )
        .unwrap();
        #[cfg(feature = "rendering")]
        assert_eq!(raster(&store), before);
        apply(
            &store,
            json!([
                {"op":"import_component","source":{"node_id":component},"mode":"copy"},
                {"op":"delete","target":target}
            ]),
            false,
        );
        Project::load(&store).unwrap().verify_unchanged().unwrap();
        #[cfg(feature = "rendering")]
        assert_eq!(raster(&store), before);
    }
}

#[test]
fn script_asset_url_declarations_do_not_grant_remote_code_or_files_outside_project_roots() {
    for kind in ["image", "script", "shader"] {
        let (temp, store, _, _, _) = fixture("2.21", None);
        let outside = temp.path().join("private.png");
        fs::write(
            &outside,
            include_bytes!("../../examples/assets/crop-source.png"),
        )
        .unwrap();
        let file = url::Url::from_file_path(outside).unwrap().to_string();
        let remote = if kind == "image" {
            "https://user:private@example.com/image.png"
        } else {
            "https://example.com/code"
        };
        for uri in [file.as_str(), remote] {
            let operations = tempfile::NamedTempFile::new().unwrap();
            fs::write(operations.path(), json!([{"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy","options":{"script_assets":{"ui:generator":[{"kind":kind,"uri":uri}]}}}]).to_string()).unwrap();
            let before = fs::read(store.path()).unwrap();
            let metadata = cli(&store, &["inspect", store.path().to_str().unwrap()]);
            let result = cli_result(
                &store,
                &[
                    "apply",
                    store.path().to_str().unwrap(),
                    "--operations",
                    operations.path().to_str().unwrap(),
                    "--expected-revision",
                    metadata["revision"].as_str().unwrap(),
                    "--expected-dependency-revision",
                    metadata["dependency_revision"].as_str().unwrap(),
                ],
            );
            assert!(
                matches!(
                    result["error"]["code"].as_str(),
                    Some("ASSET_NOT_FOUND" | "INVALID_DOCUMENT")
                ),
                "{result}"
            );
            assert!(
                !result.to_string().contains("private@example.com"),
                "{result}"
            );
            assert_eq!(fs::read(store.path()).unwrap(), before);
            assert!(!temp.path().join("project/assets").exists());
        }
    }
}

#[cfg(feature = "rendering")]
#[test]
#[ignore = "requires the verified Chromium runtime and real software WebGL"]
fn script_file_url_shaders_and_url_images_keep_pixels_after_capture_and_offline_recopy() {
    use lazypencil::{
        dynamic::DynamicRuntime,
        render,
        script::{self, ScriptRuntime},
    };
    use skia_safe::{Color, Data, Image};

    let mut evidence = Vec::new();
    for version in ["2.19", "2.21"] {
        for http in [false, true] {
            let server = http.then(Server::new);
            let (temp, store, uri, shader_uri, _) = fixture(version, server.as_ref());
            if !http {
                image(
                    &url::Url::parse(&uri).unwrap().to_file_path().unwrap(),
                    false,
                );
            }
            let output = json!([{"id":"shader","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":shader_uri,"uniforms":{"tex":uri}}}]);
            fs::write(
                temp.path().join("project/library/generator #1.js"),
                format!("/** @schema {version} */\nreturn {output};"),
            )
            .unwrap();
            let mut runtime = DynamicRuntime::new(temp.path())
                .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
                .with_executable(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
            let render = |store: &Store, runtime: &mut DynamicRuntime| {
                let mut project = Project::load_with_remote_worker(
                    store,
                    Path::new(env!("CARGO_BIN_EXE_lazypencil")),
                    None,
                )
                .unwrap();
                let mut fonts = project.load_fonts().unwrap();
                let layout = script::compute(
                    &mut project,
                    &mut fonts,
                    &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
                )
                .unwrap();
                let result =
                    render::png_with_runtime(&project, &layout, &Default::default(), runtime, None)
                        .unwrap();
                let image = Image::from_encoded(Data::new_copy(&result.png))
                    .unwrap()
                    .make_raster_image(None, None)
                    .unwrap();
                assert_eq!(
                    image.peek_pixels().unwrap().get_color((10, 10)),
                    Color::YELLOW
                );
                result
            };
            let before = render(&store, &mut runtime);
            let copied = apply(&store, copy(&uri, &shader_uri), false);
            let target = copied["generated_ids"]["0"]["id"].as_str().unwrap();
            let component = copied["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            fs::remove_dir_all(temp.path().join("project/library")).unwrap();
            drop(server);
            fs::write(
                temp.path().join("project/lazypencil.toml"),
                "[runtime.browser]\nnetwork_policy='offline'\n",
            )
            .unwrap();
            let after = render(&store, &mut runtime);
            assert_eq!(after.png, before.png);
            let manifest = serde_json::Map::from_iter([(
                component.to_owned(),
                json!([{"kind":"image","uri":uri},{"kind":"shader","uri":shader_uri}]),
            )]);
            apply(
                &store,
                json!([
                    {"op":"import_component","source":{"node_id":component},"mode":"copy","options":{"script_assets":manifest}},
                    {"op":"delete","target":target}
                ]),
                false,
            );
            let recopy = render(&store, &mut runtime);
            assert_eq!(recopy.png, before.png);
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let kind = if http { "http" } else { "file_url" };
                let prefix = format!("codec-script-image-urls-{version}-{kind}");
                for (phase, result) in [("before", &before), ("after", &after), ("recopy", &recopy)]
                {
                    fs::write(
                        Path::new(&directory).join(format!("{prefix}-{phase}.png")),
                        &result.png,
                    )
                    .unwrap();
                }
                evidence.push(json!({"document_version":version,"source_kind":kind,"width":before.width,"height":before.height,"png_hash":lazypencil::document::hash(&before.png)}));
            }
        }
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(
            Path::new(&directory).join("codec-script-image-urls.json"),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":evidence}).to_string(),
        )
        .unwrap();
    }
}
