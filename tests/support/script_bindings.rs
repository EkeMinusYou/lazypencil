use super::*;
use lazypencil::document::hash;

#[path = "script_generated_overrides.rs"]
mod generated_sources;
#[path = "script_override_render.rs"]
mod overrides;

fn bindings_fixture() -> (TempDir, Store, Vec<u8>) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("original")).unwrap();
    fs::create_dir(temp.path().join("assets")).unwrap();
    let code = br##"/**
 * @schema 2.21
 * @input selected: ref = "old-badge"
 */
return [
 {id:"literal",type:"ref",ref:"old-badge",descendants:{"old-label":{content:"$old.caption"}}},
 {id:"input",type:"ref",ref:pencil.input.selected,x:20},
 {id:"variable",type:"rectangle",x:40,width:10,height:10,fill:"$old.tone",name:"$old.tone",metadata:{type:"example",keep:"$old.tone"}},
 {id:"group",type:"frame",x:60,width:10,height:10,layout:"none",theme:{"old.mode":"dark"},children:[
  {id:"nested",type:"rectangle",width:10,height:10,fill:"$old.tone"}
 ]}
];"##.to_vec();
    fs::write(temp.path().join("original/draw.js"), &code).unwrap();
    fs::copy(
        temp.path().join("original/draw.js"),
        temp.path().join("assets/draw.js"),
    )
    .unwrap();
    let inputs = json!({"selected":"old-badge"});
    write(
        &temp.path().join("main.pen"),
        json!({"version":"2.21",
        "themes":{"old.mode":["light","dark"],"copied.mode":["light","dark"]},
        "variables":{
            "old.tone":{"type":"color","value":"#00ff00"},
            "old.caption":{"type":"string","value":"Consumer"},
            "copied.tone":{"type":"color","value":[{"value":"#ff0000"},{"theme":{"copied.mode":"dark"},"value":"#0000ff"}]},
            "wrong.tone":{"type":"color","value":"#ff00ff"},
            "copied.caption":{"type":"string","value":"Copied label"}},
        "children":[{"id":"definitions","type":"frame","enabled":false,"width":0,"height":0,"children":[
            {"id":"old-badge","type":"frame","reusable":true,"layout":"none","width":10,"height":10,"fill":"#00ff00","children":[
                {"id":"old-label","type":"text","enabled":false,"content":"Old label","fontFamily":"Noto Sans JP","fontSize":10}]},
            {"id":"copied-badge","type":"frame","reusable":true,"layout":"none","width":10,"height":10,"fill":"#0000ff","children":[
                {"id":"copied-label","type":"text","enabled":false,"content":"Original label","fontFamily":"Noto Sans JP","fontSize":10}]},
            {"id":"generator","type":"script","reusable":true,"width":80,"height":20,"scriptUri":"assets/draw.js","inputs":inputs,
             "metadata":{"type":"example","keep":[1,2],"lazypencil":{"script_bindings":{
                 "version":1,"script_uri":"assets/draw.js","inputs_hash":hash(inputs.to_string().as_bytes()),
                 "components":{"old-badge":"copied-badge"},
                 "variables":{"old.tone":"copied.tone","copied.tone":"wrong.tone","old.caption":"copied.caption"},
                 "themes":{"old.mode":"copied.mode"},"descendants":{"old-badge":{"old-label":"copied-label"}}
             }}}
            }],"layout":"none"},
            {"id":"use","type":"ref","ref":"generator"}
        ]}),
    );
    fs::remove_dir_all(temp.path().join("original")).unwrap();
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store, code)
}

fn pixel(png: &[u8], x: i32) -> Color {
    Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
        .peek_pixels()
        .unwrap()
        .get_color((x, 5))
}

