//! Start reproductions from Cargo's completed outputs during one complete build.
use super::*;
use std::{collections::BTreeSet, sync::mpsc};

#[derive(Default)]
struct Artifacts {
    binaries: BTreeMap<String, Binary>,
    application: Option<Binary>,
    pending: Vec<u8>,
}

impl Artifacts {
    fn observe(&mut self, bytes: &[u8], target: &Path) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        ensure!(
            self.pending.len() <= 64 * 1024 * 1024,
            "Cargo event exceeds its output limit"
        );
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = consumed + end;
            if end != consumed {
                let event: Value = serde_json::from_slice(&self.pending[consumed..end])?;
                if let Some((name, binary)) = artifact(&event, target)? {
                    if let Some(name) = name {
                        ensure!(
                            self.binaries.insert(name, binary).is_none(),
                            "Duplicate test executable"
                        );
                    } else {
                        ensure!(
                            self.application.replace(binary).is_none(),
                            "Duplicate application executable"
                        );
                    }
                }
            }
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    fn catalog(&self, commit: &str) -> Catalog {
        Catalog {
            source_commit: commit.into(),
            binaries: self.binaries.clone(),
        }
    }

    fn ready(&self, required: &BTreeSet<String>) -> bool {
        self.application.is_some() && required.iter().all(|name| self.binaries.contains_key(name))
    }
}

