//! Immutable local responses, isolated to a page and its attached descendants.
//!
//! This layer performs no filesystem reads. Its caller must capture and verify
//! the project resource inventory before enabling interception or navigation.

use super::{Browser, Budget, Purpose, failed};
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

const MAX_FILES: usize = 4096;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;
const MAX_SESSIONS: usize = 128;
const MAX_PENDING: usize = 4096;

struct File {
    mime: String,
    bytes: Vec<u8>,
}

/// A bounded inventory of original local URLs and captured response bytes.
/// Query strings and fragments select the same immutable file, as with native
/// file loading. Unknown local URLs are denied, never read from the host.
pub struct LocalFiles {
    files: BTreeMap<String, File>,
}

impl LocalFiles {
    pub fn new(files: impl IntoIterator<Item = (String, String, Vec<u8>)>) -> Result<Self> {
        let mut inventory = BTreeMap::new();
        let mut total = 0usize;
        for (url, mime, bytes) in files {
            if inventory.len() >= MAX_FILES
                || bytes.len() > MAX_FILE_BYTES
                || mime.is_empty()
                || mime.len() > 256
                || !mime.is_ascii()
                || mime.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(invalid("Invalid or oversized local browser resource"));
            }
            total = total
                .checked_add(bytes.len())
                .filter(|total| *total <= MAX_TOTAL_BYTES)
                .ok_or_else(|| invalid("Local browser inventory exceeds 128 MiB"))?;
            let key = local_key(&url)?;
            if inventory.insert(key, File { mime, bytes }).is_some() {
                return Err(invalid("Local browser URLs identify the same file"));
            }
        }
        if inventory.is_empty() {
            return Err(invalid("Local browser inventory is empty"));
        }
        Ok(Self { files: inventory })
    }

    fn response(&self, params: &Value) -> Result<(&'static str, Value, usize)> {
        let request_id = identifier(&params["requestId"])?;
        let request = &params["request"];
        let url = request["url"]
            .as_str()
            .ok_or_else(|| failed("Missing intercepted local URL"))?;
        let file = local_key(url).ok().and_then(|key| self.files.get(&key));
        let method = request["method"].as_str();
        if let Some(file) = file
            && matches!(method, Some("GET" | "HEAD"))
        {
            let bytes = if method == Some("HEAD") {
                &[][..]
            } else {
                &file.bytes
            };
            return Ok((
                "Fetch.fulfillRequest",
                json!({"requestId":request_id,"responseCode":200,
                    "responseHeaders":[
                        {"name":"Content-Type","value":file.mime},
                        {"name":"Content-Length","value":file.bytes.len().to_string()},
                        {"name":"Cache-Control","value":"no-store"}
                    ],"body":STANDARD.encode(bytes)}),
                bytes.len(),
            ));
        }
        Ok((
            "Fetch.failRequest",
            json!({"requestId":request_id,"errorReason":"BlockedByClient"}),
            0,
        ))
    }
}

