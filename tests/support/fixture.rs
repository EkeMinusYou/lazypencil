// Independent test targets share these roles but do not all invoke every role.
#![allow(dead_code)]

#[path = "../../xtask/src/test_support/install.rs"]
mod native;

pub fn helper() -> std::path::PathBuf {
    native::helper(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}

pub fn install(path: &std::path::Path, arguments: &[String]) {
    native::install(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        path,
        arguments,
    )
    .unwrap();
}
