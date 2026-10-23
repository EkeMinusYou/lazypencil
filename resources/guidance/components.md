# Components and atomic editing

Search existing reusable definitions before creating another button, field, card, or repeated row. Reuse the source component when its structure and meaning fit the task. Library definitions are read-only through a document-bound connection. Do not modify a global definition when the user requested a change to one instance.

Distinguish the stored root instance, an inherited descendant, and a component definition. Inherited descendants are identified by the root instance ID and the nested instance path. Normal frame hierarchy does not create a new component namespace. Use only editing operations advertised by the current server; do not substitute an update to the root when a descendant operation is unavailable.

Read an inherited node with read_nodes using node_id and an instance_path array. Its expanded properties include variable references and property origins; get_layout supplies calculated measurements. To change only that instance, use update or set_override with the same address. reset_override restores inheritance; replace_override replaces a descendant; set_slot_children replaces a complete frame child array. Use the returned client bindings for later operations instead of guessing generated IDs. set_theme changes selected axes without replacing every inherited axis.

For resolved properties and script-generated descendants of a used instance, pass a calculation's snapshot_id to read_nodes with that inherited address. This calculated view matches the layout and PNG tree and preserves source namespaces and themes. An unused library definition still uses the ordinary library_alias query; it is outside the bound document's calculation. Generated children cannot be edited as stored overrides; edit their owning script instead.

Inspect get_document capabilities before detaching or importing a component. Stored and inherited instance detachment keeps nested references and variables. External library references stay linked, moved asset paths keep their original targets, and library variables use their original namespace. Detachment checks the resolved result before saving. External scripts retain their linked execution context through versioned metadata; treat unsupported scope-preservation diagnostics as a failed atomic batch. Keep a reference when the user wants future changes to the component definition to propagate.

An omitted property inherits its value. Removing an override restores inheritance; null is not a generic reset command. A descendant override with type is a full node replacement. Replacing children replaces the array, including the empty array; it does not merge old children. Slot suggestions help discovery but do not prohibit other valid content.

Use client_ref for a newly created element that later operations need. For example, an insert can declare client_ref `button`, and a later insert can set ref to {"result_of":"button"}. Use the returned bindings for later requests. Keep related structural changes in one batch so missing references are never persisted midway through the operation.

External script detachment retains its execution and input documents through versioned lazypencil metadata. Keep that metadata and its declared library dependencies when editing the detached node. This does not make a standalone copy of the code or library, and Pencil round-trip rendering of this extension is unverified. Use ordinary source/input edits for a new context; do not manually fabricate binding hashes.

Read the current revision and dependency revision before planning an edit. On conflict, read again and reconcile the user's intent with the actual document. Retry an ambiguous saved request with the same request ID and payload; use a new ID for a different edit.

Use import_component with an explicit mode. link creates a reference to a local or library definition; a file source also registers its import alias. copy creates local reusable definitions and closes their static component, variable, theme, image, and shader dependencies. The returned id is the placed instance; component_id identifies its reusable definition. Copied definitions are stored in a disabled frame so they do not add visible template artwork. Reuse one imported definition across repeated controls instead of importing it for every instance.

Sources can select a declared library with {"library_alias":"ui","node_id":"button"}, a project file with {"uri":"ui.lib.pen","import_alias":"ui","node_id":"button"}, or a bundled style with {"style_id":"standard-ui","node_id":"button"}. Read the style's editable example before selecting an ID. Use expected_source_revision when the source document hash is known. File sources remain within allowed project roots. copy leaves no new source import; link keeps the source live.

Copy name collisions fail by default. variable_prefix and component_prefix can give a kit its own names. Per-source variables, components, and themes maps can explicitly rename a source name or reuse a local target. Error details identify the source_key. Reused variables require the same type; reused themes must contain all source values. String-valued shader image samplers with local relative URLs can be copied independently. The copy retains the root variable/alias/theme graph and moves only its literal image values into small ordinary imported .pen libraries alongside captured images. Text uses retain the exact original string; changing the copied variables later still changes both text and image uses. Copy captures all theme alternatives. Explicitly reused consumer variables keep their consumer file bindings. Relative parent traversal and required intermediate directories are preserved. These immutable resource bundles have hashed inventories and use the same guarded publication as other managed assets. Copying scripts and reusing a contained definition remain unsupported. Absolute string image paths, nonlocal image inputs, source symlink semantics that collapse distinct assets onto conflicting projected paths, and paths colliding under case-insensitive names are rejected rather than silently changing text or images. Do not bypass these diagnostics by flattening a component or substituting an unrelated asset.

For example, a batch can import a bundled button, then place its returned instance:

```json
[
  {"op":"import_component","source":{"style_id":"standard-ui","node_id":"button"},"mode":"copy","options":{"variable_prefix":"kit.","component_prefix":"Kit "},"client_ref":"primary"},
  {"op":"update","target":{"result_of":"primary"},"set":{"x":40,"y":40}}
]
```

Inspect the imported structure, measure layout, and render the result. Source libraries and original asset files are never edited by an import. A dry run validates staged bytes without creating managed files or document history. A committed request can be replayed even after its original source is unavailable.
