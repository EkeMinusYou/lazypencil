//! Cache only the reviewed, file-only native pinch bridge build recipe.
use super::*;

const SCRIPT: &str = "build.rs";
const NATIVE: &str = "src/tui/gestures/macos.m";
// Review the script's inputs and output allowlist before accepting a new recipe.
const SCRIPT_SHA256: &str = "cefd5aefe1369c1c4a16470a3f8bf8352e0fc23a6646f07acdf7634abeb79f1a";

pub(super) fn capture(
    source: &Path,
    manifest: &Manifest,
    package: &toml::Value,
) -> Result<Option<Value>> {
    let configured = package["package"].get("build");
    if !source.join(SCRIPT).exists() {
        ensure!(
            configured.is_none_or(|value| value.as_bool() == Some(false)),
            "Custom build scripts disable application reuse"
        );
        return Ok(None);
    }
    ensure!(
        configured.is_none_or(|value| value.as_str() == Some(SCRIPT)),
        "Unreviewed application build script"
    );
    for name in [SCRIPT, NATIVE] {
        let entry = manifest
            .files
            .iter()
            .find(|entry| entry.path == name)
            .context("Untracked native bridge build input")?;
        let path = source.join(name);
        ensure!(
            !path.is_symlink() && util::file_digest(&path)? == entry.sha256,
            "Native bridge build input differs from its source manifest"
        );
    }
    ensure!(
        util::file_digest(&source.join(SCRIPT))? == SCRIPT_SHA256,
        "Review the changed application build script before reusing its outputs"
    );
    // The native cache already binds the runner image, Rust compiler and flags.
    // Also bind the tools selected by PATH and Xcode for this application script.
    let tools = if cfg!(target_os = "macos") {
        let tool = |name: &str| -> Result<Value> {
            let path = PathBuf::from(process::text(Process::new("which").arg(name))?.trim())
                .canonicalize()?;
            Ok(json!({"path":path,"sha256":util::file_digest(&path)?}))
        };
        let environment = [
            "CPATH",
            "C_INCLUDE_PATH",
            "OBJC_INCLUDE_PATH",
            "LIBRARY_PATH",
            "COMPILER_PATH",
            "DEVELOPER_DIR",
            "TOOLCHAINS",
        ]
        .map(|name| (name, std::env::var(name).ok()))
        .into_iter()
        .collect::<BTreeMap<_, _>>();
        json!({
            "clang":tool("clang")?,
            "clang_version":process::text(Process::new("clang").arg("--version"))?,
            "ar":tool("ar")?,
            "sdk":process::text(Process::new("xcrun").arg("--show-sdk-path"))?,
            "sdk_version":process::text(Process::new("xcrun").arg("--show-sdk-version"))?,
            "environment":environment,
        })
    } else {
        Value::Null
    };
    Ok(Some(json!({"recipe":"native-pinch-v1","tools":tools})))
}

pub(super) fn allowed(parts: &[&str]) -> bool {
    let ["build", package, rest @ ..] = parts else {
        return false;
    };
    let Some(suffix) = package.strip_prefix("lazypencil-") else {
        return false;
    };
    if suffix.len() != 16 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    match rest {
        ["build-script-build" | "invoked.timestamp" | "output" | "root-output" | "stderr"] => true,
        [file] => {
            *file == format!("build_script_build-{suffix}")
                || *file == format!("build_script_build-{suffix}.d")
        }
        ["out", "pinch.o" | "liblazypencil_pinch.a"] => true,
        _ => false,
    }
}
