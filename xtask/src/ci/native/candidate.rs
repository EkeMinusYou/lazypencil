//! Verify an unpacked candidate without access to native build/source trees.

use super::*;
use crate::{dist::binary, performance};

struct Hidden(Vec<(PathBuf, PathBuf)>);

impl Hidden {
    fn new(paths: &[PathBuf], temporary: &Path) -> Result<Self> {
        let mut hidden = Self(Vec::new());
        for (index, path) in paths.iter().enumerate() {
            let target = temporary.join(format!("candidate-hidden-{index}"));
            ensure!(
                !target.exists() && !target.is_symlink(),
                "Candidate isolation destination already exists"
            );
            ensure!(
                path.is_dir() && !path.is_symlink(),
                "Candidate isolation requires regular directories"
            );
            fs::rename(path, &target)?;
            hidden.0.push((path.clone(), target));
        }
        Ok(hidden)
    }

    fn restore(&mut self) -> Result<()> {
        while let Some((path, target)) = self.0.last() {
            fs::rename(target, path)
                .with_context(|| format!("Cannot restore {}", path.display()))?;
            self.0.pop();
        }
        Ok(())
    }
}

impl Drop for Hidden {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("Cannot restore candidate smoke inputs: {error:#}");
        }
    }
}

pub fn run(s: &Session) -> Result<()> {
    ensure!(
        !s.target.starts_with(&s.root) && !s.root.starts_with(&s.target),
        "Candidate build/source roots must be separate"
    );
    let archive = s.temporary.join("lazypencil-candidate.tar.gz");
    let result = binary::create(&binary::Arguments {
        executable: s.executable("release"),
        source_archive: s.temporary.join("native-source.tar.gz"),
        evidence: s.temporary.join("release-evidence"),
        output: archive.clone(),
    })?;
    let installed = s.temporary.join("candidate-install");
    let verified = binary::verify(&archive, Some(&installed))?;
    ensure!(
        verified["source_revision"] == std::env::var("GITHUB_SHA")?,
        "Candidate source differs from this hosted commit"
    );
    let executable = installed.join("bin/lazypencil");
    let inputs = tempfile::Builder::new()
        .prefix("candidate-input-")
        .tempdir_in(&s.temporary)?;
    fs::create_dir(inputs.path().join(".git"))?;
    fs::write(
        inputs.path().join("worker.js"),
        "/* @schema 2.21 */ return [{id:'pixel',type:'rectangle',width:16,height:16,fill:'#ffff00'}];\n",
    )?;
    fs::write(
        inputs.path().join("scene.pen"),
        util::encode(
            &json!({"version":"2.21","children":[{"id":"worker","type":"script","scriptUri":"worker.js","width":16,"height":16}]}),
        )?,
    )?;
    let image = s.temporary.join("candidate-smoke.png");
    let mut hidden = Hidden::new(&[s.root.clone(), s.target.clone()], &s.temporary)?;
    let smoke = (|| -> Result<Value> {
        let version = process::checked(
            Command::new(&executable)
                .arg("--version")
                .current_dir(inputs.path()),
            Duration::from_secs(30),
        )?;
        ensure!(
            String::from_utf8(version)?.trim()
                == format!(
                    "lazypencil {}",
                    verified["package"]["version"]
                        .as_str()
                        .context("Missing package version")?
                ),
            "Installed version mismatch"
        );
        let stdout = process::checked(
            Command::new(&executable)
                .current_dir(inputs.path())
                .env("LAZYPENCIL_CONFIG_DIR", inputs.path().join("configuration"))
                .arg("--json")
                .arg("--data-dir")
                .arg(inputs.path().join("private"))
                .args(["render", "scene.pen", "--output"])
                .arg(&image),
            Duration::from_secs(60),
        )?;
        let report: Value = serde_json::from_slice(&stdout)?;
        ensure!(
            report["node_count"] == 2
                && report["diagnostics"] == json!([])
                && report["dynamic_artifacts"] == json!([])
                && report["script_executions"]
                    .as_array()
                    .is_some_and(|items| items.len() == 1),
            "Installed candidate did not execute and render the generated node"
        );
        performance::png(&image, &report, 16, 16)?;
        Ok(report)
    })();
    hidden.restore()?;
    let report = smoke?;
    util::publish(
        &s.temporary.join("candidate-smoke.json"),
        &util::encode(&json!({
            "source_commit":std::env::var("GITHUB_SHA")?,"candidate":result,"verified":verified,
            "source_tree_available":false,"native_build_tree_available":false,"image":"candidate-smoke.png","render":report,
        }))?,
        false,
    )?;
    println!(
        "Verified unpacked candidate version and native JavaScript pixels with both build trees unavailable"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_build_trees_are_restored_on_error_panic_and_partial_setup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        for path in [&source, &target] {
            fs::create_dir(path).unwrap();
            fs::write(path.join("keep"), b"original").unwrap();
        }
        assert!(Hidden::new(&[source.clone(), temp.path().join("missing")], temp.path()).is_err());
        assert_eq!(fs::read(source.join("keep")).unwrap(), b"original");
        let panic = std::panic::catch_unwind(|| {
            let _hidden = Hidden::new(&[source.clone(), target.clone()], temp.path()).unwrap();
            assert!(!source.exists() && !target.exists());
            panic!("simulated interrupted smoke");
        });
        assert!(panic.is_err());
        for path in [&source, &target] {
            assert_eq!(fs::read(path.join("keep")).unwrap(), b"original");
        }
        let result = (|| -> Result<()> {
            let _hidden = Hidden::new(&[source.clone(), target.clone()], temp.path())?;
            anyhow::bail!("simulated smoke failure")
        })();
        assert!(result.is_err());
        assert!(source.is_dir() && target.is_dir());
    }
}
