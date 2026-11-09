# App screens

Identify the user's current task and its primary action. Establish navigation, page context, main content, and supporting actions. Give the main content enough space before refining decorative details. Use a consistent control height, label style, focus or selection convention, and spacing between related fields.

Use existing components and tokens when editing an application. Distinguish primary, secondary, and destructive actions through both labeling and appearance. Design the state the user requested, with representative data and explicit empty, error, or loading content when those states are part of the task.

## Form fields

Treat a field's label, input surface, supporting instructions, and validation message as one related group. A useful starting point is a vertical layout with a small gap, a constrained width, and content-sized height:

```text
Field group (vertical layout, fit_content height)
  Label (wraps at the field width when needed)
  Input surface (fills field width, padding positions its value)
  Supporting instructions (optional, wraps and grows in height)
  Error message (optional, wraps and grows in height)
```

Let the surrounding form's layout position the next field after the group's actual height. Do not reserve guessed message heights or move later fields by hand when copy wraps. Use `typography` for constrained text boxes and `layout` for alignment inside the input surface. Adapt input height, gaps, and type to the existing design; a multiline input or a horizontal form may need a different structure.

Supporting instructions and errors can appear together. Keep instructions that remain useful while showing a specific error, and distinguish the error with text as well as styling. For an absent message, set that message node's `enabled` to false so it leaves normal flow; empty text or opacity alone may retain unwanted space. Preserve intentionally reserved space when the design calls for stable alignment across fields.

When no existing field component fits, get_style with id `standard-ui` provides the reusable `form-field` example. It has separate supporting and error text nodes; the error is disabled by default. The example also shows an email field instance with both messages visible and an error border. Read the definition before overriding its label, value, messages, or message visibility. Its width follows a layout parent, with a standalone fallback of 360; constrain the parent or set the instance width for the target layout. This is an editable design example, not an interactive input or a validation engine.

Start a new screen with standard-ui, or dense-workspace for an information-heavy working environment. Use the style as a coherent starting point; preserve stronger brand or reference constraints. Inspect long labels, validation messages, table cells, and the location of the primary action at the target size.
