# App screens

For a new screen or substantial redesign, express its primary user task in one sentence. Identify the main object being read or edited, frequent actions, and supporting information. Allocate space and group related actions around that task before refining typography, color, borders, and decoration. Use consistent control heights, label styles, selection cues, and spacing for related controls.

Use existing components and tokens when editing an application. Read `components` when controls should share design changes, and `forms` for input fields. Distinguish primary, secondary, and destructive actions through labeling and appearance. Represent the requested states, with realistic data and explicit empty, error, or loading content when those states are part of the task.

Choose composition from the content: an editor may emphasize a canvas and contextual tools; a list may prioritize search and rows; a detail view may group information and actions. For focused edits, preserve the surrounding navigation and reading order. When the user requests a broader design improvement and the current arrangement obscures the primary task, reconsider region placement and proportions within that scope. Retain useful conventions and avoid imposing the same sidebar-and-canvas layout on unrelated tasks.

Check domain relationships that help users understand the content: units, ordering, shared axes, alignment, and mappings between controls and their effects. Decorative plausibility does not establish that a chart, timeline, map, or editor communicates these relationships correctly.

For multiple screens, improve representative task and entry screens first, then inspect the effects of shared components and tokens at relevant sizes and themes.

Use `layout` to decide which regions can shrink, which text can wrap, and which groups should stack at the requested width. Inspect long labels, validation messages, table cells, and required actions at that size.

For a new visual direction, standard-ui and dense-workspace are optional starting points for conventional or dense interfaces. Select them only when they suit the task; existing conventions, brand constraints, and reference material guide the choice.
