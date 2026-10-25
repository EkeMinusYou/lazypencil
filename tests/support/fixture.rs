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

pub fn process(path: &std::path::Path, record: serde_json::Value) {
    let table = path.with_extension("response.json");
    std::fs::write(
        &table,
        serde_json::to_vec(&serde_json::json!({"*": record})).unwrap(),
    )
    .unwrap();
    install(
        path,
        &["fixture-command".into(), table.to_str().unwrap().into()],
    );
}
