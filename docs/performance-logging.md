# Conversation and tool timing

New CLI/TUI conversation transcripts (`turn-TURN.jsonl` under the document's
conversation directory) record `recorded_at_unix_ms` and `elapsed_us` on every
event. The first is wall time in milliseconds since the Unix epoch; the second
uses a monotonic clock from transcript creation. Historical transcripts remain
readable and do not acquire inferred timestamps.

Match ACP `tool_call` and terminal `tool_call_update` events by `toolCallId` to
measure the host-observed call interval. Match `permission_request` and
`permission_response` by tool-call ID to identify approval waits. These are event
receipt times, not provider-side timestamps. Gaps outside tool and permission
intervals include model generation, transport and adapter overhead; they are not
a measurement of model inference alone. Parallel call intervals overlap and
must not be added together as wall time.

Executed MCP calls also return `_meta["lazypencil/timing"]` and, for an owned
conversation, append a `tool_timing` record to the existing bounded per-turn
`design-usage-CONNECTION/TURN.jsonl` journal. The host copies journal records into
the transcript at turn completion, including failed/cancelled turns. Use the
timing object's own timestamps, not the later transcript-copy time. The journal
remains available if the host cannot finish its transcript.

Each timing object contains a unique `call_id`, `received_at_unix_ms`,
`finished_at_unix_ms`, `is_error`, and these monotonic durations in microseconds:

| Field | Interval |
| --- | --- |
| `queue_us` | MCP handler entry through document-worker start, including admission and queue waits |
| `execution_us` | Document-worker execution through result preparation |
| `server_us` | Sum of queue and execution time |

Server durations exclude the performance-journal write, response transport and
model processing. Calls cancelled while queued record zero execution time when
the worker drains them; cancellation during execution records the actual worker
completion even if the client has stopped waiting. A process killed before
completion can leave only the host's start event. Unregistered tools and requests
rejected before queue admission do not produce worker timing records. A worker
that finishes after its turn journal has been frozen cannot append to that turn
or a later one. A missing completion is not evidence of zero execution time.

Timing metadata contains no arguments, document content, image payloads, error
messages, authentication identity or credentials. Existing transcript redaction,
journal size limits, ownership checks and conversation deletion/retention apply.
A timing-write failure does not change the tool result; its response metadata
includes `lazypencil/timing_log_error` with an error code. Standalone MCP clients
receive timing metadata but do not create a conversation journal.
