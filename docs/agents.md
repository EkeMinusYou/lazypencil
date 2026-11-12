# AI agents and prompt commands

Use Codex or Claude to create and edit designs from the terminal workspace or the
`prompt` command. An ACP adapter connects lazypencil to your chosen AI runtime.
If you prefer an existing MCP client, follow the [MCP setup](mcp.md) instead.

## Install and authenticate an adapter

Install the adapter for the agent you use:

```sh
# Codex
npm install -g @agentclientprotocol/codex-acp@latest

# Claude
npm install -g @agentclientprotocol/claude-agent-acp@latest
```

| Agent | Executable | Runtime |
| --- | --- | --- |
| Codex | `codex-acp` | Codex App Server |
| Claude | `claude-agent-acp` | Claude Agent SDK |

Sign in using the adapter's instructions: [Codex ACP](https://github.com/agentclientprotocol/codex-acp)
or [Claude ACP](https://github.com/agentclientprotocol/claude-agent-acp).
Authentication stays with the selected runtime. For an adapter that advertises an
ACP authentication method, `--auth-method ID` selects that method; otherwise use
the runtime's login command.

## Send a prompt

```sh
lazypencil prompt design.pen --agent codex "Improve the spacing and hierarchy"
lazypencil prompt new.pen --create --agent claude "日本語の設定画面を作って"
lazypencil prompt design.pen --image reference.png --style standard-ui "Use this reference"
```

`--create` creates a document and refuses to overwrite an existing file. The agent
works from the document's directory and receives its path, the design tools, and
guidance for inspecting, editing, validating and rendering the result.

| Option | Purpose |
| --- | --- |
| `--agent codex` / `claude` | Choose the AI runtime |
| `--model MODEL` | Select a model advertised by the adapter |
| `--mode MODE` | Select an advertised permission mode |
| `--style ID` | Apply a design style; overrides the project's default style |
| `--image PATH` | Attach a reference image; repeat for several images |
| `--auth subscription` | Use the runtime's subscription login; the default |
| `--auth api` | Use explicitly configured API authentication |
| `--session UUID` | Continue a saved local conversation |
| `--timeout SECONDS` | Set a turn's time limit; the default is 600 seconds |

Save recurring choices under `[agents.codex]` or `[agents.claude]` in your
[user configuration](configuration.md#user-agent-profiles). Project defaults can
be shared after [reviewing and trusting the project's executable settings](configuration.md#project-agent-profiles).
Explicit command-line options take precedence.

For a custom adapter, use `--adapter /absolute/path/to/program` and repeat
`--adapter-arg=VALUE` for its arguments. A replacement adapter replaces the entire
configured command and argument list. Relative executable paths in settings
resolve against the configuration file that defines them.

### Authentication and models

Subscription mode removes API-key and provider-override environment variables
from the adapter's environment so they do not accidentally select API billing.
API mode preserves them for the runtime's configured API route. Select and sign
in to the route you intend to use; lazypencil checks the adapter's reported route
before sending a prompt and stops if it conflicts with your selection.

The TUI header shows the connected adapter's model, effort and account information.
Preparing a session leaves the draft unsent until you submit it. Account identity
is displayed for the session and is not saved in conversation logs.

`--require-auth-status` also requires an authentication report from adapters that
do not advertise the standard reporting capability. Use it for custom adapters
when an unknown route should prevent submission.

`--model` and `--mode` accept the connected adapter's advertised values. With the
standard adapters, the initial modes are Codex `read-only` and Claude `default`.
Custom adapters have no implicit permission mode; select one if needed.

For Codex, `--fast` enables Fast mode and `--fast=false` disables it for the current
connection. You can save the choice as `[agents.codex] fast`. Omission leaves the
adapter's inherited setting in effect. If inheritance is unreliable, pass the
choice explicitly; see the upstream [Fast-mode issue](https://github.com/agentclientprotocol/codex-acp/issues/285).

### Project design guidance

Project `[design]` settings can provide guides, styles and a default style. Review
`lazypencil config show design.pen` and run `lazypencil config trust design.pen`
before using project-supplied instructions. Your current request takes precedence
over project guidance. See [project guides and styles](configuration.md#project-guides-and-styles).

Font and rendering preferences follow the same user/project settings in CLI,
MCP and TUI operations. See [configuration](configuration.md).

## Diagnose agent setup

```sh
lazypencil doctor --agent codex
lazypencil doctor --agent claude --timeout-seconds 10
```

Follow the report's suggested fixes for missing executables, Node.js, login or
connection problems. Doctor checks adapter connectivity and reported login state
without sending a model prompt. For report meanings and script-friendly output,
see [setup diagnostics](doctor.md).

## Permissions and cancellation

By default, `--document-tools allow` lets the agent use this conversation's
registered lazypencil tools with one-time approvals. It applies to those document
tools, rather than granting general shell or filesystem access. Use
`--document-tools ask` to review each request yourself.

Other approval requests display the adapter's choices. In a noninteractive
invocation, unresolved approvals are rejected and the command fails. This also
applies when `--non-interactive` is supplied:

```sh
lazypencil --json prompt design.pen --non-interactive "Inspect the current design"
```

Permission modes belong to the AI runtime. Review the connected adapter's mode
and approval choices before allowing broader access.

Ctrl+C cancels an active request and rejects pending approvals. An unresponsive
adapter is stopped after five seconds. Setup has a separate 60-second limit.
Edits already saved remain in the document; use [checkpoints](checkpoints.md) to
review or restore them. Before retrying an interrupted request, inspect the
current design so you can see what completed.

In the TUI, submitting text while the agent works can steer the active turn.
Use `/queue <text>` to send it as the next turn instead. Steering requires
codex-acp 2.3.0+ or claude-agent-acp 0.100.0+ with the advertised capability;
other adapters queue follow-ups. Images always go to a subsequent turn.
If delivery cannot be confirmed, the queue pauses for your review.

### Change permission modes in the TUI

Open **Agent permissions…** from the command palette or use `/permissions`
(`/mode`). The picker lists the connected adapter's modes, descriptions and
current working directory. The header shows the confirmed effective mode.

Choose **This conversation only**, **Save as project default**, or **Save as global
default**. Temporary choices last until the TUI closes. Saved choices follow
command-line > project > user > built-in precedence on future launches.
Saving updates only the selected agent's `mode`.

Project saving shows the destination and resulting agent settings. **Trust & apply**
grants execution trust for those reviewed settings. Full-access and bypass modes
require an additional confirmation. To answer an outstanding tool approval, use
that approval's original choices; changing mode affects later requests.

During an active turn, choose **Apply after turn**. Queued prompts wait for the
adapter to confirm the change. Press `c` in the picker to cancel an unsent change.
If applying or saving fails, the UI reports the effective mode and pauses queued
prompts as needed. Reconnect after a mode-change timeout before sending again.

## Images and assets

Repeat `--image` for static PNG, JPEG or WebP files. Use `--image-max-size WIDTHxHEIGHT`
to choose a smaller send size. Defaults allow eight images, 20 MiB per image,
50 MiB in total for source data and for send data, and 32 megapixels per image.
See [image attachments](image-attachments.md) for clipboard input, resizing and
configurable budgets.

A reference image is sent to the agent as an attachment. Ask the agent to add it
to the design if you also want a document layer. Through MCP, the agent can import
an attachment using `put_asset`:

```json
{"kind":"image","source":{"attachment_id":"img_<sha256>"}}
```

Use the returned asset URI in the document. Imported assets stay with the project;
saved reference images stay with the conversation. Each conversation can retain
up to 256 distinct images.

## Conversations and events

The [TUI](tui.md#opening-a-document) restores the last selected or submitted
conversation for each document. Use `tui FILE --new-conversation` for a fresh one.
The CLI prints a local conversation UUID; reuse it explicitly to continue:

```sh
lazypencil prompt design.pen --agent codex --session LOCAL_UUID "Make the main action clearer"
```

Use the same document, adapter settings and authentication mode. Only one live
frontend can use a local conversation at a time.

When the adapter supports native session loading, lazypencil reconnects to it.
Otherwise, it starts a replacement session with the current document and excerpts
from up to four recent turns. Reattach previous images if the new request needs
them. Reopening a conversation leaves failed or interrupted requests for explicit review.

If native loading fails, check the error before opting into a replacement session:

```sh
lazypencil prompt design.pen --agent codex --session LOCAL_UUID --restart-session "Continue from the current document"
```

The same options are available on `tui`. A replacement keeps the local conversation
and its files, while giving the agent the current document and recent context.
It does not recreate the previous model's full internal state.

### Read saved conversations without an agent

```sh
lazypencil conversations list design.pen --json
lazypencil conversations turns design.pen LOCAL_UUID --json
lazypencil conversations view design.pen LOCAL_UUID TURN_UUID
lazypencil conversations read design.pen LOCAL_UUID TURN_UUID --json
```

Use `view` for readable text and `read` for raw JSONL. The original document must
still exist at its registered path. These commands read local records and do not
need a running adapter.

Lists accept `--limit 1..200` and `--cursor`; results return `next_cursor` and
`previous_cursor`. Refresh the list to see concurrent additions. `view --json`
returns a text page and its next cursor. `read` returns up to 64 KiB; continue with
`--offset` set to `next_offset` and the returned `--expected-revision`.
If the stored content changes, refresh rather than reusing an old cursor or revision.

Use the [TUI log browser](tui.md#saved-conversation-logs) for interactive browsing.

### Select local conversation state

Select another conversation in the TUI's saved browser with `u`, or press `N` to
create an empty one. **New empty conversation** is also in the command palette.
Finish or cancel the active request and pending image/file operations first.

Each conversation keeps its own draft, images, transcript and native session ID.
Selection leaves the draft unsent. If a conversation belongs to another document,
uses incompatible agent settings or is already open, the current conversation stays
selected; correct the settings or close its other frontend before trying again.

### Delete local conversation data

Review an inactive conversation, then use the returned revision to confirm deletion:

```sh
lazypencil conversations delete design.pen LOCAL_UUID --json
lazypencil conversations delete design.pen LOCAL_UUID --expected-revision REVIEWED_SHA256 --json
```

The review shows the document, full local UUID, size, draft and image counts.
Close the conversation or select another one before deleting it. Confirmation
removes its local drafts, reference images, transcripts and usage records. The
`.pen` file, checkpoints, imported project assets and provider-side history remain.

If cleanup is interrupted after deletion is committed, run:

```sh
lazypencil conversations cleanup design.pen --json
lazypencil conversations cleanup --all --json
```

`deleted: true` means the deletion was committed; `cleanup_complete: false` means
file cleanup still needs attention. Repeat cleanup for the remaining work.
`--all` also handles records whose original documents have moved or disappeared.

For deletion across conversations, configure and review
[conversation retention](configuration.md#local-conversation-retention).

### Local logs and diagnostics

Conversation records live in the lazypencil user data directory, under
`documents/DOCUMENT_KEY/conversations/LOCAL_UUID/`. They include your requests,
responses, permission decisions and document revisions. Image payloads are
represented by size/hash metadata in transcripts; the saved images are separate.

Detailed connection errors are saved as `adapter-error-UUID.json`. The displayed
error gives the path. These files can contain private text or credentials supplied
by the adapter; inspect them locally before sharing. Deleting the conversation
also deletes its diagnostics.

A transcript-storage failure fails the turn and preserves its draft. Saved document
edits remain available even if logging or the connection later fails. Back up the
user data directory if you want to preserve conversations as well as `.pen` files.

### Design-resource records

Logs record the guide/style IDs, versions and hashes supplied to a turn. Successful
`get_guidance`, `get_style` and `list_styles` responses produce `design_resource`
events when the turn ends. These identify the resources made available to the agent;
a catalogue listing records the list response, rather than each style's contents.

Records include the turn UUID and relevant document/configuration revisions.
Resource bodies, image payloads and credentials are omitted. Interrupted turns
can also leave local `design-usage-CONNECTION/TURN.jsonl` records for diagnosis.

### Machine-readable events

`--json prompt` writes one JSON event per line (NDJSON). Events include `start`,
`initialized`, `session_restarted`, `local_context`, `ready`, `auth_status`, `update`,
`agent_notification`, `permission_request`, `permission_response`, `design_resource`,
`tool_timing`, `turn_complete`, `turn_error`, `connection_error`, `complete` and
`failed`. A command failure can also produce a terminal `error` event.

`session_origin` in `ready` is `new`, `loaded` or `restarted`. `local_context` means
recent local context was supplied while continuity is being re-established.
Diagnostic paths appear as `adapter_diagnostic_path`, or
`adapter_diagnostic_write_error` when saving the diagnostic failed.

Transcripts allow up to 64 MiB per turn and incoming ACP lines up to 16 MiB.
See [conversation and tool timing](performance-logging.md) for timestamps and
queue/execution duration fields.

## Herdr conversation status

Inside a Herdr pane, conversation activity appears as `working`, waiting for a
permission decision as `blocked`, and readiness for another request as `idle`.
The integration is automatic in Herdr.app (`herdr-core`, with
`HC_TERMINAL_ID`).
Herdr.app receives OSC 7501 reports through the controlling terminal, including
`done` when a turn finishes and `clear` on exit. Redirected CLI output remains
free of status sequences. Reports contain activity rather than conversation
contents, and failures to update the sidebar leave the design session running.

See [herdr-core's status reference](https://github.com/herdrdev/herdr-core/blob/master/schema/README.md#status)
for the reporting protocol.

## Library embedding and development

The host negotiates ACP protocol v1 through the Rust `agent-client-protocol` SDK.
Dependency versions are maintained in [Cargo.toml](../Cargo.toml).
Use `Conversation::open_with_executable(args, data_dir, lazypencil_binary)` with an
absolute executable path. Direct `AgentHost` callers also set
`AgentOptions.supervisor_executable`; detailed diagnostics can be enabled with
`AgentOptions.diagnostic_directory` pointing to an existing private directory.

Run `cargo xtask test --test agent` for process/protocol checks. Installed-adapter
and live-workflow tests are opt-in; live requests consume account usage. See
[development](development.md#focused-development-checks),
[agent tests](../tests/agent.rs) and [workflow tests](../tests/agent_workflow.rs).
