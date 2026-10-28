// Shared namespace regressions: fast layout checks and actual Chromium pixels.
#![allow(dead_code)]

use lazypencil::{
    cdp::LocalFiles, operations::EditRequest, project::Project, script::ScriptRuntime, service,
    store::Store,
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;

pub const BASE: &str = "file:///lazypencil/generated-site/";
pub const STYLES: [&str; 27] = [
    "direct",
    "reference",
    "descendant",
    "replacement",
    "children",
    "inherited",
    "nested",
    "scope-direct",
    "scope-reference",
    "scope-descendant",
    "scope-replacement",
    "scope-children",
    "scope-metadata",
    "scope-descendant-metadata",
    "scope-envelope",
    "scope-descendant-envelope",
    "scope-envelope-metadata",
    "scope-descendant-envelope-metadata",
    "scope-empty",
    "scope-qualified",
    "scope-nested",
    "scope-stale",
    "scope-stale-metadata",
    "scope-stale-descendant-metadata",
    "scope-stale-envelope-metadata",
    "scope-stale-descendant-envelope-metadata",
    "scope-default-shift-metadata",
];

fn nested(style: &str) -> bool {
    matches!(style, "nested" | "scope-nested")
}

fn library(style: &str) -> &str {
    match style {
        "scope-empty"
        | "scope-metadata"
        | "scope-descendant-metadata"
        | "scope-envelope-metadata"
        | "scope-descendant-envelope-metadata" => "",
        "scope-qualified" => "base:leaf",
        value if value.starts_with("scope-stale") || value == "scope-default-shift-metadata" => {
            "not-declared"
        }
        _ => "base",
    }
}

pub fn rgba(style: &str) -> [u8; 4] {
    if style == "scope-qualified" {
        [0, 0, 255, 255]
    } else if style == "inherited"
        || (style.starts_with("scope-")
            && !matches!(
                style,
                "scope-empty"
                    | "scope-metadata"
                    | "scope-descendant-metadata"
                    | "scope-stale"
                    | "scope-envelope-metadata"
                    | "scope-descendant-envelope-metadata"
            ))
    {
        [255, 0, 0, 255]
    } else {
        [0, 255, 0, 255]
    }
}

fn write(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn resources(path: &str) -> Value {
    json!({"lazypencil":{"browser_resources":{"version":1,"roots":[{"url":BASE,"path":path}]}}})
}

fn site(path: &Path, color: &str) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("index.html"), "<!doctype html><link rel='icon' href='data:,'><link rel='stylesheet' href='color.css'><body></body>").unwrap();
    fs::write(
        path.join("color.css"),
        format!("html,body{{margin:0;background:{color}}}"),
    )
    .unwrap();
}

fn apply(store: &Store, id: &str, operations: Value) -> Value {
    let project = Project::load(store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({"request_id":id,
        "expected_revision":project.root_document().revision(),"expected_dependency_revision":project.dependency_revision(),"operations":operations})).unwrap();
    service::apply_request(store, &request).unwrap_or_else(|error| panic!("{id}: {error:?}"))
}

