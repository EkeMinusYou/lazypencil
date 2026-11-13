# Verify the result

## Choose checks for the change

For queries about stored data and edits limited to organizational names or nonvisual metadata, inspect the relevant stored result. Use validate_document with scope: "structure" when changes can affect IDs, references, variables, themes, or node structure. It does not execute scripts. A successful structural check establishes the requested structural properties, not visual quality or full graphics support.

For layout or visual questions, and changes to visible content, geometry, appearance, or dynamic behavior, use the relevant calculated evidence:

- Geometry: validate_document with scope: "layout" provides measured diagnostics and can execute scripts. Inspect affected sizes, positions, line counts, baselines, font sources, overflow, and clipping.
- Appearance: render and inspect the target image and relevant close-ups. Check hierarchy, reading order, spacing, wrapping, repeated elements, image crops, and any required actions. Compare existing designs with their initial view. Use real content, the requested size, and the themes or variants relevant to the change.

Respect the requested scope. A selected node does not narrow a document-wide inspection, and a local edit does not authorize unrelated repairs. Report the checks actually performed, remaining diagnostics, and any unavailable image or runtime. Stop refinement at the agreed limit.

## Shared components and variables

For elements intended to remain linked, inspect their references to the shared definition and intentional instance overrides. Matching pixels alone do not establish that link. After a shared change, inspect affected instances with representative content, sizes, and required themes. After a local override, confirm the definition and other instances retain their values. Structural validation does not detect every missed opportunity for reuse.

get_variables separates raw definitions from resolved values and source documents. Use a node address for its inherited theme, and property when the consumer or library namespace matters. A theme argument is a hypothetical query and does not edit the document. Check all variables relevant to the change; one returned page does not establish validity of the entire project.

## Diagnostics and pagination

Use validate_document independently of a UI or attached issue summary. Read current affected nodes before repairing a reported problem; captured summaries may be stale. Follow next_cursor with the returned snapshot_id for layout diagnostic pages. Read all pages needed for the requested scope, including every page for a document-wide inspection. Error/warning counts cover the whole calculation, even when findings are on later pages.

Warnings need interpretation: an intentional clipped input value can differ from an accidental clipped heading. UNKNOWN_PROPERTY identifies an unrecognized property, often a spelling error; its stored presence does not establish an effect. Arbitrary keys inside metadata and script inputs are not node-property warnings. The complete flag covers the requested checks, not every paint or dynamic acquisition.

For RESOURCE_LIMIT from component or variable expansion, see Expansion limits in [component-import](component-import.md); the budget applies to ordinary calculations as well as imports.

After a repair, rerun the applicable checks on current inputs. For layout validation, omit the pre-edit snapshot_id and cursor, then inspect the relevant diagnostic pages and images.

## Retained calculations

Reuse one returned snapshot_id for measurements, calculated reads, and rendering of the same inputs. A snapshot belongs to its MCP connection and can expire or be evicted; TUI preview IDs and earlier connections' IDs cannot substitute for it. Before parallel calculated reads, obtain one retained calculation and share its ID.

On SNAPSHOT_UNAVAILABLE, omit snapshot_id and cursor to obtain one fresh calculation, then restart the relevant reads or pages with its returned ID. Repeating the unavailable ID or starting many independent recalculations does not recover it. Keep edit batches sequential and use their latest revisions.

## Dependency preconditions

When a calculation reports runtime_dependency_revision, include it as expected_runtime_dependency_revision in edits based on that preview, together with the ordinary document and dependency revisions. This checks local files discovered by script execution. On a conflict, obtain and inspect a fresh calculation. Browser acquisitions have separate identities; refresh them explicitly when current web content is required.

Structural repairs to code that cannot run can omit the runtime precondition. Such an edit is based on stored structure, not a current visual check.

## Missing inputs

For missing declared local image or code files, inspect asset_diagnostics. Keep both ordinary revision preconditions while uploading replacements and updating references, or removing affected nodes/properties. Partial repairs may retain existing missing references, but cannot introduce new missing targets or resource kinds. Reload after a missing file appears or a path is redirected.

A saved repair with assets_complete: false cannot yet be previewed. True indicates declared-file availability, not rendering compatibility or successful execution. A null status or dependency_error means discovery was incomplete. Follow `typography` for missing fonts and font locks, or the corresponding dynamic guide for runtime failures. Once inputs are restored, perform a fresh calculation and visual check when the change affects the rendered result. Any requested comparison with another renderer needs its own reference images and evaluation conditions.
