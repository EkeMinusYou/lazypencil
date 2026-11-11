# MCP integration and editing reference

Connect an MCP client to let an AI agent open, inspect, edit and render `.pen`
designs. Register `lazypencil mcp` once per client; one connection can work with
several documents. See the [registration commands](../README.md#mcp-and-ai-prompts)
for Codex and Claude Code.

## Open a document

The server command is:

```sh
lazypencil mcp
```

Call `open_document` with an absolute path:

```json
{"path":"/absolute/path/design.pen"}
```

The response provides `document_id`, `path`, `revision` and `version`. To create
an empty document, pass the same input to `create_document`; the parent directory
must exist and an existing destination is preserved.

Pass the returned `document_id` to every document-specific tool, even with only
one file open. For example, inspect its metadata with `get_document`:

```json
{"document_id":"DOCUMENT_ID"}
```

IDs belong to the current connection. Reopen paths after reconnecting. Reopening
a path on the same connection reuses its existing ID. Use `close_document` with
`{"document_id":"DOCUMENT_ID"}` to release it when finished.

### Find files and configure paths

`list_documents` discovers `.pen` files under `directory`, recursively by default.
Use `query` to filter collected relative paths, `recursive` to change traversal,
and `limit` (1–500) with `cursor` to page results. Repeat the same directory,
recursive and query values on later pages.

Discovery stops at 10,000 documents, 100,000 entries or five seconds. Check
`complete`, `stop_reason`, `entries_scanned` and `unreadable_directories` before
treating a result as a complete listing. Symlinks are skipped; `.gitignore` is not
applied. Listings are retained for five minutes, up to four at a time.
After `SNAPSHOT_UNAVAILABLE`, restart without a cursor.

Relative paths resolve from the server's working directory, which is reported
in its initialization instructions. To restrict document selection, repeat
`--root DIRECTORY` when starting the server. Without roots, explicitly selected
`.pen` files can belong to different projects.

`--font PATH` adds font inputs. `--data-dir` or `LAZYPENCIL_DATA_DIR` selects data
and history storage. If you override `LAZYPENCIL_LOCK_DIR`, use the same absolute
directory for cooperating lazypencil processes.

Each document uses its own [project configuration](configuration.md). Opening it
leaves project instructions and executable settings subject to their existing
local trust decisions.

## Current tools

| Tool | Behavior |
| --- | --- |
| `list_documents` | Bounded, paged recursive discovery with explicit completion status |
| `open_document`, `create_document` | Select or create a document and return its connection-local ID |
| `close_document` | Cancel work and release a document handle, its calculations and runtimes |
| `get_document` | Document metadata, revisions, dependencies, diagnostics, capabilities, and the guidance entry point |
| `get_guidance` | Built-in English guides, trusted project guides, automatic project rules, versions, hashes, and an index |
| `list_icons` | Pinned vector icon names, aliases, tags, versions, hashes, licenses, and pack availability |
| `list_styles`, `get_style` | Built-in and trusted project styles, tokens, guide references, font requirements, and editable `.pen` examples |
| `search_nodes` | Shallow, byte-bounded candidate search with exact totals, addresses and optional calculated visual bounds |
| `read_nodes` | One exact stored, expanded or calculated target, with properties, provenance and explicitly requested descendants |
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

Use the tools and input schemas advertised by the connected server. Unknown
arguments are rejected. Success responses include an `outputSchema`; domain
failures use `isError: true` with a structured error code and readable text.
`render` returns metadata plus a separate MCP image block containing the PNG.

Guidance/style tools can omit `document_id` only when requesting built-in resources.
`list_icons` is document-independent. Layout, image export, font inventory and
calculated node reads require a build with rendering enabled.

## Editing and inspecting

A typical edit follows this sequence:

1. Open the document and read `get_document` for its current revisions and diagnostics.
2. Read relevant nodes, components and variables. Use `get_guidance` for supported operations.
3. Apply a small batch with `apply_operations` and current revision preconditions.
4. Validate layout, then render and inspect the image.

For example, an `apply_operations` request can update a known heading:

```json
{
  "document_id": "DOCUMENT_ID",
  "request_id": "change-title-1",
  "expected_revision": "REVISION_FROM_GET_DOCUMENT",
  "expected_dependency_revision": "DEPENDENCY_REVISION_FROM_GET_DOCUMENT",
  "operations": [
    {"op":"update","target":"heading","set":{"content":"My project"}}
  ]
}
```

Omit the dependency revision only when the document has no external dependencies.
Use a new request ID for a different edit. Retry an uncertain request with its
original ID and identical payload to recover the committed result without applying
it twice. `dry_run: true` validates the batch without saving.

`get_layout`, `render`, `apply_operations` and `restore_checkpoint` require
`expected_revision`. Supply `expected_dependency_revision` when external inputs
participate. Read/validation tools can also use optional revision preconditions.

For example, call `get_layout` after editing:

```json
{
  "document_id":"DOCUMENT_ID",
  "expected_revision":"REVISION_FROM_EDIT",
  "expected_dependency_revision":"DEPENDENCY_REVISION_FROM_EDIT"
}
```

Then call `render` with its calculation ID and revisions:

```json
{
  "document_id":"DOCUMENT_ID",
  "snapshot_id":"SNAPSHOT_FROM_LAYOUT",
  "expected_revision":"REVISION_FROM_LAYOUT",
  "expected_dependency_revision":"DEPENDENCY_REVISION_FROM_LAYOUT"
}
```

Replace uppercase placeholders with values returned by the corresponding calls.
Node and region exports are crops of the composed document, including overlapping
content. See [graphics](graphics.md) for crop behavior, effects and export limits.

### Document metadata and paging

`get_document` returns identity, revisions, root summaries, diagnostics,
dependencies, capabilities and configuration status. Collection previews allow
up to 100 entries and 64 KiB each. Check `collections` for `total`, `offset`,
`returned` and `truncated`; `omitted_fields` identifies oversized optional fields.

To read a full collection, supply `collection`, `limit` (1–500, default 100),
and then the returned `next_cursor`. Available collections are `roots`,
`diagnostics`, `dependencies`, `runtime_dependencies`, `asset_diagnostics`,
`attachments` and `font_diagnostics`, when listed in the overview.

Pages allow up to 3 MiB of collection data and can end before the requested count.
Entries are not shortened. Use CLI inspection for a single entry too large to
return. Cursors bind the document, connection, collection and revisions;
restart after an input change.

`include_configuration: true` returns effective settings and their origins.
`include_capabilities: true` returns detailed supported features. An optional
[font lock](fonts.md#reproducibility-lockfile) appears in configuration metadata;
layout and rendering check its baseline, while structural edits remain available
for repairs.

Unknown node properties produce `UNKNOWN_PROPERTY` warnings and remain stored.
Review spelling suggestions before continuing. Boolean properties require booleans,
including resolved variables and override values; a string such as `"false"`
is invalid. Arbitrary metadata and script-input keys have their own schemas.

### Project guidance and styles

`get_guidance` without an ID returns the workflow and automatic project guides.
Other guides are fetched by ID. `list_styles` and `get_style` expose built-in and
approved project styles, tokens and editable examples. Use a style as a starting
point for a new screen while preserving existing conventions during local edits.
The user's requested language applies to responses and design content.

Project instructions require local review through `config show FILE` and
`config trust FILE`. MCP tools cannot grant approval. `PROJECT_NOT_TRUSTED`
means the user must review the configured instructions before continuing.
See [design trust](configuration.md#explicit-design-trust).

## Components and inherited nodes

Find candidates with `search_nodes`, then pass a candidate's `read_target` with
the current `document_id` to `read_nodes`. For an inherited label:

```json
{
  "document_id":"DOCUMENT_ID",
  "node_id":"instance",
  "instance_path":["nested-instance","label"],
  "depth":0
}
```

`read_nodes` returns one target in `nodes`. Depth defaults to 0 and accepts 0–16;
zero returns `children_count`, while larger values include descendants.
It requires `node_id` and accepts no name/type filters or pagination arguments.
Use `search_nodes` for discovery and paging.

Expanded reads report `view: "expanded"`, addresses, properties, property origins
and source nodes. Variable references remain references. `expand_instances: true`
expands the selected target; `instance_path` selects a specific inherited target.
`library_alias` follows a declared import, including nested aliases such as `ui:base`.

### Calculated and generated nodes

Use `calculated: true` to evaluate layout and scripts:

```json
{"document_id":"DOCUMENT_ID","calculated":true,"node_id":"chart","depth":2}
```

Alternatively supply a retained `snapshot_id` to read the same calculation without
rerunning scripts. Calculated reads report resolved `properties`, their sources,
effective theme, measured `layout`, source IDs and `generated_by`. Declarative
sizing stays in `properties`; measured positions and dimensions are in `layout`.

Use generated IDs exactly as returned. Generated nodes are read-only; follow
`generated_by` to edit the owning script or inputs. Calculated queries cover the
document's effective tree and reject `library_alias` and `expand_instances: true`.
Use stored/expanded reads for unused library definitions.

### Lightweight node search

`search_nodes` accepts a case-insensitive `name` substring, exact `type`, and an
optional `root: {node_id, instance_path}`. Filters combine with AND and include
the root and its descendants. An absent root is an error; zero matches is a valid
search result.

Stored nodes are the default. `expand_instances: true` includes inherited nodes;
`library_alias` searches declared definitions. `calculated: true` evaluates scripts
and layout, while `snapshot_id` reuses a calculation. Calculated searches reject
`library_alias` and `expand_instances: true`.

Candidates include `id`, `read_target`, `name`, `name_truncated`, `type`, `parent`,
`children_count`, `bounds`, `enabled`, `origin` and `generated_by`. Calculated
`bounds` are document-space visual bounds; uncalculated bounds/enabled are null.
Use `read_target` instead of constructing an address from a displayed ID.

For detail reads, retain the search context: `snapshot_id` for calculated nodes,
`expand_instances: true` for expanded nodes, and `library_alias` for libraries.
Compact `status` fields identify missing inputs and resolution errors; use
`get_document` to read the full diagnostics.

Pages default to 20 candidates and accept 1–100, within 64 KiB. Names are shown up
to 256 Unicode characters; filtering uses the full name and IDs remain complete.
`total_matches` is exact for the selected view/scope. Follow `next_cursor` until
null, repeating the query and calculation ID. Page size may change.

`REVISION_CONFLICT` means inputs changed; `SNAPSHOT_UNAVAILABLE` means the
calculation expired. Start one fresh search without cursor/snapshot, then share
its new ID for subsequent reads and rendering.

CLI inspection remains independently paged:

```sh
lazypencil inspect design.pen --calculated --node chart --depth 2
```

Each CLI invocation recalculates layout. Omit `--calculated` to inspect stored
content, including scripts that cannot currently execute.

### Editing instances

An inherited `update` or `set_override` changes that instance's override.
`reset_override` restores inheritance; `replace_override` and `set_slot_children`
replace content and return generated IDs through `client_ref` / `client_refs`.
Use `result_of` to refer to those IDs later in the same batch.

`set_theme` changes theme axes. Inherited descendants cannot be moved or deleted
as stored roots. `detach_instance` materializes one instance boundary, preserving
its appearance, variables and resource origins. Stored roots keep their IDs;
inherited targets become stored replacements with new IDs, reported under
`generated_ids[OPERATION_INDEX].address`.

Detaching leaves other nested instances linked and source libraries unchanged.
Script code stays linked to its original execution/input scopes. Internal
`metadata.lazypencil` records preserve those origins; leave them intact when
editing serialized documents. A missing or invalid origin binding requires a
fresh copy or repair rather than guessing a source context.

`add_import` and `remove_import` update the declared library map. A batch must
leave all component and variable references resolvable before it can save.

## Variables and themes

`get_variables` separates raw `variables` and `themes` from `resolved_values`,
`sources` and `effective_theme`. Raw aliases stay aliases. Each resolved value
identifies its definition and terminal-value document. Failed values remain in
raw definitions and add named diagnostics; `resolution_complete` covers the
returned page.

Without selectors, the tool lists variables defined in the selected document.
Use `library_alias` for a declared library and `name` for one exact local name
or qualified reference such as `ui:spacing`.

`node_id` and optional `instance_path` select a stored/inherited node and its
ancestor/instance theme. Add `property` to resolve a variable in that property's
source namespace:

```json
{
  "document_id":"DOCUMENT_ID",
  "node_id":"instance",
  "instance_path":["label"],
  "property":"fill",
  "name":"ink",
  "theme":{"mode":"dark"}
}
```

`theme` adds a hypothetical override and leaves the document unchanged.
Explicit axes/values must be declared in the loaded project.

Use expected revisions for stale-read protection. `limit` accepts 1–500, default
100; names are sorted, and `next_cursor` binds the query, context and revisions.
Queries allow up to 128 inherited path elements and 128 theme overrides. Selector
strings and theme axes/values allow up to 4,096 UTF-8 bytes; path elements are
nonempty IDs without `/`.

```sh
lazypencil variables design.pen --name ui:spacing
lazypencil variables design.pen --node instance --instance-path label --property fill --name ink --theme mode=dark
```

## Validation scopes

| Scope | Checks |
| --- | --- |
| `structure` (default) | Document structure, references, variables and themes |
| `layout` | Structure plus measured text, sizing, overflow, font and icon diagnostics; executes scripts |

Warnings keep `valid: true`; diagnostic errors return `valid: false`. Calculation
failures are tool errors. `error_count`, `warning_count` and `total_diagnostics`
cover the complete calculation, including entries outside the current page.
Validation inspects structure or layout; render the result separately to inspect
paints, browser captures and shader pixels.

Layout validation returns a reusable `snapshot_id`. Pass it with the returned
revisions to `get_layout`, `render` or later diagnostic pages. A structural request
cannot include a calculation ID.

Diagnostics accept `limit` 1–500, default 100, and `cursor`. Layout/PNG results
include an excerpt of up to 100 diagnostics and 64 KiB, with `diagnostics_truncated`.
Use `validate_document` to page the complete set. After a repair, calculate again
without the pre-edit snapshot or cursor.

```sh
lazypencil validate design.pen
lazypencil validate design.pen --scope layout --limit 100
```

CLI diagnostic errors exit with code 4 and include the report in structured error
details. Warnings return success. Separate CLI invocations recompute calculations.

## Importing components

Use an `import_component` operation inside `apply_operations`. Select `mode: "link"`
to retain the source definition, or `mode: "copy"` for independent editable content.
The operation accepts `parent`, `index` and `client_ref` and returns the placed
instance `id`, reusable `component_id` and `source_revision`.

Choose a `source` object:

| Source | Example |
| --- | --- |
| Declared library | `{"library_alias":"ui","node_id":"button"}` |
| Project file | `{"uri":"library/ui.lib.pen","import_alias":"ui","node_id":"button"}` |
| Design style | `{"style_id":"standard-ui","node_id":"button"}` |
| Local reusable definition | `{"node_id":"button"}` |

File sources resolve beside the selected document and must remain inside allowed
project roots. Optional `expected_source_revision` rejects a changed source.
Application-owned style examples adapt to the consumer's document version while
their source revision still identifies the original example. User-owned examples
retain their declared versions.

A copy operation can look like this inside the request's `operations` array:

```json
{"op":"import_component","source":{"library_alias":"ui","node_id":"button"},"mode":"copy","options":{"variable_prefix":"kit.","component_prefix":"Kit "},"client_ref":"primary"}
```

`link` preserves source variables and asset origins. A file source registers its
explicit alias; a style source publishes a managed library with a default
`style-STYLE_ID` alias. Supply `import_alias` to choose another. Alias collisions
and saved import cycles are rejected.

`copy` captures static component, variable, theme and resource dependencies and
allocates new IDs. Required definitions live under a disabled `Imported components`
frame; the placed instance is visible. `id_map` uses qualified source keys, escaping
literal `%` as `%25` and `:` as `%3A` inside each segment. Source files stay unchanged.

### Resolve collisions and reuse definitions

Names that collide fail by default. `options.variables`, `options.components` and
`options.themes` map source keys to `{"action":"rename","name":"new-name"}` or
`{"action":"reuse","target":"local-target"}`. Component reuse selects a local
reusable node ID; variable/theme reuse selects a local name.

Variable reuse checks types; theme reuse requires the source values.
`variable_prefix` applies before collision checks. `component_prefix` applies to
existing display names. Unused or misspelled resolution keys are rejected.

If reused components have different descendant IDs, add an `options.component_paths`
mapping. For example, this is the content of `options`:

```json
{
  "components":{"ui:badge":{"action":"reuse","target":"local-badge"}},
  "component_paths":{"ui:badge":{"source-label":"local-label"}}
}
```

Both paths must exist in their definitions. Stored replacement bodies keep their
own children and IDs. The copy translates applicable overrides and rejects
ambiguous or colliding destinations.

### Resource and script copies

Images and shaders are copied into content-addressed files under `assets/lazypencil`.
Relative image strings, aliases and theme alternatives preserve their meaning.
Some copies use managed bundles or internal bindings to preserve source paths;
embedded data URLs already contain their image bytes and stay unchanged.

Copying scripts preserves their code and input/output scopes without executing or
rewriting JavaScript. Declare dependencies selected only inside code with
`options.script_assets`, keyed by the qualified source script node. See
[dynamic content](dynamic-content.md#script-copy-and-detach) for scope rules.

Dry runs validate/stage the same inputs without publishing files or history.
Captured inputs and staged outputs each allow up to 128 MiB per batch, with 64 MiB
per managed file. Bundle limits are 4,096 members, 128 MiB payload and 1 MiB of
member paths. Oversized edit reports fail before publication.

An interrupted or conflicting save can leave complete unreferenced asset files.
Use [asset maintenance](#project-asset-maintenance) to review them. A retry of a
committed request returns the recorded IDs/URIs without reopening its source.

## Checkpoints and retention

`get_history` lists edits newest first and returns `next_before_sequence` when
more entries remain. Follow that cursor even when a page is shorter than its
requested limit. `restore_checkpoint` restores document contents as a new edit,
checking current document and dependency revisions.

See [checkpoints](checkpoints.md) for CLI pruning, expired request IDs and cleanup.
Pruning is an explicit local operation, separate from the MCP tools.

## Managed assets

`put_asset` stores immutable code or images and returns a relative URI and SHA-256.
For example, create a script asset with:

```json
{"document_id":"DOCUMENT_ID","kind":"script","source":{"text":"return [];"}}
```

For an uploaded image:

```json
{"document_id":"DOCUMENT_ID","kind":"image","source":{"mime_type":"image/png","data":"BASE64_DATA"}}
```

Static PNG, JPEG and WebP inputs are normalized to PNG with orientation applied.
Code allows up to 1 MiB, source/normalized images up to 20 MiB each, and decoded
images up to 32 megapixels. The 4 MiB MCP argument limit also applies to encoded data.

Files are placed under `assets/lazypencil` beside the document. Identical content
is reused and conflicting files are preserved. Uploading alone leaves the document
unchanged; reference the returned URI through `apply_operations`.

`read_asset` reads captured script/shader text by hash after it has become a
referenced dependency. It cannot read an arbitrary path. CLI equivalents are
`assets put design.pen --kind image --input reference.jpg` and
`assets read design.pen --hash HASH`.

### Project asset maintenance

Close other lazypencil frontends, then review unreferenced managed assets:

```sh
lazypencil assets prune design.pen --project-root /absolute/project --json
lazypencil assets prune design.pen --project-root /absolute/project --expected-revision REVIEWED_SHA256 --json
```

Choose a root containing all documents that share the asset directories and the
data directory containing their history. Review scans documents, imports,
checkpoints and declared resources. Unknown files, unresolved/dynamic references
and other uncertain inputs retain assets conservatively and appear in `uncertain`.
Include references from outside the chosen root before applying deletion.

The review binds current inputs and inventory; changes require a fresh review.
Results report `deleted`, `restored`, `cleanup_pending` and `errors`. A batch can
complete some removals before interruption. Repeat review/apply to finish cleanup.
A newly referenced quarantined asset can be restored if its payload is intact;
otherwise `RECOVERY_REQUIRED` identifies the item needing attention.

Each pass allows 30 seconds, 100,000 entries, 64 directory levels, 256 project
snapshots and 512 MiB of cumulative reads. Other active frontends cause
`LOCK_UNAVAILABLE`; close them before retrying. This operation is local and has
no automatic or MCP deletion counterpart.

## Repairing missing assets

Inspection and structural editing work with missing declared images, code and
libraries. `assets_complete: false` and `asset_diagnostics` identify the original
URI, source document, requested/resolved path and resource kind.

Upload a replacement and update its URI, restore the file, or remove the affected
node/property. Repairs may be saved in several batches while existing missing
references remain; new missing targets or kinds are rejected.

For a missing library, rebind its alias with `remove_import` and `add_import` in
one batch, or remove its instances and import together. Remaining component and
variable references must resolve. For missing configured fonts/directories,
repair [font settings](configuration.md#font-precedence-and-fallback).

After restoring or redirecting an input, read fresh document/dependency revisions.
Rendering and measured layout require the necessary inputs to be available.
Structural repair can omit a runtime dependency precondition when broken code
cannot execute. Checkpoint restoration can restore historical missing references
and reports the resulting diagnostics.

`assets_complete: true` describes the declared asset scan. Execution and rendered
output are checked separately. A resolution failure can leave status null with
`resolution_error`; a failed project load reports `dependency_error`.
The repair scan allows up to 1,024 requested-path/resource-kind pairs. Resources
chosen only by running code require successful execution to discover.

## Script execution

Insert a `script` node with `scriptUri`, dimensions and optional `inputs`.
`read_asset` returns `script_header` or `script_header_error` for source repairs;
`get_guidance` with `id: "scripts"` explains typed inputs.
See [JavaScript](dynamic-content.md#javascript) for supported headers and limits.

`get_layout` evaluates scripts and returns generated geometry and execution hashes.
Render its `snapshot_id` for the same calculation. `runtime_dependencies` and
`runtime_dependency_revision` identify files discovered during execution,
separately from document-declared dependencies.

For edits based on a calculated runtime input, also supply
`expected_runtime_dependency_revision`. The service recalculates and rejects
changed inputs with `REVISION_CONFLICT`, including during a dry run. Results report
`validated_runtime_dependency_revision` for the pre-edit calculation. Omit this
precondition for structural repairs that cannot run the broken script.

### Script diagnostics

`error.details.script_diagnostic` identifies the stage (`header`, `compile`,
`runtime` or `setup`), source hash, path, owning node and available source position.
Lines are one-based; columns are one-based UTF-8 byte offsets in the original
source, including the header. `location_kind` is `source`, `declaration`,
`call_site`, `end_of_source` or `unavailable`.

`source_excerpt` contains up to three lines, each limited to a 240-character window.
Each entry includes `line`, `text`, `start_column`, `truncated_start` and
`truncated_end`. Use these structured fields instead of parsing an engine stack.
Some failures, such as primitive throws or parent timeouts, have no source position.

### Read source after a failed calculation

When a failure includes `failure_capture_id`, `read_asset` can retrieve the failing
script by its `source_hash` on that connection. The response reports
`failed_calculation: true` and has no reusable layout `snapshot_id`.

Up to four failed code files of 1 MiB each are retained for five minutes per
document. Changed dependencies return `REVISION_CONFLICT`; changed root documents,
expiration, eviction or unknown hashes leave the code unavailable with
`ASSET_NOT_FOUND`. Installing or changing an icon pack involved in the failed
calculation also invalidates that capture. Recalculate to acquire fresh diagnostics.

## Browser and shader rendering

Install [Chromium](runtimes.md), then call `render` with `time` (seconds, default 0)
and `refresh` (default false) as needed. Metadata includes `dynamic_artifacts`
with capture identities, times, redacted URLs, browser/renderer details and hashes.
A layout snapshot identifies geometry and local inputs; page captures additionally
identify time-dependent web content.

Browser nodes support HTTP(S), HTML data URLs and `about:blank`, dimensions, zoom,
scroll and basic desktop/phone/tablet modes. CLI/MCP captures use isolated contexts
and can reuse cached images; `refresh: true` requests current pixels. Persistent
per-node DOM belongs to TUI previews. Authenticated captures use an explicitly
selected [profile](runtimes.md#dedicated-authentication-profiles).

GLSL supports scalar/vector/matrix inputs, arrays, image samplers and exact
`@resolution` / `@time` directives. Effects can sample `pencil:object` and
`pencil:background`; fills cannot sample the object. Use the
[shader guide](../resources/guidance/shaders.md) and
[dynamic-content reference](dynamic-content.md) for syntax, origins and limits.

### Explicit HTTP image acquisition

Ordinary inspection lists HTTP image references without requesting their bytes.
Use `get_document` with `acquire_images: true` to fetch them explicitly. Layout,
render and image copy/import can also contact external services or local addresses.
Network-capable tools advertise `openWorldHint: true`.

Deferred images report `IMAGE_ACQUISITION_REQUIRED`. Declared URL identities remain
in dependencies; acquired byte hashes enter runtime dependencies and calculation
identity. Configure `runtime.browser.image_origins` to restrict origins or
`network_policy = "offline"` to block acquisition.

## TUI preview control

A TUI-owned server additionally advertises `get_preview_state` and `set_preview`.
Use that conversation's advertised connection: standalone MCP clients do not
control an unrelated TUI. Host-owned servers are named `lazypencil_bound_UUID`.

Call `get_preview_state` with `document_id` to read the requested camera,
`selected_node` and `preview.displayed`. The displayed frame includes its actual
revision, view, document bounds and `preview_snapshot_id`. That ID describes the
TUI frame and cannot be reused as an MCP calculation ID.

Search named targets with calculated `search_nodes`, resolve ambiguous matches,
then use their exact calculated IDs. For example:

```json
{"document_id":"DOCUMENT_ID","target":{"kind":"nodes","node_ids":["login","register"]}}
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

Omit `target` to adjust the current view. `zoom` and `zoom_factor` are mutually
exclusive. Values must be finite and factors positive. Inherited/generated targets
use calculated IDs. A successful operation returns `status: "updated"` after a
complete frame is available, with the camera, bounds, revisions and selected IDs.

Camera operations preserve document geometry and editing selection and pause
playback. `INVALID_ARGUMENT` identifies an unknown or empty target;
`REVISION_CONFLICT` identifies stale geometry; `BUSY` means a dialog or attachment
view currently prevents camera control. A newer manual view or cancellation
interrupts the pending operation. Use `render` to inspect actual pixels.

## Snapshots, limits, and cancellation

A document retains up to four calculations for five minutes. Their IDs and cursors
belong to that document and connection. `SNAPSHOT_UNAVAILABLE` requires a fresh
calculation without the old ID/cursor. `REVISION_CONFLICT` requires reading current
inputs before continuing.

| Limit | Response |
| --- | --- |
| Tool arguments or structured results over 4 MiB | Reduce the request or page the result |
| PNG content over 8 MiB | Reduce scale or export a smaller region |
| Retained connection payload over 256 MiB | Close unused documents or calculate after earlier work is released |
| More than 64 open/retiring handles | Close documents before opening more |
| More than four document owners with persistent runtimes | Wait for idle cleanup or close an unused document |

Rendering and runtime limits also apply; see [graphics](graphics.md#large-exports)
and [runtime settings](configuration.md#browser-shader-and-raster-cache-limits).
Retained payload accounting covers caches and captured inputs, while native and
browser-process memory have separate limits.

MCP cancellation stops queued work before execution and checks running work at
safe boundaries. Cancellation before document replacement keeps the original
file. Once replacement starts, the save finishes its history commit. Inspect the
current document/history after an uncertain request and retry with the original
request ID and identical payload.

### Connection scheduling and retention

Operations run sequentially per document, with at most two document operations
executing per connection. Up to four per document and sixteen overall are admitted.
Waiting plus admitted ordinary requests are limited to 32 per document / 64 per
connection and 16 MiB / 64 MiB of serialized input. Excess returns `RESOURCE_LIMIT`;
retry after earlier work completes or is cancelled.

`close_document` uses separate admission capacity, cancels the handle's work, and
waits for calculations, caches and owned processes to be released. It allows up
to eight close requests with 4 KiB input each. Reopening creates a new ID.
Closing stdin ends the server and releases all document owners.

New calculations can evict that document's oldest snapshots. Other documents'
current snapshots stay intact; close unused documents if they occupy the shared
budget. Idle runtime processes close after 60 seconds by default, while cached
images and unexpired calculations can remain available.

For implementation and protocol tests, see [architecture](design.md),
[MCP tests](../tests/mcp.rs) and [focused checks](development.md#focused-development-checks).
