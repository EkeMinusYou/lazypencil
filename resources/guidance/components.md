# Components and atomic editing

Search existing reusable definitions before creating another button, field, card, or repeated row. Reuse the source component when its structure and meaning fit the task. Library definitions are read-only through a document-bound connection. Do not modify a global definition when the user requested a change to one instance.

Distinguish the stored root instance, an inherited descendant, and a component definition. Inherited descendants are identified by the root instance ID and the nested instance path. Normal frame hierarchy does not create a new component namespace. Use only editing operations advertised by the current server; do not substitute an update to the root when a descendant operation is unavailable.

Read an inherited node with read_nodes using node_id and an instance_path array. Its expanded properties include variable references and property origins; get_layout supplies calculated measurements. To change only that instance, use update or set_override with the same address. reset_override restores inheritance; replace_override replaces a descendant; set_slot_children replaces a complete frame child array. Use the returned client bindings for later operations instead of guessing generated IDs. set_theme changes selected axes without replacing every inherited axis.

For resolved properties and script-generated descendants of a used instance, pass a calculation's snapshot_id to read_nodes with that inherited address. This calculated view matches the layout and PNG tree and preserves source namespaces and themes. An unused library definition still uses the ordinary library_alias query; it is outside the bound document's calculation. Generated children cannot be edited as stored overrides; edit their owning script instead.

Request get_document with include_capabilities:true before detaching or importing a component. Stored and inherited instance detachment keeps nested references and variables. External library references stay linked, moved asset paths keep their original targets, and library variables use their original namespace. Detachment checks the resolved result before saving. External scripts retain their linked execution context through versioned metadata; treat unsupported scope-preservation diagnostics as a failed atomic batch. Keep a reference when the user wants future changes to the component definition to propagate.

An omitted property inherits its value. Removing an override restores inheritance; null is not a generic reset command. A descendant override with type is a full node replacement. Replacing children replaces the array, including the empty array; it does not merge old children. Slot suggestions help discovery but do not prohibit other valid content.

Use client_ref for a newly created element that later operations need. For example, an insert can declare client_ref `button`, and a later insert can set ref to {"result_of":"button"}. Use the returned bindings for later requests. Keep related structural changes in one batch so missing references are never persisted midway through the operation.

External script detachment retains its execution and input documents through versioned lazypencil metadata. Keep that metadata and its declared library dependencies when editing the detached node. This does not make a standalone copy of the code or library, and Pencil round-trip rendering of this extension is unverified. Use ordinary source/input edits for a new context; do not manually fabricate binding hashes.

Read the current revision and dependency revision before planning an edit. On conflict, read again and reconcile the user's intent with the actual document. Retry an ambiguous saved request with the same request ID and payload; use a new ID for a different edit.

Use import_component with an explicit mode. link creates a reference to a local or library definition; a file source also registers its import alias. copy creates local reusable definitions and closes their static component, variable, theme, image, and shader dependencies. The returned id is the placed instance; component_id identifies its reusable definition. Copied definitions are stored in a disabled frame so they do not add visible template artwork. Reuse one imported definition across repeated controls instead of importing it for every instance.

Sources can select a declared library with {"library_alias":"ui","node_id":"button"}, a project file with {"uri":"ui.lib.pen","import_alias":"ui","node_id":"button"}, or a bundled style with {"style_id":"standard-ui","node_id":"button"}. Read the style's editable example before selecting an ID. Use expected_source_revision when the source document hash is known. File sources remain within allowed project roots. copy leaves no new source import; link keeps the source live.

When the document has no suitable control, get_style with id `standard-ui` provides these reusable starting points:

| Component ID | Pattern |
| --- | --- |
| `button` | Fixed-width action with a content-sized label centered inside it. |
| `button-with-icon` | Content-sized action with the icon and label centered together. A fixed instance width keeps that group centered. |
| `breadcrumbs` | Content-sized labels and a separator in reading order, with layout-managed gaps. |
| `sort-control` | Compact content-sized label and chevron with a small gap; not a full-width select field. |
| `text-avatar` | Fixed-size avatar with its character centered in a text box that fills it; see `typography`. |
| `form-field` | Label, input, and independent optional supporting/error messages in a content-height group; see `app-screen`. |

Use the existing document's components first. These examples are fallbacks, not a mandatory design system. Read their children before overriding labels or icons, adapt their tokens and spacing to the document, and reuse one definition for repeated controls. Keep the layout relationships when changing copy instead of assigning new fixed label widths or child coordinates. Long constrained labels may need a different sizing treatment; follow `layout` rather than forcing every control to hug its content.

