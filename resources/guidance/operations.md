# Document inspection and editing

## Inspect the relevant data

Read get_document for current revisions and compact metadata. Request include_configuration:true for full settings and origins, or include_capabilities:true for feature support. collections reports totals and truncation; omitted_fields identifies oversized optional details. For a needed collection, pass collection and limit, then repeat collection with each next_cursor until the required data is read. Use all pages before making a document-wide claim.

Use search_nodes with a specific name/type to find shallow candidates, then pass the returned read_target to read_nodes. Read known targets directly. Detail reads require node_id and default to depth:0; request only the descendant depth needed. Preserve the candidate's view:

| View | Read arguments |
| --- | --- |
| Stored | node_id; preserve library_alias from the search response when present |
| Expanded instance | node_id and instance_path, expand_instances:true, and library_alias from the search response when present |
| Calculated, including generated content | node_id and instance_path from read_target, plus snapshot_id; omit expand_instances and library_alias |

Calculated reads can execute scripts when no retained snapshot is supplied. Use stored reads for code repair without execution. `verification` covers snapshot lifetime and recovery. Metadata cursors and calculated IDs are scoped to their document, connection, and captured inputs. On pending-request RESOURCE_LIMIT, let an admitted request finish or cancel it before adding more work.

## Save an atomic batch

Use apply_operations with operations, a new request_id, expected_revision, and expected_dependency_revision when dependencies require it. Use client_ref for a newly created element and {"result_of":"client_ref"} in later operations of that batch. Returned bindings identify stored nodes for later batches. Keep related structural changes in one batch so incomplete references are never saved.

On a revision conflict, read current data and reconcile the intended edit. Retry an ambiguous saved request with the same request ID and payload; a different edit needs a new ID. Use the advertised operation schema, not an unrelated DSL. This guide also returns operation_schema with the complete input schema when a client abbreviates nested tool types. Node and variable definitions are JSON objects, not JSON-encoded strings. Follow `verification` for runtime dependency preconditions and checks appropriate to the change.

## Choose the edit operation

Use `update` with `target` for stored properties. It cannot replace `id`, `type`, `children`, or `descendants`. Use `insert`, `delete`, and `move` for stored structure. There is no general `replace` operation. For inherited instance properties use `set_override` with the stored instance ID and its `instance_path`; use `replace_override` or `set_slot_children` for inherited structure. Read `components` for slot requirements.

Each example below is an operations array. Substitute IDs from current reads and submit it with the current request revisions.

Change a stored label:

```json
[{"op":"update","target":"LABEL_ID","set":{"content":"Continue"}}]
```

Change a label inside an existing instance without changing the shared definition:

```json
[{"op":"set_override","target":{"node_id":"INSTANCE_ID","instance_path":["LABEL_ID"]},"set":{"content":"Continue"}}]
```

Replace a stored region atomically. Read its parent, sibling index, and references first; update references to removed nodes in the same batch where needed. This example assumes no other references to the removed region:

```json
[
  {"op":"delete","target":"OLD_REGION_ID"},
  {"op":"insert","parent":"PARENT_ID","index":0,"client_ref":"region","node":{"type":"frame","name":"Main workspace","width":640,"height":480,"children":[]}},
  {"op":"update","target":{"result_of":"region"},"set":{"fill":"#FFFFFF"}}
]
```

The inserted region receives a new ID. Use the returned `bindings.region` for subsequent reads. Recalculate layout and refocus the connected preview on the new region after inspecting it. Old descendant IDs cannot identify new children. After inserting an instance, use its returned binding in a subsequent batch for descendant overrides; `node_id` accepts a string, not a nested `result_of` object.

## Image assets and paint updates

Import a registered reference image through `put_asset` with `kind: "image"` and `source: {"attachment_id": "ID_FROM_GET_DOCUMENT"}`. Use the returned `uri` in an image paint, for example `{"type":"image","url":"RETURNED_URI","mode":"contain"}`. Referencing a path in a prompt does not import an image.

The serialized image modes are `cover` (fill the shape while preserving aspect), `contain` (show the whole image while preserving aspect), and `stretch` (fit both dimensions). Omitted mode means `cover`. UI labels such as Fill and Fit are not serialized mode values.

`update.set` replaces each named property's entire value. It does not recursively merge a paint object. To change image sizing, send the complete paint again, including `type: "image"`, the existing `url`, and the new `mode`. A partial paint such as `{"mode":"contain"}` removes its type and source and cannot be rendered. Read the existing paint before changing it. Use `unset` to remove a property rather than replacing it with an incomplete object.

## A complete editing example

This batch creates a color token, a reusable button definition, and a linked instance of that button. Use it in a new document; the token name is illustrative and should not overwrite an existing project's token without checking it first.

Submit this array as operations using the request fields above. The CLI accepts it in the file passed to --operations. The server allocates stored IDs and returns bindings for button and instance.

Additional linked buttons can reference the returned button definition ID. Follow `components` for child addresses, overrides, and placement of definitions outside screen content.

For this visible example, call get_layout with the returned revisions and render using its snapshot_id. Inspect the image and keep the user's language when adapting the label.