fn local_key(value: &str) -> Result<String> {
    if value.len() > 64 * 1024 {
        return Err(invalid("Local browser URL exceeds 64 KiB"));
    }
    let mut url = url::Url::parse(value).map_err(|_| invalid("Invalid local browser URL"))?;
    if url.scheme() != "file"
        || !url.path().starts_with('/')
        || url.host_str().is_some_and(|host| host != "localhost")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(invalid(
            "A local browser resource needs an absolute local file URL",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn invalid(message: &str) -> Error {
    Error::new("INVALID_ARGUMENT", message)
}

#[derive(Clone, Copy, Default, Serialize)]
pub struct LocalFileStats {
    pub fulfilled: u64,
    pub denied: u64,
    pub response_bytes: u64,
    pub attached_targets: u64,
}

struct Session {
    owner: String,
    parent: Option<String>,
    files: Arc<LocalFiles>,
    stats: LocalFileStats,
}

#[derive(Clone, Copy)]
enum Phase {
    Complete,
    AutoAttach,
    Resume,
}

struct Dispatch {
    session: String,
    browser_level: bool,
    method: &'static str,
    params: Value,
    next: Phase,
}

#[derive(Default)]
pub(super) struct Interception {
    sessions: BTreeMap<String, Session>,
    pending: BTreeMap<u64, (String, Phase)>,
    targets: BTreeMap<String, String>,
    browser_enabled: bool,
}

fn enable() -> Value {
    json!({"patterns":[{"urlPattern":"file:*","requestStage":"Request"}]})
}

fn auto_attach() -> Value {
    json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true})
}

fn identifier(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .ok_or_else(|| failed("Invalid local browser protocol identifier"))
}

impl Browser {
    /// Install a captured inventory before navigating this isolated page.
    /// A fresh page is required when its resource snapshot changes. Frames and
    /// workers inherit the inventory before their paused startup is resumed.
    /// After installation, browser-level requests from unowned targets are
    /// denied for the remaining lifetime of this isolated browser process.
    pub fn intercept_local_files(
        &mut self,
        session: &str,
        files: Arc<LocalFiles>,
        budget: &Budget<'_>,
    ) -> Result<()> {
        budget.check()?;
        if self.purpose != Purpose::Page
            || self.local_files.sessions.contains_key(session)
            || self.local_files.sessions.len() >= MAX_SESSIONS
        {
            return Err(invalid("Local resources require a fresh page session"));
        }
        self.local_files.sessions.insert(
            session.into(),
            Session {
                owner: session.into(),
                parent: None,
                files,
                stats: LocalFileStats::default(),
            },
        );
        self.command(Some(session), "Fetch.enable", enable(), budget)?;
        // Dedicated workers have Target and Network handlers, but no Fetch
        // handler. Browser-level Fetch also wraps their loaders. Page handlers
        // stay closer to the client and fulfill their own requests first.
        if !self.local_files.browser_enabled {
            self.local_files.browser_enabled = true;
            self.command(None, "Fetch.enable", enable(), budget)?;
        }
        self.command(Some(session), "Target.setAutoAttach", auto_attach(), budget)?;
        Ok(())
    }

    pub fn local_file_stats(&self, session: &str) -> Option<LocalFileStats> {
        self.local_files.sessions.get(session).map(|s| s.stats)
    }

    pub(super) fn dispatch_local_files(
        &mut self,
        value: &Value,
        budget: &Budget<'_>,
    ) -> Result<()> {
        if let Some(dispatch) = self.local_files.observe(value)? {
            if self.local_files.pending.len() >= MAX_PENDING {
                return Err(failed("Too many pending local browser responses"));
            }
            let id = self.send_command(
                (!dispatch.browser_level).then_some(dispatch.session.as_str()),
                dispatch.method,
                dispatch.params,
                budget,
            )?;
            self.local_files
                .pending
                .insert(id, (dispatch.session, dispatch.next));
        }
        Ok(())
    }
}

impl Interception {
    pub(super) fn remove(&mut self, session: &str) {
        let mut removed = std::collections::BTreeSet::from([session.to_owned()]);
        loop {
            let before = removed.len();
            for (id, entry) in &self.sessions {
                if entry.parent.as_ref().is_some_and(|id| removed.contains(id)) {
                    removed.insert(id.clone());
                }
            }
            if removed.len() == before {
                break;
            }
        }
        self.sessions.retain(|id, _| !removed.contains(id));
        self.pending.retain(|_, (id, _)| !removed.contains(id));
        self.targets.retain(|_, id| !removed.contains(id));
    }

    fn observe(&mut self, value: &Value) -> Result<Option<Dispatch>> {
        if let Some(id) = value["id"].as_u64()
            && let Some((session, next)) = self.pending.remove(&id)
        {
            if !session.is_empty() && !self.sessions.contains_key(&session) {
                return Ok(None);
            }
            if value.get("error").is_some() {
                // Never echo URLs, request headers, or page-provided error text.
                let stage = match next {
                    Phase::AutoAttach => "fetch_enable",
                    Phase::Resume => "target_auto_attach",
                    Phase::Complete => "response_or_resume",
                };
                return Err(
                    failed("Local browser interception command failed").details(json!({
                        "stage":stage,"protocol_code":value["error"]["code"].as_i64()
                    })),
                );
            }
            return Ok(match next {
                Phase::Complete => None,
                Phase::AutoAttach => Some(Dispatch {
                    session,
                    browser_level: false,
                    method: "Target.setAutoAttach",
                    params: auto_attach(),
                    next: Phase::Resume,
                }),
                Phase::Resume => Some(Dispatch {
                    session,
                    browser_level: false,
                    method: "Runtime.runIfWaitingForDebugger",
                    params: json!({}),
                    next: Phase::Complete,
                }),
            });
        }
        if value["sessionId"].is_null()
            && value["method"] == "Fetch.requestPaused"
            && self.browser_enabled
        {
            let frame = identifier(&value["params"]["frameId"])?;
            if let Some(session) = self.targets.get(frame).cloned() {
                return self.request(&session, &value["params"], true).map(Some);
            }
            // Never guess an inventory from a URL shared by different tabs.
            return Ok(Some(Dispatch {
                session: String::new(),
                browser_level: true,
                method: "Fetch.failRequest",
                params: json!({"requestId":identifier(&value["params"]["requestId"])? ,"errorReason":"BlockedByClient"}),
                next: Phase::Complete,
            }));
        }
        let Some(parent) = value["sessionId"].as_str() else {
            return Ok(None);
        };
        let Some(entry) = self.sessions.get(parent) else {
            return Ok(None);
        };
        match value["method"].as_str() {
            Some("Fetch.requestPaused") => self.request(parent, &value["params"], false).map(Some),
            Some("Target.attachedToTarget") => {
                let child = identifier(&value["params"]["sessionId"])?;
                let target = identifier(&value["params"]["targetInfo"]["targetId"])?;
                let page = matches!(
                    value["params"]["targetInfo"]["type"].as_str(),
                    Some("page" | "iframe")
                );
                if self.sessions.len() >= MAX_SESSIONS || self.sessions.contains_key(child) {
                    return Err(failed("Invalid or excessive local browser child sessions"));
                }
                let entry = Session {
                    owner: entry.owner.clone(),
                    parent: Some(parent.into()),
                    files: entry.files.clone(),
                    stats: LocalFileStats::default(),
                };
                self.sessions
                    .get_mut(&entry.owner)
                    .expect("inventory owner")
                    .stats
                    .attached_targets += 1;
                self.sessions.insert(child.into(), entry);
                if self.targets.insert(target.into(), child.into()).is_some() {
                    return Err(failed("Duplicate local browser child target"));
                }
                Ok(Some(Dispatch {
                    session: child.into(),
                    browser_level: false,
                    method: if page {
                        "Fetch.enable"
                    } else {
                        "Target.setAutoAttach"
                    },
                    params: if page { enable() } else { auto_attach() },
                    next: if page {
                        Phase::AutoAttach
                    } else {
                        Phase::Resume
                    },
                }))
            }
            Some("Target.detachedFromTarget") => {
                let child = identifier(&value["params"]["sessionId"])?;
                if self
                    .sessions
                    .get(child)
                    .is_some_and(|s| s.parent.as_deref() == Some(parent))
                {
                    self.remove(child);
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn request(&mut self, session: &str, params: &Value, browser_level: bool) -> Result<Dispatch> {
        let entry = &self.sessions[session];
        let owner = entry.owner.clone();
        let (method, params, bytes) = entry.files.response(params)?;
        let stats = &mut self
            .sessions
            .get_mut(&owner)
            .expect("inventory owner")
            .stats;
        if method == "Fetch.fulfillRequest" {
            stats.fulfilled += 1;
            stats.response_bytes += bytes as u64;
        } else {
            stats.denied += 1;
        }
        Ok(Dispatch {
            session: session.into(),
            browser_level,
            method,
            params,
            next: Phase::Complete,
        })
    }
}

#[cfg(test)]
mod tests;
