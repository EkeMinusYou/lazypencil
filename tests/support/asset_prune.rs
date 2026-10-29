use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

struct Fixture {
    temp: TempDir,
    root: PathBuf,
    file: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap().join("project");
        fs::create_dir(&root).unwrap();
        let file = root.join("scene.pen");
        fs::write(&file, r#"{"version":"2.21","children":[]}"#).unwrap();
        Self { temp, root, file }
    }
    fn data(&self) -> PathBuf {
        self.temp.path().join("data")
    }
    fn locks(&self) -> PathBuf {
        self.temp.path().join("locks")
    }
    fn asset(&self, bytes: &[u8], extension: &str) -> (String, PathBuf) {
        let uri = format!(
            "assets/lazypencil/{}.{extension}",
            lazypencil::document::hash(bytes)
        );
        let path = self.root.join(&uri);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        (uri, path)
    }
    fn bundle(&self) -> PathBuf {
        let members = BTreeMap::from([
            ("nested/source.js", b"return [];".as_slice()),
            ("image.png", b"image".as_slice()),
        ]);
        let files = members
            .iter()
            .map(|(name, bytes)| {
                (
                    *name,
                    json!({"sha256":lazypencil::document::hash(bytes),"bytes":bytes.len()}),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let index = serde_json::to_vec(&json!({"version":1,"files":files})).unwrap();
        let path = self.root.join(format!(
            "assets/lazypencil/{}.bundle",
            lazypencil::document::hash(&index)
        ));
        for (name, bytes) in members {
            fs::create_dir_all(path.join(name).parent().unwrap()).unwrap();
            fs::write(path.join(name), bytes).unwrap();
        }
        fs::write(path.join("bundle.json"), index).unwrap();
        path
    }
    fn command(&self, revision: Option<&str>) -> Value {
        struct Owner(Child);
        impl Drop for Owner {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        // Disk-backed output cannot deadlock a child behind an undrained pipe.
        // It lives outside the scanned project and all tests isolate lock/config.
        let output = tempfile::NamedTempFile::new_in(self.temp.path()).unwrap();
        let stderr = tempfile::NamedTempFile::new_in(self.temp.path()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        command
            .env("LAZYPENCIL_LOCK_DIR", self.locks())
            .env("LAZYPENCIL_CONFIG_DIR", self.temp.path().join("config"))
            .arg("--json")
            .arg("--data-dir")
            .arg(self.data())
            .args(["assets", "prune"])
            .arg(&self.file)
            .arg("--project-root")
            .arg(&self.root)
            .stdin(Stdio::null())
            .stdout(output.reopen().unwrap())
            .stderr(stderr.reopen().unwrap());
        if let Some(revision) = revision {
            command.arg("--expected-revision").arg(revision);
        }
        let mut child = Owner(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(35);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "Asset maintenance exceeded its response deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let text = fs::read(output.path()).unwrap();
        let value: Value = serde_json::from_slice(&text).unwrap_or_else(|error| {
            panic!(
                "{error}: {} / {}",
                String::from_utf8_lossy(&text),
                fs::read_to_string(stderr.path()).unwrap()
            )
        });
        assert_eq!(status.success(), value.get("error").is_none(), "{value}");
        value
    }
    fn checkpoint(&self, owner: &Path, before: Value, state: &str, legacy: bool) {
        let id = uuid::Uuid::new_v4().to_string();
        let namespace = lazypencil::document::hash(owner.as_os_str().as_encoded_bytes());
        let path = self
            .data()
            .join("documents")
            .join(namespace)
            .join("history")
            .join(&id);
        fs::create_dir_all(&path).unwrap();
        let before = before.to_string();
        let after = r#"{"version":"2.21","children":[]}"#;
        let mut metadata = json!({"id":id,"sequence":1,"created_at":"2026-10-30T00:00:00Z","request_id":"test","fingerprint":"fixture","state":state,"before_revision":lazypencil::document::hash(before.as_bytes()),"after_revision":lazypencil::document::hash(after.as_bytes()),"result":{},"document_path":owner});
        if legacy {
            metadata.as_object_mut().unwrap().remove("document_path");
        }
        fs::write(path.join("before.pen"), before).unwrap();
        fs::write(path.join("after.pen"), after).unwrap();
        fs::write(path.join("metadata.json"), metadata.to_string()).unwrap();
    }
}

fn candidates(value: &Value) -> Vec<PathBuf> {
    value["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("{value}"))
        .iter()
        .map(|item| PathBuf::from(item["path"].as_str().unwrap()))
        .collect()
}

#[test]
fn review_and_apply_keep_sibling_disabled_theme_and_unknown_metadata_references() {
    let fixture = Fixture::new();
    let (inactive, inactive_path) = fixture.asset(b"inactive image", "png");
    let (disabled, disabled_path) = fixture.asset(b"disabled image", "png");
    let (sibling, sibling_path) = fixture.asset(b"sibling image", "png");
    let (_, orphan) = fixture.asset(b"orphan code", "glsl");
    let bundle = fixture.bundle();
    fs::write(&fixture.file, json!({"version":"2.21","themes":{"Mode":["light","dark"]},"variables":{"hidden":{"type":"string","value":[{"value":"none","theme":{"Mode":"light"}},{"value":inactive,"theme":{"Mode":"dark"}}]}},"children":[{"type":"rectangle","id":"disabled","enabled":false,"width":1,"height":1,"fill":{"type":"image","url":disabled}}]}).to_string()).unwrap();
    fs::write(
        fixture.root.join("sibling.pen"),
        json!({"version":"2.19","metadata":{"future":{"asset":sibling}},"children":[]}).to_string(),
    )
    .unwrap();
    let before = fs::read(&fixture.file).unwrap();
    let review = fixture.command(None);
    assert_eq!(review["uncertain"], json!([]), "{review}");
    assert_eq!(
        candidates(&review)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [orphan.clone(), bundle.clone()].into_iter().collect()
    );
    assert!(
        orphan.exists() && bundle.exists(),
        "Review must not remove assets"
    );
    let result = fixture.command(review["revision"].as_str());
    assert_eq!(result["errors"], json!([]), "{result}");
    assert_eq!(result["deleted"].as_array().unwrap().len(), 2);
    assert!(!orphan.exists() && !bundle.exists());
    assert!(inactive_path.exists() && disabled_path.exists() && sibling_path.exists());
    assert_eq!(fs::read(&fixture.file).unwrap(), before);
    assert!(candidates(&fixture.command(None)).is_empty());
}

#[test]
fn every_checkpoint_state_and_legacy_owner_protect_assets_across_document_namespaces() {
    let fixture = Fixture::new();
    let (_, orphan) = fixture.asset(b"unreferenced", "glsl");
    let mut retained = Vec::new();
    for (index, state) in ["prepared", "committed", "aborted"].into_iter().enumerate() {
        let (uri, path) = fixture.asset(format!("history {state}").as_bytes(), "png");
        let owner = fixture.root.join(format!("owner-{index}.pen"));
        fs::write(&owner, r#"{"version":"2.21","children":[]}"#).unwrap();
        fixture.checkpoint(
            &owner,
            json!({"version":"2.21","children":[],"metadata":{"previous_asset":uri}}),
            state,
            index == 1,
        );
        retained.push(path);
    }
    let review = fixture.command(None);
    assert_eq!(review["uncertain"], json!([]), "{review}");
    assert_eq!(candidates(&review), vec![orphan.clone()]);
    let result = fixture.command(review["revision"].as_str());
    assert_eq!(result["errors"], json!([]), "{result}");
    assert!(!orphan.exists());
    assert!(retained.iter().all(|path| path.exists()));
}

#[test]
fn changed_sources_new_documents_and_modified_asset_bytes_reject_a_reviewed_revision() {
    for change in 0..3 {
        let fixture = Fixture::new();
        let (_, orphan) = fixture.asset(b"original", "glsl");
        let review = fixture.command(None);
        assert_eq!(candidates(&review), vec![orphan.clone()]);
        match change {
            0 => fs::write(
                &fixture.file,
                "{\"version\":\"2.21\",\"children\":[],\"metadata\":{\"changed\":true}}",
            )
            .unwrap(),
            1 => fs::write(
                fixture.root.join("new.pen"),
                "{\"version\":\"2.21\",\"children\":[]}",
            )
            .unwrap(),
            _ => fs::write(&orphan, b"externally replaced").unwrap(),
        }
        let result = fixture.command(review["revision"].as_str());
        assert_eq!(result["error"]["code"], "REVISION_CONFLICT", "{result}");
        assert!(orphan.exists());
    }
}

#[test]
fn dynamic_scripts_unknown_legacy_owners_and_symbolic_links_retain_uncertain_assets() {
    for condition in 0..3 {
        let fixture = Fixture::new();
        let (_, orphan) = fixture.asset(b"retain when uncertain", "glsl");
        match condition {
            0 => {
                fs::write(fixture.root.join("generator.js"), "return [];").unwrap();
                fs::write(&fixture.file, json!({"version":"2.21","children":[{"id":"dynamic","type":"script","scriptUri":"generator.js","width":1,"height":1}]}).to_string()).unwrap();
            }
            1 => fixture.checkpoint(
                &fixture.root.join("removed.pen"),
                json!({"version":"2.21","children":[]}),
                "committed",
                true,
            ),
            _ => std::os::unix::fs::symlink(fixture.temp.path(), fixture.root.join("redirected"))
                .unwrap(),
        }
        let review = fixture.command(None);
        assert!(
            !review["uncertain"].as_array().unwrap().is_empty(),
            "{review}"
        );
        assert!(candidates(&review).is_empty());
        let result = fixture.command(review["revision"].as_str());
        assert_eq!(result["errors"], json!([]), "{result}");
        assert!(orphan.exists());
    }
}

#[test]
fn busy_activity_and_unowned_or_tampered_assets_are_never_deleted() {
    use fs2::FileExt;
    let fixture = Fixture::new();
    let (_, owned) = fixture.asset(b"valid", "glsl");
    let (_, tampered) = fixture.asset(b"original", "png");
    fs::write(&tampered, b"different").unwrap();
    let unknown = fixture.root.join("assets/lazypencil/custom.txt");
    fs::write(&unknown, b"user data").unwrap();
    let bundle = fixture.bundle();
    fs::write(bundle.join("foreign.txt"), b"unlisted data").unwrap();
    let review = fixture.command(None);
    assert_eq!(candidates(&review), vec![owned.clone()], "{review}");
    fs::create_dir_all(fixture.locks()).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.locks().join(format!(
            "{}.lock",
            lazypencil::document::hash(b"lazypencil-managed-assets-activity-v1")
        )))
        .unwrap();
    FileExt::try_lock_shared(&lock).unwrap();
    let busy = fixture.command(review["revision"].as_str());
    assert_eq!(busy["error"]["code"], "LOCK_UNAVAILABLE", "{busy}");
    assert!(owned.exists());
    drop(lock);
    let result = fixture.command(review["revision"].as_str());
    assert_eq!(result["errors"], json!([]), "{result}");
    assert!(!owned.exists());
    assert_eq!(fs::read(&unknown).unwrap(), b"user data");
    assert_eq!(fs::read(&tampered).unwrap(), b"different");
    assert_eq!(
        fs::read(bundle.join("foreign.txt")).unwrap(),
        b"unlisted data"
    );
}

#[test]
fn the_bound_document_is_a_retention_root_inside_managed_storage_or_without_a_pen_extension() {
    for managed in [false, true] {
        let mut fixture = Fixture::new();
        let (uri, retained) = fixture.asset(b"referenced", "png");
        let (_, orphan) = fixture.asset(b"orphan", "glsl");
        let document = json!({"version":"2.21","children":[],"metadata":{"asset":uri}}).to_string();
        if managed {
            fixture.file = fixture.asset(document.as_bytes(), "lib.pen").1;
        } else {
            fs::remove_file(&fixture.file).unwrap();
            fixture.file = fixture.root.join("unconventional.design");
            fs::write(&fixture.file, document).unwrap();
        }
        let before = fs::read(&fixture.file).unwrap();
        let review = fixture.command(None);
        assert_eq!(candidates(&review), vec![orphan.clone()], "{review}");
        let result = fixture.command(review["revision"].as_str());
        assert_eq!(result["errors"], json!([]), "{result}");
        assert!(!orphan.exists());
        assert!(retained.exists());
        assert_eq!(fs::read(&fixture.file).unwrap(), before);
    }
}

#[test]
fn the_cli_reviews_pending_cleanup_and_restores_newly_referenced_quarantined_assets() {
    for referenced in [false, true] {
        let fixture = Fixture::new();
        let bytes = b"quarantined";
        let (uri, asset) = fixture.asset(bytes, "glsl");
        let operation = uuid::Uuid::new_v4().to_string();
        let directory = asset
            .parent()
            .unwrap()
            .join(format!(".lazypencil-trash-{operation}"));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("journal.json"), json!({"version":1,"operation":operation,"root":fixture.root,"document":fixture.file,"asset":{"path":asset,"digest":lazypencil::document::hash(bytes),"bundle":false,"files":{"":{"sha256":lazypencil::document::hash(bytes),"bytes":bytes.len()}}}}).to_string()).unwrap();
        fs::rename(&asset, directory.join("payload")).unwrap();
        if referenced {
            fs::write(
                &fixture.file,
                json!({"version":"2.21","children":[],"metadata":{"asset":uri}}).to_string(),
            )
            .unwrap();
        }
        let review = fixture.command(None);
        assert!(candidates(&review).is_empty());
        assert_eq!(
            review["pending"][0]["action"],
            if referenced { "restore" } else { "cleanup" },
            "{review}"
        );
        assert!(directory.join("payload").exists());
        let result = fixture.command(review["revision"].as_str());
        assert_eq!(result["errors"], json!([]), "{result}");
        assert!(!directory.exists());
        assert_eq!(asset.exists(), referenced);
        if referenced {
            assert_eq!(fs::read(asset).unwrap(), bytes);
        }
    }
}
