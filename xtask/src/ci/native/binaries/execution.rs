//! Bind execution-host evidence to the unchanged producer receipt.
use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct Execution {
    schema: u32,
    receipt_sha256: String,
    build: Value,
    host: Value,
}

fn macos_15(version: &Value) -> bool {
    let Some(version) = version.as_str() else {
        return false;
    };
    let parts = version.split('.').collect::<Vec<_>>();
    (2..=3).contains(&parts.len())
        && parts[0] == "15"
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn compatible(build: &Value, host: &Value) -> Result<()> {
    if build == host {
        return Ok(());
    }
    // Compiler caches remain image-bound. These consumers execute immutable
    // binaries and cannot invoke Cargo. Permit only hosted macOS 15 arm64 OS
    // updates; retain exact compiler, flags, profiles, paths, source and run.
    ensure!(
        build["schema"] == 2
            && host["schema"] == 2
            && build["os"] == "macos"
            && host["os"] == "macos"
            && build["arch"] == "aarch64"
            && host["arch"] == "aarch64"
            && macos_15(&build["os_version"])
            && macos_15(&host["os_version"]),
        "Compiled catalog source or compiler conditions differ"
    );
    for image in [&build["image"], &host["image"]] {
        ensure!(
            image.as_str().is_some_and(|image| !image.is_empty()
                && image.len() <= 100
                && image
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))),
            "Invalid execution image"
        );
    }
    let mut expected = host.clone();
    expected["image"] = build["image"].clone();
    expected["os_version"] = build["os_version"].clone();
    ensure!(
        build == &expected,
        "Compiled catalog source or compiler conditions differ"
    );
    Ok(())
}

impl Execution {
    pub(super) fn new(build: &Value, host: &Value, receipt_sha256: &str) -> Result<Self> {
        let execution = Self {
            schema: 1,
            receipt_sha256: receipt_sha256.to_owned(),
            build: build.clone(),
            host: host.clone(),
        };
        execution.validate(build, receipt_sha256)?;
        Ok(execution)
    }

    pub(super) fn validate(&self, build: &Value, receipt_sha256: &str) -> Result<()> {
        ensure!(
            self.schema == 1 && self.receipt_sha256 == receipt_sha256 && self.build == *build,
            "Execution evidence belongs to another compiled catalog"
        );
        compatible(build, &self.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_updates_record_both_hosts_without_changing_compiler_or_binary_identity() {
        let build = json!({"schema":2,"os":"macos","arch":"aarch64",
            "os_version":"15.7.9","image":"20260907.0337.1",
            "commit":"source","run":"run","compiler":"pinned compiler",
            "source_root":"/source","target_root":"/target","optimized_tests":true,
            "test_profile":["original"],"flags":{"RUSTFLAGS":"original"}});
        let mut host = build.clone();
        host["os_version"] = json!("15.8.2");
        host["image"] = json!("20261028.0489.1");
        let receipt = "a".repeat(64);
        let execution = Execution::new(&build, &host, &receipt).unwrap();
        assert_eq!(execution.build, build);
        assert_eq!(execution.host, host);
        execution.validate(&build, &receipt).unwrap();
        assert!(execution.validate(&build, &"b".repeat(64)).is_err());
        for (field, value) in [
            ("schema", json!(1)),
            ("os", json!("linux")),
            ("arch", json!("x86_64")),
            ("os_version", json!("16.0")),
            ("os_version", json!("15.bad")),
            ("os_version", Value::Null),
            ("image", json!("")),
            ("image", json!("unknown\nimage")),
            ("commit", json!("other")),
            ("run", json!("other")),
            ("compiler", json!("other compiler")),
            ("source_root", json!("/other")),
            ("target_root", json!("/other")),
            ("optimized_tests", json!(false)),
            ("test_profile", json!(["other"])),
            ("flags", json!({"RUSTFLAGS":"other"})),
        ] {
            let mut changed = host.clone();
            changed[field] = value;
            assert!(
                Execution::new(&build, &changed, &receipt).is_err(),
                "{field}"
            );
        }
        let mut changed = execution;
        changed.build["compiler"] = json!("tampered");
        assert!(changed.validate(&build, &receipt).is_err());
    }
}