fn artifact(event: &Value, target: &Path) -> Result<Option<(Option<String>, Binary)>> {
    if event["reason"] != "compiler-artifact"
        || !event["package_id"]
            .as_str()
            .is_some_and(|id| id.contains("#lazypencil@"))
    {
        return Ok(None);
    }
    let Some(path) = event["executable"].as_str() else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let name = if event["profile"]["test"] == true {
        ensure!(
            path.starts_with(target.join("debug/deps")),
            "Unexpected test executable path"
        );
        Some(
            if event["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.contains(&json!("lib")))
            {
                "lib".into()
            } else {
                event["target"]["name"]
                    .as_str()
                    .context("Test target name required")?
                    .into()
            },
        )
    } else if path
        == target.join("debug").join(if cfg!(windows) {
            "lazypencil.exe"
        } else {
            "lazypencil"
        })
    {
        None
    } else {
        return Ok(None);
    };
    Ok(Some((
        name,
        Binary {
            sha256: util::file_digest(&path)?,
            path,
        },
    )))
}

fn early_targets(suite: &str, targets: &[String]) -> BTreeSet<String> {
    // These targets are consumed by FocusReproduce. Other selected targets
    // remain in the complete Cargo build and final consumers, without delaying
    // the first reproduction. Standalone focus retains its full build plan.
    let mut required: BTreeSet<_> = targets
        .iter()
        .filter(|name| {
            [
                "agent",
                "user_agent_config",
                "graphics",
                "tui",
                "user_config",
                "font_sources",
                "native_layout",
                "conversation_history",
                "mcp",
                "script_scope",
            ]
            .contains(&name.as_str())
        })
        .cloned()
        .collect();
    required.insert("lib".into());
    if suite == "dynamic" {
        required.insert("dynamic_render".into());
    }
    if ["dynamic", "shader"].contains(&suite) {
        required.insert("shader_uniforms".into());
    }
    required
}

pub(super) fn catalog(raw: &[u8], target: &Path, commit: &str) -> Result<Catalog> {
    let mut events = Artifacts::default();
    events.observe(raw, target)?;
    ensure!(events.pending.is_empty(), "Incomplete Cargo event at EOF");
    Ok(events.catalog(commit))
}

pub(super) fn build_and_reproduce(s: &Session, suite: &str, targets: &[String]) -> Result<()> {
    // Preserve distinct optional-feature builds before entering the single
    // default graph. Its executable must remain immutable while tests use it.
    for command in commands::plan_with_core(&s.root, suite, targets, true)?.build {
        if !command.iter().any(|arg| arg == "--no-run") {
            s.execute(&command, 2400, "focus-build.log", true)?;
        }
    }
    let required = early_targets(suite, targets);
    let commit = std::env::var("GITHUB_SHA")?;
    let (ready, receive) = mpsc::sync_channel(1);
    parallel::run(vec![
        (
            "complete test compilation",
            Box::new(move || {
                let [_, tests] = commands::catalog_builds(&[]);
                let mut events = Artifacts::default();
                let mut published = false;
                let started = Instant::now();
                s.capture_observed(
                    &mut s.command(&tests),
                    2700,
                    "default-build.jsonl",
                    |bytes| {
                        let result = (|| -> Result<()> {
                            events.observe(bytes, &s.target)?;
                            if !published && events.ready(&required) {
                                let catalog = events.catalog(&commit);
                                util::publish(
                                    &s.temporary.join("early-test-executables.json"),
                                    &util::encode(&catalog)?,
                                    false,
                                )?;
                                println!(
                                    "Early compiler outputs ready after {:.3}s: {}",
                                    started.elapsed().as_secs_f64(),
                                    required.iter().cloned().collect::<Vec<_>>().join(", ")
                                );
                                ready.send(()).context("Reproduction receiver closed")?;
                                published = true;
                            }
                            Ok(())
                        })();
                        result.map_err(|error| std::io::Error::other(error.to_string()))
                    },
                )?;
                ensure!(events.pending.is_empty(), "Incomplete Cargo event at EOF");
                ensure!(
                    published,
                    "Cargo did not report every reproduction executable and application"
                );
                let catalog = events.catalog(&commit);
                catalog.validate(&s.target, &commit, &["mcp", "tui", "optional_icons"])?;
                // Source and compiler bytes still have to match after Cargo exits.
                let early: Catalog = util::json(&s.temporary.join("early-test-executables.json"))?;
                early.validate(&s.target, &commit, &[])?;
                for (name, binary) in early.binaries {
                    ensure!(
                        catalog
                            .binaries
                            .get(&name)
                            .is_some_and(|current| current.sha256 == binary.sha256),
                        "Compiler replaced a running early test executable"
                    );
                }
                let application = events
                    .application
                    .context("Missing compiler-reported application")?;
                ensure!(
                    util::file_digest(&application.path)? == application.sha256,
                    "Compiler replaced the running application"
                );
                s.publish_catalog(&catalog)
            }),
        ),
        (
            "affected reproductions",
            Box::new(move || {
                // The producer owns the only sender, including every error/panic
                // path. A failed build wakes this receiver immediately; the upper
                // bound also covers a producer which has not yet emitted a target.
                receive
                    .recv_timeout(Duration::from_secs(2760))
                    .context("Compiler did not provide ready reproduction outputs")?;
                s.run(Arguments {
                    stage: Stage::FocusReproduce,
                    suite: suite.into(),
                    targets: serde_json::to_string(targets)?,
                })
            }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_cargo_events_require_current_cli_and_tests_and_reject_changed_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().canonicalize().unwrap();
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        let library = target.join("debug/deps/library-test");
        let cli = target.join("debug").join(if cfg!(windows) {
            "lazypencil.exe"
        } else {
            "lazypencil"
        });
        fs::write(&library, b"current library test").unwrap();
        fs::write(&cli, b"current application").unwrap();
        let event = |path: &Path, test: bool, kind: &str| {
            format!(
                "{}\n",
                json!({"reason":"compiler-artifact","package_id":"path+file:///fixture#lazypencil@0.1.0",
                "target":{"name":"lazypencil","kind":[kind]},"profile":{"test":test},"executable":path})
            )
        };
        let mut artifacts = Artifacts::default();
        let raw = event(&library, true, "lib");
        let split = raw.len() / 2;
        artifacts
            .observe(&raw.as_bytes()[..split], &target)
            .unwrap();
        assert!(artifacts.binaries.is_empty());
        artifacts
            .observe(&raw.as_bytes()[split..], &target)
            .unwrap();
        let required = BTreeSet::from(["lib".to_owned()]);
        assert!(
            !artifacts.ready(&required),
            "A stale CLI on disk is insufficient"
        );
        artifacts
            .observe(event(&cli, false, "bin").as_bytes(), &target)
            .unwrap();
        assert!(artifacts.ready(&required));
        let compiled = artifacts.catalog("current-source");
        compiled
            .validate(&target, "current-source", &["lib"])
            .unwrap();
        fs::write(&library, b"changed after compiler event").unwrap();
        assert!(
            compiled
                .validate(&target, "current-source", &["lib"])
                .is_err()
        );
        assert!(artifacts.observe(raw.as_bytes(), &target).is_err());
        assert!(catalog(raw.trim_end().as_bytes(), &target, "source").is_err());
    }
}