#[test]
fn renamed_bindings_preserve_real_script_defaults_overrides_themes_and_live_edits_without_a_source_library()
 {
    let (temp, store, code) = bindings_fixture();
    let original_document = fs::read(store.path()).unwrap();
    let (project, calculation, initial) = snapshot(&store);
    assert_eq!(fs::read(store.path()).unwrap(), original_document);
    assert!(
        !project
            .dependencies()
            .iter()
            .any(|entry| entry.kind == "document")
    );
    assert_eq!(calculation.script_executions.len(), 1);
    let revision = calculation.script_executions[0]["bindings_revision"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(revision.len(), 64);
    for (x, color) in [
        (5, Color::BLUE),
        (25, Color::BLUE),
        (45, Color::RED),
        (65, Color::BLUE),
    ] {
        assert_eq!(pixel(&initial, x), color, "initial x={x}");
    }
    assert!(
        calculation
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("Copied label")))
    );
    let literal = calculation
        .items
        .iter()
        .find(|item| item.node.properties.get("name") == Some(&json!("$old.tone")))
        .unwrap();
    assert_eq!(literal.node.properties["metadata"]["keep"], "$old.tone");

    // An explicit same-JSON input belongs to the caller, while source output
    // literals still use the renamed definition's binding map.
    edit(
        &store,
        "caller-input",
        json!([{"op":"update","target":"use","set":{"inputs":{"selected":"old-badge"}}}]),
    );
    let (_, overridden, explicit) = snapshot(&store);
    assert_eq!(pixel(&explicit, 5), Color::BLUE);
    assert_eq!(pixel(&explicit, 25), Color::GREEN);
    assert_ne!(
        overridden.script_executions[0]["bindings_revision"],
        revision
    );

    edit(
        &store,
        "null-input",
        json!([{"op":"update","target":"use","set":{"inputs":{"selected":null}}}]),
    );
    let (_, _, reset) = snapshot(&store);
    assert_eq!(pixel(&reset, 25), Color::BLUE);

    edit(
        &store,
        "edit-copied-variable",
        json!([{"op":"set_variable","name":"copied.tone","definition":{"type":"color","value":"#ffff00"}}]),
    );
    let (_, _, changed) = snapshot(&store);
    assert_eq!(pixel(&changed, 45), Color::YELLOW);
    assert_eq!(pixel(&changed, 65), Color::YELLOW);

    // Even a same-value source override opts back into the caller's namespace.
    edit(
        &store,
        "caller-source",
        json!([{"op":"update","target":"use","set":{"scriptUri":"assets/draw.js"}}]),
    );
    let (_, plain, caller) = snapshot(&store);
    assert!(plain.script_executions[0]["bindings_revision"].is_null());
    for x in [5, 25, 45, 65] {
        assert_eq!(pixel(&caller, x), Color::GREEN);
    }

    assert_eq!(fs::read(temp.path().join("assets/draw.js")).unwrap(), code);
    assert!(!temp.path().join("original").exists());
    {
        let (_temp, local, _) = bindings_fixture();
        let (_, _, before_detach) = snapshot(&local);
        edit(
            &local,
            "local-bound-detach",
            json!([{"op":"detach_instance","target":"use"}]),
        );
        let (_, _, after_detach) = snapshot(&local);
        assert_eq!(before_detach, after_detach);
        let document = local.read().unwrap();
        assert_eq!(document.node("use").unwrap()["type"], "script");
        assert!(
            document.node("use").unwrap()["metadata"]["lazypencil"]["script_bindings"]
                .get("library")
                .is_none()
        );
    }
    {
        let (_temp, revoked, _) = bindings_fixture();
        edit(
            &revoked,
            "revoke-before-detach",
            json!([{"op":"update","target":"use","set":{"scriptUri":"assets/draw.js"}}]),
        );
        let (_, _, before_detach) = snapshot(&revoked);
        edit(
            &revoked,
            "detach-revoked",
            json!([{"op":"detach_instance","target":"use"}]),
        );
        let (_, result, after_detach) = snapshot(&revoked);
        assert_eq!(before_detach, after_detach);
        assert!(result.script_executions[0]["bindings_revision"].is_null());
        assert!(
            revoked.read().unwrap().node("use").unwrap()["metadata"]["lazypencil"]
                .get("script_bindings")
                .is_none()
        );
    }
    if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let path = std::path::PathBuf::from(root).join("script-bindings");
        fs::create_dir_all(&path).unwrap();
        for (name, png) in [
            ("initial", &initial),
            ("explicit-input", &explicit),
            ("reset-input", &reset),
            ("changed-variable", &changed),
            ("caller-source", &caller),
        ] {
            fs::write(path.join(format!("{name}.png")), png).unwrap();
        }
        fs::write(path.join("report.json"), json!({"source_commit":std::env::var("GITHUB_SHA").ok(),
            "original_source_removed":true,"document_dependency_count":0,"script_bytes_sha256":hash(&code),
            "bindings_revision":revision,"png_hashes":{"initial":hash(&initial),"explicit-input":hash(&explicit),
            "reset-input":hash(&reset),"changed-variable":hash(&changed),"caller-source":hash(&caller)},
            "explicit_input_uses_caller":true,"same_value_source_override_revokes_bindings":true,
            "live_variable_and_generated_theme_edits":true,"literal_metadata_preserved":true,
            "scope":"Real worker and reference remapping prerequisite; import copy emission and complete asset closure remain required"}).to_string()).unwrap();
    }
}

#[test]
fn binding_destinations_remain_in_the_metadata_document_when_a_caller_shadows_copied_names() {
    let (temp, store, code) = bindings_fixture();
    fs::copy(store.path(), temp.path().join("copied.lib.pen")).unwrap();
    write(
        store.path(),
        json!({"version":"2.21","imports":{"copied":"copied.lib.pen"},
        "variables":{"copied.tone":{"type":"color","value":"#ff00ff"},"copied.caption":{"type":"string","value":"Caller"}},
        "children":[{"id":"caller-definitions","type":"frame","enabled":false,"width":0,"height":0,"children":[
            {"id":"copied-badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"}]},
            {"id":"use","type":"ref","ref":"copied:generator"}]}),
    );
    let (_, calculation, initial) = snapshot(&store);
    for (x, color) in [
        (5, Color::BLUE),
        (25, Color::BLUE),
        (45, Color::RED),
        (65, Color::BLUE),
    ] {
        assert_eq!(pixel(&initial, x), color, "imported x={x}");
    }
    assert!(
        calculation
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("Copied label")))
    );
    edit(
        &store,
        "caller-local-component",
        json!([{"op":"update","target":"use","set":{"inputs":{"selected":"copied-badge"}}}]),
    );
    let (_, _, overridden) = snapshot(&store);
    assert_eq!(pixel(&overridden, 5), Color::BLUE);
    assert_eq!(pixel(&overridden, 25), Color::GREEN);
    assert_eq!(pixel(&overridden, 45), Color::RED);
    assert_eq!(fs::read(temp.path().join("assets/draw.js")).unwrap(), code);
    if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let path = std::path::PathBuf::from(root).join("script-bindings");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("imported.png"), &initial).unwrap();
        fs::write(path.join("imported-explicit-input.png"), &overridden).unwrap();
        fs::write(
            path.join("imported-report.json"),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),
            "metadata_document_owns_binding_targets":true,"caller_shadowed_names_preserved":true,
            "explicit_ref_uses_caller":true,"script_bytes_sha256":hash(&code),
            "png_hashes":{"imported":hash(&initial),"imported-explicit-input":hash(&overridden)}})
            .to_string(),
        )
        .unwrap();
    }
}
