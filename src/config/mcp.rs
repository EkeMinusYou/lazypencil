//! User-owned MCP connection settings.

use super::{UserSource, invalid, user_config_path};
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(super) const TEMPLATE: &str = "\n# [mcp]\n# Retained payload across all documents in one MCP connection, in MiB.\n# retained_mib = 256\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    /// Shared budget for retained calculations, listings and dynamic caches per connection.
    #[schemars(range(min = 1, max = 2048))]
    pub retained_mib: usize,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { retained_mib: 256 }
    }
}

impl McpSettings {
    pub(super) fn validate(self) -> Result<()> {
        if !(1..=2048).contains(&self.retained_mib) {
            return Err(invalid("mcp.retained_mib must be within 1..2048"));
        }
        Ok(())
    }

    pub(crate) fn retained_bytes(self) -> usize {
        self.retained_mib * 1024 * 1024
    }
}

pub(crate) fn load() -> Result<McpSettings> {
    let source = UserSource::load(Some(user_config_path()?), true)?;
    source.verify()?;
    Ok(source.settings.mcp)
}
