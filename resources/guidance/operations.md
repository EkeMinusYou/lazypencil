# A complete editing example

This batch creates a color token, a reusable button, and a second instance of that button. Use it in a new document; the token name is illustrative and should not overwrite an existing project's token without checking it first.

Read get_document first. For apply_operations, put this array in operations, supply a new request_id and the document's current expected_revision, and include expected_dependency_revision when required. The CLI accepts the same array in the file passed to --operations. The server allocates stored IDs and returns bindings for button and instance.

After saving, call get_layout with the returned revision and dependency revision, then render using its snapshot_id. Inspect the actual image before making further adjustments. Keep the user's language when adapting the sample label. Do not send Pencil-specific DSL commands or modify serialized IDs by hand.

## Image assets and paint updates

Import a registered reference image through `put_asset` with `kind: "image"` and `source: {"attachment_id": "ID_FROM_GET_DOCUMENT"}`. Use the returned `uri` in an image paint, for example `{"type":"image","url":"RETURNED_URI","mode":"contain"}`. Referencing a path in a prompt does not import an image.

The serialized image modes are `cover` (fill the shape while preserving aspect), `contain` (show the whole image while preserving aspect), and `stretch` (fit both dimensions). Omitted mode means `cover`. UI labels such as Fill and Fit are not serialized mode values.

`update.set` replaces each named property's entire value. It does not recursively merge a paint object. To change image sizing, send the complete paint again, including `type: "image"`, the existing `url`, and the new `mode`. A partial paint such as `{"mode":"contain"}` removes its type and source and cannot be rendered. Read the existing paint before changing it. Use `unset` to remove a property rather than replacing it with an incomplete object.
