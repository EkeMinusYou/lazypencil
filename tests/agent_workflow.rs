#![cfg(feature = "rendering")]

use lazypencil::store::{Store, create_private_directory};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[test]
#[ignore = "Sends a real subscription-backed image prompt to an explicitly installed Codex ACP adapter"]
fn codex_creates_and_inspects_a_design_from_an_image() {
    workflow(
        "codex",
        "LAZYPENCIL_TEST_CODEX_ACP",
        "codex-acp",
        "read-only",
    );
}

#[test]
#[ignore = "Sends a real subscription-backed image prompt to an explicitly installed Claude ACP adapter"]
fn claude_creates_and_inspects_a_design_from_an_image() {
    workflow(
        "claude",
        "LAZYPENCIL_TEST_CLAUDE_ACP",
        "claude-agent-acp",
        "default",
    );
}

fn workflow(agent: &str, variable: &str, default: &str, mode: &str) {
    let base = std::env::var_os("LAZYPENCIL_TEST_WORKFLOW_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lazypencil-agent-workflows"));
    let directory = base.join(format!("{agent}-{}", uuid::Uuid::new_v4()));
    create_private_directory(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let file = directory.join("design.pen");
    let data = directory.join("data");
    let events_path = directory.join("events.jsonl");
    let adapter = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into());
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png");
    let request = "日本語の設定画面を、この文書に新規作成してください。640×360の白いフレーム、見出しの本文は正確に『設定』、説明文、保存ボタンを作ります。保存ボタンは get_style で standard-ui の button を確認し、import_component の mode:copy で取り込んだインスタンスを使ってください。手で同じ部品を再作成しないでください。コピーした定義のラベルとアクセント色は必要に応じて編集してください。Noto Sans JP を使ってください。list_icons と icons ガイドで名前と形式を確かめ、Lucide の type:icon ノードを少なくとも2個、明示した寸法と塗りで配置してください。Unicode の記号や絵文字で代用しないでください。添付画像を目で確認し、左側の色をボタンのアクセントに使い、画像自体も登録済み attachment_id から put_asset で取り込んで小さな参考サムネイルとして配置してください。get_document と get_guidance を読み、apply_operations で保存し、最新の document/dependency revision で get_layout と render を呼び、返った PNG を目で確認して必要なら直してください。最後に画像の左右の主な色と実際に確認した結果を短く説明してください。使うのは文書に束縛された lazypencil MCP ツールだけです。シェル、ファイル編集ツール、外部サービス、サブエージェント、委譲は使わないでください。";
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "prompt"])
        .arg(&file)
        .args([
            "--create",
            "--agent",
            agent,
            "--auth",
            "subscription",
            "--require-auth-status",
            "--non-interactive",
            "--mode",
            mode,
            "--timeout",
            "180",
            "--adapter",
        ])
        .arg(adapter)
        .arg("--image")
        .arg(reference)
        .arg(request)
        .stdout(File::create(&events_path).unwrap())
        .stderr(File::create(directory.join("stderr.log")).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(250);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "Agent workflow exceeded its deadline: {}",
                directory.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let events: Vec<Value> = fs::read_to_string(&events_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let tool_names = tool_names(&events);
    let failure = events
        .iter()
        .rev()
        .find(|event| event["type"] == "error" || event["type"] == "failed");
    assert!(
        status.success(),
        "{agent} failed: status={status}, error={}, tools={tool_names:?}, artifacts={}",
        failure
            .map(|event| event["error"].to_string())
            .unwrap_or_default(),
        directory.display()
    );
    let complete = events
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    assert_eq!(complete["authentication"]["source"], "subscription");
    assert_eq!(complete["authentication"]["evidence"], "adapter_report");
    for required in [
        "get_document",
        "get_guidance",
        "get_style",
        "list_icons",
        "put_asset",
        "apply_operations",
        "get_layout",
        "render",
    ] {
        assert!(
            tool_names.contains(required),
            "Missing {required}; tools={tool_names:?}, artifacts={}",
            directory.display()
        );
    }
    assert!(
        !events
            .iter()
            .any(|event| event["type"] == "permission_response"
                && event["document_tool_preauthorized"] == false),
        "Unexpected non-document permission, artifacts={}",
        directory.display()
    );
    let store = Store::open(&file, Some(data.clone())).unwrap();
    let (icons, copied_components) = verify_generated_design(&store, &directory);
    let png = directory.join("generated.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(data)
        .args(["--json", "render"])
        .arg(&file)
        .arg("--output")
        .arg(&png)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(fs::read(&png).unwrap().starts_with(b"\x89PNG"));
    let ready = events
        .iter()
        .find(|event| event["type"] == "ready")
        .unwrap();
    let report = json!({"agent":agent,"authentication":complete["authentication"],"mode":ready["response"]["modes"]["currentModeId"],
        "tools":tool_names,"icons":icons,"copied_components":copied_components,"revision":store.read().unwrap().revision(),"png":png,"events":events_path});
    fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("{report}");
}

fn verify_generated_design(store: &Store, directory: &Path) -> (usize, usize) {
    assert!(contains_setting_title(store.read().unwrap().value()));
    let history = store.history(50, None).unwrap();
    assert!(!history.is_empty());
    let style_revision =
        lazypencil::guidance::get_style("standard-ui").unwrap()["example"]["hash"].clone();
    let imported = history
        .iter()
        .flat_map(|entry| {
            entry.result["generated_ids"]
                .as_object()
                .into_iter()
                .flat_map(serde_json::Map::values)
        })
        .filter(|value| value["mode"] == "copy" && value["source_revision"] == style_revision)
        .filter_map(|value| value["component_id"].as_str())
        .collect::<BTreeSet<_>>();
    let document = store.read().unwrap();
    assert!(
        document
            .locations()
            .keys()
            .any(
                |id| document.node(id).is_ok_and(|node| node["type"] == "ref"
                    && node["ref"]
                        .as_str()
                        .is_some_and(|target| imported.contains(target)))
            ),
        "Expected a remaining instance of a copied component; artifacts={}",
        directory.display()
    );
    let scene = lazypencil::project::Project::load(store)
        .unwrap()
        .resolve()
        .unwrap();
    fn count_icons(node: &lazypencil::project::ResolvedNode) -> usize {
        if node.properties.get("enabled") == Some(&Value::Bool(false)) {
            return 0;
        }
        usize::from(node.kind() == "icon" && node.icon.as_ref().is_some_and(Result::is_ok))
            + node.children.iter().map(count_icons).sum::<usize>()
    }
    let icons = scene.roots.iter().map(count_icons).sum::<usize>();
    fn uses_copied_component(
        node: &lazypencil::project::ResolvedNode,
        imported: &BTreeSet<&str>,
    ) -> bool {
        if node.properties.get("enabled") == Some(&Value::Bool(false)) {
            return false;
        }
        (node.is_instance && imported.contains(node.source_id.as_str()))
            || node
                .children
                .iter()
                .any(|child| uses_copied_component(child, imported))
    }
    assert!(
        scene
            .roots
            .iter()
            .any(|node| uses_copied_component(node, &imported)),
        "Expected a visible copied instance; artifacts={}",
        directory.display()
    );
    assert!(
        icons >= 2,
        "Expected actual resolved icons; artifacts={}",
        directory.display()
    );
    (icons, imported.len())
}

fn contains_setting_title(value: &Value) -> bool {
    value["content"] == "設定"
        || match value {
            Value::Object(object) => object.values().any(contains_setting_title),
            Value::Array(array) => array.iter().any(contains_setting_title),
            _ => false,
        }
}
fn tool_names(events: &[Value]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for event in events {
        let update = &event["notification"]["update"];
        if let Some(name) = update["name"]
            .as_str()
            .and_then(|name| name.strip_prefix("mcp__lazypencil__"))
        {
            names.insert(name.into());
        }
        if update["_meta"]["is_mcp_tool_call"] == true
            && update["rawInput"]["server"] == "lazypencil"
            && let Some(name) = update["rawInput"]["tool"].as_str()
        {
            names.insert(name.into());
        }
    }
    names
}

#[test]
#[ignore = "Revalidates explicitly selected saved agent artifacts without sending a model request"]
fn saved_agent_artifacts_keep_visible_copied_components_and_identical_pixels() {
    let directories: Vec<PathBuf> = serde_json::from_str(
        &std::env::var("LAZYPENCIL_TEST_SAVED_WORKFLOW_DIRS")
            .expect("Set a JSON array of saved workflow directories"),
    )
    .unwrap();
    assert!(!directories.is_empty());
    for directory in directories {
        let store =
            Store::open(directory.join("design.pen"), Some(directory.join("data"))).unwrap();
        let (icons, copied_components) = verify_generated_design(&store, &directory);
        let (project, snapshot) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
        assert_eq!(
            rendered.png,
            fs::read(directory.join("generated.png")).unwrap(),
            "{}",
            directory.display()
        );
        fs::write(directory.join("revalidated.png"), &rendered.png).unwrap();
        fs::write(directory.join("offline-verification.json"),serde_json::to_vec_pretty(&json!({"visible_icons":icons,"copied_components":copied_components,"revision":store.read().unwrap().revision(),"png_hash":lazypencil::document::hash(&rendered.png)})).unwrap()).unwrap();
    }
}
