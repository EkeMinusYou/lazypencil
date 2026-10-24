#![cfg(feature = "rendering")]

use lazypencil::{
    document::Document,
    project::Project,
    render::{self, RenderOptions},
    script::ScriptRuntime,
    service,
    store::Store,
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn fixture(body: &str, node: Value) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("scene.pen");
    fs::write(
        temp.path().join("draw.js"),
        format!("/**\n * @schema 2.21\n */\n{body}"),
    )
    .unwrap();
    fs::write(
        &file,
        json!({"version":"2.21","children":[node]}).to_string(),
    )
    .unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

#[test]
fn completed_layout_snapshots_do_not_retain_request_cancellation() {
    let temp = TempDir::new().unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut runtime = ScriptRuntime::new(temp.path().join("unused-worker"));
    runtime.set_cancellation(Some(cancel.clone()));
    let (project, _) =
        service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime).unwrap();
    cancel.cancel();
    project.verify_unchanged().unwrap();
    project.resolve().unwrap();
    let error = service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime)
        .err()
        .unwrap();
    assert_eq!(error.code, "CANCELLED");
}
fn snapshot(store: &Store) -> (Project, lazypencil::layout::LayoutSnapshot) {
    service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap()
}
fn raster(png: &[u8]) -> Image {
    Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}
