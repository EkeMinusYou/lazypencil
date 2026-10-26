# Terminal frontend

The initial release targets macOS Apple Silicon and Linux x64/arm64. Intel Mac is unsupported and Windows is deferred; Windows-specific notes below describe retained implementation, not an initial-release requirement.

The default build includes `lazypencil tui`, using the same document, renderer, attachment service, permissions, and live ACP conversations as the CLI. It provides viewing and prompting, without manual drawing or property editing.

```sh
lazypencil tui design.pen
lazypencil tui new.pen --create --agent claude --image reference.png
lazypencil tui design.pen --session LOCAL_CONVERSATION_UUID
lazypencil tui design.pen --image-protocol sixel
```

Install and authenticate an [ACP adapter](agents.md) first. Opening or viewing does not launch the agent. An optional prompt argument and `--image` preload a draft; submission is explicit. Agent, authentication, model, mode, permission, timeout, style, and font options have the same meanings as `prompt`. Resume with the same document, agent, adapter command/arguments, and requested authentication route. The local conversation ID is printed on exit.

Explicit image protocols use the terminal's window-size ioctl without sending capability queries. Auto mode recognizes direct `xterm-kitty` sessions; other auto environments retain the bounded capability probe. Keyboard enhancement requests encode all keys, including Tab, so supported terminals can distinguish presses from releases.

