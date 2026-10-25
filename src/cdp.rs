//! A local, synchronous Chrome DevTools Protocol transport with cancellation.

use crate::{Error, Result, runtime::InstalledBrowser};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs, io,
    net::SocketAddr,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tungstenite::{Message, WebSocket};

mod connection;
mod lifecycle;
mod local_files;
mod process;
mod readiness;
mod tabs;
use lifecycle::PageLifecycle;
pub(crate) use local_files::local_key as local_file_url;
pub use local_files::{LocalFileStats, LocalFiles};
#[cfg(any(windows, unix))]
pub use process::serve_worker as serve_process_worker;
pub use tabs::{MAX_RETAINED_PAGES, PageState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Page,
    Shader,
}

pub struct Browser {
    child: process::Owned,
    socket: WebSocket<connection::Transport>,
    next_id: u64,
    pub version: String,
    page_lifecycle: BTreeMap<String, PageLifecycle>,
    retained_tabs: VecDeque<tabs::RetainedTab>,
    local_files: local_files::Interception,
    purpose: Purpose,
    _profile: Option<tempfile::TempDir>,
}

pub struct Budget<'a> {
    end: Instant,
    pub cancel: Option<&'a CancellationToken>,
}
impl<'a> Budget<'a> {
    pub fn new(duration: Duration, cancel: Option<&'a CancellationToken>) -> Self {
        Self {
            end: Instant::now() + duration,
            cancel,
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(Error::new("CANCELLED", "Browser operation cancelled"));
        }
        if Instant::now() >= self.end {
            return Err(Error::new(
                "RENDER_TIMEOUT",
                "Browser operation exceeded its deadline",
            ));
        }
        Ok(())
    }
    pub fn remaining(&self) -> Result<Duration> {
        self.check()?;
        Ok(self.end.saturating_duration_since(Instant::now()))
    }
}

impl Browser {
    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    /// The ephemeral Chromium profile, for lifecycle diagnostics. On Unix,
    /// its private guardian owns cleanup even after this host is terminated.
    pub fn profile_path(&self) -> &Path {
        #[cfg(unix)]
        {
            self.child.profile_path()
        }
        #[cfg(not(unix))]
        {
            self._profile.as_ref().expect("host browser profile").path()
        }
    }

    /// Launch through the current lazypencil executable. Embedding hosts use
    /// `launch_with_supervisor` to supply their actual lazypencil CLI.
    pub fn launch(
        browser: &InstalledBrowser,
        purpose: Purpose,
        budget: &Budget<'_>,
    ) -> Result<Self> {
        Self::launch_inner(browser, purpose, budget, None)
    }

    /// Supply the actual lazypencil CLI when embedding this library in another executable.
    pub fn launch_with_supervisor(
        browser: &InstalledBrowser,
        purpose: Purpose,
        budget: &Budget<'_>,
        supervisor: &Path,
    ) -> Result<Self> {
        Self::launch_inner(browser, purpose, budget, Some(supervisor))
    }

