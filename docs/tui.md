# Terminal workspace

Create and refine a design while viewing its node tree, preview and AI conversation
in one terminal. For installation and agent setup, start with the
[quick start](../README.md#quick-start).

## Opening a document

```sh
lazypencil
lazypencil tui design.pen
lazypencil tui new.pen --create --agent codex
```

Running without a file opens a document picker. Type to filter, use arrows or
`j`/`k` to select, and press Enter to open. Ctrl+L switches between filtering and
entering a file or directory path;
Esc clears the filter and Ctrl+C exits. If a large directory is only partially
listed, narrow the search or enter the file's path directly.

Use a terminal at least 50 columns by 16 rows. `--create` needs an explicit filename
and preserves existing files. For scripted output, use the CLI or MCP rather than
`--json tui`.

Opening a document restores its last selected/submitted conversation, including
draft, images and saved history. Scroll up to load older messages. Reopening leaves
queued or interrupted input paused for review; it does not submit a draft.
A startup prompt replaces the restored draft and still waits for submission.

Use `tui FILE --new-conversation`, `/new-conversation`, or **Conversation · New empty
conversation…** for a fresh conversation. Previous drafts and logs stay in their
original conversation. `--session UUID` selects a specific saved conversation and
cannot be combined with `--new-conversation`.

One frontend can use a conversation at a time. Automatic reopening starts a new
one if the previous selection is unavailable and tells you why. Explicit
`--session` reports an error when that conversation cannot be opened. See
[agent conversations](agents.md#conversations-and-events) for CLI resume and recovery.

## Workspace and prompt

| Panel | Use it for |
| --- | --- |
| Nodes | Browse and search the document tree, inspect diagnostics, add references |
| Preview | Pan, zoom, select and inspect the design |
| Agent | Read responses, attach images, review references and enter requests |

Tab moves between visible panels. Existing documents start in Preview; new
documents start at Prompt. Alt+Up/Down moves between Agent's history, attachments,
references and prompt. Collapse Nodes or Agent to give Preview more room.

The header shows the connected agent's model, effort, Fast mode and account.
The activity indicator shows Idle, Working or Blocked while an approval is needed.
Write the next draft while the agent works. Submitting text steers a compatible
active turn; `/queue <text>` explicitly schedules a later turn. Images and node
references use a queued turn.

If delivery is uncertain or a turn fails, the queue pauses. Check saved logs before
using `/resume-queue`; `/clear-queue` discards queued requests. Draft edits and
attachments added later belong to the next request.

### Refer to nodes and diagnostics

Select a node in Nodes and press `a` to add it to the next prompt's References.
Selecting or fitting a node only changes what you are viewing; add a reference
when your request should target it. A prompt allows up to 16 references / 64 KiB.

Press `d` in Nodes to open Diagnostics. `s` switches between the selected subtree
and the whole document. Enter reveals a diagnostic's node, `f` fits it while
staying in Diagnostics, and Esc returns to Nodes.

## Terminal image previews

Use Kitty or WezTerm for full-resolution image previews. In another terminal,
try a block-character preview:

```sh
lazypencil tui design.pen --image-protocol halfblocks
```

Choose **Preview · Open preview externally** from the command palette to open a
PNG in another app. You can also export directly with
`lazypencil render design.pen --output design.png`.

If terminal images are missing or garbled, run `lazypencil doctor`. With tmux or
SSH, try the same document directly in a local terminal to isolate forwarding
issues. `--image-protocol kitty`, `sixel` or `halfblocks` selects the transport
explicitly; otherwise it is detected automatically.

## Preview behavior

Use arrows or `hjkl` to pan, `+`/`-` to zoom, and `0` to fit the document. These
controls move the camera without changing design elements. `f` fits the selected
node or selected shapes.

Root names appear above the design for orientation and are omitted from exported
images. Toggle them with **View · Toggle node names** or set
`[tui.preview] show_node_names = false` in user settings.

The preview canvas follows the UI theme while the design keeps its own colors.
CLI PNG exports preserve the document's transparency. **Open preview externally**
uses the preview's canvas background.

For dynamic content, `p` starts or pauses playback at 5 fps by default; the palette's
**Preview · Play…** accepts 1–10 fps. `Alt+[` / `Alt+]` steps shader time, and
**Preview · Seek shader time…** accepts a specific time. Shader seeking does not
rewind web pages or document scripts. `r` refreshes browser captures.

On very large or high-DPI viewports, preview resolution is reduced to keep
navigation usable. PNG export has its own size limits; see [graphics](graphics.md).

## UI themes

Choose **View · Theme…** in the command palette and enter `auto`, `dark`, `light`
or `monochrome`. For a persistent preference:

```toml
[tui]
theme = "auto"
```

`auto` follows the terminal background, falling back to dark when it cannot be
detected. `lazypencil tui FILE --theme light` selects a theme for one launch.
Theme changes affect the workspace and preview background, while document colors
stay the same. Restart after changing the terminal's own theme.

## Mouse and trackpad

Click a panel to focus it. Scrolling acts on the region under the pointer while
you continue typing in the focused prompt.

| Region / gesture | Action |
| --- | --- |
| Preview: wheel or two-finger scrolling | Pan vertically/horizontally in the reported direction |
| Preview: Ctrl + vertical scroll | Zoom around the pointer, keeping the document point beneath it fixed |
| Preview: Shift + vertical scroll | Pan horizontally when the terminal forwards the gesture |
| Preview: Space + left drag, middle drag, or right drag | Grab and pan the canvas; document coordinates stay unchanged |
| Preview: click / ordinary left drag | Select the topmost shape / select fully enclosed shapes |
| Preview: Shift + click/drag | Toggle a shape / add enclosed shapes to the selection |
| Preview: Esc | Clear canvas selection |
| Preview: pinch (macOS native input) | Zoom around the pointer after enabling pinch input |
| Nodes: row click / triangle click | Select a node / expand or collapse its branch |
| Nodes, diagnostics, attachments, references, and modal lists: scroll | Scroll the list without changing selection |
| Agent history / diagnostic details / Help / saved text: scroll | Scroll the text; history resumes following output at the bottom |
| File picker / command palette / checkpoints | Click a row to select, then Open / Run / Review to activate |

Selections and camera gestures change the view. Use an AI request to move, resize
or edit elements. Fit selected uses the canvas selection in Preview or the current
row in Nodes. `[` selects and fits a child; `]` selects and fits its parent.

To show clickable preview buttons and panel toggles, enable `show_controls` in
user settings. **View controls…** in the command palette also opens a menu with
zoom, fit, playback and a temporary wheel pan/zoom switch.

### Native gestures on macOS

Choose **Enable native gestures…** in the command palette, allow the requested
application in macOS Accessibility settings, then retry. Move the pointer inside
Preview and pinch or scroll with two fingers. The bridge captures gestures inside
the foreground canvas; other panes and apps retain their gestures. Move slightly
inward if a gesture at the canvas edge is not picked up.

After access is granted, local sessions start native input automatically. Set
`native_pinch = false` or `native_scroll = false` to disable either behavior.
SSH sessions use terminal-forwarded input.

If Ctrl+scroll is not forwarded by your terminal, use the view menu's wheel mode
or keyboard zoom. Space-drag depends on key-release reporting; middle/right drag
and the buttons are alternatives.

### Pointer preferences

```toml
[tui.mouse]
enabled = true
show_controls = false
native_pinch = true
native_scroll = true
wheel_zoom = false
scroll_step = 2
zoom_step = 1.1
```

`wheel_zoom = true` makes vertical scrolling zoom and Ctrl+scroll pan.
`scroll_step` accepts 1–20; `zoom_step` accepts 1.01–2.0. Native precise panning
uses the actual scroll distance. Settings apply on the next launch;
the view menu's wheel switch lasts for the current session.

`enabled = false` leaves pointer handling to the terminal. While pointer handling
is enabled, Space in Preview is reserved for drag panning; use another key for
playback. User controls belong in user configuration, not project files.

## Panel widths

Set relative widths in user settings and restart:

```toml
[tui.layout]
nodes = 22
preview = 53
agent = 25
```

Weights accept 1–10,000 and need not total 100. For example, `2`, `5`, `3` gives
20%, 50%, 30%. Collapsing a side panel expands Preview while retaining panel state.

## Keys and commands

F1 opens the complete key reference. The footer shows actions for the current
focus; errors and recovery notices stay visible until resolved. Use Alt+P for the
command palette.

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
| Prompt: `Enter`, `Ctrl+Enter`, `Ctrl+S` | Submit text and pending images; during work, steer a compatible agent or queue a follow-up |
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
| Preview: `/` | Search text, names and IDs (case-insensitive); select and fit the first matching element while typing |
| Nodes / Preview search: Enter / Esc | Keep the filter and return to navigation / clear the search. Both panes show the query in their title; Preview uses `n` / `N` to select and fit the next / previous result after Enter |
| Nodes / Preview search: `Ctrl+H` / `Ctrl+D`, `Ctrl+A` / `Ctrl+E`, `Ctrl+B` / `Ctrl+F` | Delete backward/forward, move to the start/end, or move one character backward/forward. Left/Right, Home/End and Backspace/Delete also edit the query |
| Nodes / Preview search: `Alt+B` / `Alt+F`, `Ctrl+U` / `Ctrl+K`, `Ctrl+W` / `Alt+D`, `Ctrl+Y`, `Ctrl+_` / `Ctrl+/` | Move by word, kill to the start/end, kill the previous/next word, yank, or undo; pasted text inserts at the cursor |
| Preview: `+` / `-`, `0` | Zoom or fit the document, including negative coordinates |
| Preview: `[` / `]` | Select and fit the first enabled child / parent; when unselected, select the first visible node in Preview |
| Preview: `Alt+[` / `Alt+]` | Pause and step shader evaluation time backward/forward by 0.1 seconds |
| Preview: `p` | Play/pause page captures and the shader timeline at a maximum of 5 fps |
| Palette: Preview · Play… / Pause | Start bounded playback (1–10 fps) or stop requesting frames |
| Palette: Preview · Seek shader time… | Seek shader time without modifying the document |
| Nodes: `/` | Search names and IDs; Enter keeps the filter, Esc clears it |
| Nodes: `j` / `k`, Up / Down | Select the next/previous visible node |
| Nodes: `l` / Right, `h` / Left | Expand/enter a branch, or close its parent branch and select it |
| Nodes: `Ctrl+U` / `Ctrl+D` | Move up/down by half the visible node list |
| Nodes: `Space` | Toggle a branch when no search filter is active |
| Nodes: Enter | Expand a branch, or enter its first child if already expanded; fit the selected node if it has no children |
| Nodes: `f` | Fit the selected node while keeping focus in Nodes; use `a` to add an explicit prompt reference |
| Nodes: `Esc` | Clear node and canvas selection; keep the camera |
| Attachments: arrows | Preview pending images in order |
| Attachments: `d`, Delete, Backspace | Remove a pending image without changing its source |
| Agent history: PgUp/PgDn, End | Scroll or follow new output |
| Outside Prompt: `r` | Refresh, including browser captures |
| Outside Prompt: `o`, or Preview · Open preview externally in the palette | Open a saved complete PNG in the OS viewer |
| `F1`, or `/help` | Show help |
| `Alt+P` | Open the command palette; Ctrl+P is previous-line editing |
| Palette: Checkpoints · Browse and restore checkpoints… | Browse checkpoints without starting an agent |
| `/logs`, `/conversations` then Enter | Read current saved turns or browse saved conversations |
| `Ctrl+C` | Cancel active input/agent work; exit when idle |
| Outside Prompt: `q`, or General · Quit in the palette | Stop the conversation and exit |

### Command palette and slash commands

Type in the palette to filter actions by name, category or description. Esc clears
the search, then closes the palette. Each row shows its category: Conversation,
Send queue, Images, Preview, View,
Checkpoints and General. Select an action to see its syntax, description or why
it is unavailable. Enter runs it. Actions that need arguments open an input dialog;
Esc returns to the palette. Your current prompt and attachments remain intact.

| Category | Slash commands |
| --- | --- |
| Conversation | `/permissions` (alias `/mode`), `/new-conversation`, `/logs`, `/conversations` |
| Send queue | `/queue <text>`, `/resume-queue`, `/clear-queue` |
| Images | `/attach <quoted path> [WIDTHxHEIGHT\|original]`, `/paste-image [WIDTHxHEIGHT\|original]`, `/resize-image <number> <WIDTHxHEIGHT\|original>`, `/clear-images` |
| Help | `/help` lists conversation commands; F1 and palette Help show workspace help |

Type `/` in a single-line prompt to complete conversation commands. Up/Down selects
a candidate; Tab or Enter fills its name. Enter on an exact command executes it.
With arguments or multiline text, normal editing remains available.
Use the palette for workspace actions such as theme, playback and checkpoints.

You can ask the agent to navigate Preview, for example “show both screens” or
“center the login button”. It can search the design, fit nodes and adjust the camera.
This preserves design coordinates. Clarify ambiguous names if several elements
match your request.

### Prompt shortcuts

Enter, Ctrl+Enter or Ctrl+S submits by default. Alt+Enter and Shift+Enter insert a
newline. Modified Enter keys depend on terminal support. In Ghostty, remove a
custom `shift+enter=text:\n` binding if it turns a newline into a submission, then
reload the terminal configuration.

Ctrl+V / Alt+V reads the current host's clipboard. Cmd+V is handled by the terminal.
Paths in image commands resolve from the launch directory. See
[image attachments](image-attachments.md) for file input and resizing.

Replace submit or paste shortcuts in user configuration:

```toml
[tui.keys]
submit = ["F5", "Ctrl+Alt+s"]
paste = ["F6"]
```

Each explicit list replaces that action's defaults. Submit accepts 1–8 keys;
paste accepts 0–8, with `[]` disabling its shortcuts. Supported keys include Enter,
ASCII letters with Ctrl or Alt, and F2–F24, optionally with Ctrl, Alt and Shift.
F1, Ctrl+C, Ctrl+P and legacy Ctrl+H/I/J/M aliases are reserved. Conflicting or
unsupported bindings produce `CONFIGURATION_ERROR` before the workspace opens.

Help shows the effective keys. An unbound Enter inserts a newline. Settings apply
on the next launch; dialogs and other panels retain their own controls.

### Viewing and global shortcuts

Use `[tui.navigation]` to replace viewing and global keys. Each action accepts up
to eight keys; `[]` disables it. Keep at least one panel-switching key usable while
typing.

```toml
[tui.navigation]
next_panel = ["F7"]
previous_panel = ["F8"]
help = ["F9", "?"]
commands = ["F10"]
quit = ["x"]
pan_left = ["a"]
pan_right = ["d"]
pan_up = ["w"]
pan_down = ["s"]
zoom_in = ["z"]
zoom_out = ["v"]
```

| Scope | Action fields |
| --- | --- |
| Normal panels | `next_panel`, `previous_panel`, `toggle_nodes_panel`, `toggle_agent_panel`, `help`, `commands` |
| Outside Prompt | `quit`, `refresh`, `open_preview` |
| Preview | `pan_left`, `pan_right`, `pan_up`, `pan_down`, `zoom_in`, `zoom_out`, `fit_document`, `select_child`, `select_parent`, `time_backward`, `time_forward`, `play_pause`, `search_preview`, `next_search_match`, `previous_search_match` |
| Preview and Nodes | `fit_selected` |
| Nodes, Attachments and References | `previous_item`, `next_item` |
| Nodes (including Diagnostics) | `reference_node`, `toggle_diagnostics`, `diagnostic_scope`, `search_nodes`, `clear_selection`, `activate_node`, `expand_node`, `collapse_node`, `toggle_node`, `node_page_up`, `node_page_down` |
| Attachments and References | `remove_attachment` |
| Agent history | `scroll_up`, `scroll_down`, `follow_output` |

Navigation supports printable ASCII, Enter, Space, Tab, Esc, arrows, Home/End,
PageUp/PageDown, Delete/Backspace and F1–F24 with Ctrl/Alt/Shift. Use `Plus` for `+`;
`BackTab` and `Shift+Tab` are equivalent. Keys can be shared by disjoint panels,
but conflicts in the same panel are rejected. Text keys leave prompt and search
input available. Ctrl+C remains the cancellation/exit key.

## Saved conversation logs

Open **Conversation · Current conversation logs…** or **Saved conversations…**
in the palette, or use `/logs` and `/conversations`. Press `/` in a saved
conversation or turn list to search the current page; Enter opens the selected
match, Tab returns to list shortcuts, and Esc clears the search. Browsing preserves
the current draft and design. A running agent continues while you read.

| Key | Action |
| --- | --- |
| Arrows, Enter | Select a conversation/turn and open it |
| `n`/`p`, PgDn/PgUp in lists | Next/previous list page |
| Home in lists | Refresh the first page |
| `c` / `s` | Open current/saved conversations |
| Backspace | Return to the parent list |
| Esc or `q` | Close the browser |
| Arrows, PgUp/PgDn in text | Scroll the transcript |
| `n`/`p` or Right/Left in text | Next/previous text page |
| `g` / `r` in text | First page / refresh changed content |
| `v` in text | Toggle readable text and raw JSONL |
| `u` | Select the highlighted conversation |
| `N` | Create an empty conversation |
| `d` | Review deletion of an inactive conversation |

Selecting a conversation restores its draft, images and history while keeping the
design and camera position. It reconnects using the matching agent settings and
waits for submission. Finish or cancel active agent, image or document operations
before changing selection. A busy or incompatible conversation leaves the current
selection available.

Deletion shows the UUID, document, local size, draft and image counts. `y` confirms;
`n` or Esc returns. Select another conversation before deleting the current one.
Provider history, project assets and document checkpoints remain. If cleanup is
interrupted, run `lazypencil conversations cleanup FILE` or `cleanup --all`.
See [local deletion](agents.md#delete-local-conversation-data) and
[retention settings](configuration.md#local-conversation-retention).

## Checkpoint restoration

Choose **Checkpoints · Browse and restore checkpoints…** in the palette. Entries
are newest first. PgDn/`n` opens older entries, PgUp/`p` returns to newer ones,
Home opens the latest page, and `r` reloads current document information.

Select a checkpoint and press Enter to review it. Press Enter again to restore,
or Esc to return. Restoration creates a new history entry and preserves your
prompt and attachments. Referenced assets use their current contents; restoring
a checkpoint does not restore external files.

If the document or its dependencies changed since review, reload with `r` and
review again. Esc cancels an operation while it is being prepared. If saving has
already begun, the operation finishes and reports the saved result. See
[checkpoints](checkpoints.md) for CLI restoration and history storage.

## Refresh and draft recovery

When files or settings change, the preview reloads while preserving drafts and
attachments. If an update fails, the last completed frame stays visible with a
**Showing previous preview** notice. Repair the reported problem and refresh.
A stale preview cannot be exported as the current design.

The draft is saved after a one-second editing pause and on normal exit. A forced
process stop can lose edits from that last interval. Submitted requests are kept
separately from the next draft so failure or cancellation can preserve newer input.

For workspace development, see [architecture](design.md) and
[focused checks](development.md#focused-development-checks).

### Agent permissions

Open `/permissions` (`/mode`) or **Agent permissions…** in the palette. Up/Down
selects, Enter continues, PgUp/PgDn scrolls explanations, Backspace goes back, and
Esc returns without approving a tool request.

Choose a temporary mode or save a project/global default. Changes during a turn
can be scheduled for after it finishes; `c` cancels an unsent change. Press `p` on
an outstanding approval to open mode settings, then return to answer that original
approval. See [permission modes](agents.md#change-permission-modes-in-the-tui)
for confirmation and saving behavior.
