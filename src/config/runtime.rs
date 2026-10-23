//! Captured data-only runtime limits, merged independently per field.

use super::invalid;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const TEMPLATE: &str = "\n# Omitted runtime fields inherit user preferences and built-in limits.\n# [runtime.javascript]\n# timeout_ms = 2000\n# heap_mib = 64\n# idle_seconds = 60\n# layout_timeout_ms = 10000\n# max_evaluations = 128\n# max_passes = 8\n";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSettings {
    pub javascript: ScriptSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ScriptSettings {
    #[schemars(range(min = 1, max = 60000))]
    pub timeout_ms: u64,
    #[schemars(range(min = 4, max = 512))]
    pub heap_mib: u64,
    #[schemars(range(min = 0, max = 3600))]
    pub idle_seconds: u64,
    #[schemars(range(min = 1, max = 300000))]
    pub layout_timeout_ms: u64,
    #[schemars(range(min = 1, max = 4096))]
    pub max_evaluations: usize,
    #[schemars(range(min = 1, max = 64))]
    pub max_passes: usize,
}

impl Default for ScriptSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 2000,
            heap_mib: 64,
            idle_seconds: 60,
            layout_timeout_ms: 10000,
            max_evaluations: 128,
            max_passes: 8,
        }
    }
}

impl ScriptSettings {
    pub fn validate(&self) -> Result<()> {
        for (field, value, min, max) in [
            ("timeout_ms", self.timeout_ms, 1, 60000),
            ("heap_mib", self.heap_mib, 4, 512),
            ("idle_seconds", self.idle_seconds, 0, 3600),
            ("layout_timeout_ms", self.layout_timeout_ms, 1, 300000),
            ("max_evaluations", self.max_evaluations as u64, 1, 4096),
            ("max_passes", self.max_passes as u64, 1, 64),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(format!(
                    "runtime.javascript.{field} must be within {min}..{max}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RuntimeOverrides {
    javascript: ScriptOverrides,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScriptOverrides {
    timeout_ms: Option<u64>,
    heap_mib: Option<u64>,
    idle_seconds: Option<u64>,
    layout_timeout_ms: Option<u64>,
    max_evaluations: Option<usize>,
    max_passes: Option<usize>,
}

pub(super) fn origins() -> BTreeMap<&'static str, &'static str> {
    [
        "javascript.timeout_ms",
        "javascript.heap_mib",
        "javascript.idle_seconds",
        "javascript.layout_timeout_ms",
        "javascript.max_evaluations",
        "javascript.max_passes",
    ]
    .into_iter()
    .map(|field| (field, "builtin"))
    .collect()
}

impl RuntimeOverrides {
    pub(super) fn apply(
        &self,
        settings: &mut RuntimeSettings,
        origins: &mut BTreeMap<&'static str, &'static str>,
        origin: &'static str,
    ) -> Result<()> {
        // Validate each declared scope before merging so a higher-priority
        // setting cannot hide an invalid lower-priority value.
        let values = &self.javascript;
        let mut declared = ScriptSettings::default();
        macro_rules! apply {
            ($field:ident) => {
                if let Some(value) = values.$field {
                    declared.$field = value;
                    settings.javascript.$field = value;
                    origins.insert(concat!("javascript.", stringify!($field)), origin);
                }
            };
        }
        apply!(timeout_ms);
        apply!(heap_mib);
        apply!(idle_seconds);
        apply!(layout_timeout_ms);
        apply!(max_evaluations);
        apply!(max_passes);
        declared.validate()?;
        settings.javascript.validate()
    }
}
