use lazypencil::{
    config::Snapshot,
    project::{NodeAddress, Project},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    for directory in [".git", "rules", "styles/assets"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("rules/brand.txt"),
        "BRAND_PRIVATE_RULE: 日本語の本文とブランドの強調色を使う。",
    )
    .unwrap();
    fs::write(
        root.join("rules/reference.txt"),
        "Reference-only project instructions.",
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
        root.join("styles/assets/avatar.png"),
    )
    .unwrap();
    fs::write(root.join("styles/components.pen"),json!({"version":"2.21","variables":{"accent":{"type":"color","value":"#134E4A"}},"children":[
        {"id":"button","type":"frame","name":"Project button","reusable":true,"width":100,"height":40,"fill":"$accent","children":[
            {"id":"image","type":"rectangle","width":20,"height":20,"fill":{"type":"image","url":"assets/avatar.png","mode":"contain"}}
        ]}]}).to_string()).unwrap();
    fs::write(root.join("styles/brand.json"),json!({"id":"brand","name":"Project brand","description":"Project dashboard tokens","version":"2026.1","language":"en",
        "use_cases":["dashboard"],"guidance":["layout","project:brand"],"tokens":{"accent":"#134E4A"},"instructions":"Use the project color roles.","example":"components.pen"}).to_string()).unwrap();
    fs::write(root.join("lazypencil.toml"),"version=1\n[design]\nstyles=['styles/brand.json']\ndefault_style='project:brand'\n[[design.guides]]\nid='brand'\npath='rules/brand.txt'\nversion='2026.1'\nlanguage='ja'\nsummary='Brand rules'\n[[design.guides]]\nid='reference'\npath='rules/reference.txt'\nautomatic=false\n").unwrap();
    let store = Store::init(root.join("design.pen"), Some(root.join("data"))).unwrap();
    (temp, store)
}
fn trust(store: &Store) {
    Snapshot::load(store.path())
        .unwrap()
        .design_catalog()
        .unwrap()
        .set_trust(store.data_dir(), None, true)
        .unwrap();
}
fn cli(store: &Store, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(store.data_dir())
        .args(args)
        .output()
        .unwrap()
}
fn data(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn project_instructions_require_a_grant_and_share_one_catalog_and_default_style() {
    let (_temp, store) = fixture();
    let configuration = Snapshot::load(store.path()).unwrap();
    let catalog = configuration.design_catalog().unwrap();
    assert!(!catalog.trusted(store.data_dir()).unwrap());
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
    assert_eq!(
        service::project_style(&store, "project:brand")
            .unwrap_err()
            .code,
        "PROJECT_NOT_TRUSTED"
    );
    let document = service::document_metadata(&store).unwrap();
    assert_eq!(document["configuration"]["design"]["trusted"], false);
    assert!(!document.to_string().contains("BRAND_PRIVATE_RULE"));
    assert!(
        Project::load(&store)
            .unwrap()
            .resolve()
            .unwrap()
            .roots
            .is_empty()
    );
    assert_eq!(
        catalog
            .set_trust(store.data_dir(), Some("wrong"), true)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert!(!catalog.trusted(store.data_dir()).unwrap());
    trust(&store);
    let guide = service::project_guidance(&store, None).unwrap();
    assert_eq!(guide["language"], "en");
    assert_eq!(guide["project_guides"].as_array().unwrap().len(), 1);
    assert_eq!(guide["project_guides"][0]["language"], "ja");
    assert!(
        guide["project_guides"][0]["body"]
            .as_str()
            .unwrap()
            .contains("BRAND_PRIVATE_RULE")
    );
    let reference = service::project_guidance(&store, Some("project:reference")).unwrap();
    assert_eq!(reference["body"], "Reference-only project instructions.");
    let styles = service::project_styles(&store, Some("DASHBOARD"), 100, None).unwrap();
    assert_eq!(styles["styles"].as_array().unwrap().len(), 2);
    let (_, selected) = service::project_design_start(&store, None).unwrap();
    assert_eq!(selected.unwrap()["id"], "project:brand");
    let (_, selected) = service::project_design_start(&store, Some("content-page")).unwrap();
    assert_eq!(selected.unwrap()["id"], "content-page");
    assert_eq!(
        service::project_style(&store, "project:brand").unwrap()["license"],
        Value::Null
    );
}

#[test]
fn changed_sources_invalidate_grants_revisions_and_pages_before_new_instructions_are_used() {
    let (temp, store) = fixture();
    trust(&store);
    let old = Project::load(&store).unwrap();
    let page = service::project_styles(&store, None, 1, None).unwrap();
    let guide = service::project_guidance(&store, None).unwrap();
    fs::write(temp.path().join("rules/brand.txt"), "Changed project rules").unwrap();
    assert_eq!(
        old.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let new = Project::load(&store).unwrap();
    assert_ne!(old.dependency_revision(), new.dependency_revision());
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
    trust(&store);
    assert_ne!(
        guide["hash"],
        service::project_guidance(&store, None).unwrap()["hash"]
    );
    assert_eq!(
        service::project_styles(&store, None, 2, page["next_cursor"].as_str())
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let first = service::project_styles(&store, None, 1, None).unwrap();
    let next = service::project_styles(&store, None, 3, first["next_cursor"].as_str()).unwrap();
    assert_eq!(next["styles"].as_array().unwrap().len(), 3);
    assert!(next["next_cursor"].is_null());
    let configuration = temp.path().join("lazypencil.toml");
    let before_comment = service::project_guidance(&store, None).unwrap();
    fs::write(
        &configuration,
        format!(
            "{}\n# changed\n",
            fs::read_to_string(&configuration).unwrap()
        ),
    )
    .unwrap();
    let after_comment = service::project_guidance(&store, None).unwrap();
    assert_eq!(
        before_comment["project_design"]["trust_fingerprint"],
        after_comment["project_design"]["trust_fingerprint"]
    );
    assert_ne!(
        before_comment["project_design"]["fingerprint"],
        after_comment["project_design"]["fingerprint"]
    );
}

#[test]
fn example_and_font_updates_keep_instruction_trust_while_rejecting_old_data_snapshots() {
    let (temp, store) = fixture();
    trust(&store);
    let before = Project::load(&store).unwrap();
    let style = service::project_style(&store, "project:brand").unwrap();
    let page = service::project_styles(&store, None, 1, None).unwrap();
    let fingerprint = before.configuration()["design"]["trust_fingerprint"].clone();
    let example = temp.path().join("styles/components.pen");
    fs::write(
        &example,
        fs::read_to_string(&example)
            .unwrap()
            .replace("#134E4A", "#2277AA"),
    )
    .unwrap();
    assert_eq!(
        before.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let after = Project::load(&store).unwrap();
    assert_eq!(after.configuration()["design"]["trusted"], true);
    assert_eq!(
        after.configuration()["design"]["trust_fingerprint"],
        fingerprint
    );
    assert_ne!(
        service::project_style(&store, "project:brand").unwrap()["hash"],
        style["hash"]
    );
    assert_eq!(
        service::project_styles(&store, None, 2, page["next_cursor"].as_str())
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let config = temp.path().join("lazypencil.toml");
    fs::write(
        &config,
        format!(
            "{}\n[fonts]\nlocale='en'\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    assert_eq!(
        Project::load(&store).unwrap().configuration()["design"]["trust_fingerprint"],
        fingerprint
    );
    service::project_guidance(&store, None).unwrap();
    let style = temp.path().join("styles/brand.json");
    fs::write(
        &style,
        fs::read_to_string(&style).unwrap().replace(
            "Use the project color roles.",
            "Use different project rules.",
        ),
    )
    .unwrap();
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
}

#[cfg(unix)]
#[test]
fn linked_project_styles_retain_the_requested_example_alias_and_follow_its_data_updates() {
    let (temp, store) = fixture();
    let example = temp.path().join("styles/components.pen");
    let first = temp.path().join("styles/one.pen");
    let second = temp.path().join("styles/two.pen");
    fs::rename(&example, &first).unwrap();
    fs::write(
        &second,
        fs::read_to_string(&first)
            .unwrap()
            .replace("#134E4A", "#2277AA"),
    )
    .unwrap();
    std::os::unix::fs::symlink("one.pen", &example).unwrap();
    trust(&store);
    let project = Project::load(&store).unwrap();
    let request=serde_json::from_value(json!({"request_id":"linked-project-alias","expected_revision":project.root_document().revision(),"expected_dependency_revision":project.dependency_revision(),"operations":[
        {"op":"import_component","source":{"style_id":"project:brand","node_id":"button"},"mode":"link"}
    ]})).unwrap();
    service::apply_request(&store, &request).unwrap();
    assert_eq!(
        store.read().unwrap().value()["imports"]["style-project-brand"],
        "styles/components.pen"
    );
    let before = Project::load(&store).unwrap();
    fs::remove_file(&example).unwrap();
    std::os::unix::fs::symlink("two.pen", &example).unwrap();
    assert_eq!(
        before.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let after = Project::load(&store).unwrap();
    assert_eq!(after.configuration()["design"]["trusted"], true);
    assert_eq!(
        after.resolve().unwrap().roots[0].properties["fill"],
        "#2277AA"
    );
}

#[test]
fn revocation_works_with_broken_resources_and_does_not_resurrect_a_removed_nested_configuration() {
    let (temp, store) = fixture();
    trust(&store);
    let style = temp.path().join("styles/brand.json");
    let bytes = fs::read(&style).unwrap();
    fs::write(&style, b"[").unwrap();
    data(cli(
        &store,
        &["config", "untrust", store.path().to_str().unwrap()],
    ));
    fs::write(&style, bytes).unwrap();
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
    let nested = temp.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("rules.txt"), "Nested rules").unwrap();
    let config = "[design]\n[[design.guides]]\nid='nested'\npath='rules.txt'\n";
    fs::write(nested.join("lazypencil.toml"), config).unwrap();
    let nested_store =
        Store::init(nested.join("nested.pen"), Some(store.data_dir().into())).unwrap();
    trust(&nested_store);
    fs::remove_file(nested.join("lazypencil.toml")).unwrap();
    lazypencil::config::revoke_design_trust(nested_store.path(), nested_store.data_dir()).unwrap();
    fs::write(nested.join("lazypencil.toml"), config).unwrap();
    assert_eq!(
        service::project_guidance(&nested_store, None)
            .unwrap_err()
            .code,
        "PROJECT_NOT_TRUSTED"
    );
}

#[test]
fn malformed_or_unavailable_design_resources_leave_document_data_readable_without_feeding_ai_rules()
{
    let (temp, store) = fixture();
    fs::write(
        temp.path().join("styles/brand.json"),
        b"{SECRET_SHOULD_NOT_APPEAR",
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    assert!(project.resolve().unwrap().roots.is_empty());
    assert_eq!(project.configuration()["design"]["available"], false);
    let error = service::project_guidance(&store, None).unwrap_err();
    assert_eq!(error.code, "CONFIGURATION_ERROR");
    assert_eq!(
        error.details.as_ref().unwrap()["path"],
        fs::canonicalize(temp.path().join("styles/brand.json"))
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("SECRET_SHOULD_NOT_APPEAR")
    );
    let stored =
        service::read_store_nodes(&store, &[], &serde_json::from_value(json!({})).unwrap())
            .unwrap();
    assert_eq!(stored["view"], "stored");
}

#[test]
fn resource_paths_limits_and_unknown_style_guides_are_checked_without_exposing_body_text() {
    let (temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("private.txt");
    fs::write(&target, "SECRET_SHOULD_NOT_APPEAR").unwrap();
    let source = temp.path().join("lazypencil.toml");
    fs::write(
        &source,
        format!(
            "[[design.guides]]\nid='escape'\npath={}\n",
            serde_json::to_string(target.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let config = Snapshot::load(store.path()).unwrap();
    assert_eq!(
        config.design_catalog().err().unwrap().code,
        "CONFIGURATION_ERROR"
    );
    assert!(
        !config
            .summary()
            .to_string()
            .contains("SECRET_SHOULD_NOT_APPEAR")
    );
    fs::write(
        &source,
        "[[design.guides]]\nid='large'\npath='rules/brand.txt'\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("rules/brand.txt"),
        vec![b'x'; 64 * 1024 + 1],
    )
    .unwrap();
    assert_eq!(
        Snapshot::load(store.path())
            .unwrap()
            .design_catalog()
            .err()
            .unwrap()
            .code,
        "CONFIGURATION_ERROR"
    );
}

#[test]
fn actual_cli_reviews_pins_trust_and_reads_the_same_project_catalog() {
    let (_temp, store) = fixture();
    let file = store.path().to_str().unwrap();
    let shown = data(cli(&store, &["config", "show", file]));
    assert_eq!(shown["design"]["trusted"], false);
    let rejected = cli(&store, &["guidance", "--file", file]);
    assert_eq!(rejected.status.code(), Some(4));
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    data(cli(
        &store,
        &[
            "config",
            "trust",
            file,
            "--expected-fingerprint",
            shown["design"]["fingerprint"].as_str().unwrap(),
        ],
    ));
    let guide = data(cli(&store, &["guidance", "--file", file]));
    assert_eq!(
        guide["hash"],
        service::project_guidance(&store, None).unwrap()["hash"]
    );
    let style = data(cli(&store, &["style", "project:brand", "--file", file]));
    assert_eq!(style["example"]["components"][0]["node_id"], "button");
    let catalog = data(cli(
        &store,
        &["styles", "--file", file, "--use-case", "dashboard"],
    ));
    assert_eq!(catalog["styles"].as_array().unwrap().len(), 2);
    data(cli(&store, &["config", "untrust", file]));
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
}

#[test]
fn project_style_imports_keep_the_example_origin_and_use_the_bound_stores_grant() {
    for mode in ["copy", "link"] {
        let (temp, store) = fixture();
        let before = store.read().unwrap();
        let dependency = Project::load(&store).unwrap().dependency_revision();
        let request = |revision: &str| {
            serde_json::from_value(json!({"request_id":"custom-style","expected_revision":revision,"expected_dependency_revision":dependency,"operations":[
            {"op":"import_component","source":{"style_id":"project:brand","node_id":"button"},"mode":mode,"client_ref":"control"}
        ]})).unwrap()
        };
        assert_eq!(
            service::apply_request(&store, &request(&before.revision()))
                .unwrap_err()
                .code,
            "PROJECT_NOT_TRUSTED"
        );
        assert_eq!(store.read().unwrap().revision(), before.revision());
        trust(&store);
        let result = service::apply_request(&store, &request(&before.revision())).unwrap();
        let project = Project::load(&store).unwrap();
        let node = project
            .expanded_node(&NodeAddress {
                node_id: result["bindings"]["control"].as_str().unwrap().into(),
                instance_path: vec![],
            })
            .unwrap();
        assert_eq!(node.properties["fill"], "$accent");
        let scene = project.resolve().unwrap();
        let calculated = scene
            .roots
            .iter()
            .find(|node| node.address.node_id == result["bindings"]["control"].as_str().unwrap())
            .unwrap();
        assert_eq!(calculated.properties["fill"], "#134E4A");
        assert_eq!(node.children.len(), 1);
        if mode == "link" {
            assert!(
                store.read().unwrap().value()["imports"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|uri| uri.as_str().unwrap().starts_with("styles/"))
            );
        } else {
            fs::remove_file(temp.path().join("styles/components.pen")).unwrap();
            fs::remove_file(temp.path().join("styles/assets/avatar.png")).unwrap();
            let project = Project::load(&store).unwrap();
            project.resolve().unwrap();
            #[cfg(feature = "rendering")]
            {
                let (project, snapshot) =
                    service::layout_snapshot(&store, &[], None, None).unwrap();
                assert!(
                    !lazypencil::render::png(&project, &snapshot, &Default::default())
                        .unwrap()
                        .png
                        .is_empty()
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn retargeting_a_guide_symlink_with_identical_text_invalidates_the_grant_and_snapshot() {
    let (temp, store) = fixture();
    let guide = temp.path().join("rules/brand.txt");
    let text = fs::read(&guide).unwrap();
    fs::remove_file(&guide).unwrap();
    fs::write(temp.path().join("rules/one.txt"), &text).unwrap();
    fs::write(temp.path().join("rules/two.txt"), &text).unwrap();
    std::os::unix::fs::symlink("one.txt", &guide).unwrap();
    trust(&store);
    let project = Project::load(&store).unwrap();
    fs::remove_file(&guide).unwrap();
    std::os::unix::fs::symlink("two.txt", &guide).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::project_guidance(&store, None).unwrap_err().code,
        "PROJECT_NOT_TRUSTED"
    );
}
