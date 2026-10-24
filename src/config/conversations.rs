//! User-owned, opt-in limits for reviewed local conversation retention.

use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(super) const TEMPLATE: &str = "\n[conversations.retention]\n# Limits apply when conversations prune is explicitly reviewed and committed.\n# Omitted limits keep all local conversations. Busy sessions, unsent drafts,\n# pending images and incomplete continuity are always protected.\n# max_age_days = 90\n# max_total_bytes = 1073741824\nkeep_recent_per_document = 1\n";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ConversationSettings {
    pub retention: RetentionSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionSettings {
    /// Maximum age in whole days since the last local metadata/file write.
    #[schemars(range(min = 1, max = 36500))]
    pub max_age_days: Option<u32>,
    /// Soft aggregate limit for conversation data across every document store.
    #[schemars(range(min = 1))]
    pub max_total_bytes: Option<u64>,
    /// Recent conversations kept in addition to mandatory protection rules.
    #[schemars(range(min = 0, max = 1000))]
    pub keep_recent_per_document: usize,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        Self {
            max_age_days: None,
            max_total_bytes: None,
            keep_recent_per_document: 1,
        }
    }
}

impl RetentionSettings {
    pub fn validate(&self) -> Result<()> {
        if self
            .max_age_days
            .is_some_and(|days| !(1..=36500).contains(&days))
            || self.max_total_bytes == Some(0)
            || self.keep_recent_per_document > 1000
        {
            return Err(Error::new(
                "CONFIGURATION_ERROR",
                "Conversation retention requires age 1..36500 days, positive bytes, and 0..1000 recent conversations per document",
            ));
        }
        Ok(())
    }

    pub fn has_limits(&self) -> bool {
        self.max_age_days.is_some() || self.max_total_bytes.is_some()
    }
}