fn solid(path: &Path, color: Color) {
    let mut surface = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
    surface.canvas().clear(color);
    fs::write(
        path,
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
}

#[test]
fn native_random_sequence_and_legacy_headers_match_reference_pixels_without_saving_generated_nodes()
{
    native_script_reference("script-random", "random-sequence", "codec-script-random");
}

#[test]
fn native_input_controls_match_reference_pixels_and_preserve_saved_nulls_and_stale_keys() {
    native_script_reference("script-inputs", "input-controls", "codec-script-inputs");
}

fn native_script_reference(corpus: &str, filename: &str, artifact_stem: &str) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pencil-0.3.9")
        .join(corpus);
    let original = fs::read_to_string(fixtures.join(format!("{filename}.pen"))).unwrap();
    let reference_bytes = fs::read(fixtures.join(format!("{filename}.png"))).unwrap();
    let observations: Value =
        serde_json::from_slice(&fs::read(fixtures.join("observations.json")).unwrap()).unwrap();
    assert_eq!(
        lazypencil::document::hash(&reference_bytes),
        observations["reference_sha256"].as_str().unwrap()
    );
    assert_eq!(
        lazypencil::document::hash(original.as_bytes()),
        observations["source_sha256"].as_str().unwrap()
    );
    let cases = observations["script_cases"]
        .as_u64()
        .unwrap_or_else(|| observations["cases"].as_array().unwrap().len() as u64)
        as usize;
    let reference = raster(&reference_bytes);
    let expected = reference.peek_pixels().unwrap();
    let dimensions = (
        observations["width"].as_u64().unwrap() as u32,
        observations["height"].as_u64().unwrap() as u32,
    );
    assert_eq!(
        (reference.width() as u32, reference.height() as u32),
        dimensions
    );
    for version in ["2.19", "2.21"] {
        let root = TempDir::new().unwrap();
        let mut scripts = Vec::new();
        for entry in fs::read_dir(&fixtures).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "js") {
                let destination = root.path().join(path.file_name().unwrap());
                let bytes = fs::read(&path).unwrap();
                fs::write(&destination, &bytes).unwrap();
                scripts.push((destination, bytes));
            }
        }
        let source = original.replacen("\"2.19\"", &format!("\"{version}\""), 1);
        let path = root.path().join("design.pen");
        fs::write(&path, &source).unwrap();
        let store = Store::open(&path, Some(root.path().join("data"))).unwrap();
        let (project, layout) = snapshot(&store);
        assert_eq!(layout.script_executions.len(), cases);
        let diagnostics = layout.summary()["diagnostics"].clone();
        assert!(diagnostics.as_array().unwrap().is_empty());
        let output = render::png(
            &project,
            &layout,
            &RenderOptions {
                scale: 2.0,
                node: Some("root".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((output.width, output.height), dimensions);
        let actual = raster(&output.png);
        let pixels = actual.peek_pixels().unwrap();
        let mut maximum = 0u8;
        let mut difference = 0u64;
        for y in 0..reference.height() {
            for x in 0..reference.width() {
                let a = pixels.get_color((x, y));
                let b = expected.get_color((x, y));
                for delta in [
                    a.r().abs_diff(b.r()),
                    a.g().abs_diff(b.g()),
                    a.b().abs_diff(b.b()),
                    a.a().abs_diff(b.a()),
                ] {
                    maximum = maximum.max(delta);
                    difference += u64::from(delta);
                }
            }
        }
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
        for (path, bytes) in scripts {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert!(store.history(10, None).unwrap().is_empty());
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            let directory = std::path::PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            let stem = format!("{artifact_stem}-{version}");
            fs::write(directory.join(format!("{stem}.png")), &output.png).unwrap();
            let mut report = json!({
                "source_commit":std::env::var("GITHUB_SHA").ok(),"document_version":version,
                "width":output.width,"height":output.height,"scale":2,"diagnostics":diagnostics,
                "png_hash":lazypencil::document::hash(&output.png),
                "reference_sha256":lazypencil::document::hash(&reference_bytes),
                "source_sha256":lazypencil::document::hash(source.as_bytes()),
                "native_producer":"@pen.dev/cli 0.3.9","native_document_version":"2.19",
                "verified_script_cases":cases,
                "maximum_channel_difference":maximum,"absolute_channel_difference":difference
            });
            if let Some(words) = observations["sample_words_u32"].as_array() {
                report["sampled_draws_per_script"] = json!(words.len());
            }
            fs::write(directory.join(format!("{stem}.json")), report.to_string()).unwrap();
        }
        assert_eq!(
            maximum, 0,
            "Native script pixels differ for codec {version}: {difference}"
        );
    }
}

#[test]
fn generated_geometry_is_measured_painted_and_never_saved_as_stored_children() {
    let (temp, store) = fixture(
        "const result=[];for(let n=0;n<3;n++)result.push({type:'rectangle',x:n*pencil.width/3,width:pencil.width/3-4,height:pencil.height,fill:'#2563EB'});return result;",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":180,"height":80}),
    );
    let before = fs::read(store.path()).unwrap();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.items.len(), 4);
    assert_eq!(layout.script_executions.len(), 1);
    assert_eq!(layout.bounds.width, 180.0);
    let child = &layout.items[1];
    assert_eq!(child.record.generated_by.as_ref().unwrap().node_id, "chart");
    assert!(child.record.id.contains("/$generated/"));
    let png = render::png(&project, &layout, &Default::default()).unwrap();
    let image = raster(&png.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(
        pixels.get_color((20, 20)),
        Color::from_rgb(0x25, 0x63, 0xeb)
    );
    assert_eq!(pixels.get_color((58, 20)).a(), 0);
    let request=serde_json::from_value(json!({"request_id":"derived-edit","expected_revision":layout.revision,"expected_dependency_revision":layout.dependency_revision,
        "operations":[{"op":"update","target":child.record.id,"set":{"width":10}}]})).unwrap();
    assert!(service::apply_request(&store, &request).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    fs::write(temp.path().join("preview.png"), &png.png).unwrap();
}

#[test]
fn scripts_use_parent_fill_sizes_and_content_sizing_converges_when_independent() {
    let (_temp, store) = fixture(
        "return [{type:'rectangle',width:pencil.width,height:pencil.height}];",
        json!({"id":"parent","type":"frame","width":300,"height":100,
        "children":[{"id":"chart","type":"script","scriptUri":"draw.js","width":"fill_container","height":"fill_container"}]}),
    );
    let (_, layout) = snapshot(&store);
    let child = layout
        .items
        .iter()
        .find(|item| item.record.id == "chart")
        .unwrap();
    assert_eq!(
        (child.record.rect.width, child.record.rect.height),
        (300.0, 100.0)
    );
    let (_temp, store) = fixture(
        "return [{type:'rectangle',width:120,height:30}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":"fit_content","height":"fit_content"}),
    );
    let (_, layout) = snapshot(&store);
    assert_eq!(
        (
            layout.items[0].record.rect.width,
            layout.items[0].record.rect.height
        ),
        (120.0, 30.0)
    );
}