Copy name collisions fail by default. variable_prefix and component_prefix can give a kit its own names. Per-source variables, components, and themes maps can explicitly rename a source name or reuse a local target. Error details identify the source_key. Reused variables require the same type; reused themes must contain all source values. Reuse must also satisfy the importer's dependency constraints; do not bypass a refusal by flattening the component.

Explicit component reuse can replace a reusable definition contained in a copied parent. Its placement becomes a fresh local ref; the chosen existing definition supplies its body, appearance, metadata and code. The source's placement and dimensions stay as instance overrides, and later edits to the target remain live. id_map reports the placed ID; reused_components reports the selected definition. When descendant IDs differ, supply component_paths for that reused source key, for example {"ui:badge":{"source-label":"local-label"}}. Both paths must exist in their respective definitions. Stored children and full-node replacements keep their own authored bodies. Paths to their new children follow the effective instance boundaries, including when reuse changes a ref into an ordinary group or introduces a new ref. Stored and generated refs use the same translation. Colliding destinations fail atomically. Keep missing-path and execution-context composition errors visible; do not guess which target child the user intended.

String-valued shader image samplers with local relative/absolute paths, local file URLs or HTTP(S) URLs can be copied independently. Text uses retain the exact original string; captured image origins travel with the copy. Copy captures all theme alternatives, variable aliases and required image bytes. Changing the copied variables later can select a new consumer image. Explicitly reused consumer variables retain their consumer file bindings. Ordinary relative hierarchies are preserved where possible; distinct aliases, case collisions, file/directory projection conflicts and nonrelative URLs use managed image bindings. Immutable resource bundles carry hashed inventories and use guarded publication. HTTP acquisition follows the runtime.browser network policy and the limits in the shader guide; captured copies remain local after their original network source disappears. Embedded PNG/JPEG/WebP data URLs are self-contained: copy and detach preserve their original strings and do not create separate image files. Missing assets, unapproved schemes and local paths outside allowed roots remain errors. Do not substitute an unrelated asset or a static image for requested live behavior.

For example, a batch can import a bundled button, then place its returned instance:

```json
[
  {"op":"import_component","source":{"style_id":"standard-ui","node_id":"button"},"mode":"copy","options":{"variable_prefix":"kit.","component_prefix":"Kit "},"client_ref":"primary"},
  {"op":"update","target":{"result_of":"primary"},"set":{"x":40,"y":40}}
]
```

Inspect the imported structure, measure layout, and render the result. Source libraries and original asset files are never edited by an import. A dry run validates staged bytes without creating managed files or document history. A committed request can be replayed even after its original source is unavailable.

For script copies, the importer closes the reachable source component, variable, and theme namespaces without executing JavaScript. Code remains executable JavaScript, stored as immutable copied bytes. Add files selected only inside the code to options.script_assets, keyed by the qualified source script node, with entries such as {"kind":"script","uri":"nested.js"} or {"kind":"image","uri":"alternate.png"}. A descendant code override uses its owning ref and override path, such as ui:inner/draw. URIs use that script's original output document. Include shader source and alternative files that later input values can select. Generated paths outside the manifest fail instead of opening a same-named consumer file.

Copied scripts retain code, output, saved inputs and browser-resource ownership separately, including nested generation and another independent copy. Ordinary update/reset operations deliberately choose the caller's context for an explicitly changed code or input field, even when its JSON is unchanged. Preserve private scope, binding, input-generation and browser metadata when retaining an existing context; do not fabricate or edit its hashes or namespace keys. Missing legacy provenance requires copying again from the original source, rather than guessing a same-named consumer component or file.

Generated refs and replacement children use their returned addresses. Explicit component reuse can translate instance boundaries, but missing or ambiguous descendant paths remain errors. Browser copies retain declared captured resources and the URL's owning namespace. Unknown active aliases, undeclared assets and unsupported ownership combinations must remain visible diagnostics. A successful copy does not establish Pencil round-trip execution compatibility.

Inherited scripts can override inputs, code or metadata independently before copying. Unchanged fields continue to inherit from the copied definition; null inputs use the active code header's defaults. Use the ordinary editing API to change or restore these fields, then calculate and inspect the result.

## Expansion limits

Component and variable expansion share a 128 MiB allocation-work budget across the operation's roots. Nested strings, keys and property values are counted before copying; script layout also shares the allowance across generated outputs in each pass. This is a conservative work estimate, not a process RSS limit. Exceeding it returns RESOURCE_LIMIT and preserves the document and original values. Prefer managed image files over repeating very large data URLs, and reduce the number or size of expanded instances before retrying.
