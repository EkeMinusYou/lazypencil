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
