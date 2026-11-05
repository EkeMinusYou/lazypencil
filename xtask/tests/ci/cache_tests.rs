use super::*;

#[test]
fn core_catalog_cache_survives_retention_without_colliding_with_default_catalog() {
    let f = ApplicationFixture::new();
    let default = f.identity();
    let core = cache::ApplicationIdentity::capture(
        &f.source,
        &f.target,
        "native-v2-ubuntu-24.04-fixture-core",
        cache::ApplicationScope::AllOutputs,
    )
    .unwrap();
    assert_ne!(core.compatible_primary, default.compatible_primary);
    cache::stage(&f.source, &f.target, &f.directory, &core).unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &default).is_err());
    cache::restore_compatible(
        &f.source,
        &f.target,
        &f.directory,
        &core.compatible_primary,
        &core,
    )
    .unwrap();
    let entries = [core.compatible_primary, default.compatible_primary]
        .into_iter()
        .enumerate()
        .map(|(index, key)| json!({"id":index + 1,"key":key,"ref":"refs/heads/main","version":"layout","created_at":"2026-10-29"}))
        .collect::<Vec<_>>();
    let manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    assert!(
        cache::obsolete_for_manifest(&entries, &manifest)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn release_reuse_survives_test_changes_without_restoring_test_outputs() {
    let f = ApplicationFixture::new();
    f.write_source("tests/render.rs", b"original test");
    let all = f.identity();
    let release = f.release();
    assert_ne!(all.fingerprint, release.fingerprint);
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    f.write_source("tests/render.rs", b"expanded regression");
    f.write_source("xtask/src/test_support/fixture.rs", b"new fixture");
    let current = cache::ApplicationIdentity::capture(
        &f.source,
        &f.target,
        "native-v2-ubuntu-24.04-fixture",
        cache::ApplicationScope::Release,
    )
    .unwrap();
    assert_eq!(current.primary, release.primary);
    assert_ne!(f.identity().primary, all.primary);
    fs::remove_dir_all(f.target.join("release")).unwrap();
    fs::write(&f.library, b"current debug output").unwrap();
    let value = cache::restore(&f.source, &f.target, &f.directory, &current).unwrap();
    assert_eq!(value["reused"], true);
    assert_eq!(fs::read(&f.library).unwrap(), b"current debug output");
    let manifest: Value = util::json(&f.directory.join("manifest.json")).unwrap();
    assert!(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["path"].as_str().unwrap().starts_with("release/"))
    );
    assert_eq!(
        fs::read(f.target.join("release/lazypencil")).unwrap(),
        b"executable"
    );
}

#[test]
fn release_reuse_rejects_source_resource_recipe_and_scope_changes() {
    for name in ["src/lib.rs", "resources/guidance/design.md", "Cargo.toml"] {
        let f = ApplicationFixture::new();
        if name != "Cargo.toml" {
            f.write_source(name, b"original input");
        }
        let release = f.release();
        cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
        if name == "Cargo.toml" {
            let original = fs::read_to_string(f.source.join(name)).unwrap();
            f.write_source(
                name,
                format!("{original}\n[profile.release]\nlto='thin'\n").as_bytes(),
            );
        } else {
            f.write_source(name, b"changed input");
        }
        let current = cache::ApplicationIdentity::capture(
            &f.source,
            &f.target,
            "native-v2-ubuntu-24.04-fixture",
            cache::ApplicationScope::Release,
        )
        .unwrap();
        assert!(
            cache::restore(&f.source, &f.target, &f.directory, &current).is_err(),
            "{name}"
        );
    }
    let f = ApplicationFixture::new();
    let release = f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    let current = cache::ApplicationIdentity::capture(
        &f.source,
        &f.target,
        "native-v2-ubuntu-24.04-other-build",
        cache::ApplicationScope::Release,
    )
    .unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &current).is_err());
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
}

