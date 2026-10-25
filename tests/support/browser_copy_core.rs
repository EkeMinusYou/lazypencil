use super::browser_copy_fixture as fixture;
use lazypencil::project::Project;
use serde_json::json;

#[test]
fn stored_browser_metadata_edits_preserve_context_but_explicit_same_url_revokes_it() {
    let (_temp, store) = fixture::fixture("2.21", "inherited");
    let result = fixture::apply(
        &store,
        "copy-stored-context",
        json!([{"op":"import_component","mode":"copy",
        "source":{"library_alias":"ui","node_id":"widget"},"options":fixture::options("first.",&["ui:mode","ui:base:mode"])}]),
    );
    let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
    let before = Project::load(&store).unwrap();
    let node = fixture::active(&before, id);
    let revision = before.browser_node_resources(&node).unwrap().revision;
    let definition = node.source_id.clone();
    assert_eq!(
        store.read().unwrap().node(&definition).unwrap()["type"],
        "browser"
    );
    fixture::apply(
        &store,
        "stored-metadata",
        json!([{"op":"update","target":definition,"set":{"metadata":{"type":"user","note":"updated"}}}]),
    );
    let metadata = Project::load(&store).unwrap();
    let node = fixture::active(&metadata, id);
    assert_eq!(
        metadata.browser_node_resources(&node).unwrap().revision,
        revision
    );
    assert_eq!(node.properties["metadata"]["note"], "updated");
    fixture::apply(
        &store,
        "stored-metadata-unset",
        json!([{"op":"update","target":definition,"unset":["metadata"]}]),
    );
    let unset = Project::load(&store).unwrap();
    let node = fixture::active(&unset, id);
    assert_eq!(
        unset.browser_node_resources(&node).unwrap().revision,
        revision
    );
    assert!(node.properties["metadata"].get("note").is_none());
    let raw = store.read().unwrap().node(&definition).unwrap()["url"].clone();
    fixture::apply(
        &store,
        "stored-explicit-url",
        json!([{"op":"update","target":definition,"set":{"url":raw,"metadata":{"type":"user","note":"new context"}}}]),
    );
    let changed = Project::load(&store).unwrap();
    let node = fixture::active(&changed, id);
    assert_eq!(node.browser_origin(), store.path());
    assert_ne!(
        changed.browser_node_resources(&node).unwrap().revision,
        revision
    );
}

#[test]
fn local_browser_namespaces_survive_copy_recopy_and_detach_for_both_document_versions() {
    for version in ["2.19", "2.21"] {
        for mode in fixture::MODES {
            let (_temp, frames) = fixture::study(version, mode, false);
            assert_eq!(frames.len(), 8);
        }
    }
}

#[test]
fn detached_external_browser_properties_can_be_copied_without_the_original_libraries() {
    for version in ["2.19", "2.21"] {
        for mode in fixture::MODES {
            let (_temp, frames) = fixture::study(version, mode, true);
            assert_eq!(frames.len(), 8);
        }
    }
}

#[test]
fn ordinary_metadata_preserves_browser_context_and_same_json_url_edits_revoke_it_until_reset() {
    for mode in fixture::MODES {
        let (_temp, store) = fixture::fixture("2.21", mode);
        let result = fixture::apply(
            &store,
            "copy-context-edit",
            json!([{"op":"import_component","mode":"copy",
            "source":{"library_alias":"ui","node_id":"widget"},"options":fixture::options("first.",&["ui:mode","ui:base:mode"])}]),
        );
        let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
        let before = Project::load(&store).unwrap();
        let node = fixture::active(&before, id);
        let revision = before.browser_node_resources(&node).unwrap().revision;
        let target =
            json!({"node_id":node.address.node_id,"instance_path":node.address.instance_path});
        fixture::apply(
            &store,
            "ordinary-metadata",
            json!([{"op":"update","target":target,"set":{"metadata":{"type":"user","note":"kept"}}}]),
        );
        let after = Project::load(&store).unwrap();
        let node = fixture::active(&after, id);
        assert_eq!(
            after.browser_node_resources(&node).unwrap().revision,
            revision
        );
        assert_eq!(node.properties["metadata"]["note"], "kept");
        let raw = after.expanded_node(&node.address).unwrap().properties["url"].clone();
        fixture::apply(
            &store,
            "explicit-url-metadata",
            json!([{"op":"update","target":target,"set":{"url":raw,"metadata":{"type":"user","note":"changed"}}}]),
        );
        let explicit = Project::load(&store).unwrap();
        let node = fixture::active(&explicit, id);
        assert_eq!(node.browser_origin(), store.path());
        assert_ne!(
            explicit.browser_node_resources(&node).unwrap().revision,
            revision
        );
        fixture::apply(
            &store,
            "reset-url",
            json!([{"op":"reset_override","target":target,"properties":["url"]}]),
        );
        let reset = Project::load(&store).unwrap();
        let node = fixture::active(&reset, id);
        assert_eq!(
            reset.browser_node_resources(&node).unwrap().revision,
            revision
        );
        assert_eq!(node.properties["metadata"]["note"], "changed");
    }
}
