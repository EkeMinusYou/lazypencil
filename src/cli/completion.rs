//! Shell hooks and bounded, bundled suggestions; never open a project or agent.
use crate::{Error, Result};
use clap_complete::{CompletionCandidate, Shell, env::Shells};

pub(super) fn registration(shell: Shell) -> Result<String> {
    let shells = Shells::builtins();
    let name = shell.to_string();
    let shell = shells
        .completer(&name)
        .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unsupported completion shell"))?;
    let mut bytes = Vec::new();
    // Resolve through PATH at completion time, so upgrading/replacing the
    // executable does not leave a hook bound to an obsolete binary location.
    shell
        .write_registration(
            super::COMPLETE_ENV,
            "lazypencil",
            "lazypencil",
            "lazypencil",
            &mut bytes,
        )
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| Error::new("IO_ERROR", error.to_string()))
}

pub(super) fn icon_packs() -> Vec<CompletionCandidate> {
    crate::icons::catalogue()
        .iter()
        .map(|pack| CompletionCandidate::new(pack.id.clone()))
        .collect()
}

pub(super) fn node_types() -> Vec<CompletionCandidate> {
    crate::document::NODE_TYPES
        .iter()
        .map(|name| CompletionCandidate::new(*name))
        .collect()
}

pub(super) fn guides() -> Vec<CompletionCandidate> {
    crate::guidance::guide_ids()
        .map(CompletionCandidate::new)
        .collect()
}

pub(super) fn styles() -> Vec<CompletionCandidate> {
    crate::guidance::bundled_style_ids()
        .unwrap_or_default()
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}
