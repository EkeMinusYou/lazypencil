//! A local, synchronous Chrome DevTools Protocol transport with cancellation.

use crate::{Error, Result, runtime::InstalledBrowser};
use serde_json::{Value, json};
use std::{
    fs, io,
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Page,
    Shader,
}

pub struct Browser {
    child: Child,
    socket: WebSocket<TcpStream>,
    next_id: u64,
    pub version: String,
    _profile: tempfile::TempDir,
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

    pub fn launch(
        browser: &InstalledBrowser,
        purpose: Purpose,
        budget: &Budget<'_>,
    ) -> Result<Self> {
        budget.check()?;
        let profile = tempfile::Builder::new()
            .prefix("lazypencil-chromium-")
            .tempdir()
            .map_err(|e| failed(format!("Cannot create browser profile: {e}")))?;
        let mut command = Command::new(&browser.executable);
        command
            .args([
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
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()));
        if purpose == Purpose::Shader {
            command.args([
                "--use-gl=angle",
                "--use-angle=swiftshader-webgl",
                "--enable-unsafe-swiftshader",
            ]);
        }
        let mut child = command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| failed(format!("Cannot launch Chromium: {e}")))?;
        let result = (|| {
            let port_file = profile.path().join("DevToolsActivePort");
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
            let stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))
                .map_err(|e| failed(format!("Cannot connect to local CDP: {e}")))?;
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .map_err(|e| failed(e.to_string()))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .map_err(|e| failed(e.to_string()))?;
            let config = WebSocketConfig::default()
                .max_message_size(Some(96 * 1024 * 1024))
                .max_frame_size(Some(96 * 1024 * 1024));
            let (socket, _) = tungstenite::client::client_with_config(
                format!("ws://127.0.0.1:{port}{path}"),
                stream,
                Some(config),
            )
            .map_err(|e| failed(format!("CDP handshake failed: {e}")))?;
            socket
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(50)))
                .map_err(|e| failed(e.to_string()))?;
            socket
                .get_ref()
                .set_write_timeout(Some(Duration::from_millis(50)))
                .map_err(|e| failed(e.to_string()))?;
            Ok(socket)
        })();
        let socket = match result {
            Ok(socket) => socket,
            Err(error) => {
                let error = budget.check().err().unwrap_or(error);
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut browser = Self {
            child,
            socket,
            next_id: 0,
            version: String::new(),
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
        budget.check()?;
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
        loop {
            budget.check()?;
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| failed("Invalid CDP JSON"))?;
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

    pub fn with_tab<T>(
        &mut self,
        budget: &Budget<'_>,
        f: impl FnOnce(&mut Self, &str) -> Result<T>,
    ) -> Result<T> {
        let context = self.command(
            None,
            "Target.createBrowserContext",
            json!({"disposeOnDetach":true}),
            budget,
        )?;
        let context = context["browserContextId"]
            .as_str()
            .ok_or_else(|| failed("CDP did not return a context ID"))?
            .to_string();
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
            let attached = self.command(
                None,
                "Target.attachToTarget",
                json!({"targetId":target["targetId"],"flatten":true}),
                budget,
            )?;
            let session = attached["sessionId"]
                .as_str()
                .ok_or_else(|| failed("CDP did not return a session ID"))?;
            f(self, session)
        })();
        let _ = self.command(
            None,
            "Target.disposeBrowserContext",
            json!({"browserContextId":context}),
            &Budget::new(Duration::from_millis(500), None),
        );
        result
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
