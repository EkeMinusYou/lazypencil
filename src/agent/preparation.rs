//! Warm the adapter transport without choosing a document or creating a session.
use super::*;

impl AgentHost {
    pub fn prepare(options: AgentOptions) -> Result<Self> {
        let (host_sender, host_receiver) = std::sync::mpsc::sync_channel(1);
        let (session_sender, session_receiver) = oneshot::channel();
        let original = options.clone();
        std::thread::Builder::new()
            .name("lazypencil-agent-prepare".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let runtime = match runtime {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = host_sender.send(Err(Error::new("IO_ERROR", error.to_string())));
                        return;
                    }
                };
                runtime.block_on(async {
                    let (finished, completion) = oneshot::channel();
                    match Self::spawn_task(
                        options,
                        Purpose::Conversation,
                        Some(session_receiver),
                        Some(finished),
                    ) {
                        Ok(host) => {
                            let _ = host_sender.send(Ok(host));
                            let _ = completion.await;
                        }
                        Err(error) => {
                            let _ = host_sender.send(Err(error));
                        }
                    }
                });
                runtime.shutdown_timeout(Duration::from_secs(1));
            })
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        let mut host = host_receiver
            .recv()
            .map_err(|_| Error::new("AGENT_ERROR", "Agent preparation worker stopped"))??;
        host.deferred_options = Some(session_sender);
        host.prepared_options = Some(original);
        Ok(host)
    }

    pub(crate) fn continue_prepared(&mut self, options: AgentOptions) -> bool {
        let Some(original) = &self.prepared_options else {
            return false;
        };
        // Process cwd and authentication affect native configuration before ACP
        // setup. A different project/profile must use its own fresh process.
        if original.command != options.command
            || original.args != options.args
            || original.cwd != options.cwd
            || original.subscription != options.subscription
            || original.supervisor_executable != options.supervisor_executable
        {
            return false;
        }
        self.deferred_options
            .take()
            .is_some_and(|sender| sender.send(options).is_ok())
    }

    #[cfg(feature = "tui")]
    pub(crate) fn try_next_event(&mut self) -> Option<AgentEvent> {
        self.events.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, time::Duration};
    mod fixture_support {
        pub fn helper() -> std::path::PathBuf {
            std::env::var_os("LAZYPENCIL_TEST_HELPER")
                .map(std::path::PathBuf::from)
                .expect("Prepared native helper")
        }
    }

    fn options(root: &Path) -> AgentOptions {
        let mut options = AgentOptions::new(
            fixture_support::helper(),
            crate::paths::canonical(root).unwrap(),
        );
        options.args = vec!["acp".into(), "auth-inspect-account".into()];
        options.supervisor_executable = Some(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("lazypencil"),
        );
        options.cancel_grace = Duration::from_millis(300);
        options
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    async fn prepared(host: &mut AgentHost) -> u64 {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match host.next_event().await {
                    Some(AgentEvent::Message(value)) if value["type"] == "agent_prepared" => {
                        return value["process_id"].as_u64().unwrap();
                    }
                    Some(AgentEvent::Message(value)) if value["type"] == "connection_error" => {
                        panic!("{value}")
                    }
                    None => panic!("Preparation stopped"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap()
    }
    #[test]
    fn startup_preparation_defers_the_session_and_reuses_the_same_adapter_process() {
        let temp = tempfile::tempdir().unwrap();
        runtime().block_on(async {
            let options = options(temp.path());
            let mut host = AgentHost::prepare(options.clone()).unwrap();
            let pid = prepared(&mut host).await;
            assert!(!temp.path().join("received-account-methods.jsonl").exists());
            assert!(host.continue_prepared(options));
            tokio::time::timeout(Duration::from_secs(5), async {
                while let Some(event) = host.next_event().await {
                    if let AgentEvent::Message(value) = event {
                        assert_ne!(value["type"], "connection_error", "{value}");
                        if value["type"] == "ready" {
                            break;
                        }
                    }
                }
            })
            .await
            .unwrap();
            let methods =
                std::fs::read_to_string(temp.path().join("received-account-methods.jsonl"))
                    .unwrap();
            assert_eq!(methods.lines().count(), 2, "{methods}");
            assert!(methods.contains("initialize") && methods.contains("session/new"));
            #[cfg(unix)]
            assert_eq!(
                std::fs::read_to_string(temp.path().join("fixture-agent-guardian.pid"))
                    .unwrap()
                    .parse::<u64>()
                    .unwrap(),
                pid
            );
            #[cfg(not(unix))]
            let _ = pid;
            host.close().await.unwrap();
        });
    }
    #[test]
    fn cancelled_or_different_project_preparation_never_creates_a_session() {
        let temp = tempfile::tempdir().unwrap();
        runtime().block_on(async {
            let options = options(temp.path());
            let mut host = AgentHost::prepare(options.clone()).unwrap();
            prepared(&mut host).await;
            let mut changed = options.clone();
            changed.subscription = !changed.subscription;
            assert!(!host.continue_prepared(changed));
            let other = tempfile::tempdir().unwrap();
            let mut changed = options;
            changed.cwd = crate::paths::canonical(other.path()).unwrap();
            assert!(!host.continue_prepared(changed));
            host.close().await.unwrap();
            assert!(!temp.path().join("received-account-methods.jsonl").exists());
        });
    }
}
