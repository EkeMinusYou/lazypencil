//! Captured data-only runtime limits, merged independently per field.

use super::invalid;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const TEMPLATE: &str = concat!(
    "\n# Omitted runtime fields inherit user preferences and built-in limits.\n",
    "# [runtime.javascript]\n# timeout_ms = 2000\n# heap_mib = 64\n# idle_seconds = 60\n# layout_timeout_ms = 10000\n# max_evaluations = 128\n# max_passes = 8\n",
    "\n# [runtime.browser]\n# timeout_ms = 30000\n# launch_timeout_ms = 15000\n# idle_seconds = 60\n# wait_until = \"load\"\n# ready_selector = \"\"\n# selector_state = \"visible\"\n# additional_wait_ms = 0\n# animation_policy = \"running\"\n# network_policy = \"online\"\n",
    "\n# [runtime.shader]\n# timeout_ms = 15000\n# launch_timeout_ms = 15000\n# idle_seconds = 60\n",
    "\n# [runtime.dynamic]\n# render_timeout_ms = 45000\n# cache_mib = 64\n# cache_entries = 32\n",
);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeSettings {
    pub javascript: ScriptSettings,
    pub browser: BrowserSettings,
    pub shader: ShaderSettings,
    pub dynamic: DynamicSettings,
}

