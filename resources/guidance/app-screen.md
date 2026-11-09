# App screens

Identify the user's current task and its primary action. Establish navigation, page context, main content, and supporting actions. Give the main content enough space before refining decorative details. Use a consistent control height, label style, focus or selection convention, and spacing between related fields.

Use existing components and tokens when editing an application. Distinguish primary, secondary, and destructive actions through both labeling and appearance. Design the state the user requested, with representative data and explicit empty, error, or loading content when those states are part of the task.

Choose the composition from the task. An editor may give most space to a canvas with contextual tools; a list may prioritize search, rows, and selection; a detail view may group information and related actions. Reuse the application's navigation and reading order rather than treating a settings form as the default for every screen.

At the requested width, preserve the main content and primary action. Use `layout` to decide which regions can shrink, which text can wrap, and which groups should stack. Recompose crowded regions instead of shrinking all type to preserve a wider arrangement.

Read `components` for interface control patterns. When the screen includes input fields, read `forms` for field grouping, message states, and single-line versus multiline values. Choose only the patterns needed for the requested screen.

Start a new screen with standard-ui, or dense-workspace for an information-heavy working environment. Use the style as a coherent starting point; preserve stronger brand or reference constraints. Inspect long labels, validation messages, table cells, and the location of the primary action at the target size.
