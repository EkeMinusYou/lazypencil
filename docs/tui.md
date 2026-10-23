# Terminal frontend

The default build includes `lazypencil tui`, using the same document, renderer, attachment service, permissions, and live ACP conversations as the CLI. It provides viewing and prompting, without manual drawing or property editing.

```sh
lazypencil tui design.pen
lazypencil tui new.pen --create --agent claude --image reference.png
lazypencil tui design.pen --session LOCAL_CONVERSATION_UUID
lazypencil tui design.pen --image-protocol sixel
```

Install and authenticate an [ACP adapter](agents.md) first. Opening or viewing does not launch the agent. An optional prompt argument and `--image` preload a draft; submission is explicit. Agent, authentication, model, mode, permission, timeout, style, and font options have the same meanings as `prompt`. Resume with the same document, agent, adapter command/arguments, and requested authentication route. The local conversation ID is printed on exit.

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
| `Ctrl+C` | Cancel active input/agent work; exit when idle |
| Outside Prompt: `q`, or `/quit` | Stop the conversation and exit |

Commands occupy one prompt line and are parsed without shell execution, expansion, or substitution. Paths are relative to the launch directory. Bracketed text paste stays text. `Cmd+V` is controlled by the terminal and is not assumed to carry bitmap pixels. `Ctrl+Enter` depends on terminal keyboard support; Alt+Enter and Ctrl+S provide alternatives.

During an agent turn, the submitted draft and images are locked; viewing, logs, and cancellation remain available. Success clears the submitted draft and pending list. Failure/cancellation preserves them for review and retry. Idle drafts are saved after a one-second debounce and on normal exit. An abrupt kill can lose edits within that debounce. Conversation metadata migrates from version 1 to version 2 for pending text/images.

Permission dialogs show tool details and native choices. Reject Once is initially selected when advertised. PgUp/PgDn scroll details, Enter answers, and Esc denies. Bound document tools use the shared preauthorization policy; other requests remain interactive unless `--non-interactive` rejects them.

## Checkpoint restoration

Choose **Restore checkpoint…** in the command palette, or use `/history`. The list shows committed checkpoints, newest first, with sequence, creation time, revision, and request ID. It loads 50 entries per page. PgDn/n opens older entries, PgUp/p returns to newer entries, Home opens the latest page, and `r` explicitly reloads the current document/dependency revisions.

Select an entry with arrows and press Enter to review its ID and current/target revisions. The review does not change the document. Esc returns to the list; a second Enter requests restoration. The shared CLI/MCP service validates the checkpoint and both captured revision preconditions, then restores it as a new history entry. The submitted draft, pending images, and source files are preserved. Referenced assets use their current contents; restoration does not roll back external asset edits.

Document or dependency changes after loading the list stop restoration and display a conflict. The UI does not automatically replace the expected revisions or retry. Reload with `r`, review again, and explicitly restore when the new conditions are appropriate. Candidates and restoration use a background worker; agent submission is blocked until that operation finishes. The existing preview can continue rendering.

Esc during an operation requests cancellation and closes its overlay; Ctrl+C also requests cancellation. Lock waits and preparation check the token. Once atomic replacement starts, the service completes the history commit and reports the saved result. Exiting requests cancellation of document, image, and preview work before waiting for the adapter to shut down. The shared cancellation path is also used by CLI/MCP history and restoration. Loading a valid project/dependency snapshot is currently required; restoring through missing current assets remains part of the unfinished dependency-repair work.

## Preview and input work

`--image-protocol auto` is the default, with `kitty`, `sixel`, and `halfblocks` overrides. Kitty uses Unicode placeholders and Sixel uses palette images. Halfblocks and external PNG viewing are fallbacks.

On Unix, probing has a fixed 350 ms deadline and restores stdin flags before starting the event stream. There is no detached reader after a timeout. Plain text received during probing is retained. Windows currently needs an explicit protocol override for high-resolution images; automatic Windows probing remains unfinished. Cell dimensions come from terminal information, query replies, or a 10×20 fallback. tmux passthrough is detected without changing pane settings. Individual terminals, tmux, and SSH need separate compatibility verification.

