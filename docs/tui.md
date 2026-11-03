# Terminal frontend

## Opening a document

`lazypencil` and `lazypencil tui` open a centered recursive file picker over the empty
workspace. `lazypencil tui FILE` opens directly; a bare file argument is unsupported.
`--create` and `--session` require an explicit file. A terminal of at least 50 columns
by 16 rows is required. `--json` and noninteractive input are rejected before opening
or creating a document.

The picker lists relative `.pen` paths without parsing them. Type to filter; `/` enters
filtering explicitly, arrows or navigation-mode `j`/`k` select, Enter opens, Ctrl+L
accepts a file or search directory, Esc clears input, and Ctrl+C exits. Relative paths
use the displayed root. Discovery stops after 10,000 matches, 100,000 entries, or five
seconds, preserving partial results and the stop reason. Directory symlinks are not
followed; `.gitignore` does not hide documents. Change the root or enter a known file
when a partial scan misses it.

Adapter process/transport preparation runs in the background using user preferences
and explicit CLI options. ACP initialization, document sessions, MCP setup and preview
start after selection; a compatible process is reused. A preparation failure does not
prevent file selection. Explicit-file startup prepares the selected document session.
Drafts and preloaded images remain unsent until submission.

## Workspace and prompt

Nodes and Preview occupy the left/center panels. Agent contains history, image
attachments, explicit node references, queued follow-ups, and the multiline prompt.
Tab enters Agent at Prompt; Alt+Up/Down visits its nonempty sections. Collapsing Nodes
or Agent gives its space to Preview without losing state.

The header reports the connected adapter's actual model, effort, enabled Fast mode,
account/authentication route and local session. Connecting/missing reports are labeled.
Account identity stays in memory, outside transcripts. Agent activity is independent:
Idle, Working, or Blocked for an explicit permission response. During a turn the prompt
remains editable; submitting queues a text follow-up without replacing the active turn
or automatically reattaching its images. Failure/cancellation pauses the queue;
`/resume-queue` resumes and `/clear-queue` discards it. Reopening never submits drafts.

Nodes is a foldable, searchable tree. Selection controls viewing, not the next AI
request's target. Use `a` to add a node or selected diagnostic to References; exact
addresses and captured summaries are frozen on submission. Up to 16 references/64 KiB
can accompany a prompt. Deleted or rebound targets fail preparation instead of using
a similarly named node. The agent inspects current state before editing.

In Nodes, `d` opens Diagnostics for the selected subtree; `s` switches to all nodes.
Enter reveals/fits its target, `f` fits while staying in Diagnostics, and Esc returns
to Nodes. Branch badges include hidden descendants. These diagnostics are also
available through MCP `validate_document`; attachment or selection is not required.

## Initial terminal support

