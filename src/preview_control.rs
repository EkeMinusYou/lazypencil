//! Authenticated, invocation-local preview requests; never document edits.
use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(crate) const INSTRUCTIONS: &str = "A live TUI preview is connected. Interpret preview requests using get_preview_state and set_preview: inspect state for relative requests such as 'zoom out a little' or 'show the right side'; use a document target for the whole design, node targets for one or more elements, region targets for explicit document-space bounds, or a point target to center without fitting. set_preview supports keeping the current scale, absolute zoom, relative zoom_factor and document-space pan. For references such as 'this' or 'the selected item', read get_preview_state and inspect its selected_node.node_id with read_nodes; the current camera center is not necessarily the editing selection. When the user refers to an element by name, type or description, search with search_nodes (name/type filters, calculated:true) FIRST and inspect the matching IDs, read_target addresses and bounds; search more broadly or inspect candidates if needed. Do not guess node IDs, silently choose among ambiguous matches, or move document nodes to satisfy a preview request. When the user asks to modify a particular existing element (including 'fix A'), resolve the target and set_preview BEFORE editing so the user can follow your work. After editing, refocus if its position or size changed; if deleted, use a surviving relevant parent. Focus a new element once it exists. Preserve keyboard focus and editing selection. Display success is not visual inspection: use render to inspect pixels. If a display operation fails, do not claim it succeeded; continue authorized edits when appropriate and report the limitation.";
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PreviewTarget {
    Document,
    Nodes { node_ids: Vec<String> },
    Region { region: crate::geometry::Rect },
    Point { center: [f64; 2] },
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetPreviewRequest {
    /// Omit to adjust the current view. Nodes must be exact calculated IDs from read_nodes/get_layout.
    pub target: Option<PreviewTarget>,
    /// Fit the target with padding (default true). False centers it at the current scale. Point targets never fit.
    #[serde(default = "default_fit")]
    pub fit: bool,
    /// Absolute scale in 0.01..=16. Overrides target fitting. Cannot accompany zoom_factor.
    pub zoom: Option<f32>,
    /// Multiply the current or fitted scale; the result is clamped to 0.01..=16.
    pub zoom_factor: Option<f32>,
    /// Relative camera movement [dx, dy] in document units; positive is right/down. Applied last.
    pub pan: Option<[f64; 2]>,
    pub expected_revision: Option<String>,
    pub expected_dependency_revision: Option<String>,
}
fn default_fit() -> bool {
    true
}
impl SetPreviewRequest {
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            Error::new(
                "INVALID_ARGUMENT",
                "Specify a valid target or pan/zoom adjustment; coordinates must be finite, bounds positive, zoom 0.01..=16 and zoom_factor positive",
            )
        };
        if self.target.is_none()
            && self.zoom.is_none()
            && self.zoom_factor.is_none()
            && self.pan.is_none()
        {
            return Err(invalid());
        }
        if self.zoom.zip(self.zoom_factor).is_some()
            || self
                .zoom
                .is_some_and(|zoom| !zoom.is_finite() || !(0.01..=16.0).contains(&zoom))
            || self
                .zoom_factor
                .is_some_and(|factor| !factor.is_finite() || factor <= 0.0)
            || self
                .pan
                .is_some_and(|pan| !pan.into_iter().all(f64::is_finite))
        {
            return Err(invalid());
        }
        match &self.target {
            Some(PreviewTarget::Nodes { node_ids }) => {
                if node_ids.is_empty()
                    || node_ids.len() > 64
                    || node_ids.iter().any(|id| id.is_empty() || id.len() > 4096)
                    || node_ids
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != node_ids.len()
                {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Expected 1..64 distinct calculated node IDs of 1..4096 bytes",
                    ));
                }
            }
            Some(PreviewTarget::Region { region }) => {
                if ![
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                    region.right(),
                    region.bottom(),
                ]
                .into_iter()
                .all(f32::is_finite)
                    || region.width <= 0.0
                    || region.height <= 0.0
                {
                    return Err(invalid());
                }
            }
            Some(PreviewTarget::Point { center })
                if !center.iter().all(|value| (*value as f32).is_finite()) =>
            {
                return Err(invalid());
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ControlRequest {
    GetState,
    Set { view: SetPreviewRequest },
}
impl ControlRequest {
    fn validate(&self) -> Result<()> {
        match self {
            Self::GetState => Ok(()),
            Self::Set { view } => view.validate(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    address: SocketAddr,
    token: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireRequest {
    token: String,
    document: PathBuf,
    conversation: String,
    request: ControlRequest,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Client {
    endpoint: Endpoint,
    document: PathBuf,
    conversation: String,
}
impl Client {
    pub fn parse(value: &str, document: &std::path::Path) -> Result<Self> {
        let client: Self = serde_json::from_str(value)
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid preview endpoint"))?;
        if !client.endpoint.address.ip().is_loopback() || client.document != document {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Preview endpoint must be local and document-bound",
            ));
        }
        Ok(client)
    }
    pub fn for_conversation(&self, conversation: &str) -> Self {
        Self {
            conversation: conversation.into(),
            ..self.clone()
        }
    }
    pub fn encode(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))
    }
    pub fn state(&self, cancel: &CancellationToken) -> Result<Value> {
        self.call(ControlRequest::GetState, cancel)
    }
    pub fn set(&self, view: SetPreviewRequest, cancel: &CancellationToken) -> Result<Value> {
        self.call(ControlRequest::Set { view }, cancel)
    }
    fn call(&self, request: ControlRequest, cancel: &CancellationToken) -> Result<Value> {
        request.validate()?;
        crate::cancel::check(Some(cancel))?;
        let mut stream = TcpStream::connect_timeout(&self.endpoint.address, Duration::from_secs(1))
            .map_err(unavailable)?;
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(unavailable)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(unavailable)?;
        let wire = WireRequest {
            token: self.endpoint.token.clone(),
            document: self.document.clone(),
            conversation: self.conversation.clone(),
            request,
        };
        let mut bytes = serde_json::to_vec(&wire)
            .map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))?;
        if bytes.len() >= MAX_BYTES {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Preview request is too large",
            ));
        }
        bytes.push(b'\n');
        stream.write_all(&bytes).map_err(unavailable)?;
        let deadline = Instant::now() + TIMEOUT;
        let mut response = Vec::new();
        loop {
            crate::cancel::check(Some(cancel))?;
            if Instant::now() >= deadline {
                return Err(unavailable("Preview response timed out"));
            }
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => return Err(unavailable("Preview connection closed")),
                Ok(count) => {
                    response.extend_from_slice(&buffer[..count]);
                    if response.len() > MAX_BYTES {
                        return Err(unavailable("Preview response is too large"));
                    }
                    if response.ends_with(b"\n") {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => return Err(unavailable(error)),
            }
        }
        let value: Value = serde_json::from_slice(&response).map_err(unavailable)?;
        if let Some(error) = value.get("error") {
            let code = match error["code"].as_str() {
                Some("INVALID_ARGUMENT") => "INVALID_ARGUMENT",
                Some("REVISION_CONFLICT") => "REVISION_CONFLICT",
                Some("CANCELLED") => "CANCELLED",
                Some("BUSY") => "BUSY",
                _ => "PREVIEW_UNAVAILABLE",
            };
            return Err(Error::new(
                code,
                error["message"]
                    .as_str()
                    .unwrap_or("Preview request failed"),
            ));
        }
        Ok(value)
    }
}
fn unavailable(error: impl std::fmt::Display) -> Error {
    Error::new("PREVIEW_UNAVAILABLE", error.to_string())
}

