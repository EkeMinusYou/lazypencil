// Shared core and real-browser inputs. Individual targets use different roles.
#![allow(dead_code)]

use lazypencil::{
    cdp::LocalFiles,
    operations::EditRequest,
    project::{Project, ResolvedNode},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};
use tempfile::TempDir;

pub const BASE: &str = "file:///lazypencil/original-site/";
pub const MODES: [&str; 8] = [
    "inherited",
    "literal",
    "query",
    "metadata",
    "combined",
    "descendant-url",
    "descendant-metadata",
    "replacement",
];

pub fn options(prefix: &str, themes: &[&str]) -> Value {
    let themes = themes
        .iter()
        .map(|source| {
            (
                (*source).to_owned(),
                json!({"action":"reuse","target":"mode"}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    json!({"variable_prefix":prefix,"themes":themes})
}

pub fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

pub fn apply(store: &Store, id: &str, operations: Value) -> Value {
    let project = Project::load(store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({"request_id":id,
        "expected_revision":project.root_document().revision(), "expected_dependency_revision":project.dependency_revision(),
        "operations":operations})).unwrap();
    service::apply_request(store, &request)
        .unwrap_or_else(|error| panic!("request {id}: {error:?}"))
}

fn resources(path: &str) -> Value {
    json!({"lazypencil":{"browser_resources":{"version":1,"roots":[{"url":BASE,"path":path}]}}})
}

fn site(path: &Path, light: &str, dark: &str) {
    fs::create_dir_all(path).unwrap();
    for (name, stylesheet) in [("index.html", "light.css"), ("alternate.html", "dark.css")] {
        fs::write(path.join(name), format!("<!doctype html><link rel='icon' href='data:,'><link rel='stylesheet' href='{stylesheet}'><body></body>")).unwrap();
    }
    fs::write(
        path.join("light.css"),
        format!("html,body{{margin:0;background:{light}}}"),
    )
    .unwrap();
    fs::write(
        path.join("dark.css"),
        format!("html,body{{margin:0;background:{dark}}}"),
    )
    .unwrap();
}

pub fn fixture(version: &str, mode: &str) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    site(&temp.path().join("library/base/web"), "#ff0000", "#0000ff");
    site(&temp.path().join("library/web"), "#00ff00", "#ffff00");
    site(&temp.path().join("consumer-web"), "#ff00ff", "#00ffff");
    let variable = |light: u32, dark: u32| json!({"type":"number","value":[{"value":light},{"theme":{"mode":"dark"},"value":dark}]});
    let page = |id: &str| json!({"id":id,"type":"browser","width":"$base_width","height":16,"url":format!("{BASE}index.html"),"metadata":{"type":"example","label":"base"}});
    let mut reusable = page("page");
    reusable["reusable"] = json!(true);
    write(
        &temp.path().join("library/base/base.lib.pen"),
        &json!({"version":version,
        "metadata":resources("web"),"themes":{"mode":["light","dark"]},"variables":{"base_width":variable(32,40)},
        "children":[reusable,{"id":"panel","type":"frame","reusable":true,"width":32,"height":16,"children":[page("inside")]}]}),
    );
    let mut inner = json!({"id":"inner","type":"ref","ref":"base:page"});
    match mode {
        "inherited" => {}
        "literal" => inner["url"] = json!(format!("{BASE}index.html")),
        "query" => {
            inner["url"] = json!(url(mode));
            inner["width"] = json!("$override_width");
        }
        "metadata" => inner["metadata"] = json!({"type":"example","label":"override"}),
        "combined" => {
            inner["url"] = json!(url(mode));
            inner["width"] = json!("$override_width");
            inner["metadata"] = json!({"type":"example","label":"override"});
        }
        "descendant-url" => {
            inner["ref"] = json!("base:panel");
            inner["descendants"] = json!({"inside":{"url":url(mode),"width":"$override_width"}});
        }
        "descendant-metadata" => {
            inner["ref"] = json!("base:panel");
            inner["descendants"] =
                json!({"inside":{"metadata":{"type":"example","label":"override"}}});
        }
        "replacement" => {
            inner["ref"] = json!("base:panel");
            let mut replacement = page("replacement");
            replacement["url"] = json!(url(mode));
            replacement["width"] = json!("$override_width");
            inner["descendants"] = json!({"inside":replacement});
        }
        _ => panic!("unknown browser copy case"),
    }
    write(
        &temp.path().join("library/ui.lib.pen"),
        &json!({"version":version,"imports":{"base":"base/base.lib.pen"},
        "metadata":resources("web"),"themes":{"mode":["light","dark"]},"variables":{"override_width":variable(48,56)},
        "children":[{"id":"widget","type":"frame","reusable":true,"width":32,"height":16,"children":[inner]}]}),
    );
    write(
        &temp.path().join("main.pen"),
        &json!({"version":version,"imports":{"ui":"library/ui.lib.pen"},
        "metadata":resources("consumer-web"),"themes":{"mode":["light","dark"]},
        "children":[{"id":"linked","type":"ref","ref":"ui:widget"}]}),
    );
    let store = Store::open(
        temp.path().join("main.pen"),
        Some(temp.path().join("private")),
    )
    .unwrap();
    (temp, store)
}

pub fn active(project: &Project, id: &str) -> ResolvedNode {
    fn find(node: ResolvedNode) -> Option<ResolvedNode> {
        if node.kind() == "browser" {
            return Some(node);
        }
        node.children.into_iter().find_map(find)
    }
    let root = project
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.id() == id)
        .unwrap();
    find(root).unwrap()
}

pub struct Frame {
    pub stage: &'static str,
    pub theme: &'static str,
    pub url: String,
    pub revision: String,
    pub files: Arc<LocalFiles>,
    pub rgba: [u8; 4],
    pub width: u32,
}

pub fn url(mode: &str) -> String {
    let name = match mode {
        "combined" | "descendant-url" => "alternate.html",
        "query" => "index.html?theme=$mode#original-location",
        _ => "index.html",
    };
    format!("{BASE}{name}")
}

pub fn width(mode: &str, theme: &str) -> u32 {
    match (
        matches!(
            mode,
            "query" | "combined" | "descendant-url" | "replacement"
        ),
        theme == "dark",
    ) {
        (false, false) => 32,
        (false, true) => 40,
        (true, false) => 48,
        (true, true) => 56,
    }
}

pub fn color(mode: &str, _theme: &str) -> [u8; 4] {
    match mode {
        "combined" | "descendant-url" => [255, 255, 0, 255],
        "literal" | "query" | "replacement" => [0, 255, 0, 255],
        _ => [255, 0, 0, 255],
    }
}

pub fn capture(store: &Store, id: &str, stage: &'static str, mode: &str) -> Vec<Frame> {
    let mut frames = Vec::new();
    for theme in ["light", "dark"] {
        apply(
            store,
            &format!("{stage}-{theme}"),
            json!([{"op":"set_theme","target":id,"set":{"mode":theme}}]),
        );
        let project = Project::load(store).unwrap();
        let node = active(&project, id);
        let input = project.browser_node_resources(&node).unwrap();
        project.verify_unchanged().unwrap();
        assert_eq!(node.properties["url"], url(mode));
        assert_eq!(node.properties["width"], width(mode, theme));
        frames.push(Frame {
            stage,
            theme,
            url: node.properties["url"].as_str().unwrap().into(),
            revision: input.revision,
            files: input.files,
            rgba: color(mode, theme),
            width: node.properties["width"].as_f64().unwrap() as u32,
        });
    }
    frames
}

pub fn study(version: &str, mode: &str, detach_first: bool) -> (TempDir, Vec<Frame>) {
    eprintln!("Browser copy case: version={version}, mode={mode}, detach_first={detach_first}");
    let (temp, store) = fixture(version, mode);
    let mut frames = capture(&store, "linked", "before", mode);
    let source = if detach_first {
        apply(
            &store,
            "detach-before-copy",
            json!([{"op":"detach_instance","target":"linked"},{"op":"update","target":"linked","set":{"reusable":true}}]),
        );
        json!({"node_id":"linked"})
    } else {
        json!({"library_alias":"ui","node_id":"widget"})
    };
    let themes = if detach_first {
        if matches!(
            mode,
            "query" | "combined" | "descendant-url" | "replacement"
        ) {
            vec!["ui:mode", "ui:base:mode"]
        } else {
            vec!["ui:base:mode"]
        }
    } else {
        vec!["ui:mode", "ui:base:mode"]
    };
    let result = apply(
        &store,
        "copy-web",
        json!([
            {"op":"import_component","mode":"copy","source":source,"options":options("first.",&themes)},
            {"op":"delete","target":"linked"},{"op":"remove_import","alias":"ui"}
        ]),
    );
    let id = result["generated_ids"]["0"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let component = result["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    frames.extend(capture(&store, &id, "copied", mode));
    assert_eq!(store.read().unwrap().version(), version);
    let second_dir = temp.path().join("second");
    fs::create_dir(&second_dir).unwrap();
    site(&second_dir.join("web"), "#ffffff", "#000000");
    write(
        &second_dir.join("scene.pen"),
        &json!({"version":version,"imports":{"src":"../main.pen"},
        "metadata":resources("web"),"themes":{"mode":["light","dark"]},"children":[]}),
    );
    let second = Store::open(
        second_dir.join("scene.pen"),
        Some(temp.path().join("second-private")),
    )
    .unwrap();
    let result = apply(
        &second,
        "recopy-web",
        json!([
            {"op":"import_component","mode":"copy","source":{"library_alias":"src","node_id":component},"options":options("second.",&["src:mode"])},
            {"op":"remove_import","alias":"src"}
        ]),
    );
    let second_id = result["generated_ids"]["0"]["id"].as_str().unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    fs::remove_file(store.path()).unwrap();
    fs::remove_dir_all(temp.path().join("consumer-web")).unwrap();
    frames.extend(capture(&second, second_id, "recopied", mode));
    for dependency in Project::load(&second).unwrap().dependencies() {
        assert!(
            dependency.path.starts_with(second.path().parent().unwrap()),
            "Dependency escaped the canonical copied project: {}",
            dependency.path.display()
        );
    }
    apply(
        &second,
        "detach-after-recopy",
        json!([{"op":"detach_instance","target":second_id}]),
    );
    frames.extend(capture(&second, second_id, "detached", mode));
    assert_eq!(second.read().unwrap().version(), version);
    for frame in &frames {
        assert_eq!(frame.revision, frames[0].revision);
    }
    (temp, frames)
}