pub fn setup(version: &str, style: &str, declared: bool) -> (TempDir, Store, String) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    site(&temp.path().join("library/base/web"), "#ff0000");
    site(&temp.path().join("library/web"), "#00ff00");
    site(&temp.path().join("consumer-web"), "#ff00ff");
    fs::write(
        temp.path().join("consumer-web/unknown.html"),
        "caller-only page",
    )
    .unwrap();
    let url = format!("{BASE}index.html?literal=$unchanged#anchor");
    let page = |id: &str| {
        json!({"id":id,"type":"browser","width":32,"height":16,"url":url,
        "metadata":{"type":"example","label":"base"}})
    };
    let mut reusable = page("page");
    reusable["reusable"] = json!(true);
    write(
        &temp.path().join("library/base/base.pen"),
        json!({"version":version,"metadata":resources("web"),"children":[reusable,
        {"id":"panel","type":"frame","reusable":true,"layout":"none","width":32,"height":16,"children":[page("inside")]}]}),
    );
    if matches!(style, "scope-qualified" | "scope-default-shift-metadata") {
        site(&temp.path().join("library/base/leaf/web"), "#0000ff");
        write(
            &temp.path().join("library/base/leaf/leaf.pen"),
            json!({"version":version,"metadata":resources("web"),"children":[]}),
        );
        let base_path = temp.path().join("library/base/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&base_path).unwrap()).unwrap();
        base["imports"] = json!({"leaf":"leaf/leaf.pen"});
        if style == "scope-default-shift-metadata" {
            base["children"][0]["metadata"]["lazypencil"] =
                json!({"browser_scope":{"version":1,"url":url,"library":"leaf"}});
        }
        write(&base_path, base);
    }
    let scope_url = if style.starts_with("scope-stale") || style == "scope-default-shift-metadata" {
        format!("{BASE}stale.html")
    } else {
        url.clone()
    };
    let scope = json!({"version":1,"url":scope_url,"library":library(style)});
    let mut metadata = json!({"type":"example","label":"generated","literal":"$unchanged"});
    if style.starts_with("scope-") {
        metadata["lazypencil"] = json!({"browser_scope":scope});
    }
    let mut changes = json!({"url":url,"metadata":metadata});
    if matches!(
        style,
        "scope-envelope"
            | "scope-descendant-envelope"
            | "scope-envelope-metadata"
            | "scope-descendant-envelope-metadata"
            | "scope-stale-envelope-metadata"
            | "scope-stale-descendant-envelope-metadata"
    ) {
        let mut ordinary = metadata.clone();
        ordinary.as_object_mut().unwrap().remove("lazypencil");
        changes["metadata"] = json!({"type":"lazypencil","lazypencil":{"browser_override":{"version":1,"metadata":ordinary,"scope":scope}}});
    }
    if style.ends_with("-metadata") {
        changes.as_object_mut().unwrap().remove("url");
    }
    let direct = || json!({"type":"browser","width":32,"height":16,"url":url,"metadata":metadata});
    let reference = || {
        let mut value = changes.clone();
        value["type"] = json!("ref");
        value["ref"] = json!("base:page");
        value
    };
    let template = match style {
        "direct" | "nested" | "scope-direct" | "scope-empty" | "scope-qualified"
        | "scope-nested" | "scope-stale" => direct(),
        "reference"
        | "scope-reference"
        | "scope-metadata"
        | "scope-envelope"
        | "scope-envelope-metadata"
        | "scope-stale-metadata"
        | "scope-stale-envelope-metadata"
        | "scope-default-shift-metadata" => reference(),
        "descendant"
        | "scope-descendant"
        | "scope-descendant-metadata"
        | "scope-descendant-envelope"
        | "scope-descendant-envelope-metadata"
        | "scope-stale-descendant-metadata"
        | "scope-stale-descendant-envelope-metadata" => {
            json!({"type":"ref","ref":"base:panel","descendants":{"inside":changes}})
        }
        "replacement" | "scope-replacement" => {
            json!({"type":"ref","ref":"base:panel","descendants":{"inside":direct()}})
        }
        "children" | "scope-children" => {
            json!({"type":"ref","ref":"base:panel","children":[reference()]})
        }
        "inherited" => json!({"type":"ref","ref":"base:page"}),
        _ => panic!("Unknown generated browser style"),
    };
    let code = if nested(style) {
        fs::write(
            temp.path().join("library/nested.js"),
            format!("/* @schema 2.21 */ return [{template}];"),
        )
        .unwrap();
        "/* @schema 2.21 */ return [{type:'script',scriptUri:'nested.js',width:32,height:16}];"
            .into()
    } else {
        format!("/* @schema 2.21 */ return [{template}];")
    };
    fs::write(temp.path().join("library/draw.js"), &code).unwrap();
    let mut library = json!({"version":version,"imports":{"base":"base/base.pen"},"children":[
        {"id":"generator","type":"script","reusable":true,"scriptUri":"draw.js","width":32,"height":16} ]});
    if declared {
        library["metadata"] = resources("web");
    }
    write(&temp.path().join("library/ui.pen"), library);
    write(
        &temp.path().join("main.pen"),
        json!({"version":version,"imports":{"ui":"library/ui.pen"},
        "metadata":resources("consumer-web"),"children":[{"id":"use","type":"ref","ref":"ui:generator"}]}),
    );
    let store = Store::open(
        temp.path().join("main.pen"),
        Some(temp.path().join("private")),
    )
    .unwrap();
    (temp, store, code)
}

pub fn copy(store: &Store, style: &str) -> Value {
    let mut options = json!({});
    if nested(style) {
        options["script_assets"] = json!({"ui:generator":[{"kind":"script","uri":"nested.js"}]});
    }
    apply(
        store,
        "copy-generator",
        json!([
        {"op":"import_component","mode":"copy","source":{"library_alias":"ui","node_id":"generator"},"options":options},
        {"op":"delete","target":"use"},{"op":"remove_import","alias":"ui"}]),
    )
}

pub struct Frame {
    pub stage: &'static str,
    pub url: String,
    pub revision: String,
    pub files: Arc<LocalFiles>,
    pub rgba: [u8; 4],
}

