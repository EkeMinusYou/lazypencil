#[path = "script_override_fixture.rs"]
mod fixture;
use fixture::*;
use lazypencil::{project::Project, service};
use serde_json::json;
use std::fs;

#[test]
fn inherited_script_overrides_copy_without_running_code_and_survive_independent_recopy() {
    for version in ["2.19", "2.21"] {
        for mode in MODES {
            let (temp, store) = fixture(version, mode);
            let original = fs::read(store.path()).unwrap();
            let source = fs::read(temp.path().join("library/ui.pen")).unwrap();
            service::apply_request(&store, &request(&store, "preview", operations(), true))
                .unwrap();
            assert_eq!(fs::read(store.path()).unwrap(), original);
            assert!(!temp.path().join("assets").exists());
            assert!(store.history(10, None).unwrap().is_empty());
            let mut failed = operations().as_array().unwrap().clone();
            failed.push(json!({"op":"update","target":"missing","set":{"x":1}}));
            assert!(
                service::apply_request(&store, &request(&store, "failed", json!(failed), false))
                    .is_err()
            );
            assert_eq!(fs::read(store.path()).unwrap(), original);
            assert!(!temp.path().join("assets").exists());
            let result = apply(&store, "copy", operations());
            let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
            let component = result["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            assert_eq!(
                fs::read(temp.path().join("library/ui.pen")).unwrap(),
                source
            );
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            let script = active(&store, id);
            assert_eq!(
                script.properties["metadata"]["keep"],
                if ["metadata", "combined"].contains(mode) {
                    "override"
                } else {
                    "base"
                }
            );
            let code = if ["code", "combined", "nullable"].contains(mode) {
                UI
            } else {
                BASE
            };
            assert_eq!(
                fs::read(
                    temp.path()
                        .join(script.properties["scriptUri"].as_str().unwrap())
                )
                .unwrap(),
                code.as_bytes()
            );
            Project::load(&store).unwrap().resolve().unwrap();
            let (copied, id) = recopy(&store, component, &temp.path().join("recopied"));
            fs::remove_file(store.path()).unwrap();
            fs::remove_dir_all(temp.path().join("assets")).unwrap();
            let script = active(&copied, &id);
            assert_eq!(
                script.properties["metadata"]["keep"],
                if ["metadata", "combined"].contains(mode) {
                    "override"
                } else {
                    "base"
                }
            );
            Project::load(&copied).unwrap().resolve().unwrap();
            let target = fixture::active(&copied, &id).address;
            apply(
                &copied,
                "detach",
                json!([{"op":"detach_instance","target":target}]),
            );
            let detached = fixture::active(&copied, &id);
            assert!(!detached.is_instance);
            assert_eq!(
                detached.properties["metadata"]["keep"],
                if ["metadata", "combined"].contains(mode) {
                    "override"
                } else {
                    "base"
                }
            );
            Project::load(&copied).unwrap().resolve().unwrap();
        }
    }
}
