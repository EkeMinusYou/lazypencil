use crate::{Error, Result};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

static CLI_HANDLER: AtomicBool = AtomicBool::new(false);

pub(crate) fn cli_handler_installed() -> bool {
    CLI_HANDLER.load(Ordering::Relaxed)
}

pub fn token() -> CancellationToken {
    static TOKEN: OnceLock<CancellationToken> = OnceLock::new();
    TOKEN.get_or_init(CancellationToken::new).clone()
}
pub fn install_cli_handler() -> Result<()> {
    let cancel = token();
    ctrlc::set_handler(move || cancel.cancel())
        .map_err(|e| Error::new("IO_ERROR", format!("Cannot install interrupt handler: {e}")))?;
    CLI_HANDLER.store(true, Ordering::Relaxed);
    Ok(())
}

pub fn check(cancel: Option<&CancellationToken>) -> Result<()> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        Err(Error::new("CANCELLED", "Operation cancelled"))
    } else {
        Ok(())
    }
}
