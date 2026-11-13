# Components and instances

## Decide what should stay shared

Use a component when several elements should share future changes to their structure or appearance. Repetition alone does not require a linked definition: one-off artwork and elements that should evolve independently can remain ordinary nodes. Keep a local edit within its requested scope.

1. Search existing definitions with list_components. Follow next_cursor while looking for a suitable definition; an incomplete page is not evidence that none exists. Read candidates and their needed child depth with read_nodes, using library_alias for imported definitions. Check structure, meaning, and variables before choosing one.
2. Reuse a suitable definition. If none fits and coordinated changes are useful, create a meaningfully named node with reusable: true. Put shared layout, appearance, and defaults there; bind shared roles to appropriate variables. Use slots for regions with instance-specific structure.
3. Place linked uses with type: "ref". Use client_ref and result_of within one batch, or returned stored IDs in later batches. Read generated child IDs before addressing overrides. The `operations` guide has a tested creation example.

Loops and helper functions can place instances, but constructing independent subtrees does not establish a component link. Keep nested controls as refs when they should remain shared. Place definitions outside screen content so they do not appear as extra controls in a screen render.

## Edit definitions and instances

Edit the definition for a shared change and the instance for one occurrence. Library definitions are read-only through a consuming document; open the library itself as another document to edit its local definitions.

Distinguish the stored root instance, an inherited descendant, and a component definition. Inherited descendants are identified by the root instance ID and the nested instance path. Normal frame hierarchy does not create a new component namespace. Use only editing operations advertised by the current server; do not substitute an update to the root when a descendant operation is unavailable.

Read an inherited node with read_nodes using node_id and an instance_path array. Its expanded properties include variable references and property origins; get_layout supplies calculated measurements. To change only that instance, use update or set_override with the same address. reset_override restores inheritance; replace_override replaces a descendant; set_slot_children replaces a complete frame child array. Use the returned client bindings for later operations instead of guessing generated IDs. set_theme changes selected axes without replacing every inherited axis.

For resolved properties and script-generated descendants of a used instance, pass a calculation's snapshot_id to read_nodes with that inherited address. This calculated view matches the layout and PNG tree and preserves source namespaces and themes. Do not combine snapshot_id with expand_instances or library_alias. An unused library definition uses the ordinary library_alias query; it is outside the bound document's calculation. Generated children cannot be edited as stored overrides; edit their owning script instead.

An omitted property inherits its value. Removing an override restores inheritance; null is not a generic reset command. A descendant override with type is a full node replacement. Replacing children replaces the array, including the empty array; it does not merge old children. Slot suggestions help discovery but do not prohibit other valid content.

## Detach an instance

Query get_document with include_capabilities:true before detachment. detach_instance converts one instance boundary to ordinary nodes, preserving nested refs, variables, and asset targets. The operation checks the resolved result before saving. Keep the reference when future definition changes should propagate.

External scripts keep their execution and input documents through versioned lazypencil metadata. Retain that metadata and its declared imports: detachment does not make independent copies of the code or library. Use ordinary source/input edits to choose a new context. Unsupported scope-preservation diagnostics fail the atomic batch.

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

## Optional bundled controls

get_style with id standard-ui provides these starting points when they fit the task:

| Component ID | Pattern |
| --- | --- |
| `button` | Fixed-width action with a content-sized label centered inside it. |
| `button-with-icon` | Content-sized action with the icon and label centered together. A fixed instance width keeps that group centered. |
| `breadcrumbs` | Content-sized labels and a separator in reading order, with layout-managed gaps. |
| `sort-control` | Compact content-sized label and chevron with a small gap; not a full-width select field. |
| `text-avatar` | Fixed-size avatar with its character centered in a text box that fills it; see `typography`. |
| `form-field` | Label, input, and independent optional supporting/error messages in a content-height group; see `forms`. |

Adapt these examples to the document's tokens and layout. Read their children before changing labels or icons. For control alignment and text sizing, use the patterns below and `typography`.

## Import name collisions and reuse

Copy name collisions fail by default. variable_prefix and component_prefix can give a kit its own names. Per-source variables, components, and themes maps can explicitly rename a source name or reuse a local target. Error details identify the source_key. Reused variables require the same type; reused themes must contain all source values. Reuse must also satisfy the importer's dependency constraints; do not bypass a refusal by flattening the component.

Explicit component reuse can replace a reusable definition contained in a copied parent. Its placement becomes a fresh local ref; the chosen existing definition supplies its body, appearance, metadata and code. The source's placement and dimensions stay as instance overrides, and later edits to the target remain live. id_map reports the placed ID; reused_components reports the selected definition. When descendant IDs differ, supply component_paths for that reused source key, for example {"ui:badge":{"source-label":"local-label"}}. Both paths must exist in their respective definitions. Stored children and full-node replacements keep their own authored bodies. Paths to their new children follow the effective instance boundaries, including when reuse changes a ref into an ordinary group or introduces a new ref. Stored and generated refs use the same translation. Colliding destinations fail atomically. Keep missing-path and execution-context composition errors visible; do not guess which target child the user intended.

