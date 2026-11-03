# ACP agents and prompt commands

The initial agent host uses the official Rust `agent-client-protocol` SDK 3.5.0 and schema 1.12.0. It negotiates **ACP protocol v1**. The SDK package version is separate from the protocol version. Draft v2 and provider-specific extensions that grant additional capabilities are not enabled automatically.

## Install and authenticate an adapter

Adapters and their runtimes are separate dependencies. lazypencil does not download them while sending a prompt, parse a shell command string, or modify global MCP configuration.

| Agent | Executable | Adapter | Underlying runtime |
| --- | --- | --- | --- |
| Codex | `codex-acp` | `@agentclientprotocol/codex-acp` | Codex App Server |
| Claude | `claude-agent-acp` | `@agentclientprotocol/claude-agent-acp` | Claude Agent SDK |

Use the adapter's installation and authentication instructions: [Codex ACP](https://github.com/agentclientprotocol/codex-acp), [Claude ACP](https://github.com/agentclientprotocol/claude-agent-acp). Authenticate with the selected runtime before starting a design session. lazypencil does not read or store ChatGPT/Claude login tokens. An optional `--auth-method ID` calls an agent-managed ACP authentication method only if the adapter advertises that ID. Terminal-based authentication methods are not implemented by the host; use the runtime's own login command.

## Send a prompt

