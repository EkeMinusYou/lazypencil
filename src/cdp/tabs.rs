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
    context: String,
    session: String,
}

pub(super) struct RetainedTab {
    key: String,
    signature: String,
    tab: Tab,
    id: String,
}

impl Browser {
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
            while self.retained_tabs.len() >= MAX_RETAINED_PAGES {
                let evicted = self
                    .retained_tabs
                    .pop_front()
                    .expect("retained context bound");
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
        let mut index = 0;
        while index < self.retained_tabs.len() {
            budget.check()?;
            if keys.contains(&self.retained_tabs[index].key) {
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
        let context = self.command(
            None,
            "Target.createBrowserContext",
            json!({"disposeOnDetach":true}),
            budget,
        )?;
        let context = identifier(&context, "browserContextId")?;
        let result = (|| {
            self.command(
                None,
                "Browser.setDownloadBehavior",
                json!({"behavior":"deny","browserContextId":context}),
                budget,
            )?;
            let target = self.command(
                None,
                "Target.createTarget",
                json!({"url":"about:blank","browserContextId":context}),
                budget,
            )?;
            let target = identifier(&target, "targetId")?;
            let attached = self.command(
                None,
                "Target.attachToTarget",
                json!({"targetId":target,"flatten":true}),
                budget,
            )?;
            identifier(&attached, "sessionId")
        })();
        match result {
            Ok(session) => Ok(Tab { context, session }),
            Err(error) => {
                let _ = self.command(
                    None,
                    "Target.disposeBrowserContext",
                    json!({"browserContextId":context}),
                    &Budget::new(Duration::from_millis(500), None),
                );
                Err(error)
            }
        }
    }

    fn dispose_tab(&mut self, tab: Tab) -> Result<()> {
        let disposed = self.command(
            None,
            "Target.disposeBrowserContext",
            json!({"browserContextId":tab.context}),
            &Budget::new(Duration::from_millis(500), None),
        );
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
