---
name: review-guidance
description: Review lazypencil's complete agent guidance for redundancy, contradictions, task-specific bias, and suitability for lazypencil. Use for comprehensive guidance reviews, including repeat reviews after edits, rather than ordinary code or .pen visual reviews.
---

# Review lazypencil guidance

Review every guide against the current repository and the user's requested scope. A review request authorizes inspection and findings; change files only when the user also requests improvements or fixes.

## Establish coverage

- Read the current repository instructions and working-tree status. Review current files, including uncommitted changes, rather than relying on earlier conversation summaries or previous findings.
- Enumerate the guide catalogue in `src/guidance.rs` and read every guide body under `resources/guidance/` in full. Include transcluded content such as `resources/examples/button-operations.json`. Read large files in successive chunks; keyword searches alone do not constitute a full review.
- Check the surrounding instructions that affect interpretation: MCP startup instructions in `src/mcp.rs`, appended project guidance in `src/guidance/project.rs`, and style instructions in `resources/styles/manifest.json`. Follow referenced implementation or documentation where a claim needs verification.
- Keep a short coverage inventory for the response. Repository locations can change; follow the current catalogue and callers if these paths have moved. Do not create a tracked audit report or task diary.

## Apply the four review criteria

### Redundancy

Look for repeated instructions, warnings, negative phrasing, unnecessary internal detail, and examples that add no useful decision guidance. Assess both individual guides and the set as a whole. Account for guides being read independently: repeating a critical precondition can be justified. Prefer clear ownership and useful cross-references to indiscriminate deletion or moving all detail into the entry guide.

### Contradictions and missing conditions

Compare general rules, exceptions, examples, catalogue summaries, style instructions, and MCP startup instructions. Distinguish supported behavior, defaults, recommendations, and mandatory requirements. Check whether apparently conflicting advice applies to different tasks or read modes before calling it a contradiction.

Verify suspected API or behavior mismatches against the implementation and relevant existing tests. Pay attention to stored versus calculated data, definitions versus instances, source namespaces, update versus reset behavior, and structural versus visual verification. A passing test can support a specific behavior; it does not establish that the surrounding prose is correct.

### Bias toward a use case or the user's earlier requests

Check whether a specific example, previous failure, or preference has become an unjustified universal rule. Consider tasks outside the motivating example and the cost imposed on small edits or information queries. Dedicated task guides may be specific; their recommendations should remain scoped to those tasks. Distinguish a useful optional starting point from a compulsory visual style or workflow.

Evaluate the assistant's own previous changes by the same standard. Do not assume another review must produce more findings, or reverse a justified rule merely because the user asks whether it is biased.

### Suitability for lazypencil

Judge instructions against lazypencil's actual .pen semantics, tools, rendering, dependencies, and configuration. Shared terminology or inspiration from another application is not itself a defect. Identify concrete unsupported assumptions, unavailable operations, or workflows that do not fit this implementation. Distinguish implemented file-format support from unsupported competitor assumptions, and follow repository policy for compatibility changes. Consult competitor material only when it is necessary to resolve a specific factual question.

## Report evidence and recommendations

Report in Japanese, with findings ordered by their practical impact. For each actionable finding, give the current file and line, the affected situation, the consequence, and a focused correction. Distinguish confirmed behavior mismatches from ambiguous wording and optional editorial improvements. Avoid speculating about the author's motives or manufacturing a target number of issues.

Briefly state what was covered, including files without material findings. If no issues remain, say so with the limits of the inspection. State which checks were actually run; prose review alone does not require builds, full CI, installation, or application restarts. Do not imply exhaustive runtime verification from reading source or seeing existing tests.