#[test]
fn dimension_cycles_require_fallbacks_and_nonconvergent_scripts_fail() {
    for (width, body, success) in [
        (
            "fit_content",
            "return [{type:'rectangle',width:pencil.width,height:20}];",
            false,
        ),
        (
            "fit_content(200)",
            "return [{type:'rectangle',width:Math.min(pencil.width,100),height:20}];",
            true,
        ),
        (
            "fit_content(10)",
            "return [{type:'rectangle',width:pencil.width+1,height:20}];",
            false,
        ),
    ] {
        let (_temp, store) = fixture(
            body,
            json!({"id":"chart","type":"script","scriptUri":"draw.js","width":width,"height":20}),
        );
        let result = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        );
        if success {
            assert_eq!(result.unwrap().1.items[0].record.rect.width, 100.0);
        } else {
            assert_eq!(result.err().unwrap().code, "LAYOUT_UNRESOLVED");
        }
    }
}

#[test]
fn input_variables_resolve_even_when_the_input_name_is_a_literal_property_elsewhere() {
    let (temp, store) = fixture(
        "return [{type:'text',content:pencil.input.ref,fontSize:16,fill:'#111111'}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":180,"height":40,"inputs":{"ref":"$caption"}}),
    );
    fs::write(temp.path().join("draw.js"),"/**\n@schema 2.21\n@input ref: string = \"Default\"\n*/\nreturn [{type:'text',content:pencil.input.ref,fontSize:16,fill:'#111111'}];").unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "",
            "variables",
            &json!({"caption":{"type":"string","value":"日本語の入力"}}),
        )
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.items[1].node.properties["content"], "日本語の入力");
    let image = render::png(&project, &layout, &Default::default()).unwrap();
    assert!(!image.png.is_empty());
}

#[test]
fn declared_script_variable_inputs_still_fail_before_a_worker_is_launched() {
    let (temp, store) = fixture(
        "return [{type:'rectangle',width:pencil.input.value,height:20}];",
        json!({"id":"code","type":"script","scriptUri":"draw.js","width":40,"height":20,"inputs":{"value":"$missing"}}),
    );
    fs::write(temp.path().join("draw.js"),"/**\n@schema 2.21\n@input value: number\n*/\nreturn [{type:'rectangle',width:pencil.input.value,height:20}];").unwrap();
    let before = fs::read(store.path()).unwrap();
    let mut runtime = ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"));
    let error = service::layout_snapshot_with_runtime(&store, &[], None, None, &mut runtime)
        .err()
        .unwrap();
    assert!(error.message.contains("missing"));
    assert!(runtime.process_id().is_none());
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[test]
fn generated_script_inputs_resolve_after_code_capture_and_ignore_stale_variable_controls() {
    let (temp, store) = fixture(
        "return [{type:'script',scriptUri:'inner.js',width:40,height:20,inputs:{value:'$caption',unused:'$missing'}}];",
        json!({"id":"outer","type":"script","scriptUri":"draw.js","width":40,"height":20}),
    );
    let inner = "/**\n@schema 2.21\n@input value: string\n*/\nreturn [{type:'rectangle',width:40,height:20,fill:pencil.input.value==='日本語' && Object.keys(pencil.input).length===1?'#00ff00':'#ff0000'}];";
    fs::write(temp.path().join("inner.js"), inner).unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "",
            "variables",
            &json!({"caption":{"type":"string","value":"日本語"}}),
        )
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let before = fs::read(store.path()).unwrap();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.script_executions.len(), 2);
    assert!(
        layout
            .runtime_dependencies
            .iter()
            .any(|dependency| dependency.path.ends_with("inner.js"))
    );
    let output = render::png(&project, &layout, &Default::default()).unwrap();
    assert_eq!(
        raster(&output.png)
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::GREEN
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert_eq!(
        fs::read_to_string(temp.path().join("inner.js")).unwrap(),
        inner
    );
}