    fn launch_inner(
        browser: &InstalledBrowser,
        purpose: Purpose,
        budget: &Budget<'_>,
        supervisor: Option<&Path>,
    ) -> Result<Self> {
        budget.check()?;
        #[cfg(unix)]
        let profile: Option<tempfile::TempDir> = None;
        #[cfg(not(unix))]
        let profile = Some(
            tempfile::Builder::new()
                .prefix("lazypencil-chromium-")
                .tempdir()
                .map_err(|e| failed(format!("Cannot create browser profile: {e}")))?,
        );
        let mut command = Command::new(&browser.executable);
        command.args([
            "--headless",
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-breakpad",
            "--disable-crash-reporter",
            "--hide-scrollbars",
            "--mute-audio",
        ]);
        if let Some(profile) = &profile {
            command.arg(format!("--user-data-dir={}", profile.path().display()));
        }
        if purpose == Purpose::Shader {
            command.args([
                "--use-gl=angle",
                "--use-angle=swiftshader-webgl",
                "--enable-unsafe-swiftshader",
            ]);
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = process::Owned::spawn(
            command,
            supervisor,
            profile.as_ref().map(tempfile::TempDir::path),
            budget,
        )
        .map_err(|error| budget.check().err().unwrap_or(error))?;
        #[cfg(unix)]
        let profile_path = child.profile_path().to_path_buf();
        #[cfg(not(unix))]
        let profile_path = profile
            .as_ref()
            .expect("host browser profile")
            .path()
            .to_path_buf();
        let result = (|| {
            let port_file = profile_path.join("DevToolsActivePort");
            let (port, path) = loop {
                budget.check()?;
                if let Some(status) = child.try_wait().map_err(|e| failed(e.to_string()))? {
                    return Err(failed(format!(
                        "Chromium exited before CDP was ready ({status}); check OS sandbox permissions and runtime libraries with lazypencil --json doctor"
                    )).details(json!({"stage":"browser_startup","sandbox":"enabled","host":crate::runtime::host_status()})));
                }
                if let Ok(text) = fs::read_to_string(&port_file) {
                    let mut lines = text.lines();
                    if let (Some(port), Some(path)) = (
                        lines.next().and_then(|s| s.parse::<u16>().ok()),
                        lines.next(),
                    ) && path.starts_with("/devtools/browser/")
                        && path.len() < 256
                    {
                        break (port, path.to_string());
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let address = SocketAddr::from(([127, 0, 0, 1], port));
            connection::connect(address, &path, budget)
        })();
        let socket = match result {
            Ok(socket) => socket,
            Err(error) => {
                let error = budget.check().err().unwrap_or(error);
                child.stop();
                return Err(error);
            }
        };
        let mut browser = Self {
            child,
            socket,
            next_id: 0,
            version: String::new(),
            page_lifecycle: BTreeMap::new(),
            retained_tabs: VecDeque::new(),
            local_files: local_files::Interception::default(),
            purpose,
            _profile: profile,
        };
        let version = browser.command(None, "Browser.getVersion", json!({}), budget)?;
        browser.version = version["product"].as_str().unwrap_or("unknown").to_string();
        Ok(browser)
    }

    pub fn command(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
        budget: &Budget<'_>,
    ) -> Result<Value> {
        let id = self.send_command(session, method, params, budget)?;
        loop {
            budget.check()?;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    budget.check()?;
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| failed("Invalid CDP JSON"))?;
                    budget.check()?;
                    if let Some(lifecycle) = value["sessionId"]
                        .as_str()
                        .and_then(|session| self.page_lifecycle.get_mut(session))
                    {
                        lifecycle.observe(&value);
                    }
                    self.dispatch_local_files(&value, budget)?;
                    if value["id"] == id {
                        if let Some(error) = value.get("error") {
                            return Err(failed(format!(
                                "CDP {method} failed: {}",
                                error["message"].as_str().unwrap_or("protocol error")
                            )));
                        }
                        return Ok(value["result"].clone());
                    }
                }
                Ok(Message::Ping(_)) => {}
                Ok(Message::Close(_)) => {
                    budget.check()?;
                    return Err(failed("Chromium closed its CDP connection"));
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if pending(&error) => continue,
                Err(error) => {
                    budget.check()?;
                    return Err(failed(format!("CDP read failed: {error}")));
                }
            }
        }
    }

    // Event handlers enqueue commands without recursively reading the socket:
    // an unrelated response may arrive while a paused request is being served.
    fn send_command(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
        budget: &Budget<'_>,
    ) -> Result<u64> {
        budget.check()?;
        self.socket.get_mut().set_budget(budget);
        self.next_id += 1;
        let id = self.next_id;
        let mut request = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        match self.socket.send(Message::Text(request.to_string().into())) {
            Ok(()) => (),
            Err(tungstenite::Error::Io(error)) if pending(&error) => {}
            Err(error) => {
                budget.check()?;
                return Err(failed(format!("CDP write failed: {error}")));
            }
        }
        loop {
            budget.check()?;
            match self.socket.flush() {
                Ok(()) => break,
                Err(tungstenite::Error::Io(error)) if pending(&error) => continue,
                Err(error) => {
                    budget.check()?;
                    return Err(failed(format!("CDP flush failed: {error}")));
                }
            }
        }
        Ok(id)
    }

    /// Observe the selected session's main frame without retaining page or request data.
    pub fn watch_page_idle(&mut self, session: &str, budget: &Budget<'_>) -> Result<()> {
        let tree = self.command(Some(session), "Page.getFrameTree", json!({}), budget)?;
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or_else(|| failed("CDP did not return the main frame ID"))?;
        if self
            .page_lifecycle
            .get(session)
            .is_some_and(|lifecycle| lifecycle.matches_frame(frame))
        {
            return Ok(());
        }
        self.page_lifecycle
            .insert(session.into(), PageLifecycle::new(session, frame));
        self.command(
            Some(session),
            "Page.setLifecycleEventsEnabled",
            json!({"enabled":true}),
            budget,
        )?;
        Ok(())
    }

    pub fn page_is_idle(&self, session: &str, frame: &Value) -> bool {
        self.page_lifecycle
            .get(session)
            .is_some_and(|lifecycle| lifecycle.is_idle(session, frame))
    }

    pub fn evaluate(
        &mut self,
        session: &str,
        expression: &str,
        budget: &Budget<'_>,
    ) -> Result<Value> {
        let result = self.command(
            Some(session),
            "Runtime.evaluate",
            json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
            budget,
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            let description = exception["exception"]["description"]
                .as_str()
                .or_else(|| exception["text"].as_str())
                .unwrap_or("Page evaluation failed");
            return Err(Error::new(
                "PAGE_LOAD_FAILED",
                description
                    .lines()
                    .next()
                    .unwrap_or("Page evaluation failed")
                    .chars()
                    .take(1024)
                    .collect::<String>(),
            ));
        }
        result
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .ok_or_else(|| failed("Page evaluation returned no JSON value"))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.command(
            None,
            "Browser.close",
            json!({}),
            &Budget::new(Duration::from_millis(200), None),
        );
        self.child.stop();
    }
}
fn pending(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}
fn failed(message: impl Into<String>) -> Error {
    Error::new("RUNTIME_UNAVAILABLE", message)
}
