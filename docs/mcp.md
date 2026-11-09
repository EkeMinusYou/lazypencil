# MCP stdio server

An optional project [font lockfile](fonts.md#reproducibility-lockfile) is enforced by the common font loader, including newly selected deferred faces. `get_document.configuration.font_lock` reports its presence and hash; its captured bytes or absence participate in dependency revisions. `list_fonts`, layout and rendering report `FONT_LOCK_INVALID` or `FONT_LOCK_MISMATCH` instead of silently updating a baseline. Structural edits remain available. Baseline publication is an explicit `fonts lock FILE` CLI operation, with revision-checked updates.

Start one file-independent stdio server per client connection:

```sh
lazypencil mcp
```

Register this command once in Codex or Claude Code's user configuration; it does
not need a separate entry for each file or project. See the [README](../README.md#mcp-and-ai-prompts)
for registration commands. `--file` is no longer accepted.

Call `list_documents` with a `directory`, or open a known absolute path directly:

```json
{"path":"/absolute/path/design.pen"}
```

`open_document` returns `document_id`, canonical `path`, `revision`, and `version`
without running scripts, loading fonts, or rendering. `create_document` accepts the
same input and creates an empty 2.22 document in an existing parent directory;
it never overwrites a file. Reopening the same canonical path returns the same ID.
Up to 64 documents can remain open on one connection. IDs are not persisted: reopen
paths after reconnecting. There is no implicit active document.

Every document-specific tool requires `document_id`, including project guidance,
styles, font inventory, assets, history, validation and rendering. For example:

```json
{"document_id":"ID_FROM_OPEN_DOCUMENT","depth":1}
```

Use this input with `read_nodes`. The examples later in this guide abbreviate the
common `document_id` field; always add it for the document being operated on.
`get_guidance`, `list_styles`, and `get_style` accept omission only for built-in
resources. `list_icons` is document-independent. Document results include their ID.
Snapshots and pagination cursors are opaque, scoped to both document and connection;
using one with another document is rejected even when their contents are identical.

`list_documents` defaults to recursive discovery under the server working directory.
Use `directory`, optional `recursive`, `query`, `limit` (1..500), and `cursor`.
Paths in `documents` are relative to the returned `directory`. Discovery does not
follow symlinks or obey `.gitignore`; it stops after 10,000 documents, 100,000 entries,
or five seconds. `complete`, `stop_reason`, `entries_scanned`, and
`unreadable_directories` distinguish a full scan from partial discovery. Query text
filters collected paths, so an empty partial result does not prove absence. Results
are sorted and pages reuse the same retained listing; repeat directory, recursive,
and query with the returned cursor. Four listings are retained for five minutes.
Restart without a cursor after `SNAPSHOT_UNAVAILABLE`.

Absolute input paths are preferred. Relative paths resolve against the server's
working directory, which is included in its initialization instructions. They are
not implicitly relative to whichever client project happens to be active.
Optional repeated `--root DIRECTORY` arguments restrict discovery and document
selection to canonical paths within those directories, including symlink checks.
Without roots, explicitly selected `.pen` paths can belong to different projects.
The server does not expose arbitrary non-document file reads.

Optional repeated `--font /path/to/font.ttf` arguments select additional fonts.
`--data-dir` or `LAZYPENCIL_DATA_DIR` selects history and data storage. Locks use a
stable per-user cache independent of this choice. All cooperating processes must
use the same absolute `LAZYPENCIL_LOCK_DIR` if it is explicitly overridden.

Each opened document independently selects its nearest `lazypencil.toml`. Font
settings, captured source bytes, and discovered files remain dependencies.
`get_document.configuration` exposes them. Project design and executable trust
remain explicit local user actions; opening a document does not grant trust.

`get_document` defaults to a bounded overview: document identity, revisions, previews
of roots/dependencies/diagnostics, guidance/trust metadata, primary capability flags
and configuration paths/font status. Each collection preview contains at most 100
entries and 64 KiB of serialized data. `collections` reports exact `total`, `offset`,
`returned` and `truncated` values; an empty preview can still have entries too large
for the preview budget. Oversized optional object fields appear in `omitted_fields`
instead of silently losing their values.

Read full entries with `get_document` and `collection`: `roots`, `diagnostics`,
`dependencies`, `runtime_dependencies`, `asset_diagnostics`, `attachments` (under
`attachments.images`) or `font_diagnostics` (under `configuration.font_diagnostics`).
Only collections present in the overview's `collections` are available for paging.
Set `limit` to 1–500 (default 100), then repeat the same collection with the returned
`next_cursor` as `cursor`. The selected array contains that page; the other arrays
remain overview previews. Pages have a 3 MiB collection byte budget and can end
before `limit`. Entries are never shortened. A single entry exceeding that budget
requires CLI inspection. Cursors belong to their document/connection, collection and
document/dependency revisions; changed inputs require restarting without a cursor.

Pass `include_configuration: true` for the effective configuration, origins and user
settings, or `include_capabilities: true` for the detailed feature catalogue.
Collection previews remain bounded with either flag. A full configuration that
exceeds the overall response budget requires CLI inspection. These flags do not
change dependency tracking, trust, image acquisition or execution. CLI inspection
retains its full metadata.

Unrecognized node properties produce `UNKNOWN_PROPERTY` warnings in edit results,
document metadata, and structural/layout validation. Warnings suggest a nearby
known property name when possible. Unknown fields remain stored without changing
their spelling or values; arbitrary keys inside metadata and script inputs are not
treated as node properties. Review warnings before claiming a requested edit worked.

Known boolean attributes reject strings such as `"false"` instead of silently using
their default. Variable-capable booleans also check the resolved value's type;
instance overrides and paint/effect `enabled` flags follow the same rule. Metadata
and script input maps retain their own open-key semantics.

Owned CLI/TUI conversations use the same open-document API; private attachment and
usage grants continue to restrict their connection to the host's selected document.
Their server name is `lazypencil_bound_<UUID>`, separate from a user's configured
`lazypencil` server. Always use the tools advertised by the bound connection.
External MCP connections cannot discover or operate a running TUI.

The implementation uses the [official Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) at version 3.6.0. The currently advertised protocol is `2025-11-25`; other protocol revisions and broader real-agent workflows still require interoperability testing. Codex and Claude subscription workflows have been checked separately; see [agent integration](agents.md). MCP messages use stdout; startup errors use stderr. Closing stdin ends the server.

Document work is serialized through a bounded queue. Requests that exceed execution
capacity wait within the pending-input limits described below; cancelling a waiting
request prevents it from being enqueued. Cancellation also remains available after
admission. Keep edits sequential and reuse one retained calculation for parallel reads
instead of launching independent calculations for every node.

When the CLI/TUI conversation host owns the server, it also binds design-resource metadata to the current prompt turn. Successful guide/style/catalogue queries are journaled before their results are returned, and the host appends those records to its transcript at turn end. Tool responses keep their existing shape. Ordinary external MCP connections have no host-owned usage journal. This records served resources rather than proving model comprehension; see [the record format and limits](agents.md#design-resource-records).

## Current tools

| Tool | Behavior |
| --- | --- |
| `list_documents` | Bounded, paged recursive discovery with explicit completion status |
| `open_document`, `create_document` | Select or create a document and return its connection-local ID |
| `get_document` | Document metadata, revisions, dependencies, diagnostics, capabilities, and the guidance entry point |
| `get_guidance` | Built-in English guides, trusted project guides, automatic project rules, versions, hashes, and an index |
| `list_icons` | Pinned vector icon names, aliases, tags, versions, hashes, licenses, and pack availability |
| `list_styles`, `get_style` | Built-in and trusted project styles, tokens, guide references, font requirements, and editable `.pen` examples |
| `read_nodes` | Stored, expanded, or explicitly calculated nodes, generated properties and provenance, inherited addresses, declared libraries, and bounded pages |
| `list_components` | Reusable definitions in the document and declared libraries |
| `get_variables` | Paged variable definitions, resolved values, source documents, node/property context, and hypothetical themes |
| `apply_operations` | The shared atomic editing API with expected revisions and request-ID deduplication |
| `validate_document` | Structural checks or paged measured-layout diagnostics, with optional retained calculation reuse |
| `get_history`, `restore_checkpoint` | Committed edits and a dependency-checked restore as a new edit |
| `read_asset` | Captured JS or GLSL text by hash, UTF-8 byte chunks, script header metadata, and runtime sources from a current retained snapshot |
| `put_asset` | Immutable managed PNG, JS, or GLSL files; images are checked and normalized before storage |
| `get_layout` | Calculated positions, transforms, text metrics, overflow, and a retained snapshot ID |
| `render` | PNG image content and matching snapshot metadata |
| `get_preview_state` (TUI only) | Requested camera, completed preview state and selected node |
| `set_preview` (TUI only) | Fit nodes, a region or the document; center, pan and zoom without editing nodes |
| `list_fonts` | Each bundled/configured face, collection index, source/face hashes, styles, axes, and default fallback chain |

`list_fonts`, `get_layout`, `render`, and calculated node reads require the rendering feature. The public input schemas are generated from the same Rust types used to parse tool arguments; unknown argument fields are rejected. Every tool also advertises an object `outputSchema` generated from a Rust success contract. Contracts describe stable fields and allow additional fields for detailed capabilities, node properties and future extensions. Real-stdio tests deserialize successful results against these same contracts. Render metadata follows its contract; PNG content is returned in a separate image block. Domain errors appear as `isError: true` with a structured error code and readable text, outside the success schema.

The [catalogue](icons.md) supports bundled Lucide and optional Feather, Phosphor and Material Symbols packs. Host-owned conversations register document-bound attachments for `get_document` and `put_asset`; external MCP clients upload typed image data without that grant. Rendering and copy limits are described in [dynamic content](dynamic-content.md).

## TUI preview control

A server launched by a live TUI additionally advertises `get_preview_state` and `set_preview`. Standalone MCP clients and CLI conversations do not advertise these tools. The TUI supplies a private loopback endpoint bound to its document and conversation. The endpoint is valid only for that TUI invocation. Single-node focus is part of `set_preview`; there is no separate `focus_preview` tool.

`get_preview_state` takes `{}` and returns the selected node and `preview` state. Keyboard focus, dialogs and agent activity are not part of this response. `preview.requested_view` includes the requested center, zoom, time and document-space region. `preview.displayed` describes the last complete document frame, including its view, revision, dependency revision, `preview_snapshot_id` and document bounds; it is null before a complete frame or while showing an attachment. The preview ID identifies the TUI's displayed calculation; it is not a reusable MCP `snapshot_id`. This replaces the previously ambiguous `preview.displayed.snapshot_id` field. Obtain reusable IDs through a calculation on the current MCP connection. The state also includes viewport cells/pixels, pending camera changes, playback and preview errors. A displayed revision may lag behind an edit: it identifies the actual completed frame. The query does not expose the prompt draft or change selection, playback or the camera.

For named or described targets, first use the existing `read_nodes` search with `calculated: true`, `depth: 0`, and optional `name`/`type` filters. Inspect the matching IDs, addresses and layout. Resolve ambiguity instead of guessing an ID or silently selecting one match. Use `get_preview_state` when interpreting relative requests or references to the current selection/view. Then call `set_preview`, for example:

```json
{"target":{"kind":"nodes","node_ids":["login","register"]}}
```

| Argument | Behavior |
| --- | --- |
| `target: {"kind":"document"}` | Center and fit the entire document |
| `target: {"kind":"nodes","node_ids":[...]}` | Center and fit the union of 1..64 distinct exact calculated node IDs |
| `target: {"kind":"region","region":{"x":0,"y":0,"width":400,"height":300}}` | Center and fit a finite, positive document-space rectangle |
| `target: {"kind":"point","center":[100,200]}` | Center at document coordinates while retaining the current scale |
| `fit: false` | Center a node/region/document target at the current scale instead of fitting it |
| `zoom` | Set an absolute scale from 0.01 through 16; overrides fitting |
| `zoom_factor` | Multiply the current or fitted scale; the result is clamped to 0.01..16 |
| `pan: [dx,dy]` | Shift the camera in document units, positive right/down; applied last |
| `expected_revision`, `expected_dependency_revision` | Reject stale target geometry |

Omit `target` to adjust the current view. `zoom` and `zoom_factor` cannot be combined. Coordinates and factors must be finite, and factors positive. Fitting defaults to true for node, region and document targets. Thus `{"zoom_factor":0.8}` zooms out in place; `{"target":{"kind":"nodes","node_ids":["heading"]},"fit":false}` centers the heading without changing scale. Inherited and generated nodes use their calculated IDs, not source-definition IDs. Finding multiple nodes and fitting their union changes only the camera; it does not rearrange the design.

The host refreshes the layout before resolving the target and acknowledges a complete rendered frame with `status: "updated"`, `node_ids`, target `bounds` (or null), revisions, `center`, `zoom` and `view`. The operation pauses dynamic playback but preserves time, keyboard focus and editing selection, and does not modify the document. It is a non-destructive display-side effect; relative operations mean the tool is not advertised as read-only or idempotent.

The connected conversation instructs the agent to search for and display an identified target before a targeted edit, then refocus after a geometry change. This depends on the agent following those instructions. These tools do not replace `render` for visual inspection. Unknown or empty-bounds nodes return `INVALID_ARGUMENT`; stale revisions return `REVISION_CONFLICT`; dialogs or attachment viewing return `BUSY`. Cancellation, a newer manual view, disconnection, and the 30-second deadline terminate pending mutations without claiming display success. Camera actions are not replayed when reopening saved conversations.

## Editing and inspecting

The bound project's `[design]` settings add guides and styles under `project:ID`. `get_guidance` without an ID returns the English workflow plus automatic `project_guides`, each with its own language/version/hash; reference-only guides remain available by ID. `list_styles` merges approved project styles with the three built-in styles, and `get_style` returns the actual read-only example and source hashes. With no project design settings, built-in catalogue responses retain their previous shape. Contextual responses also identify the current document/dependency revisions.

Project instructions require an explicit local user action: review `config show FILE`, then run `config trust FILE` (optionally pinning the displayed fingerprint). MCP cannot grant trust. `PROJECT_NOT_TRUSTED` blocks unapproved instructions, and agents must not grant trust on the user's behalf. `get_document.guidance.project_design` reports availability and trust metadata without exposing guide bodies. Instruction changes invalidate their grant. Example data and font-only changes remain revision-checked without revoking unchanged instructions. See [project configuration and trust](configuration.md#explicit-design-trust).

`import_component` accepts a project `style_id` through the same atomic link/copy operation as built-in styles. Its example's requested URI and physical asset origin are preserved; normal dependency closure, source revisions, immutable asset publication, and copy limitations apply. A configured default style is a reference for new designs and does not change a document merely by selection.

Start with `get_document` and `get_guidance`. Read relevant nodes and definitions before editing. Use `list_styles` and `get_style` for a suitable starting point when creating a design; preserve existing conventions during a local edit. English guidance does not change the user's requested response or design language.

An edit request has this shape:

```json
{
  "request_id": "change-title-1",
  "expected_revision": "REVISION_FROM_GET_DOCUMENT",
  "expected_dependency_revision": "DEPENDENCY_REVISION_FROM_GET_DOCUMENT",
  "operations": [
    {"op": "update", "target": "heading", "set": {"content": "My project"}}
  ]
}
```

Omit the dependency revision only when the document has no external dependencies. Supply a new request ID for a different edit, and reuse the original ID and payload to retry a request whose result was lost. `dry_run: true` runs the batch validation without saving. The `operations` guide contains a runnable example using server-generated IDs and a component reference in the same batch.

Pass the returned revision and dependency revision to `get_layout`, inspect its nodes and diagnostics, then pass its `snapshot_id` to `render`. The result includes an MCP `image` content block containing Base64 PNG data with `mimeType: image/png`; the image is not merely a local path. `node_id`, `region`, and `scale` can limit the preview. Structural validation alone does not establish a successful visual check.

Font collection files are captured dependencies. `list_fonts` enumerates every supplied face, and `get_layout` text records report the faces actually used by shaped glyph runs, including fallback, selected axes, and synthetic styling. Source `hash` and original `face_index` identify the input; `face_hash` identifies normalized standalone data. Changing the original collection invalidates the snapshot even when its selected face would normalize to the same bytes. See [fonts](fonts.md) for selection, limits, and remaining configuration/platform work.

`node_id` selects its visual bounds as a crop of the composited document. Node and region previews retain surrounding and overlapping scene context; they are not isolated transparent exports of just one node. Native background blur uses that context without Chromium. Parent opacity, transparency, and effect order are preserved through separate replacement coverage, and filter inputs can extend outside the requested output rectangle. See the `effects` guide and [graphics implementation](graphics.md) for the current contract and resource limits.

`get_document.capabilities.graphics.blend_modes` lists the exact eighteen supported native color-mode names. Outer shadows can blend against the actual prior scene, and inner shadows retain the object's alpha. Ordinary shadows, including these modes, use native CPU rendering. `linearBurn` and `linearDodge` mix colors before source-over alpha composition; they do not add alpha. Mode variables follow the same theme and property-source rules as other resolved properties. `graphics.mesh_gradient` advertises native row-major Coons mesh paints, with normalized point arrays or point objects and relative handles. Mesh fills and strokes share ordinary masks, themes, opacity, blending, and effects. See [mesh behavior, defaults, and limits](graphics.md#mesh-gradients). Complete Pencil graphics parity is not guaranteed.

## Variables and themes

`get_variables` returns raw `variables` and `themes` separately from `resolved_values`, `sources`, and `effective_theme`. An alias remains an alias in its raw definition. Each successful value reports its definition document and terminal value document; string-valued image paths remain strings and are not opened or rewritten by this query. A resolution failure omits that value and adds a named error to `diagnostics`, while retaining the raw definition. `resolution_complete` describes only the returned page. A project-wide `resolution_error` or incomplete asset status remains visible and is not repaired by a successful value query.

With no selector, list variables defined in the selected document. `library_alias` selects a declared library, including nested aliases such as `ui:base`; it is not a file path. `name` selects one exact unprefixed name or qualified reference such as `ui:spacing`. Resolution uses the same alias, type, conditional-value order, and namespace rules as rendering. Listing one scope does not flatten all imported definitions into the consumer's namespace.

`node_id` and optional `instance_path` select a stored or inherited node in that library or the selected document. Its effective theme includes actual ancestors and instance overrides. The default variable scope then becomes that node's source document. Add `property` to use the source namespace of one existing top-level property, such as `fill` or `content`; this distinguishes an instance's consumer override from the original library definition. The response's `scope_document` identifies the chosen namespace. These addresses are stored/inherited addresses, not runtime-generated script IDs.

For example, `{"node_id":"instance","instance_path":["label"],"property":"fill","name":"ink"}` queries the ink value as used by that property. `theme: {"mode":"dark"}` applies a hypothetical override after deriving that context. Each explicit axis/value must be declared in a loaded project document. The query never changes the saved theme, variable definitions, or a retained render calculation.

Optional expected document/dependency revisions reject stale reads. `limit` is 1..500 (default 100). Names are listed in sorted order, and `next_cursor` binds the page to the document, dependencies, context, and query; page size may change. Unknown fields, invalid node paths, undeclared libraries, and invalid explicit theme choices are rejected. CLI uses the same service:

Queries allow at most 128 inherited path elements and 128 explicit theme overrides. Names, selectors, path elements, and each theme axis/value are limited to 4,096 UTF-8 bytes; path elements must be nonempty IDs without `/`. Optional read preconditions can be added or omitted between pages when they still match the bound result; they are not part of the semantic query.

```sh
lazypencil variables design.pen --name ui:spacing
lazypencil variables design.pen --node instance --instance-path label --property fill --name ink --theme mode=dark
```

## Validation scopes

`validate_document` defaults to `scope: "structure"` and checks document structure, references, variable definitions, and themes without running scripts. `scope: "layout"` runs the normal script/layout pipeline and adds measured text, sizing, overflow, font, and icon diagnostics. It requires the rendering feature. Layout errors such as unresolved dimensions or script execution failures remain tool errors. A completed calculation containing diagnostic errors returns `valid: false`; warnings alone keep `valid: true`. `error_count`, `warning_count`, and `total_diagnostics` cover the whole calculation, including diagnostics beyond the returned page.

Diagnostics are queryable document state; they require neither a TUI connection nor an attached prompt reference. The common workflow returned by `get_guidance` and the `verification` guide describe inspecting issues within the user's requested scope, following relevant pages, and validating the updated document with a fresh calculation after repairs. The same guidance is included in CLI and TUI agent prompts. There is no request-phrase trigger or separate automatic repair routine.

`complete` means the requested checks completed; `validated_scopes` states which ones. Neither field establishes paint compatibility or visual quality. Layout validation does not launch browser/shader capture to check their pixels, decode every paint, or perform a rendered visual review. Unknown node types retain the existing `UNSUPPORTED_FEATURE` result rather than being silently certified.

Optional expected revisions reject stale input. A fresh read can omit them and returns the actual revisions. Layout validation retains its `snapshot_id`, runtime dependency revision, script execution hashes, and calculation metadata. Pass that ID and the returned revisions to `get_layout` or `render` to use the same calculation. Supply it again to validate retained measurements without rerunning scripts. A changed dependency or document rejects reuse; expired IDs return `SNAPSHOT_UNAVAILABLE`. A structural request cannot include a calculation ID.

Retained IDs belong to the connection that produced them and may be evicted or expire. A resumed conversation or TUI preview does not retain that connection's cache. After `SNAPSHOT_UNAVAILABLE`, request one fresh calculation with both `snapshot_id` and `cursor` omitted, then restart reads or pages with the returned ID. Repeatedly retrying an unavailable ID cannot recover it; the server does not silently substitute a different calculation.

Diagnostics use `limit` 1..500 (default 100) and `next_cursor`; the cursor binds to the requested scope and calculation, and a new page size is allowed. For layout pages, reuse the returned `snapshot_id`. Responses retain the shared 4 MiB budget. Cancellation reaches code execution and subsequent verification without poisoning retained calculations.

Layout and PNG responses include a diagnostic excerpt of at most 100 entries and
64 KiB of diagnostic data, plus document-wide `total_diagnostics`, `error_count`,
`warning_count`, and `diagnostics_truncated`. Node selection and page size do not
change these totals. Use layout validation with the returned `snapshot_id` and
its diagnostic cursor to read every diagnostic; a truncated excerpt is not a
complete validation report. CLI `layout` and `render` use the same excerpt limits;
`validate --scope layout` provides the complete paginated diagnostics.


```sh
lazypencil validate design.pen
lazypencil validate design.pen --scope layout --limit 100
```

CLI validation uses the same service and optional `--font`, expected revision, limit, and cursor arguments. Separate CLI invocations recompute layout and scripts; they do not retain MCP calculations. Diagnostic errors produce exit code 4 with the report under structured error details; warnings return success. Structural validation and variable queries remain available without the rendering feature.

## Components and inherited nodes

To read an inherited label, use `read_nodes` with `{"node_id":"instance","instance_path":["nested-instance","label"],"depth":0}`. The result has `view: "expanded"`; each node includes its `address`, `properties`, property origins, and source node. Variable references remain references in this view. `expand_instances: true` expands a selected root or searches expanded nodes. A `library_alias` selects a declared library; nested aliases follow the declared import chain, separated by `:`. These API addresses do not introduce a new serialized `.pen` reference syntax. Stored-node reads remain available when a dependency is missing and report asset diagnostics or a dependency error.

### Calculated and generated nodes

Use `read_nodes` with `{"calculated":true,"node_id":"chart","depth":2}` to explicitly run the script/layout pipeline and inspect the tree used by PNG output. To reuse a calculation already returned by `get_layout`, layout validation, `render`, or a calculated read, pass `snapshot_id` instead. That form verifies the captured document and dependency bytes and does not rerun scripts. Unknown or expired IDs return `SNAPSHOT_UNAVAILABLE`; changed captured inputs return `REVISION_CONFLICT`. IDs belong to their MCP connection. Calculated reads require the rendering feature; `get_document.capabilities.node_reads` advertises support.

The result has `view: "calculated"`, `variables_resolved: true`, calculation metadata, `total_matches`, and `next_cursor`. Each node includes its public `id`, `address`, resolved `properties`, `property_sources`, effective `theme`, `source_path`, `source_id`, `generated_by`, and the matching `layout` record. Generated properties come from the captured execution, including nested scripts and library-scoped variables. A depth of zero returns `children_count`; larger depths include nested `children`. Resolved properties still describe the node's declarative sizing rules; measured positions and sizes are in `layout`.

An inherited node can still be selected by `node_id` plus `instance_path`. A generated node can be selected by its opaque top-level `id`, as returned by this read or a layout record. Do not construct an ID from `properties.id` or save generated IDs as document edits. Generated nodes remain derived and read-only; follow `generated_by` to edit their owning script's inputs or source. A calculation covers the selected document's effective tree, including used library instances; `library_alias` is rejected. Use a stored/expanded query to inspect unused library definitions. `expand_instances` adds nothing to an already calculated tree.

Name/type filters and depth 0..16 apply to calculated reads. Limits are 1..500, default 100; matches are visited in layout order, and children included by depth can overlap other matches. Reuse both `snapshot_id` and `next_cursor` for later pages, keeping the other query fields unchanged. A different page size is allowed. Response construction is bounded to 4 MiB; reduce depth, page size, or selection after an oversized response. Cancellation reaches script execution and serialization; cancelling a read does not invalidate an otherwise current retained calculation.

```sh
lazypencil inspect design.pen --calculated --node chart --depth 2
```

Separate CLI invocations recompute scripts and layout. A cursor works only while that newly computed snapshot and query remain identical; CLI processes do not retain MCP calculation IDs. Omit `--calculated` (or both `calculated` and `snapshot_id` in MCP) to inspect stored code without execution, including code that cannot currently run. Browser and shader pixels still require `render`; calculated properties alone are not a visual review.

### Editing instances

An inherited `update` or `set_override` writes only that instance's override. `reset_override` restores inheritance by removing selected properties or the exact descendant entry. `replace_override` and `set_slot_children` allocate IDs and expose them through `client_ref` or `client_refs`; subsequent operations can use `result_of`. `set_theme` changes individual axes. Inherited nodes cannot be moved or deleted as if they were stored roots.

`detach_instance` handles stored roots and inherited instance targets, including declared external libraries. A stored root keeps its ID. An inherited target becomes a replacement in the owning instance's `descendants` map with a new stored ID. Its result includes `generated_ids[OPERATION_INDEX].address`; `client_ref` binds the new stored ID so a later operation can update it directly. Ordinary inherited descendants require the owning root address until they become stored replacements.

Detachment materializes one instance boundary, applies definition chains and ancestor overrides, removes folded deeper outer overrides, and keeps other nested references and variable references. Different source documents may use the same ID: temporary source-scoped identities prevent collisions before allocating stored IDs. References to definitions owned by the detached subtree follow the newly allocated definitions; other library references remain qualified and linked. Moved image/shader paths are relative to the selected document and retain their original file targets. Library variable references retain their namespace. Additional source-default theme axes become explicit only where needed to preserve the current resolved theme. Source libraries, assets, sibling instances, and variables are not rewritten or copied.

Before accepting each detach operation, the service resolves both trees and compares properties, effective themes, child structure, nested instance boundaries, canonical asset targets, and script execution/input origins. A scope-preservation mismatch fails the whole batch with `UNSUPPORTED_FEATURE`. This is structural verification without executing script, browser, or shader code. Shader image uniforms that use variables retain the document defining the selected terminal value, including aliases and theme conditions. The value is anchored to that document; active asset capture and rendering canonicalize the path and check project roots and dependencies. Inactive images are not opened. Stored variables and their text uses retain their original values and remain linked after detachment. No additional metadata is needed for that shader binding. This is the current lazypencil resolution policy; Pencil parity for imported variable-valued image samplers remains unverified. Independent copying of relative local image values preserves the same text/alias/theme graph using ordinary imported variable libraries, as described below; absolute local paths and conflicting projected paths use the managed sampler image bindings described below.

External script detachment uses the lazypencil extension `metadata.lazypencil.script_scope` version 1 to retain the original execution document and, when needed, the input document. The source URI remains linked to the original JS file. Generated component/variable/image references and nested scripts retain their original document scope. Default ref inputs use that execution document; explicit ref inputs retain the document that supplied them. Other metadata is preserved. The code is not rewritten or executed by the detach operation.

The binding's `library` is a declared alias relative to the document that supplied the metadata; an empty string means that document. `script_uri` identifies the bound source URI. Optional `inputs.library` and `inputs.hash` bind the stored input object; the hash is SHA-256 of compact JSON with all object keys recursively sorted. Changing the stored source URI or input value independently invalidates the corresponding binding. Explicit instance overrides use the overriding document even if their JSON values equal the definition. Invalid bindings and unknown versions are rejected. These are lazypencil semantics inside the format's metadata field, not a verified Pencil round-trip contract. Source libraries remain dependencies for this detach binding. Independent script copy composes captured script_scope, script_bindings, script_input_context and ref script_override contexts with separate input and output guards. Restoring inherited state requires its recorded provenance; missing legacy provenance remains a diagnostic.

`add_import` and `remove_import` edit the declared library map; unresolved references make the complete batch fail.

## Importing components

`import_component` is an operation inside `apply_operations`, with an explicit `mode` of `link` or `copy`. It accepts a normal `parent`, `index`, and `client_ref`. Its generated result contains the placed instance `id`, reusable `component_id`, and `source_revision`. Later operations can address the instance with `result_of`; subsequent requests can edit the copied reusable definition by its returned ID.

Choose one source shape:

```json
{"library_alias":"ui","node_id":"button"}
{"uri":"library/ui.lib.pen","import_alias":"ui","node_id":"button"}
{"style_id":"standard-ui","node_id":"button"}
```

Omit `library_alias` for a local reusable definition. File URIs resolve beside the selected document and must select a `.pen` inside allowed project roots. They cannot read arbitrary non-document files or remote URLs. A bundled style ID is selected from the built-in catalog; its example hash is the source document revision. Application-owned examples are encoded for the consuming document version; the returned `source_revision` and `expected_source_revision` still identify the original catalog example. User-owned libraries retain their declared version. Optional `expected_source_revision` rejects a changed source in addition to the ordinary request preconditions.

`link` retains the original definition, variables, and asset scopes. A file source registers its explicit alias; a style source publishes the complete pinned example as a managed `.lib.pen` and registers `style-STYLE_ID` by default. A style source can supply `import_alias` to choose another alias. Alias collisions are rejected. Linking cannot introduce a saved import cycle. A local link simply places another instance.

`copy` closes static references, variable aliases, themes, owned children, descendant replacements, slot children/suggestions, and direct image/shader file inputs. Source documents may use equal node IDs. Each stored node receives a new ID, references and descendant paths follow the corresponding copied definition, and the result's `id_map` uses qualified source keys. Each key segment escapes `%` as `%25` and literal `:` as `%3A`, so a literal source ID cannot collide with a library namespace. Qualified keys are limited to 4,096 bytes. Variables and themes have separate maps. Required definitions live under a disabled frame named `Imported components`; only the placed instance contributes visible artwork. Other nested instances remain references to copied definitions. Source libraries and assets are not changed. Imported image bytes remain exact source bytes; this is a structural file copy, not image normalization or proof that every graphic can render.

Local relative string sampler variables, including array elements, retain their root alias/theme graph on copy. Only literal image strings move into small ordinary `.pen` variable libraries beside captured image bytes. Text still resolves to the exact original string. Future edits to the copied aliases remain effective. Reused consumer variables keep the consumer file origin. All theme alternatives are captured; `../` and transit directories such as `a/../pixel.png` are preserved. Bundles live under `assets/lazypencil/HASH.bundle/`, with a hashed size/SHA-256 inventory, at most 4,096 members, 128 MiB payload and 1 MiB member paths. Path projection permits at most 64 upward traversals and 128 components per member. Nested directories are checked for links and physical path changes before atomic no-clobber publication, and requested paths are rechecked after publication. Dry runs stage the same files without creating directories. Absolute local image strings, local file URLs, HTTP(S) images and conflicting projections use managed image bindings while preserving the original strings. Embedded static-image data URLs carry their bytes in the original string and do not create separate image files. These paths use the same captured-source and dependency checks as other imported inputs.

For example:

```json
{"op":"import_component","source":{"library_alias":"ui","node_id":"button"},"mode":"copy","options":{"variable_prefix":"kit.","component_prefix":"Kit "},"client_ref":"primary"}
```

Name collisions fail by default. `options.variables`, `options.components`, and `options.themes` map a qualified source key to `{"action":"rename","name":"new-name"}` or `{"action":"reuse","target":"local-target"}`. For components, the reuse target is a local reusable node ID; for variables and themes it is the local name. Component display names only receive `component_prefix` when a source name exists. `variable_prefix` applies before collision checking. Local variables/themes can retain their own existing definitions when copied within the same document. Variable reuse checks the declared type; theme reuse requires all source values. A typo or unused per-source resolution is rejected. Changing a theme name also changes its conditions and explicit node selections, including unambiguous inherited axes.

A reusable definition contained in a copied parent can also be explicitly reused. Its position becomes a fresh, non-reusable local `ref` to the chosen target. The source's name, x/y, width/height, rotation/flips, enabled/opacity/blend mode, `layoutPosition` and `layoutIncludeStroke` remain placement overrides. The chosen target supplies its body, appearance, code, metadata and children; later edits to that target remain live. Other source definitions required by references are still copied, including definitions originally nested inside the replaced body. `id_map` reports the fresh placed node ID, while `reused_components` reports the selected definition ID.

Descendant overrides keep exact IDs when the target has the same paths. When IDs differ, `options.component_paths` maps a reused source key to explicit source-path / target-path pairs:

```json
{
  "components": {"ui:badge": {"action": "reuse", "target": "local-badge"}},
  "component_paths": {"ui:badge": {"source-label": "local-label"}}
}
```

Both paths in an explicitly supplied pair must resolve in their respective definitions. Invalid pairs, mappings for components that were not reused, and absent targets of stored overrides fail before publication. The copier translates instance boundaries when the selected definition adds or removes them. Stored `children` and full-node replacements retain their separately authored bodies and child IDs; those children are not mistaken for the substituted definition's original body. Overriding one of these new children uses the same translation in stored and generated refs. The source's parent-before-child application order is retained, and colliding destination paths fail instead of discarding an edit. Captured script component references and descendant paths use this shared mapping, without executing or rewriting JavaScript during copy. Source-only descendants removed by substitution have no generated binding; requesting one at runtime retains a missing-binding error. Runtime-authored children use the additional address contexts described below.

Independent script copy and string sampler bindings use the captured namespaces described above. Absolute local sampler paths, local `file://` string samplers, HTTP(S) image URLs and conflicting projections preserve the original strings and use managed image bindings. File URLs retain the ordinary canonical-path, allowed-root and dependency checks; URL credentials, remote authorities, queries and fragments are rejected. HTTP(S) acquisition follows the [image policy](dynamic-content.md#http-image-acquisition). Embedded PNG/JPEG/WebP data URLs retain their bytes in the original string, including copied script output, without creating separate managed image files. Captured script asset keys accept local file URLs for image/shader/script and HTTP(S) URLs for images. Repeating an already captured declaration uses its code owner's local asset; remote code is rejected. These binding capabilities are advertised separately from rendering. Explicit generated script contexts use the captured namespaces and require recorded provenance when restoring inherited state. Unknown node types cannot be structurally copied; code is not evaluated to guess missing dependencies or flatten a component. See [dynamic content](dynamic-content.md) for ownership and runtime limits.

Runtime-authored replacement children have an additional address translation. The generator's authored IDs are retained separately from internal generated IDs. Optional `descendant_contexts` in version-1 script bindings capture each source path's parent, source instance boundary and target instance boundary. This lets runtime overrides address freshly returned children, nested refs and full-node replacements after component reuse changes those boundaries. Recopy retains the original ancestry and composes the new target boundary. Replaced bodies cannot revive stale paths, and ambiguous child addresses fail explicitly. Contexts share the existing binding budget; runtime address indexes are bounded and cancellation-aware. Earlier version-1 bindings without contexts retain their declared-path behavior. The Mac gate verified seven combinations across both schemas with real workers before copy, after source removal and after recopy; all fourteen copied/recopied PNG pairs matched.

Imports stage immutable files in memory. The complete batch and captured source inputs are validated before publication; a dry run creates no files or history. Image/shader copies use `assets/lazypencil/CONTENT_HASH.EXT`, with no overwrite. Uploads and imports share this publisher. Its document directory and existing managed directories must retain their exact canonical paths, so a redirected parent or managed directory cannot redirect publication. During a save, source documents, assets, configured fonts, and configuration candidates participate in shared locking and final verification, including sources removed from the resulting document. Library/image symlinks are checked by their requested-to-canonical binding as well as bytes. The same bytes at a different target invalidate the read. Duplicate captured read sets are shared by the batch; captured source bytes and staged output bytes each have a 128 MiB batch limit, and each managed file has a 64 MiB limit. Node, document, and request limits also apply. Edit reports must fit the common 4 MiB response budget with room for transaction fields; oversized reports fail before file/document publication rather than committing a result that MCP cannot return.

A copy may read a library that already imports the consumer without adding a reverse edge to the saved import graph. A link that would create that cycle is rejected. A late conflict or interrupted save may leave complete unreferenced immutable files; it never publishes a partial document or deletes a file that another operation may use. Use the explicit [project asset maintenance](#project-asset-maintenance) review to collect eligible unreferenced files. Replaying a committed request returns its recorded IDs and URIs before attempting to reopen the original library.

## Checkpoints and retention

Checkpoint listings use byte-bounded pages and return `next_before_sequence` even
when a large result makes the page shorter than its requested entry limit. Follow
the cursor instead of inferring completion from the entry count. See
[checkpoints and retention](checkpoints.md) for explicit local history pruning,
expired-request reservations and interrupted-cleanup recovery. Pruning is not an
MCP tool.

## Managed assets

Create code with `{"kind":"script","source":{"text":"return [];"}}`. Images use `{"kind":"image","source":{"mime_type":"image/png","data":"BASE64_DATA"}}`; static PNG, JPEG, and WebP input is normalized to PNG, including orientation. Animated input, false MIME types, and oversized images are rejected. Code is limited to 1 MiB, source and normalized images to 20 MiB each, and decoded images to 32 megapixels. MCP's 4 MiB argument limit also applies to encoded data.

`put_asset` returns a relative URI and SHA-256 hash. Files live under `assets/lazypencil` beside the selected document. Identical content is reused, conflicting files are never overwritten, and asset creation does not itself edit the document or add a history entry. Link the returned URI with `apply_operations`. `read_asset` only reads captured script or shader dependencies after they have been referenced; it is not an arbitrary file reader. CLI users can use `assets put design.pen --kind image --input reference.jpg` and `assets read design.pen --hash HASH`.

Uploads and component-import publication acquire the destination's exclusive path lock in the same namespace as shared dependency locks. This includes a currently absent destination, so a cooperating publisher cannot make it appear between a saving document's final dependency check and commit. Different document/history data directories do not bypass this coordination. Publication rechecks directories and any existing bytes after obtaining the lock, including when it will reuse identical content. Lock waits retain the common five-second limit and return `LOCK_UNAVAILABLE` when it cannot be acquired. Asset publication takes no additional document lock; importing a component may already hold that document's writer lock.

CLI `assets put` accepts Ctrl+C and MCP `put_asset` accepts request cancellation while waiting for the destination, after input normalization, during staged output writes, and immediately before publishing its final name. The component-import path uses the same publisher and its editing token. Once a complete final file is published, verification and directory synchronization finish; an abandoned request can therefore leave a complete unreferenced file. CLI input reads are bounded regular-file reads, with a nonblocking open on Unix. This coordination does not prevent writes by external programs that ignore lazypencil's locks.

Stores, captured projects and CLI/MCP/TUI lifetimes hold a shared per-user managed-asset lease in the stable lock directory. A retained project keeps that lease after its Store closes, and CLI publication keeps it through stdout completion. Read-only `doctor` and `runtime status` have no managed project inputs and do not acquire a lease or create lock state. `Store::open_for_asset_maintenance` acquires the exclusive maintenance lease without waiting when another owner is active; ordinary document/dependency locks retain their existing behavior. `assets prune` uses this exclusive lease directly. New private checkpoint metadata records its canonical `document_path`; legacy records omit it, while a recorded foreign owner is rejected.

### Project asset maintenance

`lazypencil assets prune design.pen --project-root /absolute/project --json` reviews managed assets. Repeat the same command with `--expected-revision REVIEWED_SHA256` to apply. There is no automatic deletion or MCP deletion tool. Close other lazypencil frontends first; active owners return `LOCK_UNAVAILABLE` immediately. Keep the same per-user lock directory across applications and data directories.

Choose a root containing every local document that shares these asset directories, and the `--data-dir` containing the history to preserve. Collection scans `.pen` files outside managed payloads, stored strings in every theme/disabled node/unused definition/unknown metadata, imported documents, captured dependencies and browser resource roots. It checks before/after snapshots in every retained history namespace and transaction state in that data directory. New checkpoint owners and legacy owners matched to scanned document paths locate relative references. Unknown legacy owners, unresolved project inputs, symbolic links and dynamic script reference closures retain assets conservatively and appear in `uncertain`. Executing a script once cannot prove what assets its future branches will reference. Inbound references from outside the chosen project root or another user-data directory cannot be inferred; include their documents/history before reviewing. Managed payload documents become references through imports, rather than making every orphan bundle its own retention root.

Only complete content-addressed files with matching SHA-256 and bundles with a matching manifest and every declared member are candidates. Unlisted files, links, modified bytes and unknown names are retained. A bundle is collected as one unit. Review binds the root, data directory, document/source bytes, directory membership, captured dependency/configuration conditions, asset inventory and pending journals. Applying recomputes that revision and rechecks source conditions before each rename; changed inputs require another review.

Applying writes and synchronizes a private journal before a guarded, non-overwriting rename to quarantine. Parent directories are opened without following links; the moved payload is checked again. The journal remains outside the payload during incremental cleanup, including after a bundle manifest has been removed. Final cleanup moves the journal outside the empty quarantine directory before removing that directory, so either interruption point remains identifiable. Repeat review/apply to finish interrupted cleanup. If a quarantined asset becomes referenced, a complete unchanged payload is restored without overwriting a new destination; an incomplete or changed referenced payload is retained with `RECOVERY_REQUIRED`. `deleted`, `restored`, `cleanup_pending` and `errors` report individual results, including commits completed before cancellation; check these fields rather than treating `applied` as universal success. The operation is not a transaction across all assets.

Work has cooperative cancellation and a 30-second budget, 100000 entries, 64 project-directory levels, 256 captured project snapshots and a 512 MiB cumulative read budget. Reaching a bound fails explicitly instead of silently truncating the reference closure. Synchronous filesystem calls finish before the next check. These guards coordinate cooperating lazypencil processes, not arbitrary external writers that ignore locks.

## Repairing missing assets

Missing declared local images, script sources, shader sources, and shader image inputs do not prevent inspection and structural editing. `get_document`, `read_nodes`, and edit results report `assets_complete: false` and `asset_diagnostics`. Each diagnostic identifies the original URI, origin document, requested path, planned resolved path, and resource kind. Dependencies include absent resources with `availability: "missing"` and an absence revision. The same path used as both an image and code retains a dependency for each kind.

Missing declared `.pen` libraries also have absence revisions and diagnostics with `kind: "library"`, including nested imports. Inspection preserves the declaration and stored instances; no replacement definitions are invented. To rebind an alias, use `remove_import` followed by `add_import` with the replacement local `.pen` URI in one edit batch. Alternatively remove the affected instances and import together. The result must resolve its component and variable references before saving; an unresolved instance cannot be preserved by guessing the missing definition. Unused missing imports can be removed in separate batches.

Explicitly configured missing font files and font search directories are reported as `font` and `font_directory`, with the configuration file as their origin. Their absence revisions participate in normal document/dependency preconditions and structural edits. Effective configuration metadata also exposes `fonts_complete` and `font_diagnostics`; overwritten lower-priority lists are not probed. `list_fonts`, layout and rendering reject missing selected font inputs instead of silently using another face. Restore the declared input or explicitly revise its configuration, then obtain fresh revisions. Document editing does not rewrite user/project font settings or copy fonts automatically. A restored input or redirected parent invalidates the captured absence.

Use the ordinary document and dependency preconditions even when the files are absent. Upload a replacement with `put_asset` and update the node to its returned URI, or remove the affected node/property. Repairs can be saved in several batches: existing missing targets and kinds may remain, but an ordinary edit cannot introduce a different missing target or resource kind. `dry_run` follows the same rule without writing files or history. An explicit `restore_checkpoint` can restore historical missing references and reports the resulting diagnostics.

`assets_complete: true` means the declared asset scan found no missing files; it does not establish graphics compatibility, successful code execution, or visual quality. A resolution failure that prevents a complete scan produces `assets_complete: null` with `resolution_error`. A failed project load instead reports `dependency_error` without claiming an asset status. `validate_document`, `get_layout`, and `render` reject known missing assets with `ASSET_NOT_FOUND`, including the diagnostics; structural repair does not run missing code. Omit the optional runtime dependency precondition when repairing a document whose code cannot currently execute.

If a missing file appears or its parent is redirected after inspection, reload before editing; the old precondition is rejected with `REVISION_CONFLICT`. Creating ordinary parent directories alone retains the same absence revision. Existing and dangling symbolic links are tracked, and adding a new parent link or junction invalidates a captured missing path. This does not provide atomicity against uncooperative external filesystem writers. Publishing an immutable import file can itself fulfill an old absent reference; a resulting conflict leaves the original document unchanged and requires a reload, and the complete immutable file can be reused.

This repair path covers at most 1,024 unique requested-path/resource-kind pairs. Missing path probes allow at most 256 components and 40 dangling-link expansions, within the usual project and response budgets. Forbidden URLs, unreadable files, wrong file types, and unresolved parent traversal after an absent directory remain errors. Image/code/library paths still cannot escape project read roots; explicitly configured fonts retain their separate existing path policy. Runtime-generated resources still require successful code execution to discover. Resources selected only at runtime cannot be discovered without successful execution.

## Script execution

Create or read a script asset, then insert a `script` node with `scriptUri`, dimensions, and optional `inputs`. `read_asset` returns `script_header` metadata, or `script_header_error` while preserving access to the source for repairs. `get_guidance` with `id: "scripts"` explains supported inputs and the execution contract. Structural validation does not execute scripts.

`get_layout` evaluates scripts and returns execution hashes and generated geometry. Generated records have `generated_by`; their IDs cannot be edited as stored document nodes. Update the source node's inputs or link new immutable code. Render with the returned `snapshot_id` to use the same calculation. Runtime-discovered files are listed under `runtime_dependencies`, with their own `runtime_dependency_revision`. The ordinary dependency revision continues to describe document-declared dependencies. Both sets are checked when a retained snapshot is reused. Runtime sources can be read by hash while that snapshot remains current; a changed dependency reports a conflict.

For an edit based on that calculation, supply `expected_runtime_dependency_revision` alongside the two ordinary expected revisions. The service re-evaluates scripts without save locks, compares the newly captured runtime dependency hash, holds shared dependency locks during saving, and checks captured files again immediately before replacing the document. A mismatch fails with `REVISION_CONFLICT`, including for a dry run. The result records `validated_runtime_dependency_revision`, describing the pre-edit calculation. This is optional so that structural repairs do not require executing broken code. It checks captured local dependencies, not the freshness of an external web page. A committed request is replayed before any script execution; its original request ID and full payload, including runtime preconditions, must match. Builds without rendering reject runtime preconditions explicitly.

The interpreter accepts script-header schemas 2.11, 2.19, 2.21 and 2.22 (document codecs are 2.19/2.21/2.22), synchronous execution, six typed input kinds, existing component references, nested scripts, and fixed random reseeding. Its documented random-sequence contract uses the reviewed Mulberry32 algorithm and owned expected values. The implementation does not claim all-schema/Pencil parity. Defaults remain 2 seconds and 64 MiB per evaluation, 1,000 output nodes, a 3 second watchdog, 8 layout passes, 128 evaluations and a 10 second layout budget. User/project [JavaScript settings](configuration.md#javascript-execution-limits) configure evaluation/heap/idle/layout limits. `script_executions[].limits` records configured policy, included in calculation identity; changed configuration invalidates old snapshots. The earlier absolute layout deadline also bounds an active worker, rather than waiting until the next evaluation to notice expiry.

Script header, compilation, and JavaScript exception diagnostics include `error.details.script_diagnostic`. `stage` distinguishes `header`, `compile`, `runtime`, and internal `setup`. When available, `line` and `column` refer to the original source, including its header, with one-based UTF-8 byte columns. LF, CRLF, CR, and Unicode line/paragraph separators are counted consistently with the interpreter. Wrapper lines are excluded. `location_kind` is `source`, `declaration`, `call_site` for an outer call into dynamic code, `end_of_source` for an incomplete body, or `unavailable`. Errors without a usable reported position have null coordinates and an empty excerpt; a primitive throw does not acquire a fabricated location.

`source_excerpt` contains at most three original source lines, each windowed to 240 Unicode scalar values. Each line has `line`, `text`, one-based UTF-8 byte `start_column`, `truncated_start`, and `truncated_end`. Long failing lines are windowed around the reported column. Source positions are debugging hints from the parser or reported exception stack; JavaScript can replace its own stack. Use captured `source_hash`, `script_path`, and `node` to identify the source and owning address, and use normal revision checks for edits. Declared source/header failures and nested runtime failures retain this context. Input/output validation and parent watchdog failures can lack source positions. The readable CLI/TUI error also includes available locations and bounded excerpts. JSON clients should use the structured coordinates instead of parsing the raw engine stack in `message`.

A failed MCP calculation can retain the failing nested script's original bytes even though it produced no layout snapshot. When `error.details.failure_capture_id` is present, call `read_asset` with the diagnostic's `source_hash` on the same connection. Ordinary UTF-8 byte chunking, offsets, limits and `script_header` / `script_header_error` apply; reading does not execute code. The response includes `failed_calculation: true` and `failure_capture_id`, with the original document and declared dependency revisions. It has no `snapshot_id`; a failure capture cannot be reused for layout, rendered output or runtime edit preconditions. Source lookup considers successful and failed captures in creation order, newest first, so an older stale calculation cannot hide newer captured code.

At most four failed code files of up to 1 MiB each are retained for five minutes, with idle expiration. Fonts, scene data and other asset bytes from the failed project are released. Each capture additionally bounds dynamic dependency guards and path bindings to 4,096 entries each, with at most 1 MiB of stored path bytes. Oversized/unverifiable captures or errors without an identified runtime script do not receive a capture ID; their original error remains intact. Cancelled calculations do not publish a new failed-code capture. Declared sources remain readable through ordinary project dependencies. CLI/TUI errors still provide diagnostic context; these failed-code captures belong only to the MCP connection that attempted the calculation.

Reads compare the current document/declared dependencies and recheck captured runtime file hashes and requested-to-canonical path bindings. Changing a parent script or library, deleting a runtime source, or redirecting a link invalidates the capture, even if the failing file's own bytes stayed the same. A changed dependency reports `REVISION_CONFLICT`; changed root documents, expiry, eviction and unknown hashes leave the code unavailable (`ASSET_NOT_FOUND`). A caller-supplied path or a file merely adjacent to the project is never sufficient to authorize a read.

Generated icon packs participate in the same checks. Captures keep installed icon file hashes and the registry's requested-to-canonical bindings, including notices, manifests, data and optional font files, without retaining the pack payloads. Missing pack directories are absence guards: installing a pack encountered during the failed attempt invalidates that capture. These absence paths share the 1 MiB path budget and have a 4,096-entry bound. Changing installed bytes or redirecting a link to byte-identical files also invalidates the capture.

An icon integrity error or another registry error without a complete file/absence guard prevents publication of a new failed-code capture. Existing captures still require all their original checks; a damaged installation does not become a partially verified capture.

Cancelling an executing script stops its worker process. Real stdio tests cover script execution, runtime-precondition evaluation, and cancellation while waiting for an edit lock. The save boundary is described below.

## Browser and shader rendering

Run `lazypencil runtime setup chromium` explicitly before rendering browser nodes or GLSL. See [runtime installation](runtimes.md) for pinned artifacts and offline setup. Static documents and script geometry do not launch Chromium.

`render` accepts `time` in seconds (default zero) and `refresh` (default false), alongside `snapshot_id`, `node_id`, `region`, and `scale`. The result contains actual PNG image content and `dynamic_artifacts`: page acquisition IDs, timestamps, redacted URL metadata, browser version, viewport and image dimensions, shader request hashes, software renderer details, and PNG hashes. A layout snapshot identifies geometry and captured local files. A page acquisition identifies a time-dependent web image; refreshing it does not change the layout snapshot ID. Shader acquisition metadata includes `time_dependent`, derived from the parsed `@time` uniform declarations used by the common renderer. The TUI uses that flag to avoid running a timeline for a static preview.

The current page path supports HTTP(S), HTML data URLs, and `about:blank`, dimensions, zoom, scroll offsets, and three basic device IDs. It waits for load, web fonts, decoded images, and two animation frames. CLI/MCP acquisitions use fresh isolated contexts; cached page images are reused until explicit refresh or bounded-cache eviction. `page_state` in acquisition metadata reports `retained: false`, `id: null`, `reused: false`, and `retained_limit: 0` for this path. The TUI separately opts into document/node page state; it does not change the MCP render contract. Dedicated authentication profiles and page capture policies are available as documented in [runtimes](runtimes.md). Device IDs beyond the declared desktop/phone/tablet set and retained MCP DOM sessions remain outside the current implementation; the former also needs its published preset semantics confirmed.

GLSL fills, strokes, and effects use a separate software WebGL process. Numbers, booleans, vectors, colors, matrices, flat numeric arrays, and scalar/one-dimensional array image samplers are supported, with exact `@resolution` and `@time` directives. Grouped declarations share their annotations. Array dimensions can use GLSL constants/expressions; the linked active range is validated by WebGL and optimized tails are omitted from upload. Each sampler array element accepts a captured image URL or a string variable with its own image origin. Unset active sampler elements are transparent; literal dimensions require exact lengths and linked expression dimensions require the active prefix. Identical images share texture units, and the actual fragment texture-unit limit is checked. Encoded sampler inputs are bounded to 64 MiB per shader request. Matrix values are column-major. Boolean vectors/arrays use numeric 0/1; scalar booleans retain JSON true/false. Effects receive prior object pixels and can sample the previously painted background, including parent layers. Documented annotation defaults, structure uniforms and macro preprocessing are implemented. Fill-stage `pencil:object` sampling remains unavailable, and complete Pencil annotation/vertex parity is not established. The `browser` and `shaders` guides describe the current contract. Browser startup is capped at 15 seconds, page jobs at 30 seconds, shader jobs at 15 seconds, and a render at 45 seconds; these deadlines overlap rather than reset at each CDP stage. Cancellation terminates an unresponsive runtime; a later request can start a fresh one.

## Snapshots, limits, and cancellation

The current server retains up to four calculation snapshots for five minutes. A missing or expired ID returns `SNAPSHOT_UNAVAILABLE`. Reusing a snapshot checks the current document and dependency revisions. An external edit invalidates its use rather than quietly returning measurements from another revision. CLI calculation IDs are not retained across separate CLI processes.

The document worker waits until either a job or its earliest maintenance deadline arrives. After 60 seconds without actual execution, it closes and reaps the script, page, and shader children independently. Active jobs finish before idle maintenance runs. Cache hits do not extend a child's lifetime. Cached rasters and unexpired calculation snapshots remain available after process cleanup, and a later calculation or explicit refresh starts the required runtime again. Expired snapshots are released even when no new request arrives. No periodic full render or extra cleanup thread is used.

Tool arguments and structured results are limited to 4 MiB. Rendered PNG content is limited to 8 MiB; choose a smaller region or scale when necessary. The renderer also limits dimensions and total pixels. The worker queue holds up to sixteen pending requests. Full capacity waits for admission; cancellation before admission prevents enqueueing. These limits are not a process-memory or latency guarantee.

Filter overscan and full intermediate node layers also count toward working-raster limits. Explicit CPU scratch reservations are shared and capped at 512 MiB; this is not a bound on total process RSS or external-runtime memory. Viewport culling is implemented, including skipping unrelated offscreen browser/shader work. Overscan/intermediate bounds remain conservative; controlled full performance acceptance remains open.

[MCP cancellation](https://modelcontextprotocol.io/specification/2025-11-25/basic/utilities/cancellation) abandons the request; a cancelled request normally has no response. Work cancelled while queued does not execute. Executing edits check cancellation while waiting for document and dependency locks, between operations, during component dependency copying and expansion, and before asset publication and document replacement. The same cancellation token covers runtime preconditions and dry runs. `put_asset` also checks cancellation while waiting for its destination lock and before final publication. CLI edits and `assets put` use Ctrl+C for these checkpoints. Cancellation is cooperative; a single bounded parse, hash, or file write can finish before the next checkpoint.

Cancellation before document replacement leaves the original document in place. Late cancellation can leave complete, immutable, unreferenced managed assets or an aborted preparation record. Once document replacement begins, the transaction finishes its journal commit. Read the current document/history after an abandoned request. Retrying the original request ID and identical payload returns a committed result when available; if it was not committed, the retry can perform the edit. Native layout and text measurement share the request token with script execution: checks cover node expansion, dimensions, positioning, visual bounds, shaping boundaries, glyph batches of at most 64, and snapshot finalization. Renderer checks begin before raster allocation and continue through the existing node/encoding boundaries. A cancelled calculation returns no partial snapshot or text block. Individual synchronous Skia calls, parsing and file operations finish before the next checkpoint; complete latency guarantees and physical-platform validation remain unfinished.

### Explicit HTTP image acquisition

Ordinary document inspection does not fetch HTTP images. `get_document` accepts `acquire_images: true` to explicitly acquire them; layout/rendering and image copy/import can also access the network, including local addresses. Network-capable tools advertise `openWorldHint: true`. Deferred images report `IMAGE_ACQUISITION_REQUIRED` and `assets_complete: false`, while structural edits remain available. Declared `remote_image_reference` dependency identities stay stable across acquisition; fetched `remote_image` byte hashes belong to runtime dependencies and snapshot identity. Use `runtime.browser.image_origins` for an optional exact-origin ceiling, or `network_policy = "offline"` to deny acquisition. See [HTTP image acquisition](dynamic-content.md#http-image-acquisition).

### Connection scheduling and retention

Each open document owns its layout, text and runtime state on its own thread. Native
Skia paragraphs stay on the thread that created them. Requests execute serially per
document, with at most two document operations executing per connection. Up to four
requests per document and sixteen overall are admitted; additional requests wait with
cancellation. Waiting and admitted ordinary requests together are limited to 32 per
document and 64 per connection, with serialized input budgets of 16 MiB per document
and 64 MiB per connection. Accounting includes arguments, request metadata and the
request ID, and starts before audit work or admission waits. These byte counts are
not process RSS limits. Excess requests return `RESOURCE_LIMIT` immediately; retry
after earlier work finishes or is cancelled. Built-in catalogue, discovery and handle
operations use the dispatcher.
A blocked image acquisition in one document does not serialize another document's work.
Copy/import acquisition happens before the save lock; revision and source verification
are repeated inside the transaction. Checkpoint restore retains its locked lookup.

`close_document` cancels running/queued requests for that handle and acknowledges only
after its native snapshots, caches and owned child processes are released. It bypasses
normal request admission so a saturated document can still be closed, using a separate
reserve of eight close requests, each limited to 4 KiB of input. Reopening the path
creates a new ID; old calculation IDs and cursors cannot be reused. EOF/connection
shutdown cancels and joins all document owners. Up to 64 open or retiring handles share
one connection; closing handles releases this capacity without reconnecting.

Retained calculations, failed-source captures, discovery listings and dynamic cache
payloads share a 256 MiB accounting budget across the connection. Accounting includes
captured input buffers, parsed JSON and indexes, expanded node/record payloads, glyph
arrays and conservative native paragraph/path allowances. Shared buffers are charged
per retained owner. This is a payload accounting limit, not an exact process RSS limit;
native allocators, browser heaps and in-flight computations have separate limits.
There may be at most four document owners with persistent JavaScript/page/shader
runtimes. Idle cleanup releases their owner capacity; existing worker timeout and heap
settings still apply.

A new calculation evicts the requesting document's oldest snapshots as needed. Dynamic
cache pressure drops that cache and its workers while preserving the response. If other
owners still fill the budget, a new calculation fails explicitly with `RESOURCE_LIMIT`;
close unused documents or retry after idle cleanup. Other documents' current snapshots
are not silently replaced. Snapshot/failure TTL and per-document entry limits still
apply; `SNAPSHOT_UNAVAILABLE` means request a fresh calculation without an old ID.