fn capture(store: &Store, stage: &'static str, style: &str) -> Frame {
    let saved = fs::read(store.path()).unwrap();
    let (project, layout) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    let nodes = layout
        .items
        .iter()
        .filter(|item| item.record.enabled && item.node.kind() == "browser")
        .collect::<Vec<_>>();
    assert_eq!(nodes.len(), 1, "{style}/{stage}");
    let node = &nodes[0].node;
    assert!(node.generated_by.is_some());
    let url = node.properties["url"].as_str().unwrap();
    assert_eq!(url, format!("{BASE}index.html?literal=$unchanged#anchor"));
    let label = if style == "inherited" {
        "base"
    } else {
        "generated"
    };
    assert_eq!(node.properties["metadata"]["label"], label);
    if label == "generated" {
        assert_eq!(node.properties["metadata"]["literal"], "$unchanged");
    }
    let input = project.browser_node_resources(node).unwrap();
    let caller = project.browser_page_resources(store.path(), url).unwrap();
    assert_ne!(
        input.revision, caller.revision,
        "Generated pages must not borrow caller resources"
    );
    project.verify_unchanged().unwrap();
    assert_eq!(saved, fs::read(store.path()).unwrap());
    Frame {
        stage,
        url: url.into(),
        revision: input.revision,
        files: input.files,
        rgba: rgba(style),
    }
}

fn recopy(temp: &TempDir, store: &Store, version: &str, component: &str) -> Store {
    let second_dir = temp.path().join("second");
    site(&second_dir.join("web"), "#ffffff");
    write(
        &second_dir.join("scene.pen"),
        json!({"version":version,"imports":{"src":"../main.pen"},"metadata":resources("web"),"children":[]}),
    );
    let second = Store::open(
        second_dir.join("scene.pen"),
        Some(temp.path().join("second-private")),
    )
    .unwrap();
    apply(
        &second,
        "recopy-generator",
        json!([
        {"op":"import_component","mode":"copy","source":{"library_alias":"src","node_id":component}},
        {"op":"remove_import","alias":"src"}]),
    );
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    fs::remove_dir_all(temp.path().join("consumer-web")).unwrap();
    fs::remove_file(store.path()).unwrap();
    second
}

pub fn study(version: &str, style: &str) -> (TempDir, Vec<Frame>) {
    eprintln!("Generated browser copy: schema={version}, style={style}");
    let (temp, store, code) = setup(version, style, true);
    let mut frames = vec![capture(&store, "before", style)];
    let response = copy(&store, style);
    let component = response["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    let document = store.read().unwrap();
    let copied = document.node(component).unwrap();
    assert_eq!(
        fs::read(temp.path().join(copied["scriptUri"].as_str().unwrap())).unwrap(),
        code.as_bytes()
    );
    if style != "inherited" {
        assert!(
            copied
                .pointer("/metadata/lazypencil/script_bindings/browser_library")
                .is_some()
        );
    }
    if style.starts_with("scope-")
        && !style.starts_with("scope-stale")
        && style != "scope-default-shift-metadata"
    {
        let aliases = copied
            .pointer("/metadata/lazypencil/script_bindings/browser_libraries")
            .unwrap()
            .as_object()
            .unwrap();
        let target = aliases[library(style)].as_str().unwrap();
        assert!(!target.is_empty());
        assert_ne!(target, library(style));
    }
    if style == "direct" {
        let path = temp.path().join(copied["scriptUri"].as_str().unwrap());
        fs::write(
            &path,
            code.replace("index.html?literal=$unchanged#anchor", "unknown.html"),
        )
        .unwrap();
        let (project, layout) = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .unwrap();
        let page = &layout
            .items
            .iter()
            .find(|item| item.record.enabled && item.node.kind() == "browser")
            .unwrap()
            .node;
        let url = page.properties["url"].as_str().unwrap();
        assert_eq!(url, format!("{BASE}unknown.html"));
        assert!(page.generated_by.is_some());
        assert!(project.browser_page_resources(store.path(), url).is_ok());
        match project.browser_node_resources(page) {
            Err(error) => assert_eq!(error.code, "ASSET_NOT_FOUND"),
            Ok(_) => panic!(
                "Undeclared generated page was accepted: {url}, owner={}",
                page.browser_origin().display()
            ),
        }
        fs::write(path, &code).unwrap();
    }
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    frames.push(capture(&store, "copied", style));
    let second = recopy(&temp, &store, version, component);
    frames.push(capture(&second, "recopied", style));
    assert!(
        frames
            .iter()
            .all(|frame| frame.revision == frames[0].revision)
    );
    assert!(
        Project::load(&second)
            .unwrap()
            .dependencies()
            .iter()
            .all(|dependency| dependency.path.starts_with(second.path().parent().unwrap()))
    );
    (temp, frames)
}

pub fn reject_missing_namespace() {
    for version in ["2.19", "2.21"] {
        let (temp, store, _) = setup(version, "direct", false);
        copy(&store, "direct");
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let result = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        );
        assert_eq!(result.err().unwrap().code, "ASSET_NOT_FOUND");
    }
}

