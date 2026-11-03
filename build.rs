use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=src/tui/gestures/macos.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("CARGO_FEATURE_TUI").is_none()
    {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = output.join("pinch.o");
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => panic!("Unsupported macOS architecture"),
    };
    assert!(
        Command::new("clang")
            .args([
                "-arch",
                architecture,
                "-fobjc-arc",
                "-fblocks",
                "-O2",
                "-c",
                "src/tui/gestures/macos.m",
                "-o"
            ])
            .arg(&object)
            .status()
            .unwrap()
            .success(),
        "Cannot build native pinch input"
    );
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(output.join("liblazypencil_pinch.a"))
            .arg(object)
            .status()
            .unwrap()
            .success(),
        "Cannot archive native pinch input"
    );
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=lazypencil_pinch");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
}
