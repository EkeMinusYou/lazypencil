//! Connection-wide admission for retained payloads and persistent runtime owners.
use super::*;
use std::sync::{Arc, Mutex};

const MAX_PENDING_REQUESTS: usize = 64;
const MAX_PENDING_PER_DOCUMENT: usize = 32;
const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_PENDING_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CLOSE_REQUESTS: usize = 8;
const MAX_CLOSE_BYTES: usize = 4096;

pub(super) struct Admission {
    total: Arc<tokio::sync::Semaphore>,
    pending: Arc<Budget>,
    inputs: Arc<Budget>,
    closes: Arc<Budget>,
    close_inputs: Arc<Budget>,
    documents: Mutex<std::collections::HashMap<String, std::sync::Weak<DocumentAdmission>>>,
}
struct DocumentAdmission {
    execution: Arc<tokio::sync::Semaphore>,
    pending: Arc<Budget>,
    inputs: Arc<Budget>,
}
pub(super) struct Registration {
    _pending: Lease,
    _inputs: Lease,
    document: Option<DocumentRegistration>,
    close: bool,
}
struct DocumentRegistration {
    queue: Arc<DocumentAdmission>,
    _pending: Lease,
    _inputs: Lease,
}
pub(super) struct AdmissionPermit {
    _registration: Registration,
    _document: Option<tokio::sync::OwnedSemaphorePermit>,
    _connection: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            total: Arc::new(tokio::sync::Semaphore::new(16)),
            pending: Budget::new(MAX_PENDING_REQUESTS),
            inputs: Budget::new(MAX_PENDING_BYTES),
            closes: Budget::new(MAX_CLOSE_REQUESTS),
            close_inputs: Budget::new(MAX_CLOSE_REQUESTS * MAX_CLOSE_BYTES),
            documents: Mutex::default(),
        }
    }
}
impl Admission {
    /// Reserve input before any await, including time spent outside execution admission.
    pub fn register(
        &self,
        document: Option<&str>,
        bytes: usize,
        close: bool,
    ) -> Result<Registration> {
        if document.is_some_and(|id| id.len() > 128) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "document_id exceeds 128 bytes",
            ));
        }
        if close && bytes > MAX_CLOSE_BYTES {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "close_document input exceeds 4 KiB",
            ));
        }
        let reserve = |budget: &Arc<Budget>, amount, scope| {
            budget.reserve(amount).map_err(|error| {
                Error::new("RESOURCE_LIMIT", "MCP pending input capacity is full; cancel pending requests or retry after they finish")
                    .details(json!({"scope":scope,"budget":error.details}))
            })
        };
        let pending = reserve(
            if close { &self.closes } else { &self.pending },
            1,
            "connection_requests",
        )?;
        let inputs = reserve(
            if close {
                &self.close_inputs
            } else {
                &self.inputs
            },
            bytes,
            "connection_input_bytes",
        )?;
        let document = if !close && let Some(id) = document {
            let queue = {
                let mut documents = self.documents.lock().expect("document admission");
                documents.retain(|_, queue| queue.strong_count() > 0);
                let queue = documents
                    .get(id)
                    .and_then(std::sync::Weak::upgrade)
                    .unwrap_or_else(|| {
                        Arc::new(DocumentAdmission {
                            execution: Arc::new(tokio::sync::Semaphore::new(4)),
                            pending: Budget::new(MAX_PENDING_PER_DOCUMENT),
                            inputs: Budget::new(MAX_PENDING_DOCUMENT_BYTES),
                        })
                    });
                documents.insert(id.to_owned(), Arc::downgrade(&queue));
                queue
            };
            Some(DocumentRegistration {
                _pending: reserve(&queue.pending, 1, "document_requests")?,
                _inputs: reserve(&queue.inputs, bytes, "document_input_bytes")?,
                queue,
            })
        } else {
            None
        };
        Ok(Registration {
            _pending: pending,
            _inputs: inputs,
            document,
            close,
        })
    }

    pub async fn reserve(
        &self,
        registration: Registration,
        context: &tokio_util::sync::CancellationToken,
        request: &tokio_util::sync::CancellationToken,
    ) -> Result<AdmissionPermit> {
        if context.is_cancelled() || request.is_cancelled() {
            return Err(Error::new(
                "CANCELLED",
                "The request was cancelled before admission",
            ));
        }
        let document = if let Some(document) = &registration.document {
            Some(acquire(document.queue.execution.clone(), context, request).await?)
        } else {
            None
        };
        let connection = if registration.close {
            None
        } else {
            Some(acquire(self.total.clone(), context, request).await?)
        };
        Ok(AdmissionPermit {
            _registration: registration,
            _document: document,
            _connection: connection,
        })
    }
}
async fn acquire(
    queue: Arc<tokio::sync::Semaphore>,
    context: &tokio_util::sync::CancellationToken,
    request: &tokio_util::sync::CancellationToken,
) -> Result<tokio::sync::OwnedSemaphorePermit> {
    tokio::select! {
        biased;
        _ = request_cancelled(context, request) => Err(Error::new("CANCELLED", "The request was cancelled before admission")),
        permit = queue.acquire_owned() => permit.map_err(|_| Error::new("INTERNAL_ERROR", "Document admission stopped")),
    }
}

