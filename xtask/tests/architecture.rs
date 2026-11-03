use lazypencil_xtask::util;
use std::fs;

#[test]
fn development_tools_remain_an_independent_workspace_without_native_application_dependencies() {
    let root = util::root();
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("xtask/Cargo.toml")).unwrap()).unwrap();
    assert!(manifest.get("workspace").is_some());

    fn dependencies(value: &toml::Value) {
        let Some(table) = value.as_table() else {
            return;
        };
        for (name, value) in table {
            if matches!(
                name.as_str(),
                "dependencies" | "dev-dependencies" | "build-dependencies"
            ) {
                for (name, dependency) in value.as_table().unwrap() {
                    assert!(
                        dependency.get("path").is_none(),
                        "Tool dependency must not couple to a local application crate: {name}"
                    );
                }
            } else {
                dependencies(value);
            }
        }
    }
    dependencies(&manifest);

    let lock: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("xtask/Cargo.lock")).unwrap()).unwrap();
    for package in lock["package"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap();
        assert!(
            !matches!(name, "lazypencil" | "skia-safe" | "skia-bindings"),
            "Heavy application dependency in tool workspace: {name}"
        );
    }
}
