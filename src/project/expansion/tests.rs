use super::*;
use base64::Engine;
use std::fs;

fn fixture(version: &str, variables: bool) -> (tempfile::TempDir, Project, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD
            .encode(include_bytes!("../../../examples/assets/crop-source.png"))
    );
    let text = "ちょっと変換\n$literal 😀".repeat(128);
    let fill = if variables {
        json!({"type":"shader","url":"sample.glsl","uniforms":{"tex":"$image_alias"}})
    } else {
        json!({"type":"image","url":uri})
    };
    let mut children = vec![
        json!({"id":"card","type":"frame","reusable":true,"width":40,"height":20,"children":[
        {"id":"image","type":"rectangle","width":40,"height":20,"fill":fill},
            {"id":"text","type":"text","content":if variables {"$copy_text"} else {&text},"fontSize":12}
        ]}),
    ];
    for index in 0..24 {
        children.push(json!({"id":format!("copy{index}"),"type":"ref","ref":"card"}));
    }
    children.push(
        json!({"id":"generator","type":"script","scriptUri":"generator.js","width":40,"height":20}),
    );
    fs::write(
        temp.path().join("sample.glsl"),
        "uniform sampler2D tex; void main() { gl_FragColor = texture2D(tex, vec2(0.5)); }",
    )
    .unwrap();
    fs::write(
        temp.path().join("generator.js"),
        format!("/** @schema {version} */\nreturn [];"),
    )
    .unwrap();
    fs::write(
        store.path(),
        json!({"version":version,"variables":{
        "image_source":{"type":"string","value":uri},
        "image_alias":{"type":"string","value":"$image_source"},
        "copy_text":{"type":"string","value":text}
    },"children":children})
        .to_string(),
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    (temp, project, uri, text)
}

#[test]
fn root_and_variable_expansion_share_a_preallocation_budget_without_changing_values() {
    for version in ["2.19", "2.21"] {
        for variables in [false, true] {
            let (_temp, project, uri, text) = fixture(version, variables);
            let scene = project.resolve().unwrap();
            for root in &scene.roots[..25] {
                let fill = &root.children[0].properties["fill"];
                assert_eq!(
                    if variables {
                        &fill["uniforms"]["tex"]
                    } else {
                        &fill["url"]
                    },
                    &uri
                );
                assert_eq!(root.children[1].properties["content"], text);
            }
            let mut budget = Expansion {
                remaining: 64 * 1024,
                ..Default::default()
            };
            let error = project.resolve_bounded(false, &mut budget).err().unwrap();
            assert_eq!(error.code, "RESOURCE_LIMIT");
            assert!(
                budget.nodes < 100,
                "Every instance expanded before the byte limit was checked"
            );
            project.verify_unchanged().unwrap();
        }
    }
}