#[test]
fn release_cache_rejects_compiled_test_inputs_and_debug_entries_before_overlay() {
    let f = ApplicationFixture::new();
    f.write_source("tests/embedded.txt", b"embedded fixture");
    let release = f.release();
    let dep = f.target.join("release/deps/lazypencil-1234567890abcdef.d");
    fs::write(
        &dep,
        format!("binary: {}/tests/embedded.txt\n", f.source.display()),
    )
    .unwrap();
    assert!(cache::stage(&f.source, &f.target, &f.directory, &release).is_err());
    f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    let path = f.directory.join("manifest.json");
    let mut report: Value = util::json(&path).unwrap();
    report["files"].as_array_mut().unwrap().push(json!({"path":"debug/deps/liblazypencil-1234567890abcdef.rlib","sha256":"invalid","bytes":1,"mode":420}));
    fs::write(&path, util::encode(&report).unwrap()).unwrap();
    fs::remove_dir_all(f.target.join("release")).unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &release).is_err());
    assert!(!f.target.join("release").exists());
}

#[test]
fn rejected_release_cli_cache_does_not_prune_verified_debug_outputs() {
    let f = ApplicationFixture::new();
    f.write_source(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"lazypencil\"\nversion = \"0.1.0\"\n",
    );
    let release = f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    fs::write(
        f.directory.join("files/release/lazypencil.map"),
        b"tampered",
    )
    .unwrap();
    let output = f.directory.with_extension("outputs");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "ci",
            "cache",
            "application",
            "restore-or-prune",
            "--release-only",
        ])
        .arg("--source")
        .arg(&f.source)
        .arg("--target")
        .arg(&f.target)
        .arg("--directory")
        .arg(&f.directory)
        .args(["--native-key", "native-v2-ubuntu-24.04-fixture"])
        .arg("--github-output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["reused"],
        false
    );
    assert_eq!(fs::read(&f.library).unwrap(), b"compiled fixture library");
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "release_reused=false\n"
    );
    assert!(!lazypencil_xtask::evidence::link_map::path(&f.target).exists());
}

#[test]
fn application_reuse_checks_all_outputs_before_overlaying_and_never_reuses_images_or_dependencies()
{
    let f = ApplicationFixture::new();
    #[cfg(unix)]
    {
        util::chmod(&f.library, 0o755).unwrap();
        fs::File::open(&f.library)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_789),
            ))
            .unwrap();
        fs::hard_link(&f.library, f.target.join("debug/liblazypencil.rlib")).unwrap();
    }
    fs::write(f.target.join("release-smoke.png"), b"old evidence").unwrap();
    fs::write(
        f.library
            .parent()
            .unwrap()
            .join("libdependency-1234567890abcdef.rlib"),
        b"dependency",
    )
    .unwrap();
    f.stage();
    #[cfg(unix)]
    {
        let archive = f._temp.path().join("checkpoint.tar.gz");
        let before = fs::metadata(&f.library).unwrap().modified().unwrap();
        cache::artifact::pack(&f.directory, &archive).unwrap();
        fs::remove_dir_all(&f.directory).unwrap();
        cache::artifact::unpack(&archive, &f.directory).unwrap();
        let file = f
            .directory
            .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib");
        assert_eq!(util::raw_mode(&file).unwrap(), 0o755);
        assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), before);
    }
    fs::remove_dir_all(&f.target).unwrap();
    let report = cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap();
    assert_eq!(report["files"], if cfg!(unix) { 4 } else { 3 });
    assert_eq!(fs::read(&f.library).unwrap(), b"compiled fixture library");
    assert!(!f.target.join("release-smoke.png").exists());
    fs::remove_dir_all(&f.target).unwrap();
    fs::write(
        f.directory
            .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib"),
        b"tampered",
    )
    .unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    assert!(!f.target.exists());
}

