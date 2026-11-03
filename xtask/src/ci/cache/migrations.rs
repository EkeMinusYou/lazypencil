//! Reviewed bridges for retained cache payloads, separate from current identities.
//!
//! Retire a bridge only after its source keys are no longer retained or used by
//! supported workflows. A source edit alone does not establish cache expiry.

use super::*;

pub(super) fn previous_native_tree(
    tree: &str,
    compiler: &str,
    flags: &BTreeMap<String, String>,
) -> Option<&'static str> {
    let baseline = BTreeMap::from([
        ("CARGO_BUILD_JOBS".into(), "1".into()),
        ("CARGO_INCREMENTAL".into(), "0".into()),
        ("CARGO_PROFILE_DEV_DEBUG".into(), "0".into()),
        ("CARGO_PROFILE_TEST_DEBUG".into(), "0".into()),
    ]);
    let audited = compiler.lines().any(|s| s == "release: 1.94.0")
        && compiler
            .lines()
            .any(|s| s == "commit-hash: 4a4ef493e3a1488c6e321570238084b38948f6db")
        && *flags == baseline;
    if !audited {
        return None;
    }
    match tree {
        // The 0.1.0 release changes only this package's version in Cargo.toml/lock.
        // Dependency versions, native inputs and compiler flags are identical.
        // Application outputs are pruned and must pass their own source checks.
        "9d9ec938606c1c29c9ac36d0868af4b0bf3d61be85d66449287258edbb680e1b" => {
            Some("4cd30528eaee7f6025192ea093fe203e7c59be06b95b84c8cf0c00f66790f6fa")
        }
        // The optional Fontations readers leave native inputs unchanged.
        // Restore dependencies only; Cargo must recheck unified Rust features.
        "11d1597cbd47f826b985b5e514b42c0ff8aa9d4a3d378264e88ac38781b0e160" => {
            Some("80b3b3fc770be2245a731ad66682615c3b4979af5c410c5bf196d10c61aba68b")
        }
        // The tool migration adds only a terminal test dependency and a Cargo alias.
        // Production dependency declarations, native sources and build flags are unchanged.
        // Removing the retired Shell entry points does not change dependency build inputs.
        // New byte-preservation attributes apply only to xtask notice files.
        "b8192b978f7a069656ed03fe0fc8bd0f97725ce33cd9f1787360534b73e8b397" => {
            Some("b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d")
        }
        "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7" => {
            Some("54c7d2d9cc2e1b5e2d3a0297c8648f27b1aff64c76c685a30e63864b41d60a9f")
        }
        "b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d" => {
            Some("c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7")
        }
        _ => None,
    }
}

impl ApplicationIdentity {
    /// One reviewed release-only transition. All native/default/test inputs
    /// remain bound to the old fingerprint; callers may restore debug only.
    pub fn previous_debug_identity(&self) -> Result<Option<Self>> {
        if self.release_only
            || self.recipe["release_recipe"] != "root-rustc-release-map-app-cgu16-v2"
        {
            return Ok(None);
        }
        let mut old = self.clone();
        old.recipe["release_recipe"] = json!("root-rustc-release-map-v1");
        old.fingerprint = util::digest(stable_json(&old.recipe));
        old.primary = self.primary.replace(&self.fingerprint, &old.fingerprint);
        old.checkpoint = self.checkpoint.replace(&self.fingerprint, &old.fingerprint);
        (old.compatible_primary, old.compatible_prefix) = old.compatible_keys(false)?;
        Ok(Some(old))
    }
}
