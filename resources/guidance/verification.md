# Verify the result

Use three complementary checks:

1. Structure: validate IDs, references, variables, themes, and the edit batch. Structural success does not prove that every graphics feature is supported or that the design looks good.
2. Geometry: inspect computed sizes, positions, line counts, baselines, font sources, overflow, and clipping. Use the same layout snapshot for measurements and image rendering when it is retained. If it expires, request a new calculation explicitly rather than pretending an old snapshot was reused.
3. Appearance: inspect the full target image and relevant close-ups. Check hierarchy, reading order, consistent spacing, text wrapping, repeated elements, visual balance, image crops, and the visibility of the main action.

Use `validate_document` with `scope: "layout"` to obtain measured diagnostics. It can run scripts, and its snapshot ID and returned revisions can be reused by `get_layout`, `render`, and later validation pages. Reuse the snapshot when paging diagnostics; an expired or changed calculation must be requested again explicitly. Read all relevant diagnostic pages. Error/warning counts cover the whole calculation, so errors can exist beyond the current page. Warnings do not automatically invalidate a document. `complete` only describes the requested checks; it does not certify every paint, browser capture, shader, or visual result.

When inspecting tokens, `get_variables` keeps raw definitions separate from resolved values and their source documents. Use a node address for its inherited theme, and `property` when its namespace matters, including consumer overrides of library properties. Explicit `theme` selections are hypothetical queries and do not edit the design. Preserve aliases in later edits unless the user requests a change to the definition. A successful page does not prove that every variable or the whole project is valid.

Check the target size and real content rather than a convenient empty state. For existing designs, compare the requested region with the initial preview and verify that unrelated regions retain their conventions. Check the required themes or variants separately.

When a calculation reports runtime_dependency_revision, include it as expected_runtime_dependency_revision in subsequent edits based on that preview, together with the ordinary document and dependency revisions. The service checks local files discovered by script execution before saving. If the precondition fails, obtain and inspect a fresh calculation. This does not freeze external web content; inspect acquisition IDs and explicitly refresh pages when needed. Structural repairs to broken scripts can omit the runtime precondition, but must not be described as based on a current visual check.

If a font, asset, runtime, or rendering capability is unavailable, report the diagnostic and its effect on the result. Do not replace the feature silently or claim visual inspection without an image. Stop refinement at the agreed limit and report remaining issues alongside the saved result.

For a document with missing declared local image or code files, inspect `asset_diagnostics` and retain both ordinary revision preconditions. Upload replacement assets and update their references, or remove the affected nodes/properties. Existing missing references may remain during partial repairs; do not introduce new missing targets or resource kinds. A saved structural repair with `assets_complete: false` still cannot be previewed. `assets_complete: true` describes declared-file availability, not rendering compatibility or successful code execution. A null status or `dependency_error` means discovery was incomplete. Reload after a missing file appears or a path is redirected. Omit the optional runtime precondition when the existing code cannot run, and perform a fresh visual check after repair.

Do not treat a successful screenshot as proof of parity with another design product. That requires a separate evaluation with agreed tasks and conditions.
