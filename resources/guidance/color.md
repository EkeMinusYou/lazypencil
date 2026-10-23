# Color, emphasis, and states

Assign colors to roles: canvas, surface, primary text, secondary text, boundary, primary action, and semantic status. Store repeated roles as variables and reuse them. Make the primary action easy to locate without making every control equally prominent.

Use surface and spacing differences to separate regions before adding stronger borders or shadows. A restrained palette usually makes hierarchy easier to maintain. Gradients and decorative effects should support the intended visual direction and content, rather than substitute for structure.

Check text against its actual background, including image fills, transparency, and gradients. Inspect small labels and muted text at the final display scale. Do not communicate success, warning, selection, or error through color alone; add a label, symbol, border, or other visible cue as appropriate.

Respect existing theme axes and variable aliases. A component may inherit a theme and override individual axes. Inspect the resolved result in each theme the task requires. Changing a shared variable can affect many screens, so distinguish a document-wide token change from a local override.

The supplied styles are starting palettes. Adapt them deliberately when the user's brand or reference requires it, while preserving readable contrast and consistent roles.