Rendering/encoding runs on a dedicated thread, which owns the cached layout and script/browser/shader runtimes. Requests coalesce, superseded work is cancelled, and zooms render the requested document region. Captured document/dependency contents are checked once per second without repainting unchanged content. Preview dimensions are limited to 8192 pixels per side and 16 megapixels. Root/dependency revisions are checked again before publication. Errors clear the preview rather than label incomplete output as complete. Kitty updates reuse one app-owned ID; exit deletes only that ID.

User `config.toml` and project `lazypencil.toml` supply font files/search paths, default family, fallback chain, and locale through the same captured settings as CLI and MCP. Explicit project fields override user fields; paths stay relative to their declaring file. Configuration edits, nearer configuration creation, font membership changes, and symlink retargeting invalidate the cached project. The next preview reload preserves pending draft/images and does not rewrite explicit stored font names. Separate real PTY cases change project and user defaults between the original TrueType and CFF fixtures and verify changed Kitty pixels without manually refreshing or losing the Japanese draft. See [configuration](configuration.md).

The worker tracks successful document images separately from cached layout data. A failed or cancelled render does not suppress the next periodic attempt merely because its layout is current. A successfully published, unchanged preview still skips repeated encoding. The new recovery and live-font worker regressions await hosted native verification; the latest Intel PTY font-reload timeout remains under investigation.

Script previews use the same captured `[runtime.javascript]` limits as CLI and MCP. Policy changes invalidate the preview, and zero idle time permits immediate script-worker cleanup while retaining the completed image. The new `script-config` PTY case exercises a timeout, source/policy repair, decoded green pixels, another policy update, no remaining owned child, and retained draft/document bytes. This twelfth normal PTY case awaits GHA execution with the new native binary.

Script, page, and shader processes are reused during consecutive work and expire independently after 60 seconds without execution. The preview thread wakes at the earliest expiry even if it has no pending request, and cleanup runs outside the request mutex. It keeps the preview, captured layout, and raster cache, so cleanup itself neither repaints nor changes the document. `r` outside Prompt explicitly reloads the layout and page captures, starting the required processes again. The idle timeout is currently fixed; runtime configuration and animation controls remain unfinished.

Clipboard/file input runs in a child process with a five-second watchdog, bounded output, and cancellation. PNG/JPEG/WebP input is normalized before attachment. Batch additions enforce count and source/normalized totals together. On Unix, cancellation terminates the process group and reaps the worker before returning. arboard provides native clipboard backends, whose availability and limitations still need native OS verification. Tests never read or overwrite the user's general clipboard.

## Verification and remaining scope

Unix builds select Crossterm 0.29's `use-dev-tty` polling backend. A Linux PTY regression exposed a resize/key readiness race in the previous Mio backend: the three F1 bytes remained unread for the entire deadline, and an otherwise ignored extra key caused the pending F1 to be delivered. The upstream [Mio event reader](https://github.com/crossterm-rs/crossterm/blob/36d95b26a26e64b0f8c12edfe11f410a6d56a812/src/event/source/unix/mio.rs) can return a resize before processing another readiness notification. The [poll backend](https://github.com/crossterm-rs/crossterm/blob/36d95b26a26e64b0f8c12edfe11f410a6d56a812/src/event/source/unix/tty.rs) retains unread input as readable. It blocks while idle; this change adds no periodic input polling. Windows retains its native backend, as checked in the target's feature graph; that graph is not a native Windows execution test.

The basic PTY case now alternates widths and opens/closes help eight times, requiring the actual visible response after each key. It sends no recovery key and keeps the original deadline. Timeout diagnostics include the unread-input count, and failed Rust PTY cases preserve their generated logs, raw terminal bytes, and final screen. The original backend failed the strengthened case with three unread bytes; the poll backend passed on Linux and passed all nine PTY cases on macOS.

