use base64::Engine;
use lazypencil::{operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;
#[cfg(unix)]
#[path = "support/lock_wait.rs"]
mod lock_wait;

fn write(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir_all(temp.path().join("library/base")).unwrap();
    let png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=").unwrap();
    fs::write(temp.path().join("library/pixel.png"), png).unwrap();
    write(
        &temp.path().join("library/base/base.lib.pen"),
        json!({"version":"2.21","variables":{
        "tone":{"type":"color","value":[{"value":"#ff0000"},{"theme":{"mode":"dark"},"value":"#0000ff"}]}},
        "children":[{"id":"shared","name":"Dot","type":"frame","reusable":true,"layout":"none","width":20,"height":20,"fill":"$tone","children":[{"id":"label","type":"text","content":"X","fontSize":10,"fill":"#ffffff"}]}]}),
    );
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({"version":"2.21","imports":{"base":"base/base.lib.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"surface":{"type":"color","value":"#eeeeee"},"label_text":{"type":"string","value":"Save"}},
        "children":[{"id":"card","name":"Card","type":"frame","reusable":true,"layout":"none","width":160,"height":80,"fill":"$surface","children":[
            {"id":"shared","name":"Swatch","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"$surface"},
            {"id":"own-use","type":"ref","ref":"shared","x":40},
            {"id":"action","type":"ref","ref":"base:shared","x":20,"descendants":{"label":{"content":"$label_text"}}},
            {"id":"picture","type":"rectangle","x":90,"width":20,"height":20,"fill":{"type":"image","url":"pixel.png"}}
        ]}]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"children":[]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}
fn request(store: &Store, id: &str, operations: Value, dry_run: bool) -> EditRequest {
    let metadata = service::document_metadata(store).unwrap();
    serde_json::from_value(json!({"request_id":id,"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":operations,"dry_run":dry_run})).unwrap()
}
fn operation(mode: &str) -> Value {
    json!({"op":"import_component","source":{"library_alias":"ui","node_id":"card"},"mode":mode,"client_ref":"widget"})
}

#[test]
fn copied_sampler_arrays_keep_all_literal_images_after_removing_the_source_library() {
    let (temp, store) = fixture();
    let source = temp.path().join("library/ui.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    document["children"][0]["children"][3]["fill"] = json!({"type":"shader","url":"sample.glsl","uniforms":{"images":["pixel.png","pixel.png"]}});
    write(&source, document);
    fs::write(temp.path().join("library/sample.glsl"),"precision highp float;uniform sampler2D images[2];void main(){gl_FragColor=texture2D(images[0],vec2(0.5))+texture2D(images[1],vec2(0.5));}").unwrap();
    let edits = json!([operation("copy"),{"op":"remove_import","alias":"ui"}]);
    let before = fs::read(store.path()).unwrap();
    let preview = service::apply_request(
        &store,
        &request(&store, "array-preview", edits.clone(), true),
    )
    .unwrap();
    assert_eq!(
        preview["generated_ids"]["0"]["assets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
    let saved =
        service::apply_request(&store, &request(&store, "array-copy", edits, false)).unwrap();
    let target = saved["generated_ids"]["0"]["id"].as_str().unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let project = Project::load(&store).unwrap();
    let scene = project.resolve().unwrap();
    let root = scene.roots.iter().find(|node| node.id() == target).unwrap();
    let picture = root
        .children
        .iter()
        .find(|node| {
            node.properties["type"] == "rectangle" && node.properties["fill"]["type"] == "shader"
        })
        .unwrap();
    let inputs = picture.properties["fill"]["uniforms"]["images"]
        .as_array()
        .unwrap();
    assert_eq!(inputs.len(), 2);
    for value in inputs {
        assert!(value.as_str().unwrap().starts_with("assets/lazypencil/"));
        let path = project
            .resolve_file(picture.property_source("fill"), value.as_str().unwrap())
            .unwrap();
        assert!(project.asset(&path).is_ok());
    }
    assert!(
        !project
            .dependencies()
            .iter()
            .any(|entry| entry.kind == "document")
    );
}

#[test]
fn cancelled_imports_and_dry_runs_leave_inputs_outputs_and_history_unchanged() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let source = temp.path().join("library/ui.lib.pen");
    let cancel = tokio_util::sync::CancellationToken::new();
    for dry_run in [false, true] {
        let request = request(
            &store,
            "cancelled-import",
            json!([operation("copy")]),
            dry_run,
        );
        cancel.cancel();
        // Cancellation must win before reading a source that disappeared.
        let backup = temp.path().join("source-backup.pen");
        fs::rename(&source, &backup).unwrap();
        let error = service::apply_request_with_cancellation(&store, &request, &[], Some(&cancel))
            .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        fs::rename(&backup, &source).unwrap();
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
    }
    service::apply_request_with_cancellation(
        &store,
        &request(&store, "fresh-import", json!([operation("copy")]), false),
        &[],
        Some(&tokio_util::sync::CancellationToken::new()),
    )
    .unwrap();
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn cli_cancels_a_copy_before_publication_when_a_reader_holds_its_target_path() {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let image = fs::read(temp.path().join("library/pixel.png")).unwrap();
    let target = temp.path().join(format!(
        "assets/lazypencil/{}.png",
        lazypencil::document::hash(&image)
    ));
    let reader = Store::init(
        temp.path().join("reader.pen"),
        Some(temp.path().join("reader-data")),
    )
    .unwrap();
    let locks = reader
        .lock_dependencies(std::slice::from_ref(&target))
        .unwrap();
    let operations = temp.path().join("operations.json");
    fs::write(&operations, json!([operation("copy")]).to_string()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .arg("--json")
        .arg("--data-dir")
        .arg(store.data_dir())
        .arg("apply")
        .arg(store.path())
        .arg("--operations")
        .arg(operations)
        .arg("--expected-revision")
        .arg(metadata["revision"].as_str().unwrap())
        .arg("--expected-dependency-revision")
        .arg(metadata["dependency_revision"].as_str().unwrap());
    let child = lock_wait::WaitingProcess::spawn(&mut command);
    assert!(!target.exists());
    let started = Instant::now();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output();
    assert!(started.elapsed() < Duration::from_millis(1500));
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "CANCELLED"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
    drop(locks);
    service::apply_request(
        &store,
        &request(
            &store,
            "copy-after-reader",
            json!([operation("copy")]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(fs::read(target).unwrap(), image);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn cli_cancels_after_copy_publication_while_waiting_for_a_source_library_lock() {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let source = Store::open(
        temp.path().join("library/ui.lib.pen"),
        Some(store.data_dir().into()),
    )
    .unwrap();
    source.history(1, None).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(source.lock_file_path())
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let operations = temp.path().join("operations.json");
    fs::write(&operations, json!([operation("copy")]).to_string()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["--json", "apply"])
        .arg(store.path())
        .arg("--operations")
        .arg(&operations)
        .args([
            "--expected-revision",
            metadata["revision"].as_str().unwrap(),
            "--expected-dependency-revision",
            metadata["dependency_revision"].as_str().unwrap(),
        ]);
    let child = lock_wait::WaitingProcess::spawn(&mut command);
    let started = Instant::now();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output();
    assert!(started.elapsed() < Duration::from_millis(1500));
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "CANCELLED");
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    // Publication is immutable; cancellation can leave a complete, unreferenced asset.
    let expected = fs::read(temp.path().join("library/pixel.png")).unwrap();
    let retained = fs::read_dir(temp.path().join("assets/lazypencil"))
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(fs::read(retained[0].path()).unwrap(), expected);
    drop(lock);
    service::apply_request(
        &store,
        &request(&store, "retry-copy", json!([operation("copy")]), false),
    )
    .unwrap();
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn link_keeps_original_components_variables_and_assets_without_copying_files() {
    let (temp, store) = fixture();
    let library = fs::read(temp.path().join("library/ui.lib.pen")).unwrap();
    let result = service::apply_request(
        &store,
        &request(&store, "link", json!([operation("link")]), false),
    )
    .unwrap();
    let document = store.read().unwrap();
    let id = result["bindings"]["widget"].as_str().unwrap();
    assert_eq!(document.node(id).unwrap()["ref"], "ui:card");
    assert!(document.value().get("variables").is_none());
    assert!(!temp.path().join("assets").exists());
    assert_eq!(
        fs::read(temp.path().join("library/ui.lib.pen")).unwrap(),
        library
    );
    assert!(
        Project::load(&store)
            .unwrap()
            .dependencies()
            .iter()
            .any(|d| d.kind == "image")
    );
}

#[test]
fn copy_closes_component_variable_image_and_override_dependencies() {
    let (temp, store) = fixture();
    let source = fs::read(temp.path().join("library/ui.lib.pen")).unwrap();
    let request = request(
        &store,
        "copy",
        json!([
            operation("copy"),{"op":"remove_import","alias":"ui"},
            {"op":"update","target":{"result_of":"widget"},"set":{"name":"Copied card"}}
        ]),
        false,
    );
    let result = service::apply_request(&store, &request).unwrap();
    let document = store.read().unwrap();
    let id = result["bindings"]["widget"].as_str().unwrap();
    let component = result["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    assert_eq!(document.node(id).unwrap()["ref"], component);
    assert_eq!(document.node(id).unwrap()["name"], "Copied card");
    let definition = document.node(component).unwrap();
    let children = &definition["children"];
    assert_eq!(children[1]["ref"], children[0]["id"]);
    assert_ne!(
        children[0]["id"],
        document.node(children[2]["ref"].as_str().unwrap()).unwrap()["id"]
    );
    let dot = document.node(children[2]["ref"].as_str().unwrap()).unwrap();
    let label = dot["children"][0]["id"].as_str().unwrap();
    assert_eq!(children[2]["descendants"][label]["content"], "$label_text");
    assert!(
        children[3]["fill"]["url"]
            .as_str()
            .unwrap()
            .starts_with("assets/lazypencil/")
    );
    assert_eq!(document.value()["variables"]["tone"]["type"], "color");
    assert_eq!(document.value()["themes"]["mode"], json!(["light", "dark"]));
    assert_eq!(
        fs::read(temp.path().join("library/ui.lib.pen")).unwrap(),
        source
    );
    assert!(
        Project::load(&store)
            .unwrap()
            .dependencies()
            .iter()
            .all(|d| d.kind != "document")
    );
    assert_eq!(store.history(10, None).unwrap().len(), 1);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    Project::load(&store).unwrap().resolve().unwrap();
    let replay = service::apply_request(&store, &request).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["generated_ids"], result["generated_ids"]);
}

#[test]
fn dry_run_and_a_later_invalid_operation_create_no_managed_files() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let query = request(
        &store,
        "preview",
        json!([operation("copy"),{"op":"remove_import","alias":"ui"}]),
        true,
    );
    let preview = service::apply_request(&store, &query).unwrap();
    assert_eq!(preview["dry_run"], true);
    assert!(
        !preview["generated_ids"]["0"]["assets"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
    let invalid = request(
        &store,
        "invalid",
        json!([operation("copy"),{"op":"update","target":"absent","set":{"x":1}}]),
        false,
    );
    assert!(service::apply_request(&store, &invalid).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
}

#[test]
fn variable_component_and_theme_conflicts_require_explicit_decisions() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "",
            "variables",
            &json!({"tone":{"type":"color","value":"#00ff00"}}),
        )
        .unwrap();
    document
        .set_property("", "themes", &json!({"mode":["dark","light"]}))
        .unwrap();
    document.insert_element("/children",0,&json!({"id":"existing","type":"rectangle","reusable":true,"name":"Card","width":1,"height":1}).to_string()).unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let before = fs::read(store.path()).unwrap();
    let error = service::apply_request(
        &store,
        &request(&store, "conflict", json!([operation("copy")]), false),
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_OPERATION");
    assert_eq!(fs::read(store.path()).unwrap(), before);
    let mut copy = operation("copy");
    copy["options"] = json!({"variable_prefix":"kit.","component_prefix":"Kit ","themes":{"ui:mode":{"action":"rename","name":"kit.mode"}}});
    let result =
        service::apply_request(&store, &request(&store, "rename", json!([copy]), false)).unwrap();
    let document = store.read().unwrap();
    assert_eq!(document.value()["variables"]["tone"]["value"], "#00ff00");
    assert_eq!(
        document.value()["variables"]["kit.tone"]["value"][1]["theme"]["kit.mode"],
        "dark"
    );
    assert_eq!(
        document
            .node(
                result["generated_ids"]["0"]["component_id"]
                    .as_str()
                    .unwrap()
            )
            .unwrap()["name"],
        "Kit Card"
    );
    assert_eq!(document.value()["themes"]["mode"][0], "dark");
}

#[test]
fn explicit_variable_reuse_checks_types_and_component_reuse_avoids_extra_files() {
    let (temp, store) = fixture();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "",
            "variables",
            &json!({"brand":{"type":"color","value":"#00ff00"},"wrong":{"type":"string","value":"text"}}),
        )
        .unwrap();
    document.insert_element("/children",0,&json!({"id":"existing","type":"frame","reusable":true,"name":"Existing","width":15,"height":15}).to_string()).unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let mut invalid = operation("copy");
    invalid["options"] = json!({"variables":{"ui:base:tone":{"action":"reuse","target":"wrong"}}});
    let before = fs::read(store.path()).unwrap();
    assert_eq!(
        service::apply_request(
            &store,
            &request(&store, "wrong-reuse-type", json!([invalid]), false)
        )
        .unwrap_err()
        .code,
        "INVALID_OPERATION"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
    let mut copy = operation("copy");
    copy["options"] = json!({"variables":{"ui:base:tone":{"action":"reuse","target":"brand"}}});
    service::apply_request(
        &store,
        &request(&store, "reuse-variable", json!([copy]), false),
    )
    .unwrap();
    assert!(
        store.read().unwrap().value()["variables"]
            .get("tone")
            .is_none()
    );
    let count = fs::read_dir(temp.path().join("assets/lazypencil"))
        .unwrap()
        .count();
    let mut reuse = operation("copy");
    reuse["options"] = json!({"components":{"ui:card":{"action":"reuse","target":"existing"}}});
    let result = service::apply_request(
        &store,
        &request(&store, "reuse-component", json!([reuse]), false),
    )
    .unwrap();
    assert_eq!(result["generated_ids"]["0"]["component_id"], "existing");
    assert_eq!(
        fs::read_dir(temp.path().join("assets/lazypencil"))
            .unwrap()
            .count(),
        count
    );
}

#[test]
fn bundled_style_imports_support_both_modes_and_an_explicit_source_revision() {
    for mode in ["link", "copy"] {
        let (_temp, store) = fixture();
        let style = lazypencil::guidance::get_style("standard-ui").unwrap();
        let op = json!({"op":"import_component","mode":mode,"source":{"style_id":"standard-ui","node_id":"button"},"expected_source_revision":style["example"]["hash"],"client_ref":"button"});
        let dry = request(&store, "style-preview", json!([op]), true);
        service::apply_request(&store, &dry).unwrap();
        assert!(!store.path().parent().unwrap().join("assets").exists());
        let result =
            service::apply_request(&store, &request(&store, "style", json!([op]), false)).unwrap();
        let document = store.read().unwrap();
        let button = document
            .node(result["bindings"]["button"].as_str().unwrap())
            .unwrap();
        if mode == "link" {
            assert_eq!(button["ref"], "style-standard-ui:button");
        } else {
            assert_eq!(
                document.node(button["ref"].as_str().unwrap()).unwrap()["name"],
                "Primary action"
            );
        }
        Project::load(&store).unwrap().resolve().unwrap();
    }
}

#[test]
fn stale_source_revision_and_unknown_name_policies_do_not_save() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let mut op = operation("copy");
    op["expected_source_revision"] = json!("stale");
    assert_eq!(
        service::apply_request(&store, &request(&store, "stale", json!([op]), false))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let mut op = operation("copy");
    op["options"] = json!({"variables":{"typo:variable":{"action":"rename","name":"changed"}}});
    assert!(
        service::apply_request(
            &store,
            &request(&store, "unknown-policy", json!([op]), false)
        )
        .is_err()
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
}

#[cfg(feature = "rendering")]
#[test]
fn copied_and_linked_instances_have_identical_pixels_after_sources_are_removed() {
    use lazypencil::render::{self, RenderOptions};
    let (temp, store) = fixture();
    let linked = service::apply_request(
        &store,
        &request(&store, "render-link", json!([operation("link")]), false),
    )
    .unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let before = render::png(&project, &snapshot, &RenderOptions::default()).unwrap();
    let linked_id = linked["bindings"]["widget"].as_str().unwrap();
    service::apply_request(&store,&request(&store,"render-copy",json!([
        {"op":"delete","target":linked_id},operation("copy"),{"op":"remove_import","alias":"ui"}
    ]),false)).unwrap();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let after = render::png(&project, &snapshot, &RenderOptions::default()).unwrap();
    assert_eq!((before.width, before.height), (160, 80));
    assert_eq!(after.png, before.png);
    assert_eq!(after.assets[0].hash, before.assets[0].hash);
}

#[test]
fn actual_cli_can_import_a_bundled_button() {
    let (_temp, store) = fixture();
    let metadata = service::document_metadata(&store).unwrap();
    let path = store.path().parent().unwrap().join("ops.json");
    write(
        &path,
        json!([{"op":"import_component","source":{"style_id":"standard-ui","node_id":"button"},"mode":"copy"}]),
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["--json", "apply"])
        .arg(store.path())
        .arg("--operations")
        .arg(path)
        .arg("--expected-revision")
        .arg(metadata["revision"].as_str().unwrap())
        .arg("--expected-dependency-revision")
        .arg(metadata["dependency_revision"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["generated_ids"]["0"]["component_id"].is_string());
}

#[cfg(unix)]
#[test]
fn symbolic_managed_directories_are_rejected_without_writing_through_them() {
    let (temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("assets")).unwrap();
    let before = fs::read(store.path()).unwrap();
    assert_eq!(
        service::apply_request(
            &store,
            &request(&store, "symlink", json!([operation("copy")]), false)
        )
        .unwrap_err()
        .code,
        "ASSET_CONFLICT"
    );
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[test]
fn a_file_source_registers_a_link_but_copy_keeps_no_source_import() {
    for mode in ["link", "copy"] {
        let (_temp, store) = fixture();
        let mut document = store.read().unwrap();
        document.remove_property("", "imports").unwrap();
        fs::write(store.path(), document.text()).unwrap();
        let op = json!({"op":"import_component","mode":mode,"source":{"uri":"library/ui.lib.pen","import_alias":"kit","node_id":"card"},"client_ref":"new"});
        let result =
            service::apply_request(&store, &request(&store, "file-import", json!([op]), false))
                .unwrap();
        let document = store.read().unwrap();
        if mode == "link" {
            assert_eq!(document.value()["imports"]["kit"], "library/ui.lib.pen");
        } else {
            assert!(document.value().get("imports").is_none());
        }
        assert!(
            document
                .node(result["bindings"]["new"].as_str().unwrap())
                .is_ok()
        );
    }
}

#[test]
fn copying_a_library_that_imports_the_consumer_does_not_create_a_saved_cycle() {
    let (temp, store) = fixture();
    let mut document = store.read().unwrap();
    document.remove_property("", "imports").unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let library = temp.path().join("library/ui.lib.pen");
    let mut source =
        lazypencil::document::Document::parse(fs::read_to_string(&library).unwrap()).unwrap();
    source
        .set_property("/imports", "consumer", &json!("../main.pen"))
        .unwrap();
    fs::write(library, source.text()).unwrap();
    let copy = json!({"op":"import_component","mode":"copy","source":{"uri":"library/ui.lib.pen","import_alias":"kit","node_id":"card"}});
    service::apply_request(&store, &request(&store, "copy-cycle", json!([copy]), false)).unwrap();
    assert!(store.read().unwrap().value().get("imports").is_none());
    let before = fs::read(store.path()).unwrap();
    let link = json!({"op":"import_component","mode":"link","source":{"uri":"library/ui.lib.pen","import_alias":"kit","node_id":"card"}});
    assert_eq!(
        service::apply_request(&store, &request(&store, "link-cycle", json!([link]), false))
            .unwrap_err()
            .code,
        "REFERENCE_CYCLE"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn equal_bytes_do_not_hide_retargeted_library_or_image_symlinks() {
    for kind in ["library", "image"] {
        let (temp, store) = fixture();
        let (alias, first, second) = if kind == "library" {
            let first = temp.path().join("library/ui.lib.pen");
            let second = temp.path().join("library/ui-copy.lib.pen");
            fs::copy(&first, &second).unwrap();
            let alias = temp.path().join("library/alias.lib.pen");
            let mut document = store.read().unwrap();
            document
                .set_property("/imports", "ui", &json!("library/alias.lib.pen"))
                .unwrap();
            fs::write(store.path(), document.text()).unwrap();
            (alias, first, second)
        } else {
            let alias = temp.path().join("library/pixel.png");
            let first = temp.path().join("library/first.png");
            let second = temp.path().join("library/second.png");
            fs::copy(&alias, &first).unwrap();
            fs::copy(&alias, &second).unwrap();
            fs::remove_file(&alias).unwrap();
            let mut document = store.read().unwrap();
            document
                .insert_element(
                    "/children",
                    0,
                    &json!({"id":"visible","type":"ref","ref":"ui:card"}).to_string(),
                )
                .unwrap();
            fs::write(store.path(), document.text()).unwrap();
            (alias, first, second)
        };
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let snapshot = Project::load(&store).unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        assert_eq!(
            snapshot.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT",
            "{kind}"
        );
    }
}

#[test]
fn local_link_and_copy_preserve_existing_tokens_and_reuse_identical_managed_bytes() {
    let (temp, store) = fixture();
    let imported = service::apply_request(
        &store,
        &request(
            &store,
            "initial",
            json!([operation("copy"),{"op":"remove_import","alias":"ui"}]),
            false,
        ),
    )
    .unwrap();
    let component = imported["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    let variables = store.read().unwrap().value()["variables"].clone();
    let files = fs::read_dir(temp.path().join("assets/lazypencil"))
        .unwrap()
        .count();
    for mode in ["link", "copy"] {
        let options = if mode == "copy" {
            json!({"component_prefix":"Local "})
        } else {
            json!({})
        };
        let op = json!({"op":"import_component","source":{"node_id":component},"mode":mode,"options":options});
        service::apply_request(
            &store,
            &request(&store, &format!("local-{mode}"), json!([op]), false),
        )
        .unwrap();
        assert_eq!(store.read().unwrap().value()["variables"], variables);
        assert_eq!(
            fs::read_dir(temp.path().join("assets/lazypencil"))
                .unwrap()
                .count(),
            files
        );
    }
}

#[test]
fn changed_managed_copy_targets_are_rejected_without_overwriting_them() {
    let (temp, store) = fixture();
    let result = service::apply_request(
        &store,
        &request(&store, "first-copy", json!([operation("copy")]), false),
    )
    .unwrap();
    let uri = result["generated_ids"]["0"]["assets"][0]["uri"]
        .as_str()
        .unwrap();
    let file = temp.path().join(uri);
    fs::write(&file, b"changed by an external writer").unwrap();
    let before = fs::read(store.path()).unwrap();
    let mut copy = operation("copy");
    copy["options"] = json!({"variable_prefix":"new.","component_prefix":"New ","themes":{"ui:mode":{"action":"reuse","target":"mode"}}});
    assert_eq!(
        service::apply_request(
            &store,
            &request(&store, "changed-copy", json!([copy]), false)
        )
        .unwrap_err()
        .code,
        "ASSET_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert_eq!(fs::read(file).unwrap(), b"changed by an external writer");
}

#[test]
fn direct_file_imports_cannot_escape_the_allowed_project_root() {
    let (_temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    let path = outside.path().join("outside.pen");
    write(
        &path,
        json!({"version":"2.21","children":[{"id":"secret","type":"rectangle","reusable":true}]}),
    );
    let before = fs::read(store.path()).unwrap();
    let op = json!({"op":"import_component","mode":"copy","source":{"uri":path,"import_alias":"outside","node_id":"secret"}});
    assert_eq!(
        service::apply_request(&store, &request(&store, "outside", json!([op]), false))
            .unwrap_err()
            .code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[test]
fn copy_uses_explicit_source_identity_when_literal_ids_shadow_qualified_references() {
    let (temp, store) = fixture();
    let library = temp.path().join("library/ui.lib.pen");
    let mut source =
        lazypencil::document::Document::parse(fs::read_to_string(&library).unwrap()).unwrap();
    source
        .set_property("/imports", "other", &json!("base/base.lib.pen"))
        .unwrap();
    source
        .set_property("/children/0/children/0", "id", &json!("base:shared"))
        .unwrap();
    source
        .set_property("/children/0/children/1", "ref", &json!("base:shared"))
        .unwrap();
    source
        .set_property("/children/0/children/2", "ref", &json!("other:shared"))
        .unwrap();
    fs::write(library, source.text()).unwrap();
    let mut document = store.read().unwrap();
    document
        .insert_element(
            "/children",
            0,
            &json!({"id":"ui:card","type":"rectangle","width":1,"height":1}).to_string(),
        )
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    assert!(
        service::apply_request(
            &store,
            &request(&store, "shadowed-link", json!([operation("link")]), false)
        )
        .is_err()
    );
    let result = service::apply_request(
        &store,
        &request(&store, "explicit-copy", json!([operation("copy")]), false),
    )
    .unwrap();
    let report = &result["generated_ids"]["0"];
    assert_ne!(
        report["id_map"]["ui:base%3Ashared"],
        report["id_map"]["ui:base:shared"]
    );
    let document = store.read().unwrap();
    let definition = document
        .node(report["component_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        definition["children"][1]["ref"],
        report["id_map"]["ui:base%3Ashared"]
    );
    assert_eq!(
        definition["children"][2]["ref"],
        report["id_map"]["ui:base:shared"]
    );
    assert_eq!(document.node("ui:card").unwrap()["type"], "rectangle");
}

#[test]
fn linking_a_style_preserves_comments_and_escapes_in_unrelated_imports() {
    let (_temp, store) = fixture();
    let source = r#"{"version":"2.21","imports":{
      // Keep the existing library comment.
      "ui":"library/u\u0069.lib.pen"
    },"children":[],"extra":1.000e2}"#;
    fs::write(store.path(), source).unwrap();
    let op = json!({"op":"import_component","mode":"link","source":{"style_id":"standard-ui","node_id":"button"}});
    service::apply_request(
        &store,
        &request(&store, "style-preservation", json!([op]), false),
    )
    .unwrap();
    let document = store.read().unwrap();
    assert!(
        document
            .text()
            .contains("// Keep the existing library comment.")
    );
    assert!(
        document
            .text()
            .contains(r#""ui":"library/u\u0069.lib.pen""#)
    );
    assert!(document.text().contains("1.000e2"));
    assert!(document.value()["imports"]["style-standard-ui"].is_string());
}