Project `[design]` settings can supply automatic guides, reference-only guides, styles, and a default style. The shared CLI/TUI conversation host reads the same catalogue as the bound MCP server. A local user must first review `config show FILE` and explicitly grant `config trust FILE`; unapproved or unavailable instructions stop a new turn before an adapter is started. The default style applies when `--style` is omitted, and an explicit `--style` takes precedence. Existing design conventions remain the starting point for edits. See [project design configuration](configuration.md#project-guides-and-styles).

Startup sends the workflow, automatic project guide content, style tokens/instructions, example identities and reusable component IDs. It fetches a complete example through MCP when needed instead of putting every example document into the initial prompt. Turn-start records keep initial guide/style IDs, versions and hashes, including the project instruction fingerprint. Successful guide/style queries made through the owned MCP server are also recorded for that turn, as described below. The user's request takes precedence over project rules, selected styles, and standard guidance.

User font preferences from `config.toml` merge with per-field project overrides. The host passes an absolute `LAZYPENCIL_CONFIG_DIR` in its MCP server descriptor before the adapter changes to the document directory, so a relative launch-time setting selects the same file in nested projects. This requires no global environment or agent configuration changes. A deterministic ACP test queries the actual MCP child and verifies its selected user configuration and font paths. Persistent adapter and runtime preferences are described in [configuration](configuration.md).

```sh
lazypencil prompt design.pen --agent codex "Improve the spacing and hierarchy"
lazypencil prompt new.pen --create --agent claude "日本語の設定画面を作って"
lazypencil prompt design.pen --image reference.png --style standard-ui "Use this reference"
lazypencil --json prompt design.pen --non-interactive "Inspect the current design"
```

`--create` refuses to overwrite any existing file. The working directory is the document's directory. The agent receives a dedicated `lazypencil mcp` configuration with private document-scoped grants and the absolute document path. It calls `open_document` first and passes its returned `document_id` to subsequent document tools. It also receives the original English workflow guide, the requested style, and the user's text. Instructions ask the agent to inspect the document, use current revisions, check layout, render the result, and preserve the user's language. The host does not implement its own LLM loop and does not claim that an instruction guarantees visual verification.

For an explicitly configured executable, use `--adapter /absolute/path/to/program` and repeat `--adapter-arg=VALUE`. Arguments are passed as a vector, without shell expansion. For example, a project-local Node adapter can be started with an absolute Node executable and a script path argument. Persist per-agent defaults in user `config.toml` under `[agents.codex]` / `[agents.claude]`, and select the default agent with `[agents] default = "codex"`. Project `lazypencil.toml` can override these fields after a separate local `config trust FILE --executables` grant. Explicit CLI values override both scopes; an explicit adapter replaces the command and its entire argument list. Relative executable paths resolve against the physical file supplying that adapter. Each turn rechecks the captured project table and execution grant, and turn-start records include the selected project root, scope and fingerprint. A design-instruction grant does not authorize execution, and MCP cannot grant permission. The grant covers settings rather than executable or script bytes. See [project agent profiles](configuration.md#project-agent-profiles) for review, revocation, reload and validation status.

`--auth subscription` is the default. The child does not inherit `OPENAI_API_KEY`, `CODEX_API_KEY`, `ANTHROPIC_API_KEY`, or `ANTHROPIC_AUTH_TOKEN`. It also removes `DEFAULT_AUTH_REQUEST`, OpenAI/Anthropic base-URL overrides, and Claude cloud-provider environment switches, so those overrides do not silently select API access. `--auth api` preserves the environment for explicitly configured API access. Neither option reads credentials or signs the user out to enforce a login method. Native settings, key helpers, and managed policy remain owned by the runtime.

The host consumes the adapters' read-only `_auth/status_update` reports. It normalizes `account`, `api_key`, `gateway`, `external`, and `none` into subscription, API-key, gateway, external-provider, and unauthenticated states. Logs retain only the route and optional plan. The TUI additionally displays a bounded account email/name/ID in memory, without serializing it into events, transcripts or saved state. Tokens, arbitrary labels and vendor metadata are discarded. After a document is selected, TUI startup prepares the actual provider session without sending a prompt, reports its resolved model/reasoning effort and account, and retains the connection for explicit submission. Drafts remain unsent, startup permission requests are rejected, and shutdown cancels session setup. The same adapter, working directory and inherited runtime environment are used for startup and prompting. The evidence is explicitly `adapter_report`; it is not an independently audited invoice or a promise of model access. OpenAI distinguishes subscription and API authentication and provides account introspection through its runtime. See [official authentication documentation](https://learn.chatgpt.com/docs/auth) and [App Server authentication](https://learn.chatgpt.com/docs/app-server#auth-endpoints).

Before sending each prompt, the reported route must match the request: subscription rejects API keys, gateways, and external providers; API rejects subscription authentication. An unauthenticated state is rejected. A conflicting report during a turn triggers ACP cancellation and preserves committed edits. There is no automatic retry or login through another route. The host does not change stored credentials to force a match. For API mode, configure/authenticate the runtime for the intended API route, or explicitly select an advertised agent-managed `--auth-method`.

When an adapter advertises its `authStatus` capability, the host waits up to five seconds for a usable report and blocks unverified prompts. `--require-auth-status` also enforces this for other adapters, including development wrappers. Older adapters without this capability may operate with unknown authentication unless that flag is set; startup events label the source unknown. A previous conversation's report is cleared at reconnect and is not reused as evidence for the new process.

`--model MODEL` selects a value from the adapter's advertised model configuration, including grouped selectors, and requires the adapter to confirm the new value. `--mode MODE` likewise validates the advertised permission mode and applies it through ACP. Unsupported values fail before prompting. With the standard executables, the initial native mode is Codex `read-only` or Claude `default`; an explicit CLI or user-profile adapter override has no implicit mode, so specify one when needed. Applied modes and configuration options are recorded in the ready event. Model catalogs describe options; actual access is established by a successful inference turn.

`--fast` enables Codex Fast mode and `--fast=false` disables it for the connection. Omission leaves the adapter's effective speed setting untouched; actual inheritance of Codex defaults and saved native settings depends on the adapter. `[agents.codex] fast` supplies the same preference in user or trusted project settings; explicit CLI values take precedence. The host applies the advertised `fast-mode` selector after changing the model and requires confirmation before prompting. Missing, ambiguous or unsupported selectors fail explicitly. Codex ACP currently has an upstream [Fast/priority inheritance bug](https://github.com/agentclientprotocol/codex-acp/issues/285); use an explicit `fast = true` or `--fast` until it is fixed. The host does not advertise ACP boolean config controls, so Codex ACP exposes its `on`/`off` select form. These controls do not edit Codex's global configuration. Fast mode availability and increased usage follow [OpenAI's speed settings](https://learn.chatgpt.com/docs/agent-configuration/speed).

## Images and assets

Repeat `--image` for static PNG, JPEG, or WebP files. The default limits are eight images, 20 MiB per source and normalized image, 50 MiB total before and after normalization, and 16 megapixels per image. Files are decoded one at a time, orientation is applied, and immutable PNG copies are retained with the conversation. No image is silently resized or omitted; explicit `--image-max-size` can select a send size. User [attachment budgets](configuration.md#image-attachment-budgets) can change the count and aggregate limits.

Images are sent as ACP image blocks containing MIME-typed Base64 data. The host checks the negotiated image capability; an unsupported agent receives no text-only substitute. Each image also has an ordered display name and attachment ID. The prompt does not insert reference images into the document automatically.

The session's MCP connection receives a private manifest bound to that document. `get_document` exposes registered image metadata. `put_asset` can import one with:

```json
{"kind":"image","source":{"attachment_id":"img_<sha256>"}}
```

The normal atomic edit operations then reference the returned asset URI. Unknown IDs, grants for another document, modified stored files, and image attachments requested as code are rejected. The connection's grant file is removed on normal cleanup; stored conversation images and already imported project assets remain. On Unix, a private close-on-exec owner file is held exclusively by the conversation host. Each owned MCP request checks the matching connection identity and a nonblocking shared-lock probe; losing the host expires the grant even when its JSON file survives a hard crash. Cached MCP access cannot substitute another connection's identity. When an owned connection expires, every tool call, including document reads and edits, fails with `AGENT_ERROR` before dispatch. A save already in progress keeps the existing atomic-save rules; revocation does not undo committed edits. Standalone MCP without conversation grants retains its ordinary behavior. Old transient Unix grants without an owner require reconnecting; saved conversation images and session metadata are unchanged. Each conversation currently retains at most 256 distinct images. Local browsing, selection and reviewed deletion are described below; user-owned retention and global cleanup are documented in [configuration](configuration.md#local-conversation-retention). Clipboard capture follows the [image-input reference](image-attachments.md).

## Conversations and events

Every invocation prints a local conversation UUID. Resume it with the same document, adapter arguments, and requested authentication mode:

```sh
lazypencil prompt design.pen --agent codex --session <local-uuid> "Make the main action clearer"
```

The native session ID is saved before sending the prompt. Reload occurs only when the adapter advertises `loadSession`; an ordinary transcript is not treated as resumable agent state. If that capability is absent, CLI and TUI create a new native session within the same local conversation and report `session_restarted`. The ready event identifies its `session_origin` as `new`, `loaded`, or `restarted`. A conversation lock prevents concurrent local reuse. There is no automatic replay of a failed prompt. The shared host supports multiple sequential turns on one live connection; the CLI reloads the native session between process invocations.

A replacement native session receives the current document path/revision and descriptive excerpts from at most four recent local turn transcripts. Each request excerpt has at most 1,024 Unicode characters. History scanning stops after 4,096 directory entries and reads at most the first and last 64 KiB of each selected transcript. Only matching conversation/document/turn records count; corrupt or unavailable history is disclosed, and incomplete turns are not labeled successful. This is a bounded context summary, not restored model memory. The prompt instructs the agent to inspect the current document, treat past requests as context, and execute only the current request. Previous images remain registered but are not automatically reattached. Existing transcripts and committed edits are preserved.

An advertised `session/load` failure is reported without silently opening a new session or sending the pending prompt. After checking the error, explicitly use `lazypencil prompt design.pen --session LOCAL_UUID --restart-session "Continue from the current document"`, or the same options on `tui`, to replace the native session with the same bounded context. This option requires a local conversation ID and applies only to the next connection; it does not relax model, authentication, permission, or document-binding checks. Library `AgentHost` callers retain strict loading by default and must opt into fallback and provide frontend context themselves.

A replacement session saves `continuity_pending` before its first prompt can be sent. Cancellation, connection failure, and a failed local turn preserve this flag. The next explicit request adds current document context and the same bounded descriptive history even when native loading succeeds, and reports `local_context` rather than claiming another native restart. A successfully completed local turn clears the flag; a final transcript-write failure restores it with the draft and pending attachments. Local metadata version 3 records this state. Versions 1 and 2 upgrade on opening without discarding drafts, native IDs, or stored images; unknown newer versions are rejected before saving.

Metadata, normalized images, and one JSONL transcript per turn live below the document's directory in the lazypencil **user data store**, not alongside the `.pen` source. Unix directories/files are created with private permissions. Logs include guide/style versions and hashes, user text, native session metadata, updates, permission decisions, and the actual final document revision. Image payloads in events are replaced with length/hash metadata in transcripts and CLI NDJSON. Adapter stderr is continuously drained without retaining its contents; connection errors report the drained byte count where available. ACP request and transport errors expose a host-defined phase and the numeric protocol error code when available. Adapter error messages and arbitrary error data are omitted from terminal output, NDJSON and transcripts. Host cancellation, authentication-route checks and setup deadlines retain their own errors across the SDK boundary; remote error data cannot select a host error code.

`--json prompt` is NDJSON rather than a single JSON object. Event types currently include `start`, `initialized`, `session_restarted`, `local_context`, `ready`, `auth_status`, `update`, `agent_notification`, `permission_request`, `permission_response`, `design_resource`, `tool_timing`, `turn_complete`, `turn_error`, `connection_error`, `complete`, `failed`, and a terminal CLI `error` on failure. Unknown update fields and extension notifications are preserved; the recognized authentication notification is normalized as described above. Text terminal output removes control characters; NDJSON uses ordinary JSON escaping. Per-turn transcripts have a 64 MiB limit, and incoming ACP lines have a 16 MiB limit. A final transcript-write failure reports `failed` and restores the draft and pending attachments; it does not undo committed document edits. If restoring local metadata also fails, the failure is reported explicitly and the live host retains the draft in memory.

New transcript events include wall-clock and monotonic timestamps. Owned MCP calls also persist queue and execution durations, including errors and drained cancellations, without adding tool arguments or result bodies to the timing records. See [conversation and tool timing](performance-logging.md) for fields, correlation, storage and measurement limits.

### Read saved conversations without an agent

```sh
lazypencil conversations list design.pen --json
lazypencil conversations turns design.pen LOCAL_UUID --json
lazypencil conversations read design.pen LOCAL_UUID TURN_UUID --json
lazypencil conversations view design.pen LOCAL_UUID TURN_UUID
```

These read-only commands use the document's local data directory and work while a frontend owns the conversation or after its adapter has been removed. They do not authenticate, launch an adapter, resume a session, clear a draft, or resend a prompt. The document must still exist at its registered path. This is browsing local records, separate from restoring a model's internal context.

Lists accept `--limit 1..200` and `--cursor`, returning `next_cursor` and `previous_cursor`. Entries use stable ascending UUID order rather than an inferred chronology. Listings are live (`snapshot: false`): concurrent new entries before the current cursor appear after refreshing. Cursors are bound to the document and listing; a conversation cursor cannot select turns or another conversation. Invalid recognized entries remain visible as unavailable instead of being treated as successful history.

`read` returns up to 64 KiB of the original JSONL, including an unfinished final record. Follow `next_offset` with `--offset` and the returned `--expected-revision`; continuation requires that revision. Every page checks the entire transcript before and after reading, so even a same-size edit with a preserved modification time invalidates continuation. Normal pages end at UTF-8 boundaries. Damaged UTF-8 returns display text with replacement characters plus exact `bytes_base64`, with `utf8_valid: false`; no saved suffix is silently discarded. This is a bounded raw transcript interface, not a formatted TUI history browser.

`view` decodes complete user requests and streamed responses, thinking text, tool status and completion/failure markers. It prints a bounded text page; `--json` returns the page with its `next_cursor`, supplied to `--cursor` for continuation. Display cursors bind the document, session, turn, whole transcript revision and formatter version. Text values, including long strings and JSON Unicode escapes, are decoded incrementally; complete records are not deserialized or accumulated. Unknown, ambiguous, unfinished or out-of-profile JSON records remain visible as sanitized original text. Tool details and exact original bytes remain available through `read` or the TUI's raw mode. A content change rejects continuation and requires an explicit refresh.

Reads retain the existing 64 MiB per-turn bound, accept cancellation and have a 30 second cooperative deadline. Listings keep one page and one lookahead in memory; transcript hashing uses a 64 KiB buffer. Conversation metadata and first-record identity are checked against the selected document/session/turn. Recognized symlink directories, symlink file leaves, non-regular files and foreign headers are rejected. This does not make local logs tamper-proof against their filesystem owner. The [TUI's saved-log browser](tui.md#saved-conversation-logs) uses these shared APIs. Explicit TUI selection is described below; [reviewed deletion](#delete-local-conversation-data) and [user-owned aggregate retention](configuration.md#local-conversation-retention) are available.

### Select local conversation state

CLI requests continue to choose a local UUID with `prompt --session UUID` or `tui --session UUID`. Library frontends can call `Conversation::select(Some(id))` while idle, or `select(None)` to create an empty local conversation. The TUI exposes these through the saved browser’s `u` and `N` keys and the **New empty conversation** palette action. Selecting the current ID is a no-op.

Selection keeps each conversation’s own draft, pending/retained images, transcripts, and native session ID. It validates and leases the target before closing the previous adapter and revoking its connection grants, then resets live permissions, events and displayed authentication. The shared selection API does not itself send either draft. The TUI then prepares the selected session in the background, negotiating native loading or the bounded continuity fallback; inference still requires explicit submission. A target must match the document, configured agent, adapter fingerprint and requested authentication route; incompatible or busy targets fail without replacing the current local state. Choose the corresponding CLI agent/adapter settings to open an incompatible session.

Agent requests, image input and local document operations must finish or be cancelled before TUI selection. The short registry lease serializes session opening with local lifecycle operations; the active conversation retains its own exclusive lease. Symlink directories, metadata/lock symlinks and non-regular leaves are rejected.

### Delete local conversation data

```sh
# Review an inactive conversation without changing or sending its draft.
lazypencil conversations delete design.pen LOCAL_UUID --json
# Copy revision from that review, then explicitly commit the deletion.
lazypencil conversations delete design.pen LOCAL_UUID --expected-revision REVIEWED_SHA256 --json
# Finish committed deletions interrupted during filesystem cleanup.
lazypencil conversations cleanup design.pen --json
```

The review identifies the full local UUID, document, byte/file totals, draft characters and image counts. The shared API holds an exclusive conversation lease while hashing every local file with a 64 KiB buffer. It has a 30 second cooperative deadline and a 100,000 entry/64-level limit; a missing, busy, foreign, unsupported-format or changed conversation is rejected. Revision checking covers contents, names and filesystem identities, including same-length content changes whose modification time was preserved. Metadata/lock symlinks and non-regular files are rejected; ordinary symlink leaves inside the payload are recorded without following their targets.

Commit atomically moves the reviewed directory out of the live conversation namespace under the stable registry lease, then removes its local drafts, attachments, transcripts and usage journals. The `.pen`, document checkpoints, imported project assets, provider-side conversation history and authentication stores remain. An inactive unsent draft may be deleted only through this explicit reviewed operation. Every live CLI/TUI keeps its conversation leased, including while idle; select another or close it first. No adapter or authentication is needed.

After the rename, cancellation or cleanup failure cannot restore the conversation. Results retain `deleted: true` and report `cleanup_complete`, any cleanup error and directory-sync errors. A private journal outside the payload allows `cleanup` to finish after a crash or partial recursive removal, even if `session.json` has already disappeared. Cleanup leaves busy, foreign, malformed or symlink tombstones available for diagnosis. The TUI's `d` review and `y` confirmation call the same API.

Cleanup waiting observes the shared deadline and cancellation while a bounded internal worker retains standard non-following filesystem removal. Cleanup may finish while the frontend remains alive; process exit leaves the journal recoverable. `conversations cleanup --all` works after original documents have moved or disappeared. Cleanup results include total counts, at most 200 detailed removed/retained records and explicit truncation. Cancellation retains completed results. The TUI shows pending cleanup and directory-sync warnings, and a late interrupt cannot turn a committed deletion into a cancelled review.

User `[conversations.retention]` settings control a reviewed aggregate `conversations prune` across all local document stores. Defaults keep everything; active sessions, drafts, pending images and incomplete continuity are protected even when they prevent satisfying the byte limit. See [retention policy and commands](configuration.md#local-conversation-retention).

### Design-resource records

Automatic project guides use the identity of the embedded guide resource. Fetching the same project guide separately adds catalogue metadata and produces a different contextual identity. Compare the corresponding response form; `source_hash` identifies the raw guide-file bytes.

The shared CLI/TUI host gives its owned MCP process a document- and connection-bound usage grant. Each prompt publishes a new turn UUID, also present in `start`, `complete`, and `failed`. For successful `get_guidance`, `get_style`, and `list_styles` calls, the server syncs a bounded metadata record before returning the result. A guide record includes its ID, version, language and resource hash; the workflow also records each automatic project guide included with it. Styles record their full resource hash, including their example. A style catalogue record describes the returned page's IDs/versions and a SHA-256 of the complete structured response (`hash_scope: "response"`); a catalogue page does not count as reading each style's full instructions. Project revisions and available instruction/catalogue fingerprints accompany the records.

At turn completion, cancellation, or failure, the host freezes the journal, validates it, and appends `design_resource` events to that turn's transcript and NDJSON before the final host result. These events therefore arrive when the turn ends, rather than being a live progress feed. Failed queries are not recorded as retrieved resources. A request captured for an older turn cannot append to a newer turn. Normal connection close removes its grant. A journal write or capacity failure makes the resource tool fail; an unreadable or corrupt journal makes the turn fail and retains the draft. Each record is limited to 64 KiB and each turn's journal to 4 MiB. Resource bodies, tokens, examples, source paths and image payloads are omitted from these records.

The journals remain under `design-usage-CONNECTION/TURN.jsonl` in the conversation's user-data directory, alongside their small lock file, so an interrupted transcript has bounded per-turn evidence available for diagnosis. This is a local record of results prepared by the owned server: it does not prove network delivery, the model's internal reasoning, or that the model applied the instructions. It is not a tamper-proof audit against a process with the user's filesystem access. Unix usage grants have the same live-owner requirement as image grants; their private manifest version is 2, while the retained Windows implementation keeps version 1. An already connected MCP process cannot append after its owner expires. Resuming the same local conversation reclaims recognized expired image/usage manifests and owner files under its exclusive conversation lock, including owner files left before a manifest was published. Cleanup holds only a shared probe, so it cannot make an expired grant appear live. Unknown or malformed records and symlinks are retained; image files, transcripts, usage journals and project assets are preserved. Standalone MCP clients have no host-owned turn journal. Unreferenced project assets use the separate explicit `assets prune` review.

Journal operations wait up to five seconds for a busy peer, checking cancellation while waiting. The timeout retains `LOCK_UNAVAILABLE` and reports the elapsed/allowed wait in error details. Unrelated I/O errors are not retried as contention.

## Permissions and cancellation

`--document-tools allow` is the default for an explicitly requested document prompt. It selects **allow once** only for registered tools on this conversation's bound lazypencil MCP server. Claude's programmatic tool name is checked against the registry; Codex's name-free approvals are correlated by session and tool-call ID with explicit MCP/server/tool metadata from its earlier tool-call event. Titles are never authorization evidence. Completed calls expire from this correlation, other sessions/servers and unknown tool names are rejected, and no persistent allow option is selected.

Use `--document-tools ask` to disable this preauthorization. Remaining permission requests display the adapter's options when stdin and stderr are terminals. `--non-interactive`, or a non-terminal invocation, rejects unresolved permissions. An agent's later `end_turn` or cancellation after rejection does not hide a non-interactive denial: the CLI exits with an agent error. The host advertises no filesystem or terminal execution callbacks. Other unsupported client requests receive the SDK's method-not-found response.

Authorization for document edits does not grant arbitrary shell or filesystem access. The document-bound MCP server is not a sandbox for the external agent. The native modes above are requested and acknowledged through ACP; they do not prove OS-level isolation for every runtime, hook, plugin, or explicitly supplied adapter. Broader runtime diagnostics and cross-platform restriction tests remain required. A blanket approval mode is not used to make tests pass.

Ctrl+C sends `session/cancel` for an active turn and denies pending permissions.
An unresponsive adapter is terminated after five seconds. `--timeout` defaults to
600 seconds per turn; setup has its own 60-second deadline. Closing reaps the owned
adapter. Unix guardians watch a separate close-on-exec ownership channel; Windows
workers join a host-owned Job before spawning the adapter. Owner death revokes
host-bound grants and stops ordinary owned descendants. Deliberately escaping
processes and universal OS-crash cleanup are outside that guarantee.

The TUI can send text into a running turn through `_session/steering`, while
`/queue <text>` keeps the normal sequential prompt lifecycle. Steering is enabled
only for an advertised capability and recognized versions with a host-owned idle
fallback (codex-acp 2.3.0+, claude-agent-acp 0.100.0+). The host sends
`_meta.steering.idleBehavior = "promptRequired"` so a completion race cannot silently
start another turn. A `promptRequired` receipt leaves the input queued; an `injected`
receipt records it as user input in the active transcript. Unconfirmed delivery stays
in a paused queue for review, including after reopening. Steering neither resets the
turn deadline nor changes document-tool permissions, authentication or image grants.

Committed edit batches remain in the document after cancellation or a crash. The CLI rereads the actual document after stopping, records its revision, and leaves committed history available through `history` and `restore`. It does not automatically roll back or retry. Invalid final document data produces a failure rather than a successful completion event.

## Library embedding and development

Use `Conversation::open_with_executable(args, data_dir, lazypencil_binary)` to bind
MCP and process supervision to an absolute, existing lazypencil executable. Library
`AgentHost` callers also set `AgentOptions.supervisor_executable`. The CLI uses its
own executable. A saved transcript is not an independently resumable provider session.

Run `cargo xtask test --test agent` for deterministic process/protocol checks with
isolated settings and the native helper. Direct invocations can set
`LAZYPENCIL_TEST_HELPER` to an already built helper. Installed-adapter and live-workflow
tests are opt-in; the latter send real prompts and consume account usage. See their
filters/options in `tests/agent.rs` and `tests/agent_workflow.rs` and the
[development guide](development.md).
