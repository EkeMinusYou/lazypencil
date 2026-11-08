//! Connection-wide admission for retained payloads and persistent runtime owners.
use super::*;
use std::sync::{Arc, Mutex};

pub(super) struct Admission {
    total: Arc<tokio::sync::Semaphore>,
    documents: Mutex<std::collections::HashMap<String, std::sync::Weak<tokio::sync::Semaphore>>>,
}
pub(super) struct AdmissionPermit {
    _document: Option<tokio::sync::OwnedSemaphorePermit>,
    _connection: tokio::sync::OwnedSemaphorePermit,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            total: Arc::new(tokio::sync::Semaphore::new(16)),
            documents: Mutex::default(),
        }
    }
}
impl Admission {
    pub async fn reserve(
        &self,
        document: Option<&str>,
        context: &tokio_util::sync::CancellationToken,
        request: &tokio_util::sync::CancellationToken,
    ) -> Result<AdmissionPermit> {
        let document = if let Some(id) = document {
            let queue = {
                let mut documents = self.documents.lock().expect("document admission");
                documents.retain(|_, queue| queue.strong_count() > 0);
                let queue = documents
                    .get(id)
                    .and_then(std::sync::Weak::upgrade)
                    .unwrap_or_else(|| Arc::new(tokio::sync::Semaphore::new(4)));
                documents.insert(id.to_owned(), Arc::downgrade(&queue));
                queue
            };
            Some(acquire(queue, context, request).await?)
        } else {
            None
        };
        let connection = acquire(self.total.clone(), context, request).await?;
        Ok(AdmissionPermit {
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