pub(super) const MAX_RETAINED_BYTES: usize = 256 * 1024 * 1024;
#[cfg(feature = "rendering")]
pub(super) const MAX_RUNTIME_OWNERS: usize = 4;
pub(super) struct Resources {
    pub execution: Arc<tokio::sync::Semaphore>,
    pub retained: Arc<Budget>,
    #[cfg(feature = "rendering")]
    pub runtimes: Arc<Budget>,
}
impl Default for Resources {
    fn default() -> Self {
        Self {
            execution: Arc::new(tokio::sync::Semaphore::new(2)),
            retained: Budget::new(MAX_RETAINED_BYTES),
            #[cfg(feature = "rendering")]
            runtimes: Budget::new(MAX_RUNTIME_OWNERS),
        }
    }
}
pub(super) struct Budget {
    limit: usize,
    used: Mutex<usize>,
}
pub(super) struct Lease {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Budget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: Mutex::new(0),
        })
    }
    pub fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Lease> {
        let mut lease = Lease {
            budget: self.clone(),
            bytes: 0,
        };
        lease.resize(bytes)?;
        Ok(lease)
    }
}
impl Lease {
    pub fn resize(&mut self, bytes: usize) -> Result<()> {
        let mut used = self.budget.used.lock().expect("resource budget");
        let next = used.saturating_sub(self.bytes).saturating_add(bytes);
        if next > self.budget.limit {
            return Err(Error::new("RESOURCE_LIMIT", "The connection resource budget is full; close unused documents or retry after idle cleanup")
                .details(json!({"limit":self.budget.limit,"used":*used,"requested":bytes})));
        }
        *used = next;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        *self.budget.used.lock().expect("resource budget") -= self.bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_registrations_bound_counts_per_document_and_connection() {
        let admission = Admission::default();
        let first = (0..MAX_PENDING_PER_DOCUMENT)
            .map(|_| admission.register(Some("first"), 1, false).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            admission
                .register(Some("first"), 1, false)
                .err()
                .unwrap()
                .code,
            "RESOURCE_LIMIT"
        );
        let second = (0..MAX_PENDING_PER_DOCUMENT)
            .map(|_| admission.register(Some("second"), 1, false).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            admission
                .register(Some("third"), 1, false)
                .err()
                .unwrap()
                .code,
            "RESOURCE_LIMIT"
        );
        let closes = (0..MAX_CLOSE_REQUESTS)
            .map(|_| {
                admission
                    .register(Some("first"), MAX_CLOSE_BYTES, true)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            admission
                .register(Some("first"), 1, true)
                .err()
                .unwrap()
                .code,
            "RESOURCE_LIMIT"
        );
        drop(closes);
        drop(first);
        drop(second);
        assert_eq!(*admission.pending.used.lock().unwrap(), 0);
        assert_eq!(*admission.inputs.used.lock().unwrap(), 0);
        assert_eq!(*admission.closes.used.lock().unwrap(), 0);
        assert_eq!(*admission.close_inputs.used.lock().unwrap(), 0);
        admission.register(Some("first"), 1, false).unwrap();
    }

    #[test]
    fn pending_inputs_bound_bytes_and_failed_reservations_release_capacity() {
        let admission = Admission::default();
        let mut inputs = Vec::new();
        for document in ["first", "second", "third", "fourth"] {
            inputs.push(
                admission
                    .register(Some(document), MAX_PENDING_DOCUMENT_BYTES, false)
                    .unwrap(),
            );
            assert_eq!(
                admission
                    .register(Some(document), 1, false)
                    .err()
                    .unwrap()
                    .code,
                "RESOURCE_LIMIT"
            );
        }
        assert_eq!(*admission.pending.used.lock().unwrap(), 4);
        assert_eq!(*admission.inputs.used.lock().unwrap(), MAX_PENDING_BYTES);
        assert_eq!(
            admission.register(None, 1, false).err().unwrap().code,
            "RESOURCE_LIMIT"
        );
        admission
            .register(Some("first"), MAX_CLOSE_BYTES, true)
            .unwrap();
        assert_eq!(
            admission
                .register(None, MAX_CLOSE_BYTES + 1, true)
                .err()
                .unwrap()
                .code,
            "INVALID_ARGUMENT"
        );
        drop(inputs.pop());
        admission
            .register(None, MAX_PENDING_DOCUMENT_BYTES, false)
            .unwrap();
        drop(inputs);
        assert_eq!(*admission.pending.used.lock().unwrap(), 0);
        assert_eq!(*admission.inputs.used.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn pending_waiter_cancellation_releases_inputs_and_close_bypasses_execution() {
        let admission = Admission::default();
        let context = tokio_util::sync::CancellationToken::new();
        let request = tokio_util::sync::CancellationToken::new();
        let mut active = Vec::new();
        for document in ["first", "second", "third", "fourth"] {
            for _ in 0..4 {
                active.push(
                    admission
                        .reserve(
                            admission.register(Some(document), 17, false).unwrap(),
                            &context,
                            &request,
                        )
                        .await
                        .unwrap(),
                );
            }
        }
        let waiting_cancel = tokio_util::sync::CancellationToken::new();
        let waiting = admission.reserve(
            admission.register(Some("first"), 100, false).unwrap(),
            &context,
            &waiting_cancel,
        );
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        waiting_cancel.cancel();
        assert_eq!(waiting.await.err().unwrap().code, "CANCELLED");
        assert_eq!(*admission.pending.used.lock().unwrap(), 16);
        assert_eq!(*admission.inputs.used.lock().unwrap(), 16 * 17);
        let close = admission.reserve(
            admission.register(Some("first"), 100, true).unwrap(),
            &context,
            &request,
        );
        tokio::time::timeout(Duration::from_millis(100), close)
            .await
            .unwrap()
            .unwrap();
        drop(active);
        assert_eq!(*admission.pending.used.lock().unwrap(), 0);
        assert_eq!(*admission.inputs.used.lock().unwrap(), 0);
        let cancelled = admission.register(Some("first"), 100, false).unwrap();
        assert_eq!(
            admission
                .reserve(cancelled, &waiting_cancel, &request)
                .await
                .err()
                .unwrap()
                .code,
            "CANCELLED"
        );
        assert_eq!(*admission.inputs.used.lock().unwrap(), 0);
    }

    #[test]
    fn reservations_are_shared_atomic_and_release_only_their_own_bytes() {
        let budget = Budget::new(100);
        let mut first = budget.reserve(60).unwrap();
        let second = budget.reserve(40).unwrap();
        assert_eq!(budget.reserve(1).err().unwrap().code, "RESOURCE_LIMIT");
        assert!(first.resize(61).is_err());
        assert_eq!(*budget.used.lock().unwrap(), 100);
        drop(second);
        first.resize(80).unwrap();
        let third = budget.reserve(20).unwrap();
        drop(first);
        assert_eq!(*budget.used.lock().unwrap(), 20);
        drop(third);
        assert_eq!(*budget.used.lock().unwrap(), 0);
    }

    #[cfg(feature = "rendering")]
    #[test]
    fn independent_native_owners_share_retention_capacity_and_close_releases_it() {
        use super::super::actor::{Actor, Seed};
        fn request(actor: &mut Actor, name: &str, arguments: Value, close: bool) -> CallToolResult {
            let (reply, receiver) = oneshot::channel();
            let job = Job {
                name: name.into(),
                arguments,
                reply,
                cancel: tokio_util::sync::CancellationToken::new(),
                timing: timing::Timer::new(),
                usage: Ok(None),
                _admission: None,
            };
            if close {
                actor.close(job);
            } else {
                actor.enqueue(job);
            }
            receiver.blocking_recv().unwrap()
        }
        let temp = tempfile::TempDir::new().unwrap();
        let resources = Arc::new(Resources::default());
        let mut actors = Vec::new();
        let mut revisions = Vec::new();
        for index in 0..2 {
            let store = Store::init(
                temp.path().join(format!("{index}.pen")),
                Some(temp.path().join("data")),
            )
            .unwrap();
            revisions.push(store.read().unwrap().revision());
            actors.push(
                Actor::spawn(
                    format!("doc{index}"),
                    Seed {
                        store,
                        fonts: vec![],
                        attachments: None,
                        usage: None,
                        preview_control: None,
                    },
                    resources.clone(),
                )
                .unwrap(),
            );
        }
        let first = request(
            &mut actors[0],
            "get_layout",
            json!({"expected_revision":revisions[0]}),
            false,
        );
        assert_ne!(first.is_error, Some(true), "{first:?}");
        let retained = *resources.retained.used.lock().unwrap();
        assert!(retained > 0);
        // Simulate other retained owners filling all remaining connection capacity.
        let pressure = resources
            .retained
            .reserve(MAX_RETAINED_BYTES - retained)
            .unwrap();
        let denied = request(
            &mut actors[1],
            "get_layout",
            json!({"expected_revision":revisions[1]}),
            false,
        );
        assert_eq!(
            denied.structured_content.as_ref().unwrap()["error"]["code"],
            "RESOURCE_LIMIT"
        );
        drop(pressure);
        let second = request(
            &mut actors[1],
            "get_layout",
            json!({"expected_revision":revisions[1]}),
            false,
        );
        assert_ne!(second.is_error, Some(true), "{second:?}");
        assert!(*resources.retained.used.lock().unwrap() >= retained * 2);
        request(&mut actors[0], "close_document", json!({}), true);
        let remaining = *resources.retained.used.lock().unwrap();
        assert!(remaining > 0 && remaining < retained * 2);
        request(&mut actors[1], "close_document", json!({}), true);
        assert_eq!(*resources.retained.used.lock().unwrap(), 0);
    }
}
