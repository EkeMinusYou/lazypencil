use lazypencil_xtask::{
    ci::{self, cache, focus},
    dist::{self, Entry, Manifest},
    util,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

fn flags() -> BTreeMap<String, String> {
    [
        ("CARGO_BUILD_JOBS", "1"),
        ("CARGO_INCREMENTAL", "0"),
        ("CARGO_PROFILE_DEV_DEBUG", "0"),
        ("CARGO_PROFILE_TEST_DEBUG", "0"),
    ]
    .map(|(k, v)| (k.into(), v.into()))
    .into()
}
const COMPILER: &str = "rustc 1.94.0\nrelease: 1.94.0\ncommit-hash: 4a4ef493e3a1488c6e321570238084b38948f6db\nhost: x86_64-unknown-linux-gnu\n";
const TREE: &str = "b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d";

struct ApplicationFixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    directory: PathBuf,
    library: PathBuf,
    dep_info: PathBuf,
}

impl ApplicationFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let directory = temp.path().join("cache");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            "[package]\nname='lazypencil'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(source.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        let entries = ["Cargo.toml", "src/lib.rs"]
            .map(|name| {
                let bytes = fs::read(source.join(name)).unwrap();
                Entry {
                    path: name.into(),
                    sha256: util::digest(&bytes),
                    bytes: bytes.len() as u64,
                    mode: 0o644,
                }
            })
            .into();
        let manifest = Manifest {
            format_version: 1,
            package: json!({}),
            source_date_epoch: 0,
            git: json!({"revision":"1111111111111111111111111111111111111111","dirty":false}),
            files: entries,
        };
        fs::write(
            source.join("SOURCE-MANIFEST.json"),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
        let fingerprint = target.join("debug/.fingerprint/lazypencil-1234567890abcdef");
        fs::create_dir_all(&fingerprint).unwrap();
        fs::write(fingerprint.join("lib-lazypencil.json"), "{}").unwrap();
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        let library = target.join("debug/deps/liblazypencil-1234567890abcdef.rlib");
        fs::write(&library, b"compiled fixture library").unwrap();
        let dep_info = target.join("debug/deps/lazypencil-1234567890abcdef.d");
        fs::write(
            &dep_info,
            format!("library: {}/src/lib.rs\n", source.display()),
        )
        .unwrap();
        Self {
            _temp: temp,
            source,
            target,
            directory,
            library,
            dep_info,
        }
    }
    fn identity(&self) -> cache::ApplicationIdentity {
        cache::ApplicationIdentity::capture(
            &self.source,
            &self.target,
            "native-v2-ubuntu-24.04-fixture",
            cache::ApplicationScope::AllOutputs,
        )
        .unwrap()
    }
    fn write_source(&self, name: &str, bytes: &[u8]) {
        let path = self.source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let mut manifest: Manifest = util::json(&self.source.join(dist::MANIFEST)).unwrap();
        manifest.files.retain(|entry| entry.path != name);
        manifest.files.push(Entry {
            path: name.into(),
            bytes: bytes.len() as u64,
            sha256: util::file_digest(&path).unwrap(),
            mode: 0o644,
        });
        fs::write(
            self.source.join(dist::MANIFEST),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
    }
    fn release(&self) -> cache::ApplicationIdentity {
        let directory = self
            .target
            .join("release/.fingerprint/lazypencil-1234567890abcdef");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("bin-lazypencil.json"), b"{}").unwrap();
        let directory = self.target.join("release/deps");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("lazypencil-1234567890abcdef"), b"executable").unwrap();
        fs::write(
            directory.join("lazypencil-1234567890abcdef.d"),
            format!("binary: {}/src/lib.rs\n", self.source.display()),
        )
        .unwrap();
        fs::write(self.target.join("release/lazypencil"), b"executable").unwrap();
        fs::write(
            lazypencil_xtask::evidence::link_map::path(&self.target),
            b"mapped executable",
        )
        .unwrap();
        cache::ApplicationIdentity::capture(
            &self.source,
            &self.target,
            "native-v2-ubuntu-24.04-fixture",
            cache::ApplicationScope::Release,
        )
        .unwrap()
    }
    fn stage(&self) {
        cache::stage(
            &self.source,
            &self.target,
            &self.directory,
            &self.identity(),
        )
        .unwrap();
    }
}

#[path = "ci/selection.rs"]
mod selection;

#[path = "ci/workflows.rs"]
mod workflows;

#[path = "ci/consumers.rs"]
mod consumers;

#[path = "ci/commands.rs"]
mod commands;

#[path = "ci/identity.rs"]
mod identity;

#[path = "ci/cache_tests.rs"]
mod cache_tests;
