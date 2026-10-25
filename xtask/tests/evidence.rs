use lazypencil_xtask::{
    dist::{Entry, Manifest},
    evidence,
    notices::skia,
    util,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    target: PathBuf,
    events: PathBuf,
    records: Vec<Value>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("source");
        let target = base.join("target");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir(&target).unwrap();
        for (name, data) in [
            (
                "Cargo.toml",
                "[package]\nname='lazypencil'\nversion='0.1.0'\nrust-version='1.94'\n",
            ),
            ("Cargo.lock", "version=4\n"),
            ("LICENSE", "MIT original\n"),
            ("README.md", "Fixture source\n"),
            ("src/main.rs", "fn main() {}\n"),
        ] {
            fs::write(root.join(name), data).unwrap();
        }
        let files = util::walk(&root)
            .unwrap()
            .into_iter()
            .map(|p| Entry {
                path: p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                sha256: util::file_digest(&p).unwrap(),
                bytes: fs::metadata(&p).unwrap().len(),
                mode: 0o644,
            })
            .collect();
        let manifest = Manifest {
            format_version: 1,
            package: json!({"name":"lazypencil","version":"0.1.0","rust_version":"1.94"}),
            source_date_epoch: 0,
            git: json!({"revision":"a".repeat(40),"dirty":false}),
            files,
        };
        fs::write(
            root.join("SOURCE-MANIFEST.json"),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
        let binary = target.join("release/lazypencil");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"new release binary").unwrap();
        let mut records = vec![
            json!({"reason":"compiler-artifact","package_id":"root#0.1.0","target":{"name":"lazypencil","kind":["bin"]},"manifest_path":root.join("Cargo.toml"),"profile":{"test":false,"opt_level":"3"},"fresh":false,"executable":binary}),
        ];
        for (name, libraries) in [
            (
                "skia_bindings",
                skia::TARGETS
                    .iter()
                    .copied()
                    .chain(["skia-bindings"])
                    .collect::<Vec<_>>(),
            ),
            ("rquickjs_sys", vec!["quickjs"]),
        ] {
            let out = target.join(format!("release/build/{name}-selected/out"));
            let directory = if name == "skia_bindings" {
                out.join("skia")
            } else {
                out.clone()
            };
            fs::create_dir_all(&directory).unwrap();
            for lib in &libraries {
                fs::write(
                    directory.join(format!("lib{lib}.a")),
                    format!("native archive {lib}"),
                )
                .unwrap();
            }
            records.push(json!({"reason":"compiler-artifact","package_id":format!("{name}#1.0.0"),"target":{"name":name,"kind":["lib"]},"manifest_path":root.join("Cargo.toml"),"profile":{"test":false},"features":["default"],"fresh":true}));
            records.push(json!({"reason":"build-script-executed","package_id":format!("{name}#1.0.0"),"out_dir":out,"linked_libs":libraries.iter().map(|n|format!("static={n}")).collect::<Vec<_>>(),"linked_paths":[directory],"env":[["SECRET_FIXTURE","must not be copied"]]}));
        }
        records.push(json!({"reason":"build-finished","success":true}));
        let events = temp.path().join("cargo.jsonl");
        let f = Self {
            _temp: temp,
            root,
            target,
            events,
            records,
        };
        f.save();
        f
    }
    fn save(&self) {
        let mut data = "a non-JSON build-tool line\n".to_owned();
        for record in &self.records {
            data.push_str(&serde_json::to_string(record).unwrap());
            data.push('\n');
        }
        fs::write(&self.events, data).unwrap();
    }
    fn observe(&self) -> anyhow::Result<(PathBuf, std::collections::BTreeMap<String, Value>, u64)> {
        evidence::cargo_observations(&self.events, &self.root, &self.target)
    }
}
#[test]
fn selected_native_inputs_pair_by_package_identity_without_copying_environment() {
    let mut f = Fixture::new();
    fs::create_dir(
        f.target
            .join("release/build/skia_bindings-newer-unselected"),
    )
    .unwrap();
    let (binary, selected, ignored) = f.observe().unwrap();
    assert_eq!(binary, f.target.join("release/lazypencil"));
    assert_eq!(ignored, 1);
    assert_eq!(selected["skia_bindings"]["fresh"], true);
    assert_eq!(selected["rquickjs_sys"]["fresh"], true);
    assert!(
        !serde_json::to_string(&selected)
            .unwrap()
            .contains("must not be copied")
    );
    assert_eq!(
        evidence::native_archives(&f.target, &selected).unwrap()["skia_bindings"].len(),
        6
    );
    evidence::source_identity(&f.root, &"a".repeat(40)).unwrap();
    f.records[1]["fresh"] = false.into();
    f.save();
    let (_, selected, _) = f.observe().unwrap();
    assert_eq!(selected["skia_bindings"]["fresh"], false);
    assert_eq!(selected["rquickjs_sys"]["fresh"], true);
}
#[test]
fn incomplete_ambiguous_or_stale_cargo_artifacts_cannot_authorize_evidence() {
    for mutation in [
        "failed",
        "missing-completion",
        "duplicate-completion",
        "post-completion",
        "cached",
        "test-profile",
        "debug-profile",
        "wrong-manifest",
        "wrong-name",
        "duplicate-app",
        "duplicate-native",
        "unpaired-native",
        "duplicate-native-build",
        "native-escape",
        "missing-native-fresh",
    ] {
        let mut f = Fixture::new();
        match mutation {
            "failed" => f.records.last_mut().unwrap()["success"] = false.into(),
            "missing-completion" => {
                f.records.pop();
            }
            "duplicate-completion" => f
                .records
                .push(json!({"reason":"build-finished","success":true})),
            "post-completion" => f.records.push(f.records[0].clone()),
            "cached" => f.records[0]["fresh"] = true.into(),
            "test-profile" => f.records[0]["profile"]["test"] = true.into(),
            "debug-profile" => f.records[0]["profile"]["opt_level"] = "0".into(),
            "wrong-manifest" => f.records[0]["manifest_path"] = json!(f.target.join("Cargo.toml")),
            "wrong-name" => f.records[0]["executable"] = json!(f.target.join("release/other")),
            "duplicate-app" => {
                f.records.insert(1, f.records[0].clone());
            }
            "duplicate-native" => {
                f.records.insert(2, f.records[1].clone());
            }
            "unpaired-native" => f.records[1]["package_id"] = "another#1.0.0".into(),
            "duplicate-native-build" => {
                f.records.insert(3, f.records[2].clone());
            }
            "missing-native-fresh" => f.records[1]["fresh"] = Value::Null,
            _ => f.records[2]["out_dir"] = json!(f.root),
        }
        f.save();
        assert!(f.observe().is_err(), "{mutation}");
    }
    let mut f = Fixture::new();
    for _ in 0..2 {
        f.records.insert(1,json!({"reason":"build-script-executed","package_id":"unrelated#1","out_dir":"ignored","linked_libs":[],"linked_paths":[]}));
    }
    f.save();
    f.observe().unwrap();
}
#[test]
fn native_archive_aliases_must_match_and_every_selected_library_must_exist() {
    for mutation in ["missing", "different-alias", "same-alias", "windows-search"] {
        let f = Fixture::new();
        let (_, mut selected, _) = f.observe().unwrap();
        let directory = f.target.join("release/build/rquickjs_sys-selected/out");
        match mutation {
            "missing" => fs::remove_file(directory.join("libquickjs.a")).unwrap(),
            "different-alias" => fs::write(directory.join("quickjs.lib"), b"different").unwrap(),
            "same-alias" => {
                fs::copy(
                    directory.join("libquickjs.a"),
                    directory.join("quickjs.lib"),
                )
                .unwrap();
            }
            _ => {
                fs::rename(
                    directory.join("libquickjs.a"),
                    directory.join("quickjs.lib"),
                )
                .unwrap();
                selected.get_mut("rquickjs_sys").unwrap()["linked_paths"] =
                    json!([format!("native={}", directory.display())]);
            }
        }
        let actual = evidence::native_archives(&f.target, &selected);
        assert_eq!(
            actual.is_ok(),
            matches!(mutation, "same-alias" | "windows-search"),
            "{mutation}"
        );
    }
}
#[test]
fn distributed_source_identity_rejects_changed_dirty_unlisted_duplicate_and_escaped_inputs() {
    for mutation in [
        "revision",
        "dirty",
        "content",
        "extra",
        "duplicate",
        "escape",
        "mode",
    ] {
        let f = Fixture::new();
        let path = f.root.join("SOURCE-MANIFEST.json");
        let mut manifest: Value = util::json(&path).unwrap();
        match mutation {
            "revision" => manifest["git"]["revision"] = "b".repeat(40).into(),
            "dirty" => manifest["git"]["dirty"] = true.into(),
            "content" => fs::write(f.root.join("src/main.rs"), b"changed").unwrap(),
            "extra" => fs::write(f.root.join("extra.rs"), b"unlisted").unwrap(),
            "duplicate" => {
                let row = manifest["files"][0].clone();
                manifest["files"].as_array_mut().unwrap().push(row);
            }
            "escape" => manifest["files"][0]["path"] = "../outside".into(),
            _ => manifest["files"][0]["mode"] = 0o777.into(),
        }
        fs::write(path, util::encode(&manifest).unwrap()).unwrap();
        assert!(
            evidence::source_identity(&f.root, &"a".repeat(40)).is_err(),
            "{mutation}"
        );
    }
}
#[cfg(unix)]
#[test]
fn linked_release_binary_native_archive_or_source_never_substitutes_external_bytes() {
    use std::os::unix::fs::symlink;
    for name in [
        "source/src/main.rs",
        "target/release/lazypencil",
        "target/release/build/rquickjs_sys-selected/out/libquickjs.a",
    ] {
        let f = Fixture::new();
        let file = f._temp.path().join(name);
        let external = f._temp.path().join("external");
        fs::rename(&file, &external).unwrap();
        symlink(external, file).unwrap();
        if name.starts_with("source") {
            assert!(evidence::source_identity(&f.root, &"a".repeat(40)).is_err());
        } else {
            let result = f
                .observe()
                .and_then(|(_, selected, _)| evidence::native_archives(&f.target, &selected));
            assert!(result.is_err());
        }
    }
}
