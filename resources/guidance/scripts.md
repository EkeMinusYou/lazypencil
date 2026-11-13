# Script-generated designs

Use a script for parameterized geometry, charts, or repeated structures when ordinary components would be awkward. When editing an existing script, read its node and captured source first. read_asset includes script_header metadata or a header diagnostic. Change saved inputs or publish new immutable source with put_asset, then update scriptUri through apply_operations.

## Author source and inputs

Start new source with a header comment declaring @schema 2.22. Existing @schema 2.21, @schema 2.19, and legacy @schema 2.11 headers are accepted without rewriting the source. Header compatibility is separate from supported document versions; it does not enable 2.11 document files.

Declare inputs with @input name: type = default. Supported types are number, string, boolean, color, enum, and ref. Defaults are JSON values; an unquoted hexadecimal color is also accepted.

Missing or null inputs use the declared default. Without one, numbers use 0 clamped to their limits, strings and refs use an empty string, booleans use false, colors use #000000, and enums use their first option. These values are transient and are not written to the document or header. Saved keys absent from the header remain in the file and are excluded from runtime input, including variable lookup. Declared values of invalid types and enum values outside the declared options are errors.

The script body receives pencil.width, pencil.height, and pencil.input and must synchronously return a node array. Use document variables in the stored inputs to bind values to themes. A nonempty ref input must select an existing reusable component; an empty ref means no selection, so branch or supply fallback geometry before generating a component instance. A null override resets to the source header's default, whose component is resolved in the script's own library. Explicit nonnull selections retain their input document's scope. Output may use existing component references, but cannot declare reusable components. Stored and generated children have separate identities: generated layout records include generated_by. To change their design, edit that source node or its code and inputs.

## Read generated results

Use search_nodes with calculated:true to discover generated content. Pass its returned read_target and snapshot_id to read_nodes, following search pages with that same snapshot and cursor. A known target can also be read with calculated:true or a retained calculation from layout, rendering, or validation. The view contains resolved properties, sources, themes, generated_by, and matching layout records.

Generated IDs are read-only: follow generated_by to the owning script and edit its source or inputs. For code repair without execution, omit calculated and snapshot_id and read the stored owner. Follow `operations` for read modes and depth, and `verification` for snapshot reuse.

## Runtime and sizing

Execution has no DOM, filesystem, network, timers, or host module loader. Each request starts with a fresh JS context. Math.random uses Mulberry32 with seed 0 and restarts its sequence on each evaluation, independently of dimensions and inputs.

Defaults are 2 seconds and 64 MiB per evaluation, with a fixed 1,000-node output cap including owned nested children. Captured runtime.javascript settings control evaluation time, heap, idle reaping, layout budget and evaluation/pass counts. Request get_document with include_configuration:true to inspect configuration.settings.runtime, and inspect script_executions[].limits when diagnosing a resource failure. These limits are host policy, separate from pencil.input. Exceptions, allocation failures, cycles, deadlines, and invalid output fail the calculation; catching an interpreter interruption does not make it successful. Reduce excessive work when it exceeds these limits.

Prefer definite sizes or parent fill sizes for scripts that read their dimensions. A content-sized dimension that the script reads needs an explicit fallback, such as fit_content(200). A script that does not read its dimensions can fit independent output geometry. Nonconvergent layouts stop with a diagnostic. Script evaluation and generation are bounded; do not solve a failed cycle by inventing an arbitrary fixed size.

## Diagnose failures

On a header or JavaScript failure, inspect error.details.script_diagnostic and the captured node, script_path, and source_hash. Coordinates are one-based original-source lines and UTF-8 byte columns, including the header. Excerpts contain at most three lines of 240 characters each; start_column and truncation flags identify clipped windows. Distinguish the throwing source, a dynamic-code call_site, a header declaration, and end_of_source. A null position means no usable location was reported; do not invent a line. Exception stacks can be replaced by script code, so treat positions as debugging hints. Read the captured source to repair inputs or publish a corrected source, then recalculate and inspect the result.

When a nested failure returns failure_capture_id, call read_asset with source_hash on the same MCP connection to read its full original source in UTF-8 byte chunks. Follow next_offset until it is null. failure_capture_id identifies a failed-code capture, not a usable layout snapshot. Captures expire after five minutes and retain at most four code files. Changed documents, dependencies or path targets invalidate the capture; recalculate to obtain a current diagnostic. read_asset cannot open arbitrary paths, and a header error does not prevent reading the captured source. If a capture is unavailable, use the declared owner/source for repair through ordinary document operations.

## Copy scripts and track dependencies

import_component with mode copy preserves executable code and inherited input/code/metadata overrides without running the script or freezing output. Follow `components` for asset manifests, name collisions, and field-context behavior; keep private binding metadata when preserving a copied context.

get_layout returns script execution hashes and runtime_dependencies for files discovered in generated content. Ordinary dependency_revision covers document-declared dependencies. Snapshots capture the runtime dependency bytes too; a changed runtime file invalidates the snapshot. Use the returned snapshot_id to inspect that output and follow `verification` when editing from it.
