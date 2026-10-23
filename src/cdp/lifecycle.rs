//! Main-frame lifecycle state, scoped to one isolated page acquisition.

use serde_json::Value;

pub(super) struct PageLifecycle {
    session: String,
    frame: String,
    loader: Option<String>,
    idle: bool,
}

impl PageLifecycle {
    pub fn new(session: &str, frame: &str) -> Self {
        Self {
            session: session.into(),
            frame: frame.into(),
            loader: None,
            idle: false,
        }
    }

    pub fn observe(&mut self, event: &Value) {
        if event["method"] != "Page.lifecycleEvent"
            || event["sessionId"].as_str() != Some(self.session.as_str())
            || event["params"]["frameId"].as_str() != Some(self.frame.as_str())
        {
            return;
        }
        let Some(loader) = event["params"]["loaderId"].as_str() else {
            return;
        };
        // Identifiers are opaque. Keep one bounded current loader, never an
        // event queue, request URLs, headers, or page content.
        if loader.is_empty() || loader.len() > 512 {
            return;
        }
        match event["params"]["name"].as_str() {
            Some("init") => {
                self.loader = Some(loader.into());
                self.idle = false;
            }
            Some("networkIdle") if self.loader.as_deref() == Some(loader) => {
                self.idle = true;
            }
            _ => {}
        }
    }

    pub fn is_idle(&self, session: &str, frame: &Value) -> bool {
        self.idle
            && session == self.session
            && frame["id"].as_str() == Some(self.frame.as_str())
            && frame["loaderId"].as_str() == self.loader.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(session: &str, frame: &str, loader: &str, name: &str) -> Value {
        json!({"method":"Page.lifecycleEvent","sessionId":session,
            "params":{"frameId":frame,"loaderId":loader,"name":name}})
    }

    #[test]
    fn idle_requires_the_current_session_main_frame_and_loader() {
        let mut state = PageLifecycle::new("session", "main");
        let frame = json!({"id":"main","loaderId":"new"});
        state.observe(&event("session", "main", "old", "init"));
        state.observe(&event("session", "main", "old", "networkIdle"));
        assert!(!state.is_idle("session", &frame));
        state.observe(&event("session", "main", "new", "init"));
        for unrelated in [
            event("other", "main", "new", "networkIdle"),
            event("session", "child", "new", "networkIdle"),
            event("session", "main", "old", "networkIdle"),
            event("session", "main", "new", "networkAlmostIdle"),
            event("session", "main", "new", "load"),
            json!({"method":"Page.lifecycleEvent"}),
        ] {
            state.observe(&unrelated);
            assert!(!state.is_idle("session", &frame));
        }
        state.observe(&event("session", "main", "new", "networkIdle"));
        assert!(state.is_idle("session", &frame));
        assert!(!state.is_idle("other", &frame));
        assert!(!state.is_idle("session", &json!({"id":"child","loaderId":"new"})));
        state.observe(&event("session", "main", "next", "init"));
        assert!(!state.is_idle("session", &frame));
        state.observe(&event("session", "main", "new", "networkIdle"));
        assert!(!state.idle);
    }

    #[test]
    fn missing_empty_and_oversized_loaders_do_not_replace_current_state() {
        let mut state = PageLifecycle::new("s", "f");
        state.observe(&event("s", "f", "real", "init"));
        for loader in ["", &"x".repeat(513)] {
            state.observe(&event("s", "f", loader, "init"));
            state.observe(&event("s", "f", loader, "networkIdle"));
        }
        assert_eq!(state.loader.as_deref(), Some("real"));
        assert!(!state.idle);
        state.observe(&event("s", "f", "real", "networkIdle"));
        assert!(state.is_idle("s", &json!({"id":"f","loaderId":"real"})));
    }
}