#[test]
fn null_reference_overrides_use_the_default_component_from_the_script_library() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("lib");
    fs::create_dir(&library).unwrap();
    let code = "/**\n@schema 2.21\n@input target: ref = \"button\"\n*/\nreturn [{type:'ref',ref:pencil.input.target,width:pencil.width,height:pencil.height}];";
    fs::write(library.join("draw.js"), code).unwrap();
    fs::write(library.join("ui.pen"),json!({"version":"2.21","children":[
        {"id":"button","type":"rectangle","reusable":true,"fill":"#ff0000","width":30,"height":20},
        {"id":"chart","type":"script","reusable":true,"scriptUri":"draw.js","width":30,"height":20}
    ]}).to_string()).unwrap();
    let path = temp.path().join("main.pen");
    let source = json!({"version":"2.21","imports":{"ui":"lib/ui.pen"},"children":[
        {"id":"button","type":"rectangle","reusable":true,"x":100,"fill":"#0000ff","width":30,"height":20},
        {"id":"chart","type":"ref","ref":"ui:chart","inputs":{"target":null,"unused":"$missing"}}
    ]}).to_string();
    fs::write(&path, &source).unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    let (project, layout) = snapshot(&store);
    let output = render::png(
        &project,
        &layout,
        &RenderOptions {
            node: Some("chart".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        raster(&output.png)
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::RED
    );
    assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    assert_eq!(fs::read_to_string(library.join("draw.js")).unwrap(), code);
}

#[test]
fn reference_inputs_keep_the_scope_of_the_document_that_supplies_them() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("lib");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("draw.js"),"/**\n@schema 2.21\n@input target: ref\n*/\nreturn [{type:'ref',ref:pencil.input.target,x:0,y:0,width:pencil.width,height:pencil.height}];").unwrap();
    fs::write(library.join("ui.pen"),json!({"version":"2.21","children":[
        {"id":"button","type":"rectangle","reusable":true,"fill":"#FF0000","width":30,"height":20},
        {"id":"chart","type":"script","reusable":true,"scriptUri":"draw.js","width":30,"height":20}
    ]}).to_string()).unwrap();
    let file = temp.path().join("main.pen");
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"lib/ui.pen"},"children":[
        {"id":"button","type":"rectangle","reusable":true,"x":100,"fill":"#0000FF","width":30,"height":20},
        {"id":"chart","type":"ref","ref":"ui:chart","inputs":{"target":"button"}}
    ]}).to_string()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    let (project, layout) = snapshot(&store);
    let image = render::png(
        &project,
        &layout,
        &RenderOptions {
            node: Some("chart".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        raster(&image.png)
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::BLUE
    );
    assert!(
        store
            .read()
            .unwrap()
            .text()
            .contains("\"target\":\"button\"")
    );
}

#[test]
fn generated_assets_have_their_own_snapshot_dependencies_and_invalidate_stale_previews() {
    let (temp, store) = fixture(
        "return [{type:'rectangle',width:40,height:20,fill:{type:'image',url:'generated.png'}}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20}),
    );
    solid(&temp.path().join("generated.png"), Color::BLUE);
    let base = Project::load(&store).unwrap().dependency_revision();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.dependency_revision, base);
    assert_eq!(layout.runtime_dependencies.len(), 1);
    assert_eq!(
        raster(
            &render::png(&project, &layout, &Default::default())
                .unwrap()
                .png
        )
        .peek_pixels()
        .unwrap()
        .get_color((10, 10)),
        Color::BLUE
    );
    solid(&temp.path().join("generated.png"), Color::RED);
    assert_eq!(
        render::png(&project, &layout, &Default::default())
            .err()
            .unwrap()
            .code,
        "REVISION_CONFLICT"
    );
    let (fresh, next) = snapshot(&store);
    assert_eq!(next.dependency_revision, base);
    assert_ne!(next.id, layout.id);
    assert_eq!(
        raster(&render::png(&fresh, &next, &Default::default()).unwrap().png)
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::RED
    );
}