#[test]
fn public_resolution_rejects_small_source_with_excessive_instance_expansion() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(
        temp.path().join("large.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let text = "x".repeat(16 * 1024);
    let mut children = vec![
        json!({"id":"card","type":"frame","reusable":true,"children":[
            {"id":"text","type":"text","content":text,"fontSize":12}
        ]}),
    ];
    for index in 0..256 {
        children.push(json!({"id":format!("copy{index}"),"type":"ref","ref":"card"}));
    }
    let document = json!({"version":"2.21","children":children}).to_string();
    fs::write(store.path(), &document).unwrap();
    let configuration = temp.path().join("lazypencil.toml");
    fs::write(&configuration, "[runtime.document]\nexpansion_mib=1\n").unwrap();
    let project = Project::load(&store).unwrap();
    let error = project.resolve().err().unwrap();
    assert_eq!(error.code, "RESOURCE_LIMIT");
    assert_eq!(error.details.as_ref().unwrap()["limit_bytes"], 1024 * 1024);
    assert_eq!(
        project.expanded_library_roots(None).err().unwrap().code,
        "RESOURCE_LIMIT"
    );
    assert_eq!(
        project.validate_structure().unwrap_err().code,
        "RESOURCE_LIMIT"
    );
    assert_eq!(fs::read_to_string(store.path()).unwrap(), document);
    project.verify_unchanged().unwrap();
    let metadata = crate::service::document_metadata(&store).unwrap();
    assert_eq!(
        metadata["capabilities"]["document_expansion"]["allocation_budget_bytes"],
        1024 * 1024
    );
    fs::write(&configuration, "[runtime.document]\nexpansion_mib=32\n").unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let project = Project::load(&store).unwrap();
    assert_eq!(project.resolve().unwrap().roots.len(), 257);
    assert_eq!(project.expanded_library_roots(None).unwrap().len(), 257);
    project.validate_structure().unwrap();
    fs::remove_file(configuration).unwrap();
    assert_eq!(
        Project::load(&store)
            .unwrap()
            .document_settings()
            .expansion_bytes(),
        256 * 1024 * 1024
    );
}

#[test]
fn inherited_raw_properties_are_bounded_even_when_values_are_not_resolved() {
    let (_temp, project, _, _) = fixture("2.21", false);
    let mut budget = Expansion {
        remaining: 64 * 1024,
        ..Default::default()
    };
    let mut rejected = false;
    for node in project.root_document().value()["children"]
        .as_array()
        .unwrap()
    {
        match project.expand(&project.root, node, None, &[], &mut Vec::new(), &mut budget) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.code, "RESOURCE_LIMIT");
                rejected = true;
                break;
            }
        }
    }
    assert!(
        rejected,
        "Stored reads and structural edits bypassed the aggregate limit"
    );
    project.verify_unchanged().unwrap();
}

#[cfg(feature = "rendering")]
#[test]
fn configured_expansion_budget_bounds_static_layout_and_preview() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    fs::write(store.path(), json!({"version":"2.21","children":[
        {"id":"first","type":"rectangle","width":4,"height":4,"metadata":{"payload":"x".repeat(600000)}},
        {"id":"second","type":"rectangle","width":4,"height":4,"metadata":{"payload":"x".repeat(600000)}}
    ]}).to_string()).unwrap();
    let configuration = temp.path().join("lazypencil.toml");
    for limit in [1, 8] {
        fs::write(
            &configuration,
            format!("[runtime.document]\nexpansion_mib={limit}\n"),
        )
        .unwrap();
        let result = crate::service::layout_snapshot(&store, &[], None, None);
        if limit == 1 {
            let error = result.err().unwrap();
            assert_eq!(error.code, "RESOURCE_LIMIT");
            assert_eq!(error.details.unwrap()["limit_bytes"], 1024 * 1024);
        } else {
            assert!(result.is_ok(), "{:?}", result.err());
        }
        #[cfg(feature = "tui")]
        {
            let mut runtime = crate::script::ScriptRuntime::current().unwrap();
            let result = crate::service::preview_layout_snapshot(&store, &[], &mut runtime);
            if limit == 1 {
                let error = result.err().unwrap();
                assert_eq!(error.code, "RESOURCE_LIMIT");
                assert_eq!(error.details.unwrap()["limit_bytes"], 1024 * 1024);
            } else {
                assert!(result.is_ok(), "{:?}", result.err());
            }
        }
    }
}

#[cfg(feature = "rendering")]
#[test]
fn independent_generated_outputs_share_the_same_allocation_budget() {
    let (_temp, mut project, _, _) = fixture("2.21", true);
    let scene = project.resolve().unwrap();
    let owner = scene
        .roots
        .iter()
        .find(|node| node.kind() == "script")
        .unwrap();
    let mut budget = Expansion {
        remaining: 64 * 1024,
        ..Default::default()
    };
    let mut generated = Vec::new();
    let mut rejected = false;
    for _ in 0..24 {
        match project.generated_children(owner, &[json!({"type":"ref","ref":"card"})], &mut budget)
        {
            Ok(nodes) => generated.extend(nodes),
            Err(error) => {
                assert_eq!(error.code, "RESOURCE_LIMIT");
                rejected = true;
                break;
            }
        }
    }
    assert!(
        !generated.is_empty() && rejected,
        "Each generator received a new independent allowance"
    );
    project.verify_unchanged().unwrap();
}
