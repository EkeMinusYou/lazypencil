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

## Import from a library or style

Use import_component to link a source definition or copy it with its dependencies. Read [component-import](component-import.md) through get_guidance before importing, resolving name collisions, or copying scripts and assets.

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