#[test]
fn runtime_dependency_preconditions_guard_saves_dry_runs_and_retries() {
    let (temp, store) = fixture(
        "return [{type:'rectangle',width:40,height:20,fill:{type:'image',url:'generated.png'}}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20}),
    );
    let image_path = temp.path().join("generated.png");
    solid(&image_path, Color::BLUE);
    let (_, layout) = snapshot(&store);
    let before = store.read().unwrap().text().to_owned();
    let mut request: lazypencil::operations::EditRequest = serde_json::from_value(json!({
        "request_id":"runtime-guard", "expected_revision":layout.revision,
        "expected_dependency_revision":layout.dependency_revision,
        "expected_runtime_dependency_revision":layout.summary()["runtime_dependency_revision"],
        "operations":[{"op":"update","target":"chart","set":{"name":"Reviewed chart"}}]
    }))
    .unwrap();
    let mut runtime = ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"));
    solid(&image_path, Color::RED);
    for dry_run in [true, false] {
        request.dry_run = dry_run;
        let error =
            service::apply_request_with_runtime(&store, &request, &[], &mut runtime).unwrap_err();
        assert_eq!(error.code, "REVISION_CONFLICT");
        assert_eq!(store.read().unwrap().text(), before);
        assert!(store.history(10, None).unwrap().is_empty());
    }
    solid(&image_path, Color::BLUE);
    request.dry_run = true;
    let dry = service::apply_request_with_runtime(&store, &request, &[], &mut runtime).unwrap();
    assert_eq!(
        dry["validated_runtime_dependency_revision"],
        layout.summary()["runtime_dependency_revision"]
    );
    assert_eq!(store.read().unwrap().text(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    request.dry_run = false;
    let saved = service::apply_request_with_runtime(&store, &request, &[], &mut runtime).unwrap();
    assert_eq!(
        saved["validated_runtime_dependency_revision"],
        dry["validated_runtime_dependency_revision"]
    );
    assert_eq!(
        store.read().unwrap().value()["children"][0]["name"],
        "Reviewed chart"
    );
    fs::remove_file(image_path).unwrap();
    fs::write(temp.path().join("draw.js"), "/** @schema 2.21 */ for(;;){}").unwrap();
    let mut unavailable = ScriptRuntime::new(temp.path().join("missing-worker"));
    let replay =
        service::apply_request_with_runtime(&store, &request, &[], &mut unavailable).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["checkpoint_id"], saved["checkpoint_id"]);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
    request.expected_runtime_dependency_revision = Some("another-snapshot".into());
    assert_eq!(
        service::apply_request_with_runtime(&store, &request, &[], &mut unavailable)
            .unwrap_err()
            .code,
        "REQUEST_ID_CONFLICT"
    );
}

#[test]
fn real_cli_checks_runtime_dependencies_before_applying_an_edit() {
    let (temp, store) = fixture(
        "return [{type:'rectangle',width:40,height:20,fill:{type:'image',url:'generated.png'}}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20}),
    );
    let image_path = temp.path().join("generated.png");
    solid(&image_path, Color::BLUE);
    let (_, layout) = snapshot(&store);
    let operations = temp.path().join("edits.json");
    fs::write(
        &operations,
        r#"[{"op":"update","target":"chart","set":{"name":"Reviewed"}}]"#,
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--data-dir")
            .arg(store.data_dir())
            .args(["--json", "apply"])
            .arg(store.path())
            .arg("--operations")
            .arg(&operations)
            .args([
                "--expected-revision",
                &layout.revision,
                "--expected-dependency-revision",
                &layout.dependency_revision,
                "--expected-runtime-dependency-revision",
                layout.summary()["runtime_dependency_revision"]
                    .as_str()
                    .unwrap(),
                "--request-id",
                "cli-runtime-guard",
            ])
            .output()
            .unwrap()
    };
    solid(&image_path, Color::RED);
    let stale = run();
    assert_eq!(
        stale.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&stale.stdout)
    );
    assert!(store.history(10, None).unwrap().is_empty());
    solid(&image_path, Color::BLUE);
    let saved = run();
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    assert_eq!(
        store.read().unwrap().value()["children"][0]["name"],
        "Reviewed"
    );
}