pub fn reject_empty_namespace_after_recopy() {
    fn closed(store: &Store) {
        let saved = fs::read(store.path()).unwrap();
        let (project, layout) = service::layout_snapshot_with_runtime(
            store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .unwrap();
        let page = &layout
            .items
            .iter()
            .find(|item| item.record.enabled && item.node.kind() == "browser")
            .unwrap()
            .node;
        let url = page.properties["url"].as_str().unwrap();
        assert!(project.browser_page_resources(store.path(), url).is_ok());
        assert_eq!(
            project.browser_node_resources(page).err().unwrap().code,
            "ASSET_NOT_FOUND"
        );
        assert_ne!(page.browser_origin(), store.path());
        project.verify_unchanged().unwrap();
        assert_eq!(saved, fs::read(store.path()).unwrap());
    }
    for version in ["2.19", "2.21"] {
        for mode in ["absent", "empty", "inactive-default"] {
            let style = if mode == "inactive-default" {
                "scope-default-shift-metadata"
            } else {
                "scope-direct"
            };
            let (temp, store, code) = setup(version, style, true);
            let owner = temp.path().join(if mode == "inactive-default" {
                "library/base/base.pen"
            } else {
                "library/ui.pen"
            });
            let mut document: Value = serde_json::from_slice(&fs::read(&owner).unwrap()).unwrap();
            if mode == "inactive-default" {
                document.as_object_mut().unwrap().remove("metadata");
                document["children"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|node| node["id"] == "page");
            } else {
                document["imports"]["empty"] = json!("empty.pen");
                let mut empty = json!({"version":version,"children":[]});
                if mode == "empty" {
                    empty["metadata"] =
                        json!({"lazypencil":{"browser_resources":{"version":1,"roots":[]}}});
                }
                write(&temp.path().join("library/empty.pen"), empty);
                let changed = code.replace("\"library\":\"base\"", "\"library\":\"empty\"");
                assert_ne!(changed, code);
                fs::write(temp.path().join("library/draw.js"), changed).unwrap();
            }
            write(&owner, document);
            closed(&store);
            let response = copy(&store, style);
            let id = response["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            closed(&store);
            let second = recopy(&temp, &store, version, id);
            closed(&second);
        }
    }
}

pub fn preserve_empty_browser_prompts() {
    for version in ["2.19", "2.21"] {
        for present in [false, true] {
            let (temp, store, _) = setup(version, "direct", true);
            let mut node = json!({"type":"browser","width":32,"height":16,"metadata":{"type":"test","label":"empty"}});
            if present {
                node["url"] = json!("");
            }
            fs::write(
                temp.path().join("library/draw.js"),
                format!("/* @schema 2.21 */ return [{node}];"),
            )
            .unwrap();
            let check = |store: &Store| {
                let (_, layout) = service::layout_snapshot_with_runtime(
                    store,
                    &[],
                    None,
                    None,
                    &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
                )
                .unwrap();
                let node = &layout
                    .items
                    .iter()
                    .find(|item| item.record.enabled && item.node.kind() == "browser")
                    .unwrap()
                    .node;
                assert_eq!(
                    node.properties.get("url"),
                    present.then(|| json!("")).as_ref()
                );
                assert_eq!(
                    node.properties["metadata"],
                    json!({"type":"test","label":"empty"})
                );
            };
            check(&store);
            let copied = copy(&store, "direct");
            let id = copied["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            check(&store);
            let second = recopy(&temp, &store, version, id);
            check(&second);
        }
    }
}

pub fn reject_uncaptured_explicit_namespace() {
    for version in ["2.19", "2.21"] {
        for style in [
            "scope-direct",
            "scope-reference",
            "scope-descendant",
            "scope-metadata",
            "scope-envelope",
        ] {
            let (temp, store, code) = setup(version, style, true);
            let response = copy(&store, style);
            let id = response["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            let document = store.read().unwrap();
            let uri = document.node(id).unwrap()["scriptUri"].as_str().unwrap();
            let alias = library(style);
            let changed = code.replace(
                &format!("\"library\":\"{alias}\""),
                "\"library\":\"consumer\"",
            );
            assert_ne!(changed, code);
            fs::write(temp.path().join(uri), changed).unwrap();
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            write(
                &temp.path().join("consumer.pen"),
                json!({"version":version,"metadata":resources("consumer-web"),"children":[]}),
            );
            apply(
                &store,
                "add-consumer-namespace",
                json!([{"op":"add_import","alias":"consumer","uri":"consumer.pen"}]),
            );
            let before = fs::read(store.path()).unwrap();
            let error = service::layout_snapshot_with_runtime(
                &store,
                &[],
                None,
                None,
                &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
            )
            .err()
            .unwrap();
            assert_eq!(
                error.code, "ASSET_NOT_FOUND",
                "{version}/{style}: {error:?}"
            );
            assert_eq!(fs::read(store.path()).unwrap(), before);
        }
    }
}