The released platform is macOS Apple Silicon, with the [README terminal baseline](../README.md#supported-environment).
Kitty and Sixel are pixel transports; halfblocks and `/open-preview` provide fallbacks.
Terminal versions, IMEs, multiplexers and SSH forwarding require separate checks.
Linux/Windows backends are retained for future support, and Intel Mac is unsupported.

## Preview behavior

Pan/zoom/fit affect the camera, not document coordinates. Static navigation can project
a previous complete image while an exact frame renders; cached feedback is labeled.
Only completed document frames can be exported. Slow rendering replaces pending work
with the latest view rather than replaying every queued camera position.

Browser pages can retain per-node DOM state for `r` refresh and bounded playback.
Preview Space or `/play [fps]` starts at 5 fps by default (1–10); `/pause` stops new
frames. `/time SECONDS` and `[`/`]` seek shader time, not page JavaScript or document
script execution. Runtime expiry, policy changes, failures and eviction can reset DOM.
See [dynamic content](dynamic-content.md) for capture identity and limits.

`--image-protocol kitty|sixel|halfblocks` selects a transport explicitly. Auto recognizes
direct Kitty or uses a bounded 350 ms probe in other environments, preserving typed
text and consuming terminal replies. Compression is used when supported. Windows
high-resolution output currently needs an explicit override. tmux uses passthrough
without changing pane settings. Direct Kitty-compatible transports use whole rectangular
placements; nested tmux retains the Unicode-placeholder transport. `NO_COLOR` disables
text colors while retaining image identity metadata.

The native cursor follows the prompt's rendered display cells and scrolling so an IME
can anchor its preedit/candidate windows. Prompt/search use the text-safe keyboard mode;
Preview accepts press/repeat/release navigation without moving on release. If Ghostty
has a `shift+enter=text:\n` override, remove it so Shift+Enter retains its modifier.
Plain Enter submits by default; Alt+Enter inserts a newline.

## Panel widths

Set relative widths in your user `config.toml` and restart the TUI:

```toml
[tui.layout]
nodes = 22
preview = 53
agent = 25
```

Each weight accepts an integer from 1 to 10000; the total does not need to be 100 (for example, `2`, `5`, `3` gives 20%, 50%, 30%). Omitted fields use the defaults above. Collapsing Nodes or Agent gives its space to Preview and preserves the other panel's share. Layout settings are user-only and appear in `config show --user` and `config schema --user`.

## Keys and commands

The footer normally occupies one row, showing at most two relevant actions for the current focus and a right-aligned help shortcut. It shows one representative binding per action, preferring non-function keys; while typing, plain text bindings such as `?` are excluded, so help falls back to a usable configured key. Alternatives remain listed in Help. Narrow terminals omit whole hints rather than wrapping them.

Panel headings omit collapse shortcuts. A collapsed panel can replace a secondary footer action with a restore shortcut, with Agent taking priority when both panels are collapsed. An extra row appears only for notifications or activity while Agent is collapsed. Informational notices expire after four seconds; errors and recovery notices stay until resolved or superseded by an explicit operation. Preview refreshes do not erase unrelated errors. Long notices are shortened with an ellipsis. Preview failures stay in Preview and the notification row without entering Agent conversation history. Node counts, diagnostics, zoom and playback time stay on their respective panel borders.

Large or high-DPI viewports automatically reduce preview raster resolution to stay within 8192 pixels per side and 16 megapixels. Camera bounds, fit, zoom and pan continue to use the full display viewport. Kitty scales the bounded raster to fill its terminal cells; oversized Sixel views use halfblocks because Sixel cannot scale the image to an arbitrary display rectangle. Ordinary viewport resolution and CLI export limits are unchanged.

| Focus / key | Action |
| --- | --- |
| Nodes / Diagnostics: `d` | Switch the left panel between Nodes and Diagnostics |
| Diagnostics: `s` | Switch between the selected node and all its descendants' diagnostics and the whole document |
| Diagnostics: `j`/`k` / arrows; `Ctrl+U`/`Ctrl+D` | Select a diagnostic; scroll its details |
| Diagnostics: `Enter`, `f`, `Esc` | Reveal and fit its node in Nodes; fit while staying in Diagnostics; return to Nodes |
| `Tab` / `Shift+Tab` | Move right / left through visible Nodes, Preview, Prompt; wrap at either end and skip collapsed panels. Agent history, attachments and references enter this cycle at Prompt |
| `F2` or `Alt+N` / `F3` or `Alt+A` | Collapse or expand Nodes / Agent, including while typing. Preview fills the freed space; expanding focuses the panel. Drafts, history and node selection are retained. Also available in the command palette |
| Nodes / Diagnostics: `a` | Add the selected node or diagnostic as a prompt reference and focus the input, expanding Agent if collapsed |
| Right region: `Alt+Up` / `Alt+Down` | Switch between conversation history, pending images, node references, and Prompt; skip empty lists |
| Agent history / Attachments / References: `Esc` | Return to Prompt |
| References: arrows / `j`/`k`; Delete / Backspace / `d` | Select a reference; remove it |
| Prompt: `Enter`, `Ctrl+Enter`, `Ctrl+S` | Submit text and pending images, or queue a text follow-up during an active turn |
| Prompt: `Alt+Enter`, `Shift+Enter` | Insert a newline (Shift+Enter requires terminal support) |
| Prompt: `Ctrl+V` / `Alt+V` | Explicitly read this host's image, copied image files, or text clipboard |
| Prompt: `Ctrl+A` / `Ctrl+E`, `Ctrl+B` / `Ctrl+F` | Move to line start/end or one character backward/forward |
| Prompt: `Ctrl+P` / `Ctrl+N`, `Alt+B` / `Alt+F` | Move to the previous/next line or backward/forward by word |
| Prompt: `Ctrl+U` / `Ctrl+K`, `Ctrl+W` / `Alt+D` | Kill to line start/end or the previous/next word; line-boundary kills join lines |
| Prompt: `Ctrl+Y`, `Ctrl+_` / `Ctrl+/` | Yank killed text within the prompt size limit, or undo an edit |
| Prompt: `/attach <quoted path> [WIDTHxHEIGHT\|original]` then Enter | Attach a static PNG, JPEG, or WebP file with an optional explicit send size |
| Prompt: `/paste-image [WIDTHxHEIGHT\|original]` then Enter | Invoke clipboard input with an optional explicit send size |
| Prompt: `/clear-images` then Enter | Remove pending images |
| Preview: `hjkl` / arrows | Pan |
| Preview: `+` / `-`, `0` | Zoom or fit the document, including negative coordinates |
| Preview: `[` / `]` | Pause and step shader evaluation time backward/forward by 0.1 seconds |
| Preview: `Space` | Play/pause page captures and the shader timeline at a maximum of 5 fps |
| Prompt: `/play [fps]`, `/pause` then Enter | Start bounded playback (1–10 fps) or stop requesting frames |
| Prompt: `/time <seconds>` then Enter | Seek shader time without modifying the document |
| Nodes: `/` | Search names and IDs; Enter keeps the filter, Esc clears it |
| Nodes: `j` / `k`, Up / Down | Select the next/previous visible node |
| Nodes: `l` / Right, `h` / Left | Expand/enter a branch, or close its parent branch and select it |
| Nodes: `Ctrl+U` / `Ctrl+D` | Move up/down by half the visible node list |
| Nodes: `Space` | Toggle a branch when no search filter is active |
| Nodes: `f` / Enter | Fit the selected node while keeping focus in Nodes; use `a` to add an explicit prompt reference |
| Nodes: `Esc` | Clear the editing target |
| Attachments: arrows | Preview pending images in order |
| Attachments: `d`, Delete, Backspace | Remove a pending image without changing its source |
| Agent history: PgUp/PgDn, End | Scroll or follow new output |
| Outside Prompt: `r` | Refresh, including browser captures |
| Outside Prompt: `o`, or `/open-preview` | Open a saved complete PNG in the OS viewer |
| `F1`, or `/help` | Show help |
| `Alt+P`, or `/commands` | Open the command palette; Ctrl+P is previous-line editing |
| `/history` then Enter | Browse checkpoints without starting an agent |
| `/logs`, `/conversations` then Enter | Read current saved turns or browse saved conversations |
| `Ctrl+C` | Cancel active input/agent work; exit when idle |
| Outside Prompt: `q`, or `/quit` | Stop the conversation and exit |

During a conversation, requests such as “show the login button”, “show both screens”, “zoom out a little” or “center the heading at this scale” can operate Preview. The agent searches names/types through the existing `read_nodes` tool, uses `get_preview_state` for the selected node and current camera, and calls the TUI-only `set_preview` tool. It can target one or multiple nodes, the entire document, a region or a point, and adjust absolute/relative zoom or pan. “Show both” fits their combined bounds without moving design elements. State queries distinguish the requested view from the last complete frame and its revision. They do not expose draft text.

The agent also receives instructions to search for and display a specific existing target before modifying it, including requests such as “fix the login button”, and to refocus after edits that change its geometry. This is agent-directed behavior, not a deterministic natural-language parser. `set_preview` returns success after a complete frame at the requested view is available. It pauses playback while preserving time, the prompt draft, keyboard focus and selected editing target. Camera state is not saved in the document. Ambiguous matches require resolution; dialogs, attachment viewing, manual camera changes and cancellation may refuse or interrupt an operation. A failed preview operation does not prohibit an otherwise authorized edit, and the agent must report the display limitation.

Ghostty users should leave Shift+Enter unbound so the negotiated keyboard protocol can preserve its Shift modifier. A custom `keybind = shift+enter=text:\n` sends an unmodified line-feed byte instead, which a nested terminal can treat as a submission. Remove that override and reload Ghostty's configuration (`Cmd+Shift+,` on macOS). The terminal and any multiplexer must forward the modified key; the TUI inserts a newline in both idle drafts and follow-ups without submitting or queueing them.

Commands occupy one prompt line and are parsed without shell execution, expansion, or substitution. Paths are relative to the launch directory. Bracketed text paste stays text. `Cmd+V` is controlled by the terminal and is not assumed to carry bitmap pixels. `Ctrl+Enter` and `Shift+Enter` depend on terminal keyboard support; plain Enter submits and Alt+Enter provides a portable newline.

The user's `config.toml` can replace prompt shortcuts, for example:

```toml
[tui.keys]
submit = ["F5", "Ctrl+Alt+s"]
paste = ["F6"]
```

Omitted actions retain their defaults; each explicit list replaces that action's list. Submit accepts 1–8 shortcuts and paste accepts 0–8 (`paste = []` disables clipboard shortcuts while `/paste-image` remains available). Keys support `Enter`, ASCII letters with `Ctrl` or `Alt`, and `F2`–`F24`, optionally combining `Ctrl`, `Alt` and `Shift`. Names are case insensitive and modifier matching is exact. Plain Enter may be configured as submit (the default); unbound Enter inserts a newline. Alt+Enter and Shift+Enter insert a newline unless explicitly bound to another prompt action. F1, Ctrl+C/Ctrl+P and legacy Ctrl+H/I/J/M aliases are reserved. Duplicate bindings, overlaps between actions and unsupported keys fail with `CONFIGURATION_ERROR`. The terminal must transmit the chosen key distinctly; configuration cannot add keyboard-protocol support to a terminal.

Bindings are captured before entering the terminal and remain fixed for that invocation. Help shows the effective keys. They act in the focused prompt, including follow-up input during active turns; permission dialogs, checkpoint dialogs, search, help and other panels retain their own input handling. Project files cannot remap user shortcuts. Normal viewing and global actions are configured separately under `[tui.navigation]`, described below.

### Viewing and global shortcuts

User `config.toml` also accepts `[tui.navigation]`. Each supplied list replaces that action's default list; omitted actions keep the keys in the table above. Up to eight keys may be assigned to an action, and `[]` disables it. Panel switching must retain at least one shortcut usable while typing, so the prompt cannot trap keyboard focus.

```toml
[tui.navigation]
next_panel = ["F6"]
previous_panel = ["F7"]
help = ["F8", "?"]
commands = ["F9"]
quit = ["x"]
pan_left = ["a"]
pan_right = ["d"]
pan_up = ["w"]
pan_down = ["s"]
zoom_in = ["z"]
zoom_out = ["v"]
```

The schema and `config show --user` list all actions and effective defaults:

| Scope | Action fields |
| --- | --- |
| Normal panels | `next_panel`, `previous_panel`, `toggle_nodes_panel`, `toggle_agent_panel`, `help`, `commands` |
| Outside Prompt | `quit`, `refresh`, `open_preview` |
| Preview | `pan_left`, `pan_right`, `pan_up`, `pan_down`, `zoom_in`, `zoom_out`, `fit_document`, `time_backward`, `time_forward`, `play_pause` |
| Preview and Nodes | `fit_selected` |
| Nodes, Attachments and References | `previous_item`, `next_item` |
| Nodes (including Diagnostics) | `reference_node`, `toggle_diagnostics`, `diagnostic_scope`, `search_nodes`, `clear_selection`, `activate_node`, `expand_node`, `collapse_node`, `toggle_node`, `node_page_up`, `node_page_down` |
| Attachments and References | `remove_attachment` |
| Agent history | `scroll_up`, `scroll_down`, `follow_output` |

Navigation supports ASCII letters and printable symbols, Enter, Space, Tab, Esc/Escape, arrows, Home/End, PageUp/PageDown, Delete/Backspace, and F1–F24. Ctrl, Alt and Shift can be combined; `Plus` names `+`. `BackTab` is the same binding as `Shift+Tab`. Already shifted symbols normalize their Shift bit across terminal protocols, so `+` and `Shift+Plus` are equivalent. Other modifiers match exactly. Keys may be shared between disjoint panels, such as Up for panning and list selection. Overlaps in the same panel, including global bindings that would intercept prompt submit/paste, fail before terminal entry.

Unmodified text bindings do not intercept prompt or node-search input. Permission and checkpoint dialogs retain their displayed acceptance/rejection keys. Ctrl+C remains the fixed emergency cancellation/exit key; its aliases and legacy Ctrl+H/I/J/M encodings cannot be rebound. Prompt shortcuts retain their own restrictions above. Help, footer and panel titles show the captured effective keys. Help supports PageUp/PageDown or arrows to scroll and Home/End to jump. Changing the configuration file affects the next invocation; it does not change the active keyboard map. Projects cannot select these user controls.

During a turn, the submitted request is frozen while the prompt holds the next editable draft. Failure/cancellation restores the request when safe or leaves it at the front of the paused queue, preserving newer input. Idle drafts are saved after a one-second debounce and on normal exit; an abrupt kill can lose edits within that interval. Conversation metadata versions 1/2 migrate to version 3 without discarding drafts, images, or continuity state.

Permission dialogs show tool details and native choices. Reject Once is initially selected when advertised. PgUp/PgDn scroll details, Enter answers, and Esc denies. Bound document tools use the shared preauthorization policy; other requests remain interactive unless `--non-interactive` rejects them.

## Saved conversation logs

Choose **Browse current conversation logs…** or **Browse saved conversations…** in the command palette. `/logs` and `/conversations` open the same views. Browsing uses shared, read-only conversation APIs and a cancellable background task. It does not authenticate, start/resume an adapter, resend an old prompt, select a different active agent, edit a document, or clear pending text/images. An already active request continues; permission dialogs retain priority. Ctrl+C cancels both a pending log read and an active request.

Lists load 50 entries at a time in stable UUID order, explicitly marked as live listings rather than inferred chronology. Arrows select an entry; Enter opens its turns or transcript. `n`/`p` or PgDn/PgUp request the next/previous list page. Home refreshes the first page, `c` opens the current conversation, and `s` opens saved conversations. Backspace returns to the parent; Esc or `q` closes the browser. Recognized invalid entries remain visible as unavailable.

Transcript pages contain up to 32 KiB of formatted requests, responses, thinking text and tool status. Strings and Unicode escapes are decoded incrementally, with bounded parser buffers instead of loading complete JSON records. Unknown/ambiguous/unfinished records remain visible as sanitized original text. Arrows or PgUp/PgDn scroll within the page; Home/End reach its edges. `n`/`p` or Right/Left move between pages, and `g` returns to the first page. The page trail is bounded by the existing 64 MiB per-turn input limit and at most 8,192 cursors. List paging needs no growing page trail. Terminal resize recalculates wrapping and scroll limits.

`v` switches to the complete raw JSONL and back. Raw terminal controls are shown as visible escapes; damaged UTF-8 also shows its exact Base64 bytes. Raw mode retains the formatted page to return to it while the revision matches. Every new page checks the complete transcript hash before and after reading. A changed file leaves the previous page visible with a notice; `r` explicitly refreshes from the first page. History pages are not replayed into live event handling and cannot answer old permission requests. Explicit reviewed deletion is described below; user-owned retention and global cleanup are documented in [configuration](configuration.md#local-conversation-retention).

To select saved local state, highlight a conversation and press `u`; the same key in its turn/text view selects that conversation. `N` creates an empty local conversation, preserving the previous draft and images in their original session. The **New empty conversation** palette action does the same. These are explicit state changes, separate from read-only browsing. They require idle agent/input/document work. Selecting the current conversation closes the browser without resetting live state.

A successful change restores the selected draft and images, closes the previous adapter, clears its permissions/events and resets displayed authentication. It leaves the document and preview position intact. The global header identifies the selected local session by its UUID prefix; exit prints its full UUID. Selection sends nothing. The selected session resumes in the background without submitting its draft. Busy, missing, foreign or incompatible agent/adapter/authentication targets leave the original selection and draft available. CLI `tui --session UUID` remains available with the corresponding settings.

`d` reviews deletion of the selected inactive local conversation. The confirmation shows its full UUID, document, local size, draft and image counts; `y` permanently deletes that reviewed state, while `n`/Esc returns. The UUID and action keys stay pinned while arrows, PgUp/PgDn and Home/End scroll the review body, including at 50 columns × 16 rows and with long paths. A changed or busy target is refused; a stale-review error scrolls into view while the UUID/controls remain visible. The current conversation must first be closed or replaced with `u`/`N`; active agent/input/document operations also block deletion. Deleted local logs and images are separate from provider history, project assets and document checkpoints, which remain. The background task preserves the current draft, images and preview, and reports a committed deletion even when cancellation occurs after its atomic rename. Pending cleanup and filesystem-sync warnings remain visible. Interrupted cleanup is recoverable with `conversations cleanup FILE` or `cleanup --all` after the file moved. See [local deletion](agents.md#delete-local-conversation-data).

## Checkpoint restoration

Choose **Restore checkpoint…** in the command palette, or use `/history`. The list shows committed checkpoints, newest first, with sequence, creation time, revision, and request ID. It loads 50 entries per page. PgDn/n opens older entries, PgUp/p returns to newer entries, Home opens the latest page, and `r` explicitly reloads the current document/dependency revisions.

Select an entry with arrows and press Enter to review its ID and current/target revisions. The review does not change the document. Esc returns to the list; a second Enter requests restoration. The shared CLI/MCP service validates the checkpoint and both captured revision preconditions, then restores it as a new history entry. The submitted draft, pending images, and source files are preserved. Referenced assets use their current contents; restoration does not roll back external asset edits.

Document or dependency changes after loading the list stop restoration and display a conflict. The UI does not automatically replace the expected revisions or retry. Reload with `r`, review again, and explicitly restore when the new conditions are appropriate. Candidates and restoration use a background worker; agent submission is blocked until that operation finishes. The existing preview can continue rendering.

Esc during an operation requests cancellation and closes its overlay; Ctrl+C also requests cancellation. Lock waits and preparation check the token. Once atomic replacement starts, the service completes the history commit and reports the saved result. Exiting requests cancellation of document, image, and preview work before waiting for the adapter to shut down. The shared cancellation path is also used by CLI/MCP history and restoration. Supported missing declared assets remain explicit diagnostics in a restored result; restoration does not make an incomplete design renderable.

## Preview and input work

Document calculation/rendering, log reads, restores, and image conversion run outside
the interactive input path. Revision/dependency changes reload the preview while
preserving drafts and attachments. Repeated preview failures stay visible without
flooding conversation history; a later refresh can recover.

Clipboard input reads only this host's clipboard and is explicit. A bounded child
worker normalizes images; conversion/storage runs on the input thread. Cancellation
reclaims unadopted candidates while retaining original attachments. See
[image input](image-attachments.md) and [attachment budgets](configuration.md#image-attachment-budgets).

Script/page/shader workers independently expire after 60 idle seconds by default;
cached pixels may remain. Configuration changes invalidate affected calculations and
acquisitions. An explicit refresh starts the required runtimes again. Dedicated
[authentication profiles](runtimes.md#dedicated-authentication-profiles) are opt-in;
anonymous rendering never adopts a personal browser profile.

For regression checks use `cargo xtask test --lib --filter tui::` and
`cargo xtask test --test tui`; see [development](development.md). PTY image/cursor
assertions do not measure physical display latency or certify every IME.