#[test]
fn release_cache_keeps_the_mapped_executable_and_rejects_partial_or_changed_pairs() {
    for (binary, map) in [(true, false), (false, true), (true, true)] {
        let f = ApplicationFixture::new();
        fs::create_dir_all(f.target.join("release")).unwrap();
        let executable = f.target.join("release/lazypencil");
        let map_path = lazypencil_xtask::evidence::link_map::path(&f.target);
        if binary {
            fs::write(&executable, b"release executable").unwrap();
        }
        if map {
            fs::write(&map_path, b"original map").unwrap();
        }
        let staged = cache::stage(&f.source, &f.target, &f.directory, &f.identity());
        if binary != map {
            assert!(staged.is_err());
            continue;
        }
        staged.unwrap();
        fs::remove_dir_all(&f.target).unwrap();
        cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap();
        assert_eq!(fs::read(&executable).unwrap(), b"release executable");
        assert_eq!(fs::read(&map_path).unwrap(), b"original map");
        fs::remove_dir_all(&f.target).unwrap();
        let manifest_path = f.directory.join("manifest.json");
        let original: Value = util::json(&manifest_path).unwrap();
        let mut partial = original.clone();
        partial["files"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["path"] != "release/lazypencil.map");
        fs::write(&manifest_path, util::encode(&partial).unwrap()).unwrap();
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
        assert!(!f.target.exists());
        fs::write(manifest_path, util::encode(&original).unwrap()).unwrap();
        fs::write(
            f.directory.join("files/release/lazypencil.map"),
            b"changed map",
        )
        .unwrap();
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
        assert!(!f.target.exists());
    }
}

