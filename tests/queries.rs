use lazypencil::{project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn write(path: &Path, value: Value) {
    fs::write(path, value.to_string()).unwrap();
}
fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    write(
        &temp.path().join("colors.pen"),
        json!({"version":"2.21", "themes":{"mode":["light","dark"]},
        "variables":{"asset":{"type":"string","value":"images/picture.png"},"brand":{"type":"color","value":"#22CC44"}},"children":[]}),
    );
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({"version":"2.21","imports":{"base":"../colors.pen"},
        "themes":{"mode":["light","dark"],"density":["regular","compact"]},
        "variables":{
            "shared":{"type":"color","value":[{"value":"#FFFFFF"},{"theme":{"mode":"dark"},"value":"#111111"}]},
            "spacing":{"type":"number","value":[{"value":12},{"theme":{"density":"compact"},"value":6}]},
            "asset":{"type":"string","value":"$base:asset"},"brand":{"type":"color","value":"$base:brand"}},
        "children":[{"id":"tile","type":"frame","reusable":true,"width":100,"height":40,"fill":"$shared",
            "children":[{"id":"label","type":"text","width":90,"height":20,"content":"$asset","fill":"$shared"}]}]}),
    );
    let path = temp.path().join("design.pen");
    write(
        &path,
        json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},
        "themes":{"mode":["light","dark"],"density":["regular","compact"]},
        "variables":{"shared":{"type":"color","value":"#CC2233"},"gap":{"type":"number","value":"$ui:spacing"},
            "flag":{"type":"boolean","value":false},"literal_token":{"type":"string","value":"Literal"}},
        "children":[{"id":"container","type":"frame","theme":{"mode":"dark","density":"compact"},"width":100,"height":40,
            "children":[{"id":"use","type":"ref","ref":"ui:tile","fill":"$shared"}]}]}),
    );
    let store = Store::open(path, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}
fn variables(store: &Store, query: Value) -> lazypencil::Result<Value> {
    service::get_variables(store, &[], &serde_json::from_value(query).unwrap(), None)
}
fn validation(store: &Store, query: Value) -> lazypencil::Result<Value> {
    service::validate_document_with_options(
        store,
        &[],
        &serde_json::from_value(query).unwrap(),
        None,
    )
}

#[cfg(not(feature = "rendering"))]
#[test]
fn core_build_reports_calculated_reads_as_unavailable_and_preserves_stored_reads() {
    let (_temp, store) = fixture();
    assert_eq!(
        service::document_metadata(&store).unwrap()["capabilities"]["node_reads"],
        json!({"calculated":false,"generated":false,"snapshot_reuse":false})
    );
    for query in [
        json!({"calculated":true}),
        json!({"snapshot_id":"retained"}),
    ] {
        let request = serde_json::from_value(query).unwrap();
        let error = service::read_store_nodes(&store, &[], &request).unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_FEATURE");
    }
    let stored = service::read_store_nodes(
        &store,
        &[],
        &serde_json::from_value(json!({"node_id":"container"})).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["view"], "stored");
    assert_eq!(stored["nodes"][0]["id"], "container");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .arg("inspect")
        .arg(store.path())
        .arg("--calculated")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "UNSUPPORTED_FEATURE");
}

