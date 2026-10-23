# Design workflow

Begin with the user's purpose, audience, content, target size, language, and reference images. State only assumptions that affect the result. Keep user-facing messages and design copy in the requested language; these English guides do not change that language.

1. Read the document, local nodes, variable definitions, reusable components, and available fonts. Query `list_icons` when adding icons and use advertised names and installed packs. Render an existing design before changing its visual conventions. Identify the region to change and the areas to preserve.
2. Read the relevant guides. Choose an available style for a new design when it fits the task. Read any trusted project_guides returned with this workflow, and follow the user request before project rules, selected style, and standard guidance. A project trust grant must come from the user; do not grant it yourself. For an existing design, prefer its own tokens, spacing, typography, and components. A style selection is a reference; it does not modify the document.
3. Define the information hierarchy and a small set of color, spacing, and type roles. Use real copy and representative data. Build the main regions first, then reusable elements and detail.
4. Apply small atomic batches using current document and dependency revisions. Use returned IDs; never invent a stored ID. Use client_ref and result_of when later operations in a batch need a newly created node.
5. Inspect computed layout and overflow diagnostics. Render the whole target and inspect smaller regions when needed. Correct wrapping, alignment, spacing, contrast, and content problems. Keep refinement within the user's time and iteration limits.
6. Report the changes, preview, saved revision, and unresolved diagnostics. Distinguish a completed visual inspection from a render that was unavailable. Do not promise visual quality from structural validation alone.

Read `layout`, `typography`, `color`, `components`, `effects`, and `verification` as needed. `scripts` covers generated geometry and runtime dependencies; `browser` and `shaders` cover optional Chromium rendering. `operations` contains a tested editing batch. Task-specific advice is in `app-screen`, `dashboard`, and `content-page`. Query the current tool list and document capabilities before using a feature; do not invent a tool or claim an unsupported feature worked.
