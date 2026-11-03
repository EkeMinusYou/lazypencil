//! Mid-turn ACP input with one owner for prompt completion and cancellation.
use super::*;
use futures::{Future, stream::FuturesUnordered};

pub(super) struct Request {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub generation: u64,
}

pub(super) fn supported(response: &InitializeResponse) -> bool {
    if !response.meta.as_ref().is_some_and(|meta| {
        meta.get("steering")
            .is_some_and(|value| value["supported"] == true)
    }) {
        return false;
    }
    let Some(info) = &response.agent_info else {
        return false;
    };
    // Older adapters advertise steering but can start an unowned turn when
    // completion races the request. These versions honor promptRequired.
    let minimum = match info.name.as_str() {
        "codex-acp" | "@agentclientprotocol/codex-acp" => [2, 3, 0],
        "claude-agent-acp" | "@agentclientprotocol/claude-agent-acp" => [0, 100, 0],
        _ => return false,
    };
    let Ok(version) = info
        .version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
    else {
        return false;
    };
    version.len() == 3 && version.as_slice() >= minimum.as_slice()
}

pub(super) struct Channels<'a> {
    pub events: &'a mpsc::Sender<AgentEvent>,
    pub shutdown: &'a CancellationToken,
    pub authentication: &'a mut watch::Receiver<Option<AuthReport>>,
    pub policy: AuthPolicy,
    pub inputs: &'a mut mpsc::Receiver<Request>,
}

async fn send(
    connection: &acp::ConnectionTo<acp::Agent>,
    session: &SessionId,
    input: Request,
) -> (String, Result<Value>) {
    let params = json!({"sessionId":session,"prompt":input.content,
        "_meta":{"steering":{"idleBehavior":"promptRequired"}}});
    let response = connection
        .send_request(ClientRequest::ExtMethodRequest(ExtRequest::new(
            "_session/steering",
            serde_json::value::to_raw_value(&params)
                .expect("JSON value")
                .into(),
        )))
        .block_task()
        .await;
    let result = response.map_err(|error| protocol_error("_session/steering", error));
    (input.id, result)
}

pub(super) async fn run_prompt<F>(
    connection: &acp::ConnectionTo<acp::Agent>,
    session: &SessionId,
    options: &AgentOptions,
    turn: &Turn,
    request: &mut F,
    channels: Channels<'_>,
) -> Result<std::result::Result<std::result::Result<PromptResponse, acp::Error>, TurnStop>>
where
    F: Future<Output = std::result::Result<PromptResponse, acp::Error>> + Unpin,
{
    let Channels {
        events,
        shutdown,
        authentication,
        policy,
        inputs,
    } = channels;
    let deadline = tokio::time::sleep(options.turn_timeout);
    tokio::pin!(deadline);
    let mut pending = FuturesUnordered::new();
    let mut pending_id = None;
    let stopped = loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break TurnStop::Cancelled,
            _ = turn.cancel.cancelled() => break TurnStop::Cancelled,
            _ = &mut deadline => break TurnStop::TimedOut,
            error = auth::wait_for_conflict(authentication, policy) => break TurnStop::Authentication(error),
            result = pending.next(), if !pending.is_empty() => {
                let (id, result): (String, Result<Value>) = result.expect("pending steering");
                pending_id = None;
                let value = match result {
                    Ok(response) if matches!(response["outcome"].as_str(), Some("injected" | "promptRequired")) => {
                        json!({"type":"steering_result","id":id,"outcome":response["outcome"]})
                    }
                    Ok(response) if response["outcome"] == "startedNewTurn" => {
                        return Err(agent_error("The adapter started an unowned steering turn despite promptRequired; closing the connection"));
                    }
                    Ok(_) => json!({"type":"steering_result","id":id,"outcome":"failed","error":agent_error("The adapter did not confirm delivery of the mid-turn input")}),
                    Err(error) => json!({"type":"steering_result","id":id,"outcome":"failed","error":error}),
                };
                emit(events, value).await?;
            },
            // Keep the prompt response unconsumed until steering is settled so
            // cancellation can still await it and history keeps its turn owner.
            response = &mut *request, if pending.is_empty() => return Ok(Ok(response)),
            input = inputs.recv(), if pending.is_empty() => {
                if let Some(input) = input {
                    if input.generation != turn.generation {
                        emit(events, json!({"type":"steering_result","id":input.id,"outcome":"promptRequired"})).await?;
                    } else {
                        pending_id = Some(input.id.clone());
                        pending.push(send(connection, session, input));
                    }
                }
            },
        }
    };
    if let Some(id) = pending_id {
        // Cancellation cannot prove whether a published request was consumed.
        emit(
            events,
            json!({"type":"steering_result","id":id,"outcome":"unknown"}),
        )
        .await?;
    }
    Ok(Err(stopped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_requires_advertisement_and_a_host_owned_idle_fallback() {
        for (name, version, advertised, expected) in [
            ("codex-acp", "2.2.2", true, false),
            ("codex-acp", "2.3.0", true, true),
            ("@agentclientprotocol/codex-acp", "2.5.0", true, true),
            (
                "@agentclientprotocol/claude-agent-acp",
                "0.100.0",
                true,
                true,
            ),
            ("claude-agent-acp", "0.99.0", true, false),
            ("codex-acp", "2.5.0", false, false),
            ("codex-acp", "2.5.0-beta", true, false),
            ("unknown", "99.0.0", true, false),
        ] {
            let response = serde_json::from_value(json!({"protocolVersion":1,
                "agentInfo":{"name":name,"version":version},
                "_meta":{"steering":{"supported":advertised}}}))
            .unwrap();
            assert_eq!(supported(&response), expected, "{name} {version}");
        }
    }
}