#[test]
fn variables_keep_definitions_and_alias_sources_separate_from_resolved_values() {
    let (temp, store) = fixture();
    let result = variables(&store, json!({})).unwrap();
    assert_eq!(result["variables"]["gap"]["value"], "$ui:spacing");
    assert_eq!(result["resolved_values"]["gap"], 12);
    assert_eq!(result["resolved_values"]["flag"], false);
    assert_eq!(result["resolved_values"]["literal_token"], "Literal");
    assert_eq!(
        result["sources"]["gap"]["definition"],
        store.path().to_str().unwrap()
    );
    assert!(
        result["sources"]["gap"]["value"]
            .as_str()
            .unwrap()
            .ends_with("/library/ui.lib.pen")
    );
    assert_eq!(result["resolution_complete"], true);
    let qualified = variables(&store, json!({"name":"ui:base:brand"})).unwrap();
    assert_eq!(qualified["resolved_values"]["ui:base:brand"], "#22CC44");
    assert!(
        qualified["sources"]["ui:base:brand"]["definition"]
            .as_str()
            .unwrap()
            .ends_with("/colors.pen")
    );
    let base = variables(&store, json!({"library_alias":"ui:base","name":"asset"})).unwrap();
    assert_eq!(base["resolved_values"]["asset"], "images/picture.png");
    assert!(
        !temp.path().join("images/picture.png").exists(),
        "A string query must not read image files"
    );
}

