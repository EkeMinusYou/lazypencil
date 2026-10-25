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
pub const STYLES: [&str; 7] = [
    "direct",
    "reference",
    "descendant",
    "replacement",
    "children",
    "inherited",
    "nested",
];

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
    let changes =
        json!({"url":url,"metadata":{"type":"example","label":"generated","literal":"$unchanged"}});
    let direct = || json!({"type":"browser","width":32,"height":16,"url":url,"metadata":{"type":"example","label":"generated","literal":"$unchanged"}});
    let reference =
        || json!({"type":"ref","ref":"base:page","url":url,"metadata":changes["metadata"]});
    let template = match style {
        "direct" | "nested" => direct(),
        "reference" => reference(),
        "descendant" => json!({"type":"ref","ref":"base:panel","descendants":{"inside":changes}}),
        "replacement" => json!({"type":"ref","ref":"base:panel","descendants":{"inside":direct()}}),
        "children" => json!({"type":"ref","ref":"base:panel","children":[reference()]}),
        "inherited" => json!({"type":"ref","ref":"base:page"}),
        _ => panic!("Unknown generated browser style"),
    };
    let code = if style == "nested" {
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
    if style == "nested" {
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
        .filter(|item| item.enabled && item.node.kind() == "browser")
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
        rgba: if style == "inherited" {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        },
    }
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
            .find(|item| item.enabled && item.node.kind() == "browser")
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