If the adapter cannot load native sessions, the next explicit submission starts a new native session with current document context and bounded local history, and the conversation panel reports the replacement. A load failure does not resend the draft. Reopen with `--session LOCAL_CONVERSATION_UUID --restart-session` to explicitly request a replacement after a load error; submit when ready. This preserves local history and stored images without reattaching past images automatically. See [recovery behavior](agents.md#conversations-and-events) and its verification limits.

If a replacement is interrupted before completing a local turn, the next explicit request supplies document context and bounded history again, including after a successful native load. The panel reports the added local context separately from creating a new native session. A retained failed draft remains editable and is never sent just by reopening the TUI.

Terminal stdin/stdout and at least 50 columns × 16 rows are required. `--json` is rejected before opening/creating a document; use `prompt --json` for structured streams. `--no-default-features` excludes rendering and TUI. `--no-default-features --features rendering` retains image export without TUI.

## Keys and commands

| Focus / key | Action |
| --- | --- |
| `Tab` / `Shift+Tab` | Cycle Prompt, Preview, Nodes, Attachments, Conversation |
| Prompt: `Enter` | Insert a newline |
| Prompt: `Alt+Enter`, `Ctrl+Enter`, `Ctrl+S` | Submit text and pending images |
| Prompt: `Ctrl+V` / `Alt+V` | Explicitly read this host's image, copied image files, or text clipboard |
| Prompt: `/attach <quoted path>` then Enter | Attach a static PNG, JPEG, or WebP file |
| Prompt: `/paste-image` then Enter | Invoke clipboard input without a key binding |
| Prompt: `/clear-images` then Enter | Remove pending images |
| Preview: `hjkl` / arrows | Pan |
| Preview: `+` / `-`, `0` | Zoom or fit the document, including negative coordinates |
| Preview: `[` / `]` | Pause and step shader evaluation time backward/forward by 0.1 seconds |
| Preview: `Space` | Play/pause page captures and the shader timeline at a maximum of 5 fps |
| Prompt: `/play [fps]`, `/pause` then Enter | Start bounded playback (1–10 fps) or stop requesting frames |
| Prompt: `/time <seconds>` then Enter | Seek shader time without modifying the document |
| Nodes: `/` | Search names and IDs; Enter keeps the filter, Esc clears it |
| Nodes: arrows, then `f` / Enter | Select and fit a node; its address accompanies the next AI request |
| Nodes: `Esc` | Clear the editing target |
| Attachments: arrows | Preview pending images in order |
| Attachments: `d`, Delete, Backspace | Remove a pending image without changing its source |
| Conversation: PgUp/PgDn, End | Scroll or follow new output |
| Outside Prompt: `r` | Refresh, including browser captures |
| Outside Prompt: `o`, or `/open-preview` | Open a saved complete PNG in the OS viewer |
| `F1`, or `/help` | Show help |
| `Ctrl+P`, or `/commands` | Open the command palette |
| `/history` then Enter | Browse checkpoints without starting an agent |
| `/logs`, `/conversations` then Enter | Read current saved turns or browse saved conversations |
| `Ctrl+C` | Cancel active input/agent work; exit when idle |
| Outside Prompt: `q`, or `/quit` | Stop the conversation and exit |

Commands occupy one prompt line and are parsed without shell execution, expansion, or substitution. Paths are relative to the launch directory. Bracketed text paste stays text. `Cmd+V` is controlled by the terminal and is not assumed to carry bitmap pixels. `Ctrl+Enter` depends on terminal keyboard support; Alt+Enter and Ctrl+S provide alternatives.

The user's `config.toml` can replace prompt shortcuts, for example:

```toml
[tui.keys]
submit = ["F5", "Ctrl+Alt+s"]
paste = ["F6"]
```

Omitted actions retain their defaults; each explicit list replaces that action's list. Submit accepts 1–8 shortcuts and paste accepts 0–8 (`paste = []` disables clipboard shortcuts while `/paste-image` remains available). Keys support `Enter`, ASCII letters with `Ctrl` or `Alt`, and `F2`–`F24`, optionally combining `Ctrl`, `Alt` and `Shift`. Names are case insensitive and modifier matching is exact. Plain Enter remains a newline; F1, Ctrl+C/Ctrl+P and legacy Ctrl+H/I/J/M aliases are reserved. Duplicate bindings, overlaps between actions and unsupported keys fail with `CONFIGURATION_ERROR`. The terminal must transmit the chosen key distinctly; configuration cannot add keyboard-protocol support to a terminal.

Bindings are captured before entering the terminal and remain fixed for that invocation. The prompt border and help show the effective keys. They act only in the idle prompt; permission dialogs, checkpoint dialogs, search, help and other panels retain their own input handling. Project files cannot remap user shortcuts. Normal viewing and global actions are configured separately under `[tui.navigation]`, described below.

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
| Normal panels | `next_panel`, `previous_panel`, `help`, `commands` |
| Outside Prompt | `quit`, `refresh`, `open_preview` |
| Preview | `pan_left`, `pan_right`, `pan_up`, `pan_down`, `zoom_in`, `zoom_out`, `fit_document`, `time_backward`, `time_forward`, `play_pause` |
| Preview and Nodes | `fit_selected` |
| Nodes and Attachments | `previous_item`, `next_item` |
| Nodes | `search_nodes`, `clear_selection`, `activate_node` |
| Attachments | `remove_attachment` |
| Conversation | `scroll_up`, `scroll_down`, `follow_output` |

Navigation supports ASCII letters and printable symbols, Enter, Space, Tab, Esc/Escape, arrows, Home/End, PageUp/PageDown, Delete/Backspace, and F1–F24. Ctrl, Alt and Shift can be combined; `Plus` names `+`. `BackTab` is the same binding as `Shift+Tab`. Already shifted symbols normalize their Shift bit across terminal protocols, so `+` and `Shift+Plus` are equivalent. Other modifiers match exactly. Keys may be shared between disjoint panels, such as Up for panning and list selection. Overlaps in the same panel, including global bindings that would intercept prompt submit/paste, fail before terminal entry.

Unmodified text bindings do not intercept prompt or node-search input. Permission and checkpoint dialogs retain their displayed acceptance/rejection keys. Ctrl+C remains the fixed emergency cancellation/exit key; its aliases and legacy Ctrl+H/I/J/M encodings cannot be rebound. Prompt shortcuts retain their own restrictions above. Help, footer and panel titles show the captured effective keys. Help supports PageUp/PageDown or arrows to scroll and Home/End to jump. Changing the configuration file affects the next invocation; it does not change the active keyboard map. Projects cannot select these user controls.

[macOS arm64 verification at source 3ea9c65](https://github.com/EkeMinusYou/lazypencil/actions/runs/40034819092) passed all 113 selected integration tests, including custom-key ACP image delivery, permission precedence, and the actual PTY case that reads user settings and submits Japanese multiline text with F5. This is hosted functional evidence; physical keyboard/IME/clipboard behavior and the other platforms remain separate checks.

During an agent turn, the submitted draft and images are locked; viewing, logs, and cancellation remain available. Success clears the submitted draft and pending list. Failure/cancellation preserves them for review and retry. Idle drafts are saved after a one-second debounce and on normal exit. An abrupt kill can lose edits within that debounce. Conversation metadata migrates from version 1 to version 2 for pending text/images.

Permission dialogs show tool details and native choices. Reject Once is initially selected when advertised. PgUp/PgDn scroll details, Enter answers, and Esc denies. Bound document tools use the shared preauthorization policy; other requests remain interactive unless `--non-interactive` rejects them.

## Saved conversation logs

Choose **Browse current conversation logs…** or **Browse saved conversations…** in the command palette. `/logs` and `/conversations` open the same views. Browsing uses shared, read-only conversation APIs and a cancellable background task. It does not authenticate, start/resume an adapter, resend an old prompt, select a different active agent, edit a document, or clear pending text/images. An already active request continues; permission dialogs retain priority. Ctrl+C cancels both a pending log read and an active request.

Lists load 50 entries at a time in stable UUID order, explicitly marked as live listings rather than inferred chronology. Arrows select an entry; Enter opens its turns or transcript. `n`/`p` or PgDn/PgUp request the next/previous list page. Home refreshes the first page, `c` opens the current conversation, and `s` opens saved conversations. Backspace returns to the parent; Esc or `q` closes the browser. Recognized invalid entries remain visible as unavailable.

Transcript pages contain up to 32 KiB of formatted requests, responses, thinking text and tool status. Strings and Unicode escapes are decoded incrementally, with bounded parser buffers instead of loading complete JSON records. Unknown/ambiguous/unfinished records remain visible as sanitized original text. Arrows or PgUp/PgDn scroll within the page; Home/End reach its edges. `n`/`p` or Right/Left move between pages, and `g` returns to the first page. The page trail is bounded by the existing 64 MiB per-turn input limit and at most 8,192 cursors. List paging needs no growing page trail. Terminal resize recalculates wrapping and scroll limits.

`v` switches to the complete raw JSONL and back. Raw terminal controls are shown as visible escapes; damaged UTF-8 also shows its exact Base64 bytes. Raw mode retains the formatted page to return to it while the revision matches. Every new page checks the complete transcript hash before and after reading. A changed file leaves the previous page visible with a notice; `r` explicitly refreshes from the first page. History pages are not replayed into live event handling and cannot answer old permission requests. Overall retention and deletion remain unfinished.

To select saved local state, highlight a conversation and press `u`; the same key in its turn/text view selects that conversation. `N` creates an empty local conversation, preserving the previous draft and images in their original session. The **New empty conversation** palette action does the same. These are explicit state changes, separate from read-only browsing. They require idle agent/input/document work. Selecting the current conversation closes the browser without resetting live state.

A successful change restores the selected draft and images, closes the previous adapter, clears its permissions/events and resets displayed authentication. It leaves the document and preview position intact. The header identifies the selected local session by its UUID prefix; exit prints its full UUID. Selection sends nothing. Native resumption starts on the next submitted request. Busy, missing, foreign or incompatible agent/adapter/authentication targets leave the original selection and draft available. CLI `tui --session UUID` remains available with the corresponding settings. Selection's live-connection and real-PTY regressions passed all three initial-release targets at `b387074` (CI `40134426833`).

`d` reviews deletion of the selected inactive local conversation. The confirmation shows its full UUID, document, local size, draft and image counts; `y` permanently deletes that reviewed state, while `n`/Esc returns. The UUID and action keys stay pinned while arrows, PgUp/PgDn and Home/End scroll the review body, including at 50 columns × 16 rows and with long paths. A changed or busy target is refused; a stale-review error scrolls into view while the UUID/controls remain visible. The current conversation must first be closed or replaced with `u`/`N`; active agent/input/document operations also block deletion. Deleted local logs and images are separate from provider history, project assets and document checkpoints, which remain. The background task preserves the current draft, images and preview, and reports a committed deletion even when cancellation occurs after its atomic rename. Pending cleanup and filesystem-sync warnings remain visible. Interrupted cleanup is recoverable with `conversations cleanup FILE` or `cleanup --all` after the file moved. See [local deletion](agents.md#delete-local-conversation-data). Initial deletion regressions passed all three release targets at `4a51d23` (CI `40139304446`); the new minimum-size and cancellation refinements await hosted verification.

The saved-log tests run on GHA. Protocol-level PTY evidence is separate from physical terminal/IME/clipboard acceptance.

## Checkpoint restoration

Choose **Restore checkpoint…** in the command palette, or use `/history`. The list shows committed checkpoints, newest first, with sequence, creation time, revision, and request ID. It loads 50 entries per page. PgDn/n opens older entries, PgUp/p returns to newer entries, Home opens the latest page, and `r` explicitly reloads the current document/dependency revisions.

Select an entry with arrows and press Enter to review its ID and current/target revisions. The review does not change the document. Esc returns to the list; a second Enter requests restoration. The shared CLI/MCP service validates the checkpoint and both captured revision preconditions, then restores it as a new history entry. The submitted draft, pending images, and source files are preserved. Referenced assets use their current contents; restoration does not roll back external asset edits.

Document or dependency changes after loading the list stop restoration and display a conflict. The UI does not automatically replace the expected revisions or retry. Reload with `r`, review again, and explicitly restore when the new conditions are appropriate. Candidates and restoration use a background worker; agent submission is blocked until that operation finishes. The existing preview can continue rendering.

Esc during an operation requests cancellation and closes its overlay; Ctrl+C also requests cancellation. Lock waits and preparation check the token. Once atomic replacement starts, the service completes the history commit and reports the saved result. Exiting requests cancellation of document, image, and preview work before waiting for the adapter to shut down. The shared cancellation path is also used by CLI/MCP history and restoration. Loading a valid project/dependency snapshot is currently required; restoring through missing current assets remains part of the unfinished dependency-repair work.

## Preview and input work

`--image-protocol auto` is the default, with `kitty`, `sixel`, and `halfblocks` overrides. Kitty uses Unicode placeholders and Sixel uses palette images. Halfblocks and external PNG viewing are fallbacks.

On Unix, probing has a fixed 350 ms deadline and restores stdin flags before starting the event stream. There is no detached reader after a timeout. Plain text received during probing is retained. Windows currently needs an explicit protocol override for high-resolution images; automatic Windows probing remains unfinished. Cell dimensions come from terminal information, query replies, or a 10×20 fallback. tmux passthrough is detected without changing pane settings. Individual terminals, tmux, and SSH need separate compatibility verification.

Rendering/encoding runs on a dedicated thread, which owns the cached layout and script/browser/shader runtimes. Requests coalesce, superseded work is cancelled, and zooms render the requested document region. Captured document/dependency contents are checked once per second without repainting unchanged content. Preview dimensions are limited to 8192 pixels per side and 16 megapixels. Root/dependency revisions are checked again before publication. Errors clear the preview rather than label incomplete output as complete. Kitty updates reuse one app-owned ID; exit deletes only that ID.

Shader time starts at zero and is initially paused. `/time 1.5` evaluates `@time` uniforms at 1.5 seconds; the completed preview's status shows `t=1.500s`. Values must be finite and nonnegative. `[` clamps backward stepping at zero. Seeking preserves the document, zoom, center, and runtime processes, and reuses the captured layout. It does not force browser acquisitions to refresh or advance JavaScript timers. Unchanged dependency polls at the same time avoid re-encoding the image. Time is local to the current TUI invocation and is not saved in the document or conversation.

`/play` starts from the current requested time at a maximum of 5 fps; `/play 2` selects a maximum of 2 fps. Only integer rates from 1 through 10 are accepted. A complete preview containing a rendered browser page or a shader with an `@time` uniform is required. Browser-only documents can play. Each accepted frame captures the existing page DOM without reloading its URL, recomputing an unchanged layout, or invalidating time-independent shader rasters. The selected time continues along the same shader timeline; page JavaScript and CSS use Chromium wall time and are not rewound by seeking. The clock follows monotonic elapsed time; slow rendering drops frames without queuing catch-up work. An automatic frame is accepted only when there is no queued request, running render or unconsumed result. It does not cancel the current render. Help/permission/checkpoint overlays and attachment viewing defer frame requests while the clock continues; returning to the document can therefore jump forward in time.

`/pause` and Space in Preview stop further frame requests; an already requested frame can finish at its fixed requested time. Until then, the status retains the complete image's time and indicates that a requested frame is finishing. `/time` and bracket stepping pause playback before seeking. Ctrl+C pauses playback after any higher-priority document, image-input or agent cancellation. A current rendering error or a completed image without visible browser acquisitions or time-dependent shaders stops playback. A superseded result cannot stop a newer timeline. Paused playback has no frame timer wakeups; ordinary dependency polls and idle runtime cleanup continue. Paused browser acquisitions use the bounded cache. Document script nodes are not given a running clock. Initial playback and LRU eviction passed at source `1936598` on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40052100177) (260 selected tests) and [Linux x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40052102399) (262), including both all-target Clippy configurations and the real PTY case. Artifact digests, required clock/backpressure/LRU successes, true/false acquisition flags, and retained images/reports were checked. The completed-image pause-status correction at `5fd4ddc` also passed [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40052437100) (260) and [Linux x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40052439098) (262), including the worker/app intermediate-state check and actual PTY. Their artifacts were verified. The early retained-page gate checks page-only playback through the real binary in a Unix PTY, including retained DOM colors, pause, draft preservation and the absence of agent startup.

TUI page captures now opt into isolated sessions per document and browser-node address/source. An explicit `r` capture refreshes the existing DOM without navigating to the configured URL again. Size, scale, zoom and scroll edits reuse the session; URL/device changes create a new one. CLI/MCP rendering keeps its fresh-context behavior. At most eight page contexts are retained, with least-recently-used eviction. Removed/disabled nodes close their contexts on the next valid calculated snapshot, while merely moving outside the viewport preserves them within the bound. Changed snapshots invalidate TUI dynamic raster acquisitions so that restoring an earlier URL cannot resurrect its old pixels.

A browser-policy change resets the TUI page process. Idle expiry, cancellation, runtime failure, eviction or document rebinding can also discard DOM state. Cached pixels may survive process expiry; an explicit refresh then opens a new session. The common renderer records `page_state` (`retained`, opaque `id`, `reused`, `retained_limit`) in acquisition metadata, retained in the completed preview summary. Cached metadata describes its original acquisition, not a currently live session. No personal profile or persistent authentication store is introduced. Stateful capture and its expanded PTY checks passed the five-platform unified CI at `e8c6ce3`; the actual PTY checks apply to the four Unix runners. Periodic playback adds a separate browser-only PTY reproduction in the early retained-page gate.

User `config.toml` and project `lazypencil.toml` supply font files/search paths, default family, fallback chain, and locale through the same captured settings as CLI and MCP. Explicit project fields override user fields; paths stay relative to their declaring file. Configuration edits, nearer configuration creation, font membership changes, and symlink retargeting invalidate the cached project. The next preview reload preserves pending draft/images and does not rewrite explicit stored font names. Separate real PTY cases change project and user defaults between the original TrueType and CFF fixtures and verify changed Kitty pixels without manually refreshing or losing the Japanese draft. See [configuration](configuration.md).

The worker tracks successful document images separately from cached layout data. A failed or cancelled render does not suppress the next periodic attempt merely because its layout is current. A successfully published, unchanged preview still skips repeated encoding. The recovery and live-font worker regressions passed hosted macOS arm64 and Linux arm64 verification in run `40000148383`; Intel verification remains in progress.

Script previews use the same captured `[runtime.javascript]` limits as CLI and MCP. Policy changes invalidate the preview, and zero idle time permits immediate script-worker cleanup while retaining the completed image. The `script-config` PTY case exercises a timeout, source/policy repair, decoded green pixels, then a policy-only timeout and repair with unchanged source, no remaining owned child, and retained draft/document bytes. A separate worker regression checks changed snapshot/dependency identities with identical PNG bytes. An unchanged image need not be retransmitted merely because policy changed. The updated assertions await hosted execution; the initial twelfth case incorrectly required a new image packet for identical pixels.

Script, page, and shader processes are reused during consecutive work and expire independently after 60 seconds without execution by default. Their idle times are configurable through `[runtime.javascript]`, `[runtime.browser]` and `[runtime.shader]`. Browser/shader acquisition deadlines and `[runtime.dynamic]` render/cache limits also use the common captured policy; changes invalidate cached previews and rasters. The new browser/shader policy integration awaits hosted verification. The preview thread wakes at the earliest expiry even if it has no pending request, and cleanup runs outside the request mutex. It keeps the preview, captured layout, and raster cache, so cleanup itself neither repaints nor changes the document. `r` outside Prompt explicitly reloads the layout and page captures, starting the required processes again. Other runtime configuration and animation controls remain unfinished.

Clipboard/file input runs in a child process with a five-second watchdog, bounded output, and cancellation. Cancellation and the whole request deadline are checked during the bounded output wait after worker exit, including when a descendant retains stdout. PNG/JPEG/WebP input is normalized before attachment. Batch additions enforce count and source/normalized totals together. On Unix, cancellation terminates the process group and reaps the worker before returning. A new inherited-stdout cancellation regression awaits hosted native verification. arboard provides native clipboard backends, whose availability and limitations still need native OS verification. Tests never read or overwrite the user's general clipboard.

## Verification and remaining scope

Unix builds select Crossterm 0.29's `use-dev-tty` polling backend. A Linux PTY regression exposed a resize/key readiness race in the previous Mio backend: the three F1 bytes remained unread for the entire deadline, and an otherwise ignored extra key caused the pending F1 to be delivered. The upstream [Mio event reader](https://github.com/crossterm-rs/crossterm/blob/36d95b26a26e64b0f8c12edfe11f410a6d56a812/src/event/source/unix/mio.rs) can return a resize before processing another readiness notification. The [poll backend](https://github.com/crossterm-rs/crossterm/blob/36d95b26a26e64b0f8c12edfe11f410a6d56a812/src/event/source/unix/tty.rs) retains unread input as readable. It blocks while idle; this change adds no periodic input polling. Windows retains its native backend, as checked in the target's feature graph; that graph is not a native Windows execution test.

The basic PTY case now alternates widths and opens/closes help eight times, requiring the actual visible response after each key. It sends no recovery key and keeps the original deadline. Timeout diagnostics include the unread-input count, and failed Rust PTY cases preserve their generated logs, raw terminal bytes, and final screen. The original backend failed the strengthened case with three unread bytes; the poll backend passed on Linux and passed all nine PTY cases on macOS.

The basic case also connects the deterministic ACP peer to the actual owned MCP process. Two prompts on that live connection retrieve different guides, a style, and a catalogue page. The test compares their received hashes with six metadata records in the two turn transcripts, checks turn separation, and requires the connection identity to remain unchanged. This extension passed on macOS arm64 and the Debian 12 arm64 container with Rust 1.94; it does not contact an LLM or use a subscription. See [design-resource records](agents.md#design-resource-records) for event timing and limits.

Terminal restoration compares all configuration fields. On Darwin only, the comparison excludes `PENDIN`, a transient kernel input-reprocessing bit. A standalone Python `setraw`/`tcsetattr` round trip reproduced that bit changing without lazypencil; [XNU's terminal code](https://github.com/apple/darwin-xnu/blob/main/bsd/kern/tty.c) handles it when canonical input is restored. The test additionally requires input to wait for Enter after exit, verifies CR-to-LF input translation, and checks restored echo and CRLF output. It does not disable restoration checks or change the application's terminal settings to clear kernel bookkeeping.

- `src/tui/tests.rs`: Ctrl+V through an injected provider to actual ACP image blocks; independently decoded RGBA/alpha; ordered images, removal, failure/restart retention, repeated turns on one connection, permission choices/cancellation, a maximum-size draft, and completion/draft ordering. Checkpoint tests cover review/cancel/restore, draft and image retention, document and image-dependency conflicts, explicit reload, 50-entry pagination boundaries, and cancellation while the writer lock remains held.
- `src/tui/preview.rs`: negative coordinates, latest viewport replacement, and image dependency changes without root revision changes.
- `tests/tui.rs`: the actual binary in a Unix PTY, Japanese multiline input, explicit submission, repeated turns, resize/help/exit, terminal restoration, forced/automatic Kitty/Sixel, and input after a probe timeout. Kitty RGBA and Sixel palette/bands are independently decoded to check design pixels. File-input validation and process cleanup avoid the clipboard. PTY checkpoint cases verify that review does not save, Esc returns to the list, confirmed restoration creates history, and a concurrent CLI edit requires explicit refresh and a new review.
- Its opt-in dynamic test combines real QuickJS, Chromium, and SwiftShader. It decodes red/green/blue Kitty pixels, waits through the real 60-second idle interval, verifies three owned children have exited, and refreshes into new processes while preserving the Japanese draft and document bytes. At source `c95d7ee`, the expanded case also seeks `/time 1` and `/time 0`, observes blue/yellow/blue shader pixels, verifies reuse of the existing runtimes and no repaint during two paused dependency polls, then exercises the original idle cleanup and refresh. The [macOS arm64 run](https://github.com/EkeMinusYou/lazypencil/actions/runs/40050825072) and [Linux x64 run](https://github.com/EkeMinusYou/lazypencil/actions/runs/40050828187) passed. It never starts an AI agent or reads the clipboard; physical terminals and other platforms remain separate checks.

Text assertions use the reconstructed VT screen rather than assuming that visible words are contiguous in raw terminal output. A captured font-preview failure contained a complete decoded Kitty image and the visible `1 nodes · 0 diagnostics` status, while cursor-position updates split the raw word into fragments. The font case now checks the actual status line, which also prevents a document path containing `diagnostics` from satisfying that check. Font-reload failures also report complete-frame counts, pixel hashes and the fixture configuration. Timeout errors include the final screen for diagnosis; the deadline and pixel/configuration checks are unchanged.

The text-only VT observer does not model graphics overlays; old `No complete preview` text alone does not establish that Kitty pixels are absent. Image assertions use decoded complete frames. The observer now decodes Kitty packets incrementally rather than replaying the entire terminal history every time a wait predicate asks for images. It retains complete RGBA frames, including alpha, handles split packets and payload-free image deletion commands, ignores capability queries, and rejects malformed/incomplete pixels and oversized packets/dimensions. Four Python tests check fragmentation, exact chunk decode counts, image integrity and bounded pending wire data. With the same retained macOS executable and Python profiler, the project-font case fell from 879,974 Base64 calls / 6.57 seconds to 658 calls / 2.63 seconds; all eleven normal PTY cases then passed. The application executable, 15-second wait and pixel checks were unchanged. This removes a measured test-observer bottleneck; the original Intel timeout and current Rust changes still await hosted verification.

`cargo xtask terminal --candidate ARCHIVE --output DIRECTORY` configures a separate real-emulator gate for hosted Linux candidates. It verifies the package/source/binary identities, extracts the existing executable, and runs Kitty and xterm/Sixel under Xvfb with software OpenGL requested. The observer decodes bounded TrueColor XWD snapshots into original PNGs and finds three large connected color markers. X11 key/resize events must visibly zoom and pan those markers, restore their positions and sizes, survive a resize round trip, and exit normally. Two stable observations are required per phase; stale/disconnected graphic remnants fail the marker checks. The task does not link the app or Skia and does not rebuild the executable. The candidate focus runs it on a representative Linux environment before the full matrix; the manual candidate workflow also provides it for Linux arm64. This implementation and its original decoder/PNG/connected-region fixtures await hosted execution.

PTY checks do not establish physical-terminal compatibility. Native macOS screenshot paste, physical Kitty/Sixel terminals, IME composition, tmux/SSH, and Linux clipboard/terminal combinations remain to be verified for the initial release. Windows verification is deferred. Persisted log browsing, remaining runtime/cache budgets, animation controls, and performance acceptance tests remain unfinished. Viewing/global key configuration still requires the hosted verification recorded with its implementation change. The full implementation checklist retains those requirements.