#[cfg(unix)]
#[test]
fn checkpoint_artifacts_reject_links_traversal_duplicates_and_corrupt_trailers_before_extraction() {
    let f = ApplicationFixture::new();
    f.stage();
    fs::write(f._temp.path().join("escaped"), b"sentinel").unwrap();
    for kind in ["symlink", "hardlink", "parent", "duplicate", "unexpected"] {
        let archive = f._temp.path().join(format!("{kind}.tar.gz"));
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let manifest = fs::read(f.directory.join("manifest.json")).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "manifest.json", manifest.as_slice())
            .unwrap();
        let report: Value = serde_json::from_slice(&manifest).unwrap();
        for (index, file) in report["files"].as_array().unwrap().iter().enumerate() {
            let name = format!("files/{}", file["path"].as_str().unwrap());
            for _ in 0..if kind == "duplicate" && index == 0 {
                2
            } else {
                1
            } {
                let mut data = fs::read(f.directory.join(&name)).unwrap();
                let mut header = tar::Header::new_gnu();
                header.set_mode(file["mode"].as_u64().unwrap() as u32);
                if index == 0 && matches!(kind, "symlink" | "hardlink") {
                    data.clear();
                    header.set_entry_type(if kind == "symlink" {
                        tar::EntryType::Symlink
                    } else {
                        tar::EntryType::Link
                    });
                    let target = if kind == "symlink" {
                        format!("{}escaped", "../".repeat(name.split('/').count() + 1))
                    } else {
                        "../../escaped".into()
                    };
                    header.set_link_name(target).unwrap();
                }
                header.set_size(data.len() as u64);
                header.set_cksum();
                if index == 0 && kind == "parent" {
                    header.as_mut_bytes()[..10].copy_from_slice(b"../escaped");
                    header.set_cksum();
                    builder.append(&header, data.as_slice()).unwrap();
                } else {
                    builder
                        .append_data(
                            &mut header,
                            if index == 0 && kind == "unexpected" {
                                "files/foreign"
                            } else {
                                &name
                            },
                            data.as_slice(),
                        )
                        .unwrap();
                }
            }
        }
        fs::write(&archive, builder.into_inner().unwrap().finish().unwrap()).unwrap();
        let parent = f._temp.path().join(format!("sandbox-{kind}"));
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("escaped"), b"sentinel").unwrap();
        let destination = parent.join("output");
        assert!(
            cache::artifact::unpack(&archive, &destination).is_err(),
            "{kind}"
        );
        assert!(!destination.exists());
        assert_eq!(
            fs::read(f._temp.path().join("escaped")).unwrap(),
            b"sentinel"
        );
        assert_eq!(fs::read(parent.join("escaped")).unwrap(), b"sentinel");
    }
    let archive = f._temp.path().join("valid.tar.gz");
    cache::artifact::pack(&f.directory, &archive).unwrap();
    let mut bytes = fs::read(&archive).unwrap();
    *bytes.last_mut().unwrap() ^= 0x80;
    fs::write(&archive, bytes).unwrap();
    let destination = f._temp.path().join("corrupt-output");
    assert!(cache::artifact::unpack(&archive, &destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn application_reuse_rejects_changed_sources_external_dep_info_and_compiled_excluded_files() {
    for dependency in [
        "outside.rs",
        ".github/workflows/ci.yml",
        "tests/fixtures/fonts/README.md",
    ] {
        let f = ApplicationFixture::new();
        let path = if dependency == "outside.rs" {
            f.source.parent().unwrap().join(dependency)
        } else {
            f.source.join(dependency)
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"input").unwrap();
        if dependency != "outside.rs" {
            let manifest_path = f.source.join(dist::MANIFEST);
            let mut manifest: Manifest = util::json(&manifest_path).unwrap();
            manifest.files.push(Entry {
                path: dependency.into(),
                bytes: 5,
                sha256: util::digest(b"input"),
                mode: 0o644,
            });
            fs::write(manifest_path, util::encode(&manifest).unwrap()).unwrap();
        }
        fs::write(&f.dep_info, format!("library: {}\n", path.display())).unwrap();
        assert!(cache::stage(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    }
    let f = ApplicationFixture::new();
    f.stage();
    fs::write(f.source.join("src/lib.rs"), b"changed").unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    for name in [
        "../outside",
        "/tmp/outside",
        "debug/deps/../../outside",
        "debug/build/anything",
        "debug/build/lazypencil-1234567890abcdef/out/generated.rs",
        "debug/build/lazypencil-1234567890abcdef/out/../../outside",
        "debug/build/other-1234567890abcdef/out/liblazypencil_pinch.a",
        "debug/build/lazypencil-1234567890abcdef/build_script_build-fedcba0987654321",
        "release-smoke.png",
        "debug/examples/report.png",
    ] {
        assert!(!cache::allowed(name));
    }
}

#[test]
fn application_reuse_rejects_same_mtime_mutations_and_unknown_generated_or_custom_build_inputs() {
    for update_manifest in [false, true] {
        let f = ApplicationFixture::new();
        f.stage();
        let path = f.source.join("src/lib.rs");
        let metadata = fs::metadata(&path).unwrap();
        fs::write(&path, b"pub fn changed() {}\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(metadata.modified().unwrap())
                    .set_accessed(metadata.accessed().unwrap()),
            )
            .unwrap();
        if update_manifest {
            let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
            let entry = manifest
                .files
                .iter_mut()
                .find(|e| e.path == "src/lib.rs")
                .unwrap();
            entry.sha256 = util::file_digest(&path).unwrap();
            entry.bytes = fs::metadata(path).unwrap().len();
            fs::write(
                f.source.join(dist::MANIFEST),
                util::encode(&manifest).unwrap(),
            )
            .unwrap();
        }
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    }
    let f = ApplicationFixture::new();
    let review = "resources/licenses/native/quickjs-review.json";
    let path = f.source.join(review);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{}\n").unwrap();
    let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    manifest.files.push(Entry {
        path: review.into(),
        bytes: 3,
        sha256: util::file_digest(&path).unwrap(),
        mode: 0o644,
    });
    fs::write(
        f.source.join(dist::MANIFEST),
        util::encode(&manifest).unwrap(),
    )
    .unwrap();
    for runtime in [
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ] {
        f.write_source(runtime, b"runtime-only fixture\n");
    }
    for path in [
        "SOURCE-MANIFEST.json",
        review,
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ] {
        fs::write(
            &f.dep_info,
            format!("library: {}/{path}\n", f.source.display()),
        )
        .unwrap();
        assert!(
            cache::stage(&f.source, &f.target, &f.directory, &f.identity()).is_err(),
            "Excluded compiler input: {path}"
        );
    }
    fs::write(f.source.join("build.rs"), b"fn main() {}\n").unwrap();
    assert!(
        cache::ApplicationIdentity::capture(
            &f.source,
            &f.target,
            "native-v2-ubuntu-24.04-fixture",
            cache::ApplicationScope::AllOutputs
        )
        .is_err()
    );
}
#[test]
fn escaped_space_inputs_are_bound_and_compile_time_environment_changes_cannot_reuse_outputs() {
    let f = ApplicationFixture::new();
    let path = f.source.join("resources/guidance/a guide.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"Embedded space\n").unwrap();
    let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    manifest.files.push(Entry {
        path: "resources/guidance/a guide.md".into(),
        bytes: 15,
        sha256: util::file_digest(&path).unwrap(),
        mode: 0o644,
    });
    fs::write(
        f.source.join(dist::MANIFEST),
        util::encode(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        &f.dep_info,
        format!(
            "library: {}/resources/guidance/a\\ guide.md\n",
            f.source.display()
        ),
    )
    .unwrap();
    f.stage();
    fs::remove_dir_all(&f.target).unwrap();
    assert!(
        cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap()["reused"]
            == true
    );
    // Test environment binding in separate processes rather than mutating global test state.
    fs::remove_dir_all(&f.directory).unwrap();
    fs::write(
        &f.dep_info,
        format!(
            "library: {}/src/lib.rs\n# env-dep:EXAMPLE_BUILD_SETTING=one\n",
            f.source.display()
        ),
    )
    .unwrap();
    let invoke = |operation: &str, value: &str| {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .env("EXAMPLE_BUILD_SETTING", value)
            .env_remove("GITHUB_OUTPUT")
            .args(["ci", "cache", "application", operation, "--source"])
            .arg(&f.source)
            .arg("--target")
            .arg(&f.target)
            .arg("--directory")
            .arg(&f.directory)
            .args(["--native-key", "native-v2-ubuntu-24.04-fixture"])
            .output()
            .unwrap()
    };
    let staged = invoke("stage", "one");
    assert!(
        staged.status.success(),
        "{}",
        String::from_utf8_lossy(&staged.stderr)
    );
    let restored = invoke("restore", "two");
    assert!(!restored.status.success());
    assert!(String::from_utf8_lossy(&restored.stderr).contains("environment"));
}
#[cfg(unix)]
#[test]
fn cache_symlinks_cannot_redirect_source_or_overlay_destinations() {
    use std::os::unix::fs::symlink;
    for location in ["source", "cache", "destination"] {
        let f = ApplicationFixture::new();
        f.stage();
        let external = f._temp.path().join("external");
        match location {
            "source" => {
                let original = f.source.join("src/lib.rs");
                fs::rename(&original, &external).unwrap();
                symlink(&external, original).unwrap();
            }
            "cache" => {
                let original = f
                    .directory
                    .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib");
                fs::rename(&original, &external).unwrap();
                symlink(&external, original).unwrap();
            }
            _ => {
                fs::create_dir(&external).unwrap();
                fs::remove_dir_all(&f.target).unwrap();
                fs::create_dir(&f.target).unwrap();
                symlink(&external, f.target.join("debug")).unwrap();
            }
        }
        assert!(
            cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err(),
            "{location}"
        );
        if external.is_dir() {
            assert_eq!(fs::read_dir(external).unwrap().count(), 0);
        }
    }
}