impl RuntimeSettings {
    pub fn validate(&self) -> Result<()> {
        self.javascript.validate()?;
        self.browser.validate()?;
        self.shader.validate()?;
        self.dynamic.validate()
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserSettings {
    #[schemars(range(min = 1, max = 300000))]
    pub timeout_ms: u64,
    #[schemars(range(min = 1, max = 60000))]
    pub launch_timeout_ms: u64,
    #[schemars(range(min = 0, max = 3600))]
    pub idle_seconds: u64,
    pub wait_until: PageWaitUntil,
    /// Main-document CSS selector, at most 512 Unicode characters; empty disables it.
    #[schemars(length(max = 512))]
    pub ready_selector: String,
    pub selector_state: PageSelectorState,
    /// Additional host-timed wait after readiness and before final asset decoding.
    #[schemars(range(min = 0, max = 60000))]
    pub additional_wait_ms: u64,
    pub animation_policy: PageAnimationPolicy,
    /// HTTP(S) and WebSocket requests; other protocol controls are separate.
    pub network_policy: PageNetworkPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageNetworkPolicy {
    #[default]
    Online,
    Offline,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageAnimationPolicy {
    #[default]
    Running,
    Paused,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageSelectorState {
    Attached,
    #[default]
    Visible,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageWaitUntil {
    #[default]
    Load,
    NetworkIdle,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 30000,
            launch_timeout_ms: 15000,
            idle_seconds: 60,
            wait_until: PageWaitUntil::Load,
            ready_selector: String::new(),
            selector_state: PageSelectorState::Visible,
            additional_wait_ms: 0,
            animation_policy: PageAnimationPolicy::Running,
            network_policy: PageNetworkPolicy::Online,
        }
    }
}

impl BrowserSettings {
    pub fn validate(&self) -> Result<()> {
        validate_browser_limits(
            "browser",
            self.timeout_ms,
            self.launch_timeout_ms,
            self.idle_seconds,
        )?;
        if self.additional_wait_ms > 60000 {
            return Err(invalid(
                "runtime.browser.additional_wait_ms must be within 0..60000",
            ));
        }
        if self.ready_selector.chars().count() > 512 || self.ready_selector.contains('\0') {
            return Err(invalid(
                "runtime.browser.ready_selector must contain at most 512 Unicode characters without NUL",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ShaderSettings {
    #[schemars(range(min = 1, max = 300000))]
    pub timeout_ms: u64,
    #[schemars(range(min = 1, max = 60000))]
    pub launch_timeout_ms: u64,
    #[schemars(range(min = 0, max = 3600))]
    pub idle_seconds: u64,
}

impl Default for ShaderSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 15000,
            launch_timeout_ms: 15000,
            idle_seconds: 60,
        }
    }
}

impl ShaderSettings {
    pub fn validate(&self) -> Result<()> {
        validate_browser_limits(
            "shader",
            self.timeout_ms,
            self.launch_timeout_ms,
            self.idle_seconds,
        )
    }
}

fn validate_browser_limits(
    scope: &str,
    timeout_ms: u64,
    launch_timeout_ms: u64,
    idle_seconds: u64,
) -> Result<()> {
    for (field, value, min, max) in [
        ("timeout_ms", timeout_ms, 1, 300000),
        ("launch_timeout_ms", launch_timeout_ms, 1, 60000),
        ("idle_seconds", idle_seconds, 0, 3600),
    ] {
        if !(min..=max).contains(&value) {
            return Err(invalid(format!(
                "runtime.{scope}.{field} must be within {min}..{max}"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DynamicSettings {
    #[schemars(range(min = 1, max = 300000))]
    pub render_timeout_ms: u64,
    #[schemars(range(min = 0, max = 256))]
    pub cache_mib: u64,
    #[schemars(range(min = 0, max = 256))]
    pub cache_entries: usize,
}

impl Default for DynamicSettings {
    fn default() -> Self {
        Self {
            render_timeout_ms: 45000,
            cache_mib: 64,
            cache_entries: 32,
        }
    }
}

impl DynamicSettings {
    pub fn validate(&self) -> Result<()> {
        for (field, value, min, max) in [
            ("render_timeout_ms", self.render_timeout_ms, 1, 300000),
            ("cache_mib", self.cache_mib, 0, 256),
            ("cache_entries", self.cache_entries as u64, 0, 256),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(format!(
                    "runtime.dynamic.{field} must be within {min}..{max}"
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
    browser: BrowserOverrides,
    shader: ShaderOverrides,
    dynamic: DynamicOverrides,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BrowserOverrides {
    timeout_ms: Option<u64>,
    launch_timeout_ms: Option<u64>,
    idle_seconds: Option<u64>,
    wait_until: Option<PageWaitUntil>,
    ready_selector: Option<String>,
    selector_state: Option<PageSelectorState>,
    additional_wait_ms: Option<u64>,
    animation_policy: Option<PageAnimationPolicy>,
    network_policy: Option<PageNetworkPolicy>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ShaderOverrides {
    timeout_ms: Option<u64>,
    launch_timeout_ms: Option<u64>,
    idle_seconds: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DynamicOverrides {
    render_timeout_ms: Option<u64>,
    cache_mib: Option<u64>,
    cache_entries: Option<usize>,
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
        "browser.timeout_ms",
        "browser.launch_timeout_ms",
        "browser.idle_seconds",
        "browser.wait_until",
        "browser.ready_selector",
        "browser.selector_state",
        "browser.additional_wait_ms",
        "browser.animation_policy",
        "browser.network_policy",
        "shader.timeout_ms",
        "shader.launch_timeout_ms",
        "shader.idle_seconds",
        "dynamic.render_timeout_ms",
        "dynamic.cache_mib",
        "dynamic.cache_entries",
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
        let mut declared = RuntimeSettings::default();
        macro_rules! apply {
            ($scope:ident, $field:ident) => {
                if let Some(value) = &self.$scope.$field {
                    assign(
                        value,
                        &mut declared.$scope.$field,
                        &mut settings.$scope.$field,
                    );
                    origins.insert(concat!(stringify!($scope), ".", stringify!($field)), origin);
                }
            };
        }
        apply!(javascript, timeout_ms);
        apply!(javascript, heap_mib);
        apply!(javascript, idle_seconds);
        apply!(javascript, layout_timeout_ms);
        apply!(javascript, max_evaluations);
        apply!(javascript, max_passes);
        apply!(browser, timeout_ms);
        apply!(browser, launch_timeout_ms);
        apply!(browser, idle_seconds);
        apply!(browser, wait_until);
        apply!(browser, ready_selector);
        apply!(browser, selector_state);
        apply!(browser, additional_wait_ms);
        apply!(browser, animation_policy);
        apply!(browser, network_policy);
        apply!(shader, timeout_ms);
        apply!(shader, launch_timeout_ms);
        apply!(shader, idle_seconds);
        apply!(dynamic, render_timeout_ms);
        apply!(dynamic, cache_mib);
        apply!(dynamic, cache_entries);
        declared.validate()?;
        settings.validate()
    }
}

fn assign<T: Clone>(value: &T, declared: &mut T, effective: &mut T) {
    declared.clone_from(value);
    effective.clone_from(value);
}