#[cfg(feature = "tui")]
pub(crate) struct Incoming {
    pub document: PathBuf,
    pub conversation: String,
    pub request: ControlRequest,
    pub reply: tokio::sync::oneshot::Sender<Result<Value>>,
    pub deadline: Instant,
}
#[cfg(feature = "tui")]
pub(crate) struct Host {
    endpoint: Endpoint,
    receiver: tokio::sync::mpsc::Receiver<Incoming>,
    task: tokio::task::JoinHandle<()>,
}
#[cfg(feature = "tui")]
impl Host {
    pub fn start() -> Result<Self> {
        let listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(unavailable)?;
        listener.set_nonblocking(true).map_err(unavailable)?;
        let endpoint = Endpoint {
            address: listener.local_addr().map_err(unavailable)?,
            token: uuid::Uuid::new_v4().to_string(),
        };
        let listener = tokio::net::TcpListener::from_std(listener).map_err(unavailable)?;
        let token = endpoint.token.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(async move {
            // Serve one bounded request at a time. An unauthenticated peer never
            // enters the UI queue, and cannot hold a connection indefinitely.
            while let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let wire = tokio::time::timeout(Duration::from_secs(1), async {
                    let mut bytes = Vec::new();
                    loop {
                        let byte = stream.read_u8().await.ok()?;
                        if byte == b'\n' {
                            break;
                        }
                        bytes.push(byte);
                        if bytes.len() >= MAX_BYTES {
                            return None;
                        }
                    }
                    serde_json::from_slice::<WireRequest>(&bytes).ok()
                })
                .await
                .ok()
                .flatten();
                let Some(wire) = wire.filter(|wire| wire.token == token) else {
                    continue;
                };
                if wire.request.validate().is_err() {
                    continue;
                }
                let (reply, mut result) = tokio::sync::oneshot::channel();
                let incoming = Incoming {
                    document: wire.document,
                    conversation: wire.conversation,
                    request: wire.request,
                    reply,
                    deadline: Instant::now() + TIMEOUT,
                };
                if sender.try_send(incoming).is_err() {
                    continue;
                }
                let response = tokio::select! {
                    result = &mut result => result.unwrap_or_else(|_| Err(unavailable("Preview host stopped"))),
                    _ = tokio::time::sleep(TIMEOUT) => Err(unavailable("Preview response timed out")),
                    // EOF means the MCP call was cancelled or its process exited.
                    _ = stream.read_u8() => continue,
                };
                let value = response.unwrap_or_else(|error| serde_json::json!({"error": error}));
                if let Ok(mut bytes) = serde_json::to_vec(&value) {
                    if bytes.len() >= MAX_BYTES {
                        bytes = serde_json::to_vec(&serde_json::json!({"error": unavailable("Preview response exceeds its size limit")})).expect("serializable error");
                    }
                    bytes.push(b'\n');
                    let _ = tokio::time::timeout(Duration::from_secs(1), stream.write_all(&bytes))
                        .await;
                }
            }
        });
        Ok(Self {
            endpoint,
            receiver,
            task,
        })
    }
    pub fn client(&self, document: &std::path::Path, conversation: &str) -> Client {
        Client {
            endpoint: self.endpoint.clone(),
            document: document.into(),
            conversation: conversation.into(),
        }
    }
    pub async fn next(&mut self) -> Option<Incoming> {
        self.receiver.recv().await
    }
}
#[cfg(feature = "tui")]
impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(all(test, feature = "tui"))]
mod tests {
    use super::*;
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn request() -> SetPreviewRequest {
        serde_json::from_value(serde_json::json!({"target":{"kind":"nodes","node_ids":["button"]}}))
            .unwrap()
    }
    #[test]
    fn preview_transport_waits_for_ui_ack_and_rejects_foreign_endpoints() {
        runtime().block_on(async {
            let mut host = Host::start().unwrap();
            let path = PathBuf::from("/design.pen");
            let client = host.client(&path, "session");
            assert!(
                Client::parse(
                    &client.encode().unwrap(),
                    std::path::Path::new("/other.pen")
                )
                .is_err()
            );
            let mut foreign = client.clone();
            foreign.endpoint.address = "192.0.2.1:1234".parse().unwrap();
            assert!(Client::parse(&foreign.encode().unwrap(), &path).is_err());
            let call = tokio::task::spawn_blocking(move || {
                client.set(request(), &CancellationToken::new())
            });
            let incoming = tokio::time::timeout(Duration::from_secs(3), host.next())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(incoming.conversation, "session");
            assert_eq!(incoming.document, path);
            assert!(!call.is_finished());
            incoming
                .reply
                .send(Ok(serde_json::json!({"status":"updated"})))
                .unwrap();
            assert_eq!(call.await.unwrap().unwrap()["status"], "updated");
        });
    }
    #[test]
    fn preview_transport_authenticates_and_cancellation_closes_pending_reply() {
        runtime().block_on(async {
            let mut host = Host::start().unwrap();
            let client = host.client(std::path::Path::new("/design.pen"), "session");
            let mut foreign = client.clone();
            foreign.endpoint.token = "wrong".into();
            let call = tokio::task::spawn_blocking(move || {
                foreign.set(request(), &CancellationToken::new())
            });
            assert_eq!(call.await.unwrap().unwrap_err().code, "PREVIEW_UNAVAILABLE");
            assert!(
                tokio::time::timeout(Duration::from_millis(100), host.next())
                    .await
                    .is_err()
            );
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            let call = tokio::task::spawn_blocking(move || client.set(request(), &task_cancel));
            let mut incoming = tokio::time::timeout(Duration::from_secs(3), host.next())
                .await
                .unwrap()
                .unwrap();
            cancel.cancel();
            assert_eq!(call.await.unwrap().unwrap_err().code, "CANCELLED");
            tokio::time::timeout(Duration::from_secs(3), incoming.reply.closed())
                .await
                .unwrap();
        });
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn camera_requests_reject_ambiguous_invalid_or_unbounded_operations() {
        for value in [
            json!({}),
            json!({"zoom": 0.0}),
            json!({"zoom": 17.0}),
            json!({"zoom": 2.0, "zoom_factor": 0.5}),
            json!({"zoom_factor": -1.0}),
            json!({"target":{"kind":"nodes","node_ids":[]}}),
            json!({"target":{"kind":"nodes","node_ids":["same","same"]}}),
            json!({"target":{"kind":"region","region":{"x":0,"y":0,"width":0,"height":10}}}),
        ] {
            let request: SetPreviewRequest = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(
                request.validate().unwrap_err().code,
                "INVALID_ARGUMENT",
                "{value}"
            );
        }
        assert!(
            serde_json::from_value::<SetPreviewRequest>(
                json!({"target":{"kind":"nodes","node_ids":["a"],"center":[0,0]}})
            )
            .is_err()
        );
        let mut request: SetPreviewRequest = serde_json::from_value(json!({"pan":[1,2]})).unwrap();
        request.pan = Some([f64::INFINITY, 0.0]);
        assert!(request.validate().is_err());
    }
}
