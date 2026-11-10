# Changelog

## 0.2.5

- Restore conversations at the latest saved response without replaying native history, hide routine completion markers, and keep activity indicators aligned with conversation status.
- Unify workspace dialogs with the command palette, improve file search and saved conversation formatting, and preserve the prompt while reviewing permissions.
- Keep native gesture warnings and agent stderr off the terminal workspace.
- Update rmcp to address the RustSec advisory and align terminal regressions with modal layouts, wrapped text and durable completion state.

## 0.2.4

- Limit slash commands to conversation management, queued prompts, attachments and scoped help. Add filtered command-name completion with keyboard selection while keeping all workspace actions in the command palette.
- Restore each document's previous TUI conversation, draft, images and paged history automatically. Add `--new-conversation`, keep concurrent TUIs on separate conversation leases, and leave queued or interrupted requests unsent until explicit submission.
- Validate boolean document properties before editing or resolving variables, including nested paints, effects and component overrides. Reject invalid values without changing the document or history.
- Bound pending MCP requests by count and input bytes across each document and connection, release reservations on cancellation, and keep separate capacity for closing documents under load.
- Keep document metadata overviews bounded and expose complete paginated roots, diagnostics, dependencies, image attachments and font diagnostics with revision-bound cursors and exact totals.
- Wait for persisted frontend completion state in terminal integration tests so asynchronous transcript persistence cannot produce premature assertions.

## 0.2.3

- Keep ordinary CLI/MCP inspection free of HTTP image requests. Explicitly acquire images with `inspect --acquire-images` or `get_document` with `acquire_images: true`; layout/rendering and image copy/import can also acquire them. Record URL declarations separately from acquired byte hashes and advertise network-capable MCP tools accurately.
- Add `runtime.browser.image_origins` to restrict initial image requests and redirects to exact HTTP(S) origins. Project settings can only narrow a user allowlist; an empty list denies HTTP images.
- Run MCP document work on independent native owners with bounded admission and two concurrent operations per connection. Move copy/import network preparation outside the save lock and revalidate inputs during publication.
- Share a 256 MiB retained-payload accounting budget across MCP calculations, failure captures, listings and dynamic caches, and limit persistent runtimes to four document owners. Add `close_document` to cancel work and release resources; reap running and waiting owners promptly on connection EOF. The payload budget is not a process RSS limit.
- Persist conversation transcripts off the terminal input path with a bounded queue, batched synchronization and durability barriers before prompt delivery, permission approval and successful completion. Preserve drafts and surface storage failures.
- Batch independent property updates while preserving comments and unrelated source, and serialize only the requested layout page.
- Add the preview node-name visibility setting and view-menu action.
- Use one CI contract assessment for enforcement and reporting. Retain selection reasons and every job's expected/actual status, including intentional skips and failed gates, in Actions summaries and JSON artifacts.

## 0.2.2

- Format mid-turn user messages consistently with ordinary prompts and separate streamed replies from the injected text.
- Keep selection outlines visible across camera updates, preserve native antialiasing when cropping previews to exact viewport dimensions, and retain precise trackpad scrolling and macOS pinch ownership.
- Support image drafts and explicitly queued image follow-ups during active turns; distinguish clipboard image validation errors and allow static images up to 32 megapixels within configured budgets.
- Add reviewed ACP permission mode controls, contextual command palette details, and Emacs editing in node and preview searches.
- Keep footer height stable when notifications change and use a consistent lowercase fast-mode label.

- Bound shared component cycle checks and honor cancellation without recursive stack growth.
- Escape control characters in human-readable errors while preserving structured messages.
- Keep large diagnostic collections out of small layout and PNG responses; expose totals and bounded excerpts alongside paginated validation.
- Prevent project settings from relaxing a user offline policy, including image acquisition during inspection.
- Index checkpoint history for direct replay and bounded pages, share consecutive immutable snapshots, and add reviewed `history --prune --keep` retention with durable expired-request reservations and resumable cleanup.
- Use personal Cargo configuration or `CARGO_TARGET_DIR` for machine-specific build caches.

## 0.2.1

- Read and edit `.pen` 2.22 documents, use 2.22 for new documents, and accept `@schema 2.22` scripts. Preserve existing 2.19/2.21 document and user-library versions, including mixed-version imports and checkpoint restoration.

- Preserve an independently scrolled node list during preview refreshes so in-progress clicks keep their target even without a selected node.

- Distinguish keyboard focus with stronger border colors in light and monochrome UI themes and remove the duplicated agent name above conversation history.

- Group TUI workspace actions by purpose in a shared command palette, slash-command and help catalog. Add palette input dialogs for images, queued prompts and playback while preserving the prompt draft. Use `/checkpoints` for editing history; `/history` remains an alias.

