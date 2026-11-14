# Component imports and copied dependencies

Read [components](components.md) for definitions, instances, and overrides. Use this guide when importing a definition or preserving script and asset dependencies in a copy.

## Import a definition

Query get_document with include_capabilities:true, then use import_component with an explicit mode:

- link places a reference to the source definition. File sources also register an import alias and remain live dependencies.
- copy imports local definitions and their static component, variable, theme, image, and shader dependencies. It places an instance and stores definitions in a disabled frame outside visible template content.

The result's id identifies the placed instance; component_id identifies its definition. Import once and reuse that definition when more linked instances are needed.

Sources can select a declared library with {"library_alias":"ui","node_id":"button"}, a project file with {"uri":"ui.lib.pen","import_alias":"ui","node_id":"button"}, or a bundled style with {"style_id":"standard-ui","node_id":"button"}. Read the style's editable example before selecting an ID. Use expected_source_revision when the source document hash is known. File sources remain within allowed project roots. copy leaves no new source import; link keeps the source live.

For example, this batch copies a bundled button and positions its instance:

```json
[
  {"op":"import_component","source":{"style_id":"standard-ui","node_id":"button"},"mode":"copy","options":{"variable_prefix":"kit.","component_prefix":"Kit "},"client_ref":"primary"},
  {"op":"update","target":{"result_of":"primary"},"set":{"x":40,"y":40}}
]
```

Inspect the imported structure and, for visible use, its layout and rendering. Imports leave the source library and asset files unchanged. A dry run checks staged bytes without creating managed files or document history; a saved request can be replayed after its original source is unavailable. Follow `operations` for request IDs, revisions, and conflicts.

## Import name collisions and reuse

Copy name collisions fail by default. variable_prefix and component_prefix can give a kit its own names. Per-source variables, components, and themes maps can explicitly rename a source name or reuse a local target. Error details identify the source_key. Reused variables require the same type; reused themes must contain all source values. Reuse must also satisfy the importer's dependency constraints; do not bypass a refusal by flattening the component.

Explicit component reuse can replace a reusable definition contained in a copied parent. Its placement becomes a fresh local ref; the chosen existing definition supplies its body, appearance, metadata and code. The source's placement and dimensions stay as instance overrides, and later edits to the target remain live. id_map reports the placed ID; reused_components reports the selected definition. When descendant IDs differ, supply component_paths for that reused source key, for example {"ui:badge":{"source-label":"local-label"}}. Both paths must exist in their respective definitions. Stored children and full-node replacements keep their own authored bodies. Paths to their new children follow the effective instance boundaries, including when reuse changes a ref into an ordinary group or introduces a new ref. Stored and generated refs use the same translation. Colliding destinations fail atomically. Keep missing-path and execution-context composition errors visible; do not guess which target child the user intended.

### Captured image dependencies

Copies capture all theme alternatives, aliases, and required image bytes, including shader string samplers. Text uses retain their original strings; captured image origins travel with the copy. Reused consumer variables keep their consumer file bindings. Relative hierarchies are preserved where possible; path collisions and nonrelative URLs use managed image bindings. Copy and detach preserve embedded PNG/JPEG/WebP data URLs without creating image files. Keep managed assets with the document. Missing assets and disallowed paths or schemes remain errors. Read [shader-reference](shader-reference.md) for URL rules and acquisition limits.

## Copy script components

For script copies, the importer closes the reachable source component, variable, and theme namespaces without executing JavaScript. Code remains executable JavaScript, stored as immutable copied bytes. Add files selected only inside the code to options.script_assets, keyed by the qualified source script node, with entries such as {"kind":"script","uri":"nested.js"} or {"kind":"image","uri":"alternate.png"}. A descendant code override uses its owning ref and override path, such as ui:inner/draw. URIs use that script's original output document. Include shader source and alternative files that later input values can select. Generated paths outside the manifest fail instead of opening a same-named consumer file.

Copied scripts retain output code, saved inputs, and browser-resource ownership separately. An explicit update to inputs or scriptUri selects the caller's context for that field, even when its JSON value is unchanged. reset_override removes the selected override and restores inheritance from the definition, including the inherited field's context. A null script input instead requests the active header's default; it does not remove the stored override.

Preserve private scope, binding, input-generation, and browser metadata when retaining existing contexts. Use ordinary editing operations to change fields rather than fabricating hashes or namespace keys. A copy without required provenance must be recreated from its original source.

Generated refs and replacement children use their returned addresses. Explicit component reuse can translate instance boundaries, but missing or ambiguous descendant paths remain errors. Browser copies retain declared captured resources and the URL's owning namespace. Unknown active aliases, undeclared assets and unsupported ownership combinations must remain visible diagnostics.

Inherited code, inputs, and user metadata can be overridden independently before copying. Unoverridden fields remain linked to the copied definition. After a visible change, calculate and render the affected instances. Edit generated content through its script owner.

## Expansion limits

Component and variable expansion share the configured runtime.document.expansion_mib allocation-work budget across the operation's roots (default 256 MiB, accepted range 1–16,384 MiB). Nested strings, keys and property values are counted before copying; script layout also shares the allowance across generated outputs in each pass. This is a conservative work estimate, not a process RSS limit. Exceeding it returns RESOURCE_LIMIT and preserves the document and original values. Prefer managed image files over repeating very large data URLs, and reduce the number or size of expanded instances before retrying.
