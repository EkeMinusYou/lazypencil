//! Bounded isolated page contexts; retained sessions belong to one browser.

use super::{Browser, Budget, failed};
use crate::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

pub const MAX_RETAINED_PAGES: usize = 8;

#[derive(Serialize)]
pub struct PageState {
    pub retained: bool,
    pub id: Option<String>,
    pub reused: bool,
    pub retained_limit: usize,
}

struct Tab {
    context: Option<String>,
    target: String,
    session: String,
}

pub(super) struct RetainedTab {
    key: String,
    signature: String,
    tab: Tab,
    id: String,
}

impl Browser {
    #[cfg(all(unix, feature = "rendering"))]
    pub(crate) fn enter_profile_owner(&mut self, owner: &str, budget: &Budget<'_>) -> Result<()> {
        budget.check()?;
        if !self.persistent_profile || uuid::Uuid::parse_str(owner).is_err() {
            return Err(failed("Invalid shared page owner"));
        }
        self.active_profile_owner = Some(owner.into());
        Ok(())
    }

    #[cfg(all(unix, feature = "rendering"))]
    pub(crate) fn leave_profile_owner(&mut self, owner: &str) -> Result<()> {
        if self.active_profile_owner.as_deref() == Some(owner) {
            self.active_profile_owner = None;
        }
        let budget = Budget::new(Duration::from_millis(500), None);
        while self
            .pending_profile_targets
            .values()
            .any(|value| value == owner)
        {
            self.command(None, "Target.getTargets", json!({}), &budget)?;
            if self
                .pending_profile_targets
                .values()
                .any(|value| value == owner)
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if self.profile_target_fault {
            return Err(failed("Browser target ownership is unavailable"));
        }
        while let Some(target) = self
            .profile_targets
            .iter()
            .find(|(_, value)| value.as_str() == owner)
            .map(|(target, _)| target.clone())
        {
            self.close_profile_target(&target, &budget)?;
        }
        let prefix = format!("{owner}:");
        let mut index = 0;
        while index < self.retained_tabs.len() {
            if self.retained_tabs[index].key.starts_with(&prefix) {
                let entry = self
                    .retained_tabs
                    .remove(index)
                    .expect("owned retained page");
                self.local_files.remove(&entry.tab.session);
                self.page_lifecycle.remove(&entry.tab.session);
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    #[cfg(all(unix, feature = "rendering"))]
    pub(crate) fn retain_profile_pages(
        &mut self,
        owner: &str,
        signatures: &std::collections::BTreeMap<String, String>,
        budget: &Budget<'_>,
    ) -> Result<()> {
        let prefix = format!("{owner}:");
        self.retain_pages(
            |entry| {
                !entry.key.starts_with(&prefix)
                    || signatures.get(&entry.key[prefix.len()..]) == Some(&entry.signature)
            },
            budget,
        )
    }

    #[cfg(all(unix, feature = "rendering"))]
    pub(crate) fn profile_page_count(&self, owner: &str) -> usize {
        let prefix = format!("{owner}:");
        self.retained_tabs
            .iter()
            .filter(|entry| entry.key.starts_with(&prefix))
            .count()
    }

    pub(super) fn observe_profile_target(&mut self, reply: &Value) -> Result<()> {
        match reply["method"].as_str() {
            Some("Target.targetCreated" | "Target.targetInfoChanged") => {
                let info = &reply["params"]["targetInfo"];
                if info["type"] == "page"
                    && let Some(owner) = info["openerId"]
                        .as_str()
                        .and_then(|opener| self.profile_targets.get(opener))
                        .cloned()
                {
                    self.profile_targets
                        .insert(identifier(info, "targetId")?, owner);
                }
            }
            Some("Target.targetDestroyed") => {
                if let Some(target) = reply["params"]["targetId"].as_str() {
                    self.profile_targets.remove(target);
                }
            }
            _ => {}
        }
        let Some(owner) = reply["id"]
            .as_u64()
            .and_then(|id| self.pending_profile_targets.remove(&id))
        else {
            return Ok(());
        };
        if reply.get("error").is_some() {
            return Ok(());
        }
        match identifier(&reply["result"], "targetId") {
            Ok(target) => {
                self.profile_targets.insert(target, owner);
                Ok(())
            }
            Err(error) => {
                self.profile_target_fault = true;
                Err(error)
            }
        }
    }

    fn create_profile_target(&mut self, options: Value, budget: &Budget<'_>) -> Result<Value> {
        budget.check()?;
        let id = self.next_id + 1;
        if let Some(owner) = &self.active_profile_owner {
            self.pending_profile_targets.insert(id, owner.clone());
        }
        let result = self.command(None, "Target.createTarget", options, budget);
        if self.next_id < id {
            self.pending_profile_targets.remove(&id);
        }
        result
    }

    fn close_profile_target(&mut self, target: &str, budget: &Budget<'_>) -> Result<()> {
        if !self.profile_targets.contains_key(target) {
            return Ok(());
        }
        let info = self.command(None, "Target.getTargets", json!({}), budget)?;
        let entries = info["targetInfos"]
            .as_array()
            .ok_or_else(|| failed("Cannot observe owned browser targets"))?;
        let mut owned = BTreeSet::from([target.to_owned()]);
        loop {
            let before = owned.len();
            for entry in entries {
                if entry["openerId"]
                    .as_str()
                    .is_some_and(|opener| owned.contains(opener))
                {
                    owned.insert(identifier(entry, "targetId")?);
                }
            }
            if owned.len() == before {
                break;
            }
        }
        let present = entries
            .iter()
            .filter_map(|entry| entry["targetId"].as_str())
            .collect::<BTreeSet<_>>();
        owned.remove(target);
        for child in owned.into_iter().chain(std::iter::once(target.to_owned())) {
            if present.contains(child.as_str())
                && let Err(error) = self.command(
                    None,
                    "Target.closeTarget",
                    json!({"targetId":child}),
                    budget,
                )
            {
                let current = self.command(None, "Target.getTargets", json!({}), budget)?;
                if current["targetInfos"]
                    .as_array()
                    .is_none_or(|entries| entries.iter().any(|entry| entry["targetId"] == child))
                {
                    return Err(error);
                }
            }
            self.profile_targets.remove(&child);
        }
        Ok(())
    }

    /// A deliberate login keeps its page open until the user closes the browser
    /// or the shared deadline/cancellation ends it. Credentials remain on disk.
    pub fn login(&mut self, url: &str, budget: &Budget<'_>) -> Result<()> {
        if !self.persistent_profile {
            return Err(crate::Error::new(
                "INVALID_ARGUMENT",
                "Login requires an explicit dedicated browser profile",
            ));
        }
        let tab = self.create_tab(budget)?;
        let mut closed = false;
        let result = (|| {
            let navigation = self.command(
                Some(&tab.session),
                "Page.navigate",
                json!({"url":url}),
                budget,
            )?;
            if navigation.get("errorText").is_some() {
                return Err(crate::Error::new(
                    "PAGE_LOAD_FAILED",
                    "Login page navigation failed",
                ));
            }
            loop {
                budget.check()?;
                if self.has_exited()? {
                    return Ok(());
                }
                let targets = match self.command(None, "Target.getTargets", json!({}), budget) {
                    Ok(targets) => targets,
                    Err(error) => {
                        // Closing the last window can close CDP before the
                        // guardian finishes collecting Chromium's children.
                        let end = std::time::Instant::now()
                            + budget.remaining()?.min(Duration::from_secs(3));
                        loop {
                            budget.check()?;
                            if self.has_exited()? {
                                return Ok(());
                            }
                            if std::time::Instant::now() >= end {
                                return Err(error);
                            }
                            std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
                        }
                    }
                };
                let targets = targets["targetInfos"]
                    .as_array()
                    .ok_or_else(|| failed("Cannot observe the login target"))?;
                if !targets
                    .iter()
                    .any(|target| target["targetId"] == tab.target)
                {
                    closed = true;
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100).min(budget.remaining()?));
            }
        })();
        if closed || self.has_exited().unwrap_or(false) {
            result
        } else {
            let disposed = self.dispose_tab(tab);
            result.and(disposed)
        }
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn retained_page_count(&self) -> usize {
        self.retained_tabs.len()
    }

    pub fn with_tab<T>(
        &mut self,
        budget: &Budget<'_>,
        f: impl FnOnce(&mut Self, &str) -> Result<T>,
    ) -> Result<T> {
        self.with_page(None, budget, |browser, session, _| f(browser, session))
    }

    /// A retained key identifies a document node; its signature changes when
    /// the configured URL or device changes. Viewport/scroll edits reuse DOM.
    pub fn with_page<T>(
        &mut self,
        retained: Option<(&str, &str)>,
        budget: &Budget<'_>,
        f: impl FnOnce(&mut Self, &str, &PageState) -> Result<T>,
    ) -> Result<T> {
        budget.check()?;
        let mut previous = retained.and_then(|(key, _)| {
            self.retained_tabs
                .iter()
                .position(|entry| entry.key == key)
                .and_then(|position| self.retained_tabs.remove(position))
        });
        if let Some(stale) = previous.take_if(|entry| {
            Some(entry.signature.as_str()) != retained.map(|(_, signature)| signature)
        }) {
            self.dispose_tab(stale.tab)?;
        }
        let reused = previous.is_some();
        if retained.is_some() && !reused {
            // Shared profiles have an independent LRU for every frontend.
            // Anonymous runtimes retain their original eight-page limit.
            let prefix = self
                .active_profile_owner
                .as_ref()
                .map(|owner| format!("{owner}:"));
            let owned = |entry: &RetainedTab| {
                prefix
                    .as_ref()
                    .is_none_or(|prefix| entry.key.starts_with(prefix))
            };
            while self
                .retained_tabs
                .iter()
                .filter(|entry| owned(entry))
                .count()
                >= MAX_RETAINED_PAGES
            {
                let position = self
                    .retained_tabs
                    .iter()
                    .position(&owned)
                    .expect("retained context bound");
                let evicted = self
                    .retained_tabs
                    .remove(position)
                    .expect("owned retained context");
                self.dispose_tab(evicted.tab)?;
            }
        }
        let (tab, id) = match previous {
            Some(previous) => (previous.tab, Some(previous.id)),
            None => (
                self.create_tab(budget)?,
                retained.map(|_| uuid::Uuid::new_v4().to_string()),
            ),
        };
        let state = PageState {
            retained: retained.is_some(),
            id,
            reused,
            retained_limit: if retained.is_some() {
                MAX_RETAINED_PAGES
            } else {
                0
            },
        };
        let result = f(self, &tab.session, &state);
        if result.is_ok()
            && let Some((key, signature)) = retained
        {
            self.retained_tabs.push_back(RetainedTab {
                key: key.into(),
                signature: signature.into(),
                tab,
                id: state.id.expect("retained context identity"),
            });
            result
        } else {
            let disposed = self.dispose_tab(tab);
            result.and_then(|value| disposed.map(|()| value))
        }
    }

    /// Close removed nodes without resetting the retained nodes that remain.
    pub fn retain_page_keys(&mut self, keys: &BTreeSet<String>, budget: &Budget<'_>) -> Result<()> {
        self.retain_pages(|entry| keys.contains(&entry.key), budget)
    }

    /// Retire stale inputs even when the owning node is outside the viewport.
    pub fn retain_page_signatures(
        &mut self,
        signatures: &std::collections::BTreeMap<String, String>,
        budget: &Budget<'_>,
    ) -> Result<()> {
        self.retain_pages(
            |entry| signatures.get(&entry.key) == Some(&entry.signature),
            budget,
        )
    }

    fn retain_pages(
        &mut self,
        keep: impl Fn(&RetainedTab) -> bool,
        budget: &Budget<'_>,
    ) -> Result<()> {
        let mut index = 0;
        while index < self.retained_tabs.len() {
            budget.check()?;
            if keep(&self.retained_tabs[index]) {
                index += 1;
            } else {
                let removed = self
                    .retained_tabs
                    .remove(index)
                    .expect("retained context index");
                self.dispose_tab(removed.tab)?;
            }
        }
        Ok(())
    }

    fn create_tab(&mut self, budget: &Budget<'_>) -> Result<Tab> {
        let mut options = json!({"disposeOnDetach":true});
        if let Some(proxy) = &self.deny_proxy {
            options["proxyServer"] = format!("http://{}", proxy.address).into();
            // Chromium otherwise bypasses proxies for localhost/link-local URLs.
            options["proxyBypassList"] = "<-loopback>".into();
        }
        let context = if self.persistent_profile {
            None
        } else {
            let context = self.command(None, "Target.createBrowserContext", options, budget)?;
            Some(identifier(&context, "browserContextId")?)
        };
        let mut created_target = None;
        let result = (|| {
            let mut download = json!({"behavior":"deny"});
            let mut target_options = json!({"url":"about:blank"});
            if let Some(context) = &context {
                download["browserContextId"] = context.clone().into();
                target_options["browserContextId"] = context.clone().into();
            }
            self.command(None, "Browser.setDownloadBehavior", download, budget)?;
            let target = self.create_profile_target(target_options, budget)?;
            let target = identifier(&target, "targetId")?;
            created_target = Some(target.clone());
            let attached = self.command(
                None,
                "Target.attachToTarget",
                json!({"targetId":target,"flatten":true}),
                budget,
            )?;
            identifier(&attached, "sessionId")
        })();
        match result {
            Ok(session) => Ok(Tab {
                context,
                session,
                target: created_target.expect("created target"),
            }),
            Err(error) => {
                if let Some(context) = &context {
                    let _ = self.command(
                        None,
                        "Target.disposeBrowserContext",
                        json!({"browserContextId":context}),
                        &Budget::new(Duration::from_millis(500), None),
                    );
                } else if let Some(target) = created_target {
                    let _ = self.command(
                        None,
                        "Target.closeTarget",
                        json!({"targetId":target}),
                        &Budget::new(Duration::from_millis(500), None),
                    );
                }
                Err(error)
            }
        }
    }

    fn dispose_tab(&mut self, tab: Tab) -> Result<()> {
        let (method, options) = match &tab.context {
            Some(context) => (
                "Target.disposeBrowserContext",
                json!({"browserContextId":context}),
            ),
            None => ("Target.closeTarget", json!({"targetId":tab.target})),
        };
        let disposed = if self.profile_targets.contains_key(&tab.target) {
            self.close_profile_target(&tab.target, &Budget::new(Duration::from_millis(500), None))
        } else {
            self.command(
                None,
                method,
                options,
                &Budget::new(Duration::from_millis(500), None),
            )
        };
        self.local_files.remove(&tab.session);
        self.page_lifecycle.remove(&tab.session);
        disposed.map(|_| ())
    }
}

fn identifier(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .map(String::from)
        .ok_or_else(|| failed(format!("CDP did not return a valid {key}")))
}