- Add selectable auto, dark, light and monochrome UI themes, with automatic light-terminal colors, user configuration, a launch override and in-session switching.

- Match preview canvases to the active UI theme across image protocols, including theme changes, avoiding black canvases in terminals that do not composite transparent image pixels correctly.

## 0.2.0

- Match the Preview canvas to the terminal background, retaining terminal transparency with Kitty and querying the background color for Sixel and halfblocks.

- Select shapes directly in Preview with clicks and drag marquees, with view-only selection outlines and multi-selection fitting. Add terminal-independent native macOS pinch zoom with explicit Accessibility authorization.

- Require macOS Tahoe 26 or later on Apple Silicon for new releases. Add a signed, notarized release workflow that updates the dedicated lazypencil Homebrew tap.
- Build and verify on macOS 26, and let bound MCP processes use the system allocator instead of the macOS 15 memory overrides.
- Authenticate notarization with an Apple Account app-specific password instead of an App Store Connect API key.
- Keep typing responsive after queued trackpad scrolling on fallback terminal image protocols by combining adjacent identical scroll events without losing steps or input boundaries.

- Keep local TUI operation errors in the bottom notification row instead of adding them to the conversation.

- Preserve the default TUI borders when pointer input is enabled; view buttons are opt-in. Opening the view menu keeps uncovered native Kitty preview pixels visible.

- Navigate the TUI with mouse/trackpad scrolling, pointer-centered zoom, Space/middle/right drag panning, clickable lists and view controls. Playback now uses `p`; user-only pointer settings control capture and wheel behavior.

- Submit TUI prompts with Enter, Ctrl+Enter, or Ctrl+S by default; use Alt+Enter or Shift+Enter for a newline. The input stays editable during a turn and queues text follow-ups.
- Attach explicit node/diagnostic references to prompts. Browsing or fitting a node alone no longer selects an implicit target for the next request.
- Configure panel widths, collapse Nodes/Agent, and use contextual footer actions and persistent preview-error diagnostics.
- Prepare document sessions in the background and display the connected adapter's actual model, effort, Fast mode, and account report without sending a prompt.
- Wait for MCP queue capacity with cancellation instead of returning `BUSY` immediately; calculation IDs remain local to their document and connection.
- Open a recursive, bounded document picker with `lazypencil` or `lazypencil tui`; retain direct opening through `tui FILE`.
- Register MCP once per client and select multiple documents with `list_documents`, `open_document`, and `create_document`. Document tools now require `document_id`; `mcp --file` has been removed. Calculation and pagination handles remain isolated by document and connection.

- Accept legacy vertical text alignment `center` as an alias of `middle` in layout and native/backdrop rendering, preserving the stored value.
- Load large macOS system emoji fonts within a separate bounded capture budget, defer system fallbacks until needed, and reuse captures when requested styles select the same installed face.
- Accept `normal` and `bold` text font weights, including string variables, without rewriting document values.
- Prevent repeated preview failures from flooding Conversation or repainting the terminal; show the actual diagnostic in Preview and list supported versions for unsupported documents.
- Use `$XDG_CONFIG_HOME/lazypencil/config.toml` for user configuration on macOS and Linux, falling back to `~/.config/lazypencil/config.toml`. The former macOS configuration location is no longer read or automatically migrated. `LAZYPENCIL_CONFIG_DIR` remains the highest-priority override.

## 0.1.0

Initial release for macOS 15.7.9 or later on Apple Silicon.

- Read and edit `.pen` 2.19 and 2.21 documents through the CLI, MCP server and TUI.
- Preserve comments, unknown fields and unrelated document content during edits.
- Render with Skia CPU and SkParagraph, using bundled Noto Sans JP and optional user fonts.
- Execute JavaScript-generated designs, capture browser nodes and render GLSL fills/effects with a separately installed Chromium runtime.
- Use Codex and Claude subscriptions through ACP adapters, with image attachments and shared design guidance.
- Browse previews with Kitty or Sixel, use Japanese input and Emacs-style prompt editing, and paste images on macOS.
- Generate Bash, Zsh and Fish completion hooks from the installed CLI.
- Include original dependency notices, matching project source and build provenance with binary distributions.

Known compatibility limitations:

- Pencil-specific browser device presets are unsupported. Omitted `deviceId` and basic `desktop`/`phone`/`tablet` modes are supported; they do not select a specific device model or User-Agent.
- Shader fills cannot sample `pencil:object`. Object sampling in shader effects is supported.

Unsupported settings are preserved in the document and produce explicit rendering errors. The release does not claim complete Pencil compatibility. Linux, Windows and Intel Mac are not initial release targets.
