use lazypencil_xtask::{
    notices::{self, toolchain},
    util,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
#[path = "../src/test_support/install.rs"]
mod install;
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    libraries: PathBuf,
    compiler: PathBuf,
    verbose: String,
    index: Value,
    distribution: Value,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("input");
        let libraries = temp.path().join("libraries");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&libraries).unwrap();
        fs::write(
            libraries.join("libstd-original.rlib"),
            b"actual installed standard library fixture",
        )
        .unwrap();
        let verbose = format!(
            "rustc pinned fixture\nhost: x86_64-unknown-linux-gnu\nrelease: {}\ncommit-hash: {}\n",
            toolchain::RELEASE,
            toolchain::COMMIT
        );
        let identity = json!({"release":toolchain::RELEASE,"commit_hash":toolchain::COMMIT,"target":"x86_64-unknown-linux-gnu","verbose":verbose.trim()});
        let index = json!({"format_version":1,"observed_toolchain":identity,"target_libraries":toolchain::library_fingerprints(&libraries).unwrap(),"registry_archives_verified":true,"registry_packages":[{"name":"original"}],"review_findings":["Preserve original publisher finding"]});
        let distribution = json!({"format_version":1,"release_ready":false,"observed_toolchain":identity,"source_url":toolchain::SOURCE_URL,"source_sha256":toolchain::SOURCE_SHA});
        let responses = temp.path().join("responses.json");
        let mut table = serde_json::Map::new();
        for (args, stdout) in [
            (vec!["--version", "--verbose"], verbose.clone()),
            (
                vec!["--print", "sysroot"],
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                vec![
                    "--print",
                    "target-libdir",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                ],
                libraries.to_string_lossy().into_owned(),
            ),
        ] {
            table.insert(
                serde_json::to_string(&args).unwrap(),
                json!({"stdout":stdout}),
            );
        }
        fs::write(&responses, util::encode(&table).unwrap()).unwrap();
        let compiler = temp.path().join("rustc-fixture");
        install::install(
            &util::root(),
            &compiler,
            &[
                "fixture-command".into(),
                responses.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        let f = Self {
            _temp: temp,
            root,
            libraries,
            compiler,
            verbose,
            index,
            distribution,
        };
        f.refresh();
        f
    }
    fn refresh(&self) {
        let inventory = self.root.join("inventory");
        if inventory.exists() {
            fs::remove_dir_all(&inventory).unwrap();
        }
        let files = [
            ("inventory.json".into(), util::encode(&self.index).unwrap()),
            (
                "TOOLCHAIN-NOTICES.txt".into(),
                b"Original notices\r\n".to_vec(),
            ),
        ]
        .into();
        notices::publish_bundle(&inventory, &files).unwrap();
        fs::write(
            self.root.join("source-distribution.json"),
            util::encode(&self.distribution).unwrap(),
        )
        .unwrap();
    }
    fn validate(&self) -> anyhow::Result<Value> {
        toolchain::validate_evidence(&self.root, &self.verbose, self.compiler.to_str().unwrap())
    }
}
#[test]
fn actual_compiler_process_metadata_and_installed_library_bytes_bind_attachments() {
    let f = Fixture::new();
    let output = f._temp.path().join("output");
    let report =
        toolchain::attach(&f.root, &output, &f.verbose, f.compiler.to_str().unwrap()).unwrap();
    assert_eq!(report["target_libraries"], 1);
    assert_eq!(report["review_findings"], f.index["review_findings"]);
    assert_eq!(
        fs::read(output.join("inventory/TOOLCHAIN-NOTICES.txt")).unwrap(),
        b"Original notices\r\n"
    );
    assert_eq!(
        toolchain::validate_evidence(&output, &f.verbose, f.compiler.to_str().unwrap()).unwrap(),
        report
    );
    assert!(toolchain::attach(&f.root, &output, &f.verbose, f.compiler.to_str().unwrap()).is_err());
}
#[test]
fn stale_foreign_unverified_or_incomplete_standard_library_attachments_are_rejected() {
    for mutation in [
        "compiler",
        "target",
        "library",
        "distribution",
        "unverified",
        "missing",
        "altered",
        "extra",
        "extra-root",
    ] {
        let mut f = Fixture::new();
        match mutation {
            "compiler" => f.verbose.push_str("foreign compiler"),
            "target" => {
                f.index["observed_toolchain"]["target"] = "aarch64-apple-darwin".into();
                f.refresh();
            }
            "library" => fs::write(
                f.libraries.join("libstd-original.rlib"),
                b"different installed library",
            )
            .unwrap(),
            "distribution" => {
                f.distribution["source_sha256"] = "0".repeat(64).into();
                f.refresh();
            }
            "unverified" => {
                f.index["registry_archives_verified"] = false.into();
                f.refresh();
            }
            "missing" => fs::remove_file(f.root.join("inventory/TOOLCHAIN-NOTICES.txt")).unwrap(),
            "altered" => fs::write(
                f.root.join("inventory/TOOLCHAIN-NOTICES.txt"),
                b"replacement",
            )
            .unwrap(),
            "extra" => fs::write(f.root.join("inventory/extra"), b"unlisted").unwrap(),
            _ => fs::write(f.root.join("extra"), b"unlisted").unwrap(),
        }
        assert!(f.validate().is_err(), "{mutation}");
    }
}
