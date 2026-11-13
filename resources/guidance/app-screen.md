# App screens

Identify the user's task and the actions it requires. Establish navigation, page context, main content, and supporting actions where relevant. Give the main content enough space before refining decoration. Use consistent control heights, label styles, selection cues, and spacing for related controls.

Use existing components and tokens when editing an application. Read `components` when controls should share design changes, and `forms` for input fields. Distinguish primary, secondary, and destructive actions through labeling and appearance. Represent the requested states, with realistic data and explicit empty, error, or loading content when those states are part of the task.

Choose composition from the content: an editor may emphasize a canvas and contextual tools; a list may prioritize search and rows; a detail view may group information and actions. Preserve an existing application's navigation and reading order.

Use `layout` to decide which regions can shrink, which text can wrap, and which groups should stack at the requested width. Inspect long labels, validation messages, table cells, and required actions at that size.

For a new visual direction, standard-ui and dense-workspace are optional starting points for conventional or dense interfaces. Select them only when they suit the task; existing conventions, brand constraints, and reference material guide the choice.