The basic case also connects the deterministic ACP peer to the actual owned MCP process. Two prompts on that live connection retrieve different guides, a style, and a catalogue page. The test compares their received hashes with six metadata records in the two turn transcripts, checks turn separation, and requires the connection identity to remain unchanged. This extension passed on macOS arm64 and the Debian 12 arm64 container with Rust 1.94; it does not contact an LLM or use a subscription. See [design-resource records](agents.md#design-resource-records) for event timing and limits.

Terminal restoration compares all configuration fields. On Darwin only, the comparison excludes `PENDIN`, a transient kernel input-reprocessing bit. A standalone Python `setraw`/`tcsetattr` round trip reproduced that bit changing without lazypencil; [XNU's terminal code](https://github.com/apple/darwin-xnu/blob/main/bsd/kern/tty.c) handles it when canonical input is restored. The test additionally requires input to wait for Enter after exit, verifies CR-to-LF input translation, and checks restored echo and CRLF output. It does not disable restoration checks or change the application's terminal settings to clear kernel bookkeeping.

- `src/tui/tests.rs`: Ctrl+V through an injected provider to actual ACP image blocks; independently decoded RGBA/alpha; ordered images, removal, failure/restart retention, repeated turns on one connection, permission choices/cancellation, a maximum-size draft, and completion/draft ordering. Checkpoint tests cover review/cancel/restore, draft and image retention, document and image-dependency conflicts, explicit reload, 50-entry pagination boundaries, and cancellation while the writer lock remains held.
- `src/tui/preview.rs`: negative coordinates, latest viewport replacement, and image dependency changes without root revision changes.
- `tests/tui.rs`: the actual binary in a Unix PTY, Japanese multiline input, explicit submission, repeated turns, resize/help/exit, terminal restoration, forced/automatic Kitty/Sixel, and input after a probe timeout. Kitty RGBA and Sixel palette/bands are independently decoded to check design pixels. File-input validation and process cleanup avoid the clipboard. PTY checkpoint cases verify that review does not save, Esc returns to the list, confirmed restoration creates history, and a concurrent CLI edit requires explicit refresh and a new review.
- Its opt-in dynamic test combines real QuickJS, Chromium, and SwiftShader. It decodes red/green/blue Kitty pixels, waits through the real 60-second idle interval, verifies three owned children have exited, and refreshes into new processes while preserving the Japanese draft and document bytes. This test passed on macOS arm64 and never starts an AI agent or reads the clipboard.

Text assertions use the reconstructed VT screen rather than assuming that visible words are contiguous in raw terminal output. A captured font-preview failure contained a complete decoded Kitty image and the visible `1 nodes · 0 diagnostics` status, while cursor-position updates split the raw word into fragments. The font case now checks the actual status line, which also prevents a document path containing `diagnostics` from satisfying that check. Font-reload failures also report complete-frame counts, pixel hashes and the fixture configuration. Timeout errors include the final screen for diagnosis; the deadline and pixel/configuration checks are unchanged.

The text-only VT observer does not model graphics overlays; old `No complete preview` text alone does not establish that Kitty pixels are absent. Image assertions use decoded complete frames. The observer now decodes Kitty packets incrementally rather than replaying the entire terminal history every time a wait predicate asks for images. It retains complete RGBA frames, including alpha, handles split packets and payload-free image deletion commands, ignores capability queries, and rejects malformed/incomplete pixels and oversized packets/dimensions. Four Python tests check fragmentation, exact chunk decode counts, image integrity and bounded pending wire data. With the same retained macOS executable and Python profiler, the project-font case fell from 879,974 Base64 calls / 6.57 seconds to 658 calls / 2.63 seconds; all eleven normal PTY cases then passed. The application executable, 15-second wait and pixel checks were unchanged. This removes a measured test-observer bottleneck; the original Intel timeout and current Rust changes still await hosted verification.

PTY checks do not establish physical-terminal compatibility. Native macOS screenshot paste, real Kitty/Sixel terminals, IME composition, tmux/SSH, Windows, and Linux clipboard/terminal combinations remain to be verified. Configurable key bindings, persisted log browsing, configurable runtime/cache budgets, animation controls, and performance acceptance tests remain unfinished. The full implementation checklist retains those requirements.
