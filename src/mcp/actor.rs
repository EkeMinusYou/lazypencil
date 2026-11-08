//! Each document owns its native objects on one thread; only jobs cross threads.
use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

pub(super) struct Seed {
    pub store: Store,
    pub attachments: Option<crate::attachments::AttachmentAccess>,
    pub usage: Option<crate::guidance::usage::UsageAccess>,
    pub preview_control: Option<crate::preview_control::Client>,
    pub fonts: Vec<PathBuf>,
}

enum Message {
    Call(u64, Job),
    Close(Job),
}

type Pending = Arc<Mutex<HashMap<u64, CancellationToken>>>;
pub(super) struct Actor {
    pub store: Store,
    sender: Option<mpsc::UnboundedSender<Message>>,
    pending: Pending,
    next: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Actor {
    pub fn spawn(id: String, seed: Seed, resources: Arc<resources::Resources>) -> Result<Self> {
        let store = seed.store.clone();
        let (sender, receiver) = mpsc::unbounded_channel();
        let pending = Pending::default();
        let tracked = pending.clone();
        let thread = std::thread::Builder::new()
            .name("lazypencil-document".into())
            // Keep bounded recursive scene expansion on the native owner thread.
            .stack_size(crate::project::SCENE_WORKER_STACK_BYTES)
            .spawn(move || run(id, seed, resources, receiver, tracked))
            .map_err(|error| {
                Error::new("IO_ERROR", format!("Cannot start document worker: {error}"))
            })?;
        Ok(Self {
            store,
            sender: Some(sender),
            pending,
            next: 0,
            thread: Some(thread),
        })
    }
    pub fn enqueue(&mut self, job: Job) {
        self.next += 1;
        let sequence = self.next;
        self.pending
            .lock()
            .expect("pending requests")
            .insert(sequence, job.cancel.clone());
        if let Err(error) = self
            .sender
            .as_ref()
            .expect("open actor")
            .send(Message::Call(sequence, job))
        {
            self.pending
                .lock()
                .expect("pending requests")
                .remove(&sequence);
            if let Message::Call(_, job) = error.0 {
                job.finish(
                    Err(Error::new("INTERNAL_ERROR", "Document worker stopped")),
                    None,
                );
            }
        }
    }
    fn cancel_pending(&self) {
        for token in self.pending.lock().expect("pending requests").values() {
            token.cancel();
        }
    }
    pub fn close(&mut self, job: Job) {
        self.cancel_pending();
        if let Some(sender) = self.sender.take()
            && let Err(error) = sender.send(Message::Close(job))
            && let Message::Close(job) = error.0
        {
            job.finish(
                Err(Error::new("INTERNAL_ERROR", "Document worker stopped")),
                None,
            );
        }
    }
    pub fn reap(&mut self) -> bool {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            let _ = self.thread.take().expect("finished actor").join();
            true
        } else {
            false
        }
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        self.cancel_pending();
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    id: String,
    seed: Seed,
    resources: Arc<resources::Resources>,
    mut receiver: mpsc::UnboundedReceiver<Message>,
    pending: Pending,
) {
    let timer = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    let mut document = DocumentWorker {
        store: seed.store,
        fonts: seed.fonts,
        attachments: seed.attachments,
        usage: seed.usage,
        preview_control: seed.preview_control,
        #[cfg(feature = "rendering")]
        resources: resources.clone(),
        #[cfg(feature = "rendering")]
        runtime_lease: None,
        #[cfg(feature = "rendering")]
        runtime_bytes: None,
        #[cfg(feature = "rendering")]
        snapshots: VecDeque::new(),
        #[cfg(feature = "rendering")]
        failed_code: VecDeque::new(),
        #[cfg(feature = "rendering")]
        scripts: None,
        #[cfg(feature = "rendering")]
        dynamic: None,
    };
    loop {
        #[cfg(feature = "rendering")]
        document.settle_resources();
        let message = timer.block_on(async {
            #[cfg(feature = "rendering")]
            if let Some(deadline) = document.maintenance_deadline() {
                return tokio::time::timeout_at(deadline.into(), receiver.recv())
                    .await
                    .ok();
            }
            Some(receiver.recv().await)
        });
        let Some(message) = message else { continue };
        let Some(message) = message else { break };
        match message {
            Message::Close(job) => {
                let started = Instant::now();
                drop(document);
                job.finish(
                    structured(json!({"document_id":id,"closed":true})),
                    Some(started),
                );
                break;
            }
            Message::Call(sequence, job) => {
                let permit = timer.block_on(async {
                    tokio::select! {
                        biased;
                        _ = job.cancel.cancelled() => None,
                        permit = resources.execution.clone().acquire_owned() => permit.ok(),
                    }
                });
                let started = Instant::now();
                let skipped = permit.is_none() || job.cancelled();
                let result = if skipped {
                    Err(Error::new(
                        "CANCELLED",
                        "The request was cancelled before execution",
                    ))
                } else {
                    #[cfg(feature = "rendering")]
                    let _pool = crate::native_memory::AutoreleasePool::new();
                    document.call_scoped(&id, &job.name, job.arguments.clone(), &job.cancel)
                };
                #[cfg(feature = "rendering")]
                document.settle_resources();
                pending.lock().expect("pending requests").remove(&sequence);
                job.finish(result, (!skipped).then_some(started));
                drop(permit);
            }
        }
    }
}

impl DocumentWorker {
    fn call_scoped(
        &mut self,
        id: &str,
        name: &str,
        mut arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<CallToolResult> {
        let kind =
            ToolKind::parse(name).ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown tool"))?;
        let actual = crate::paths::canonical(self.store.path())?;
        if actual != self.store.path() {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "The opened document path was redirected; reopen the document",
            ));
        }
        for (field, code) in [
            ("snapshot_id", "SNAPSHOT_UNAVAILABLE"),
            ("cursor", "INVALID_ARGUMENT"),
        ] {
            if let Some(value) = arguments.get(field).filter(|value| !value.is_null()) {
                let prefix = format!("{id}:");
                let token = value
                    .as_str()
                    .and_then(|value| value.strip_prefix(&prefix))
                    .ok_or_else(|| {
                        Error::new(
                            code,
                            "The calculation or cursor belongs to another document or connection",
                        )
                    })?;
                let token = token.to_owned();
                arguments[field] = json!(token);
            }
        }
        let usage = if kind.catalogue_query() {
            self.usage
                .as_ref()
                .map(|usage| usage.request(cancel))
                .transpose()?
        } else {
            None
        };
        let mut result = self.call(name, arguments, cancel)?;
        if let Some(value) = &mut result.structured_content {
            for field in ["snapshot_id", "next_cursor"] {
                if let Some(token) = value.get(field).and_then(Value::as_str) {
                    let scoped = format!("{id}:{token}");
                    value[field] = json!(scoped);
                }
            }
            value["document_id"] = json!(id);
            crate::query::check_response_size(
                value,
                MAX_TOOL_BYTES,
                Some(cancel),
                "Tool result exceeds 4 MiB; reduce depth or page size",
            )?;
            let text = value.to_string();
            if let Some(usage) = usage {
                usage.record(name, value, cancel)?;
            }
            result.content.retain(|content| content.as_text().is_none());
            result
                .content
                .insert(0, rmcp::model::ContentBlock::text(text));
        }
        Ok(result)
    }
}