### Captured image dependencies

Copies capture all theme alternatives, aliases, and required image bytes, including shader string samplers. Text uses retain their original strings; captured image origins travel with the copy. Reused consumer variables keep their consumer file bindings. Relative hierarchies are preserved where possible; path collisions and nonrelative URLs use managed image bindings. Copy and detach preserve embedded PNG/JPEG/WebP data URLs without creating image files. Keep managed assets with the document. Missing assets and disallowed paths or schemes remain errors. Read `shaders` for URL rules and acquisition limits.

## Interface control patterns

Use textGrowth: "auto" for an unconstrained single-line label and omit its width and height. The parent control may still have a fixed width, such as equally sized actions; a content-sized label does not require a content-sized button. Use a constrained text box when long names must wrap or fit an established width. Preserve the user's or existing design's alignment conventions.

- For an ordinary button, use horizontal layout, a small gap, symmetric horizontal padding, and center alignment on both axes. Center the icon and label as one group, rather than centering only the text inside a leftover text box.
- For breadcrumbs, use a content-sized horizontal row with explicit gaps between labels and separators. Let a changed label move the following separator and item through layout, rather than recalculating their x coordinates.
- For a compact sort action, keep the label and chevron together in a content-sized horizontal row with a gap. A wide select field has a different role: its constrained label may fill the available space and keep the chevron at the far edge. Do not apply one alignment rule to both.

For example, this button derives its width from its icon, label, gap, and padding. Setting a fixed width on the frame would keep the same group centered without giving the label a guessed width:

```json
{
  "type": "frame", "name": "Add member", "layout": "horizontal",
  "width": "fit_content", "height": 44, "padding": [0, 16], "gap": 8,
  "justifyContent": "center", "alignItems": "center", "fill": "#2563EB",
  "children": [
    {"type": "icon", "name": "Add member icon", "library": "lucide", "icon": "user-plus", "width": 16, "height": 16, "fill": "#FFFFFF"},
    {"type": "text", "name": "Label", "content": "メンバーを追加", "textGrowth": "auto", "fontFamily": "Noto Sans JP", "fontSize": 14, "fill": "#FFFFFF"}
  ]
}
```

Adapt colors, type, and spacing to the document.

## Copy script components

For script copies, the importer closes the reachable source component, variable, and theme namespaces without executing JavaScript. Code remains executable JavaScript, stored as immutable copied bytes. Add files selected only inside the code to options.script_assets, keyed by the qualified source script node, with entries such as {"kind":"script","uri":"nested.js"} or {"kind":"image","uri":"alternate.png"}. A descendant code override uses its owning ref and override path, such as ui:inner/draw. URIs use that script's original output document. Include shader source and alternative files that later input values can select. Generated paths outside the manifest fail instead of opening a same-named consumer file.

Copied scripts retain output code, saved inputs, and browser-resource ownership separately. An explicit update to inputs or scriptUri selects the caller's context for that field, even when its JSON value is unchanged. reset_override removes the selected override and restores inheritance from the definition, including the inherited field's context. A null script input instead requests the active header's default; it does not remove the stored override.

Preserve private scope, binding, input-generation, and browser metadata when retaining existing contexts. Use ordinary editing operations to change fields rather than fabricating hashes or namespace keys. A copy without required provenance must be recreated from its original source.

Generated refs and replacement children use their returned addresses. Explicit component reuse can translate instance boundaries, but missing or ambiguous descendant paths remain errors. Browser copies retain declared captured resources and the URL's owning namespace. Unknown active aliases, undeclared assets and unsupported ownership combinations must remain visible diagnostics. Inspect a calculated layout and rendered image after copying to verify the requested result.

Inherited code, inputs, and user metadata can be overridden independently before copying. Unoverridden fields remain linked to the copied definition. Inspect calculated output after a visible change; edit generated content through its script owner.

## Expansion limits

Component and variable expansion share the configured runtime.document.expansion_mib allocation-work budget across the operation's roots (default 256 MiB, accepted range 1–16,384 MiB). Nested strings, keys and property values are counted before copying; script layout also shares the allowance across generated outputs in each pass. This is a conservative work estimate, not a process RSS limit. Exceeding it returns RESOURCE_LIMIT and preserves the document and original values. Prefer managed image files over repeating very large data URLs, and reduce the number or size of expanded instances before retrying.
