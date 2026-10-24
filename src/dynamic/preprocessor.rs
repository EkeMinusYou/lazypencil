//! The GHA-built component has no host imports except its bounded memory.

use base64::Engine;
use std::sync::LazyLock;

const MODULE: &[u8] = include_bytes!("../../resources/shader-preprocessor/module.wasm");
static ENCODED: LazyLock<String> =
    LazyLock::new(|| base64::engine::general_purpose::STANDARD.encode(MODULE));
static HASH: LazyLock<String> = LazyLock::new(|| crate::document::hash(MODULE));

pub(super) fn encoded() -> &'static str {
    &ENCODED
}

pub(super) fn hash() -> &'static str {
    &HASH
}

#[test]
fn embedded_shader_preprocessor_matches_its_verified_component() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../resources/shader-preprocessor/build.json"
    ))
    .unwrap();
    assert_eq!(report["wasm_sha256"], hash());
    assert_eq!(report["wasm_bytes"], MODULE.len());
    assert_eq!(report["memory_maximum_bytes"], 64 * 1024 * 1024);
}