#[test]
fn real_cli_renders_script_output_and_tracks_external_source_changes() {
    let (temp, store) = fixture(
        "return [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#0000FF'}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":60,"height":30}),
    );
    let output_path = temp.path().join("preview.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "render"])
        .arg(store.path())
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        raster(&fs::read(output_path).unwrap())
            .peek_pixels()
            .unwrap()
            .get_color((20, 10)),
        Color::BLUE
    );
    let (project, layout) = snapshot(&store);
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.21 */ return [];",
    )
    .unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let (_, updated) = snapshot(&store);
    assert_ne!(updated.id, layout.id);
    assert_ne!(updated.dependency_revision, layout.dependency_revision);
}

#[test]
fn nested_scripts_are_evaluated_and_disabled_scripts_do_not_run() {
    let (temp, store) = fixture(
        "return [{type:'script',scriptUri:'nested.js',width:40,height:20}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20}),
    );
    fs::write(temp.path().join("nested.js"),"/** @schema 2.21 */ return [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#0000FF'}];").unwrap();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.script_executions.len(), 2);
    assert_eq!(layout.runtime_dependencies.len(), 1);
    assert_eq!(
        raster(
            &render::png(&project, &layout, &Default::default())
                .unwrap()
                .png
        )
        .peek_pixels()
        .unwrap()
        .get_color((10, 10)),
        Color::BLUE
    );
    let (temp, store) = fixture(
        "for (;;) {}",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20,"enabled":false}),
    );
    fs::remove_file(temp.path().join("draw.js")).unwrap();
    let (_, layout) = snapshot(&store);
    assert!(layout.script_executions.is_empty());
}

#[test]
fn output_properties_participate_in_the_snapshot_identity() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("scene.pen");
    let doc=Document::parse(r##"{"version":"2.21","children":[{"id":"rect","type":"rectangle","width":20,"height":20,"fill":"#0000FF"}]}"##.into()).unwrap();
    fs::write(&path, doc.text()).unwrap();
    let scene = Project::from_document(&path, doc, &[])
        .unwrap()
        .resolve()
        .unwrap();
    let mut changed = scene.clone();
    changed.roots[0]
        .properties
        .insert("fill".into(), json!("#FF0000"));
    let left = lazypencil::layout::compute(scene, &mut Fonts::new(&[]).unwrap()).unwrap();
    let right = lazypencil::layout::compute(changed, &mut Fonts::new(&[]).unwrap()).unwrap();
    assert_eq!(left.revision, right.revision);
    assert_eq!(
        serde_json::to_value(left.records()).unwrap(),
        serde_json::to_value(right.records()).unwrap()
    );
    assert_ne!(left.id, right.id);
}

#[test]
fn the_bundled_script_example_renders_with_the_bundled_japanese_font_without_overflow() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/script-chart.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let (project, layout) = snapshot(&store);
    assert_eq!(layout.script_executions[0]["output_root_count"], 15);
    assert!(
        layout
            .items
            .iter()
            .flat_map(|item| &item.record.diagnostics)
            .next()
            .is_none()
    );
    let image = render::png(&project, &layout, &Default::default()).unwrap();
    assert_eq!((image.width, image.height), (960, 600));
}

#[test]
fn stored_children_of_script_nodes_are_rejected_instead_of_silently_replaced() {
    let (_temp, store) = fixture(
        "return [];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":40,"height":20,
        "children":[{"id":"stored-child","type":"rectangle","width":10,"height":10}]}),
    );
    let error = service::layout_snapshot_with_runtime(
        &store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "INVALID_DOCUMENT");
    assert!(store.read().unwrap().node("stored-child").is_ok());
}

#[test]
fn cli_output_cannot_overwrite_runtime_code_even_when_its_filename_ends_in_png() {
    let (temp, store) = fixture(
        "return [{type:'script',scriptUri:'nested.png',width:20,height:20}];",
        json!({"id":"chart","type":"script","scriptUri":"draw.js","width":20,"height":20}),
    );
    let code = "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:20,fill:'#0000FF'}];";
    let path = temp.path().join("nested.png");
    fs::write(&path, code).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "render"])
        .arg(store.path())
        .arg("--output")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(fs::read_to_string(path).unwrap(), code);
}