#[test]
fn node_queries_share_the_renderers_ancestor_themes_and_property_namespaces() {
    let (_temp, store) = fixture();
    let original = fs::read(store.path()).unwrap();
    let scene = Project::load(&store).unwrap().resolve().unwrap();
    let instance = &scene.roots[0].children[0];
    let label = &instance.children[0];
    let library_value = variables(&store, json!({"node_id":"use","name":"shared"})).unwrap();
    assert_eq!(library_value["resolved_values"]["shared"], "#111111");
    assert!(
        library_value["scope_document"]
            .as_str()
            .unwrap()
            .ends_with("ui.lib.pen")
    );
    assert_eq!(
        variables(&store, json!({"node_id":"use","name":"spacing"})).unwrap()["resolved_values"]["spacing"],
        6
    );
    let root_override = variables(
        &store,
        json!({"node_id":"use","property":"fill","name":"shared"}),
    )
    .unwrap();
    assert_eq!(
        root_override["resolved_values"]["shared"],
        instance.properties["fill"]
    );
    assert_eq!(
        root_override["scope_document"],
        store.path().to_str().unwrap()
    );
    let inherited = variables(
        &store,
        json!({"node_id":"use","instance_path":["label"],"property":"content","name":"asset"}),
    )
    .unwrap();
    assert_eq!(
        inherited["resolved_values"]["asset"],
        label.properties["content"]
    );
    assert!(
        inherited["sources"]["asset"]["value"]
            .as_str()
            .unwrap()
            .ends_with("colors.pen")
    );
    let dark = variables(
        &store,
        json!({"node_id":"use","instance_path":["label"],"property":"fill","name":"shared"}),
    )
    .unwrap();
    assert_eq!(dark["resolved_values"]["shared"], label.properties["fill"]);
    let hypothetical = variables(
        &store,
        json!({"node_id":"use","name":"shared","theme":{"mode":"light"}}),
    )
    .unwrap();
    assert_eq!(hypothetical["resolved_values"]["shared"], "#FFFFFF");
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn variable_pages_reject_changed_queries_and_dependencies_and_allow_new_page_sizes() {
    let (temp, store) = fixture();
    let first = variables(
        &store,
        json!({"limit":1,"expected_revision":store.read().unwrap().revision()}),
    )
    .unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let rest = variables(&store, json!({"cursor":cursor,"limit":500})).unwrap();
    assert_eq!(rest["variables"].as_object().unwrap().len(), 3);
    assert_eq!(rest["total_matches"], 4);
    assert!(rest["next_cursor"].is_null());
    assert_eq!(
        variables(&store, json!({"cursor":cursor,"theme":{"mode":"dark"}}))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let library = temp.path().join("library/ui.lib.pen");
    let mut value: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    value["variables"]["spacing"]["value"][0]["value"] = json!(18);
    write(&library, value);
    assert_eq!(
        variables(&store, json!({"cursor":cursor}))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        variables(
            &store,
            json!({"expected_dependency_revision":first["dependency_revision"]})
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        variables(&store, json!({"name":"gap"})).unwrap()["resolved_values"]["gap"],
        18
    );
}

#[test]
fn owned_replacements_and_nested_stored_instances_keep_their_actual_theme_context() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap().value().clone();
    document["children"][0]["children"][0]["descendants"] = json!({"label":{"id":"replacement","type":"text","width":90,"height":20,"content":"Changed","fill":"$shared"}});
    write(store.path(), document.clone());
    let direct = variables(
        &store,
        json!({"node_id":"replacement","property":"fill","name":"shared"}),
    )
    .unwrap();
    let inherited = variables(
        &store,
        json!({"node_id":"use","instance_path":["label"],"property":"fill","name":"shared"}),
    )
    .unwrap();
    assert_eq!(direct["effective_theme"], inherited["effective_theme"]);
    assert_eq!(direct["effective_theme"]["mode"], "dark");
    assert_eq!(direct["resolved_values"], inherited["resolved_values"]);
    assert_eq!(direct["resolved_values"]["shared"], "#CC2233");
    document["children"][0]["children"][0]["descendants"] =
        json!({"label":{"id":"placed","type":"ref","ref":"small"}});
    document["children"].as_array_mut().unwrap().push(json!({"id":"small","type":"frame","reusable":true,"width":20,"height":20,"children":[{"id":"own_label","type":"text","content":"Local","fill":"$shared"}]}));
    write(store.path(), document);
    let nested = variables(
        &store,
        json!({"node_id":"placed","instance_path":["own_label"],"name":"gap"}),
    )
    .unwrap();
    let outer = variables(
        &store,
        json!({"node_id":"use","instance_path":["label","own_label"],"name":"gap"}),
    )
    .unwrap();
    assert_eq!(nested["resolved_values"]["gap"], 6);
    assert_eq!(nested["resolved_values"], outer["resolved_values"]);
    assert_eq!(nested["effective_theme"], outer["effective_theme"]);
}

#[test]
fn an_unrelated_broken_root_does_not_prevent_inspecting_a_valid_nodes_tokens() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap().value().clone();
    document["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"broken","type":"ref","ref":"ui:absent"}));
    write(store.path(), document);
    let result = variables(&store, json!({"node_id":"use","name":"shared"})).unwrap();
    assert_eq!(result["resolved_values"]["shared"], "#111111");
    assert!(result["resolution_error"].is_object());
    assert!(result["assets_complete"].is_null());
}

#[test]
fn variable_resolution_errors_keep_raw_definitions_available_without_running_code() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap().value().clone();
    document["children"] =
        json!([{"id":"code","type":"script","width":20,"height":20,"scriptUri":"missing.js"}]);
    write(store.path(), document.clone());
    let missing = variables(&store, json!({})).unwrap();
    assert_eq!(missing["assets_complete"], false);
    assert_eq!(missing["resolved_values"]["gap"], 12);
    document["variables"]["broken"] = json!({"type":"number","value":"$ui:absent"});
    document["variables"]["cycle_a"] = json!({"type":"string","value":"$cycle_b"});
    document["variables"]["cycle_b"] = json!({"type":"string","value":"$cycle_a"});
    write(store.path(), document);
    let result = variables(&store, json!({})).unwrap();
    assert_eq!(result["variables"]["broken"]["value"], "$ui:absent");
    assert_eq!(result["resolution_complete"], false);
    assert_eq!(result["resolved_values"]["gap"], 12);
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 3);
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "REFERENCE_CYCLE")
    );
    assert!(result["resolved_values"].get("cycle_a").is_none());
    assert!(
        result["assets_complete"].is_null(),
        "Broken structural resolution prevents a complete asset scan"
    );
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn variable_queries_reject_invalid_contexts_preconditions_and_cancellation() {
    let (_temp, store) = fixture();
    for query in [
        json!({"property":"fill"}),
        json!({"instance_path":["label"]}),
        json!({"node_id":"use","property":"absent"}),
        json!({"node_id":"use","instance_path":["no/segments"]}),
        json!({"theme":{"mode":"typo"}}),
        json!({"theme":{"unknown":"dark"}}),
        json!({"library_alias":"../../arbitrary.pen"}),
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"cursor":"bad"}),
    ] {
        assert_eq!(
            variables(&store, query.clone()).unwrap_err().code,
            "INVALID_ARGUMENT",
            "{query}"
        );
    }
    assert_eq!(
        variables(&store, json!({"expected_revision":"stale"}))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        service::get_variables(
            &store,
            &[],
            &serde_json::from_value(json!({})).unwrap(),
            Some(&cancel)
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
    assert_eq!(
        service::validate_document_with_options(
            &store,
            &[],
            &service::ValidateRequest::default(),
            Some(&cancel)
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
}

#[test]
fn oversized_variable_values_fail_the_shared_response_budget_without_truncation() {
    let (_temp, store) = fixture();
    write(
        store.path(),
        json!({"version":"2.21","variables":{"large":{"type":"string","value":"x".repeat(2*1024*1024+1)}},"children":[]}),
    );
    let original = fs::read(store.path()).unwrap();
    assert_eq!(
        variables(&store, json!({"name":"large"})).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn cli_variable_queries_use_the_same_theme_context_and_raw_aliases() {
    let (_temp, store) = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["--json", "variables"])
        .arg(store.path())
        .args([
            "--node",
            "use",
            "--name",
            "spacing",
            "--theme",
            "density=regular",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["resolved_values"]["spacing"], 12);
    assert!(result["variables"]["spacing"]["value"].is_array());
    let duplicate = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "variables"])
        .arg(store.path())
        .args(["--theme", "mode=dark", "--theme", "mode=light"])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(2));
}

#[test]
fn structural_validation_keeps_its_scope_and_checks_read_preconditions() {
    let (_temp, store) = fixture();
    let result = validation(&store, json!({})).unwrap();
    assert_eq!(result["valid"], true);
    assert_eq!(result["complete"], true);
    assert!(
        !result["validated_scopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "layout")
    );
    assert!(result.get("snapshot_id").is_none());
    assert_eq!(
        validation(&store, json!({"expected_revision":"old"}))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        validation(&store, json!({"expected_dependency_revision":"old"}))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    for query in [
        json!({"limit":0}),
        json!({"limit":501}),
        json!({"snapshot_id":"not-structural"}),
        json!({"cursor":"bad"}),
    ] {
        assert_eq!(
            validation(&store, query).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
    }
}

#[cfg(not(feature = "rendering"))]
#[test]
fn layout_validation_explicitly_rejects_builds_without_rendering() {
    let (_temp, store) = fixture();
    assert_eq!(
        validation(&store, json!({"scope":"layout"}))
            .unwrap_err()
            .code,
        "UNSUPPORTED_FEATURE"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "validate"])
        .arg(store.path())
        .args(["--scope", "layout"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
}

#[cfg(feature = "rendering")]
#[test]
fn layout_validation_pages_real_diagnostics_and_uses_the_render_calculation() {
    let (_temp, store) = fixture();
    write(
        store.path(),
        json!({"version":"2.21","children":[
        {"id":"frame","type":"frame","layout":"none","width":40,"height":20,"children":[
            {"id":"text","type":"text","width":20,"height":5,"fontSize":16,"textGrowth":"fixed-width","content":"日本語の長い文章を折り返す"},
            {"id":"box","type":"rectangle","x":50,"width":20,"height":20}]}]}),
    );
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let first = service::validation_from_snapshot(
        &project,
        &snapshot,
        &serde_json::from_value(json!({"scope":"layout","limit":1})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        first["valid"], true,
        "Warnings must not become structural errors: {first}"
    );
    assert!(first["warning_count"].as_u64().unwrap() >= 2);
    assert_eq!(first["diagnostics"].as_array().unwrap().len(), 1);
    let remaining = service::validation_from_snapshot(
        &project,
        &snapshot,
        &serde_json::from_value(
            json!({"scope":"layout","limit":500,"cursor":first["next_cursor"]}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(remaining["next_cursor"].is_null());
    assert_eq!(remaining["total_diagnostics"], first["total_diagnostics"]);
    assert_eq!(remaining["error_count"], 0);
    assert_eq!(first["snapshot_id"], snapshot.id);
    let computed = validation(&store, json!({"scope":"layout"})).unwrap();
    assert_eq!(computed["snapshot_id"], snapshot.id);
    let png = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
    assert!(!png.png.is_empty());
    assert_eq!(
        service::validation_from_snapshot(
            &project,
            &snapshot,
            &serde_json::from_value(json!({"scope":"layout","snapshot_id":"wrong"})).unwrap()
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
    let before = fs::read(store.path()).unwrap();
    assert!(store.history(10, None).unwrap().is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "validate"])
        .arg(store.path())
        .args(["--scope", "layout", "--limit", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["snapshot_id"],
        snapshot.id
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[cfg(feature = "rendering")]
#[test]
fn layout_errors_are_reported_as_invalid_and_cli_returns_a_failure_exit_code() {
    let (_temp, store) = fixture();
    write(
        store.path(),
        json!({"version":"2.21","children":[
            {"id":"frame","type":"frame","width":10,"height":10,"children":[{"id":"outside","type":"rectangle","width":20,"height":20}]},
            {"id":"missing-icon","type":"icon","library":"lucide","icon":"does-not-exist","x":40,"width":20,"height":20}]}),
    );
    let result = validation(&store, json!({"scope":"layout"})).unwrap();
    assert_eq!(result["valid"], false);
    assert_eq!(result["error_count"], 1);
    assert_eq!(result["complete"], true);
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["node_id"] == "missing-icon")
    );
    let first = validation(&store, json!({"scope":"layout","limit":1})).unwrap();
    assert_eq!(first["diagnostics"][0]["severity"], "warning");
    assert_eq!(
        first["valid"], false,
        "Errors on a later page must still invalidate this calculation"
    );
    assert_eq!(first["error_count"], 1);
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "validate"])
        .arg(store.path())
        .args(["--scope", "layout"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let failure: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["error"]["details"]["valid"], false, "{failure}");
}

#[cfg(feature = "rendering")]
#[test]
fn large_layout_diagnostic_sets_remain_paged_without_losing_totals() {
    let (_temp, store) = fixture();
    let children=(0..1001).map(|index|json!({"id":format!("child-{index}"),"type":"rectangle","x":2,"width":2,"height":2})).collect::<Vec<_>>();
    write(
        store.path(),
        json!({"version":"2.21","children":[{"id":"frame","type":"frame","layout":"none","width":1,"height":1,"children":children}]}),
    );
    let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let first = service::validation_from_snapshot(
        &project,
        &snapshot,
        &serde_json::from_value(json!({"scope":"layout","limit":7})).unwrap(),
    )
    .unwrap();
    assert_eq!(first["total_diagnostics"], 1001);
    assert_eq!(first["warning_count"], 1001);
    assert_eq!(first["diagnostics"].as_array().unwrap().len(), 7);
    assert!(first.to_string().len() < 16000);
    let next = service::validation_from_snapshot(
        &project,
        &snapshot,
        &serde_json::from_value(
            json!({"scope":"layout","cursor":first["next_cursor"],"limit":500}),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(next["diagnostics"].as_array().unwrap().len(), 500);
    assert_eq!(next["diagnostics"][0]["node_id"], "child-7");
    let mut changed = store.read().unwrap();
    changed
        .set_property("/children/0", "width", &json!(10))
        .unwrap();
    fs::write(store.path(), changed.text()).unwrap();
    assert_eq!(
        validation(
            &store,
            json!({"scope":"layout","cursor":first["next_cursor"]})
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
}
