# Form fields and messages

For screens with input fields, treat each label, input surface, supporting instructions, and validation message as one related group. Use existing field components first. A useful starting point is vertical layout with a small gap, a constrained width, and content-sized height:

```text
Field group (vertical layout, fit_content height)
  Label (wraps at the field width when needed)
  Input surface (fills field width, padding positions its value)
  Supporting instructions (optional, wraps and grows in height)
  Error message (optional, wraps and grows in height)
```

Let the surrounding form's layout position the next field after the group's actual height. Use `typography` for constrained text boxes and `components` for control alignment. Adapt input height, gaps, and type to the document. Horizontal forms may need a different grouping. Preserve intentionally reserved message space when the design calls for stable alignment across fields.

Supporting instructions and errors can appear together. Keep instructions that remain useful while showing a specific error, and distinguish the error with text as well as styling. For an absent message, set that message node's `enabled` to false so it leaves normal flow; empty text or opacity alone may retain unwanted space.

## Single-line values

When no existing field component fits, get_style with id `standard-ui` provides `form-field`. Its input has a fixed height and an inner clipping frame, `field-value-viewport`, inside the horizontal padding. The value uses `textGrowth: "auto"`, preserving the full string on one line. Long values are clipped at the inner viewport boundary, leaving the border, padding, and messages clear. Choose this treatment for a single-line input; do not wrap its value inside a fixed-height surface or shrink its font to fit.

Intentional horizontal clipping can report `LAYOUT_OVERFLOW` for `field-value` with `clipped: true`. Check that it is horizontal overflow of that value inside its viewport. Vertical overflow and overflow of other elements still need attention. Clipping depicts the visible portion of a static input; it does not add scrolling or editing behavior. Do not silently replace the stored value with a shortened string or claim automatic ellipsis.

The field's supporting and error nodes are independent; the error is disabled by default. The email instance shows both messages and an error border. Read the definition before overriding its label, value, messages, or visibility. Its width follows a layout parent, with a standalone fallback of 360; constrain the parent or set the instance width for the target layout.

## Multiline values

For a textarea or other multiline value, give the text a constrained width, allow wrapping, and let the input and field grow vertically. The following descendant property overrides adapt the bundled `form-field` structure to content-height multiline text:

```json
{
  "field-input": {"height": "fit_content", "padding": 12, "alignItems": "start"},
  "field-value-viewport": {"height": "fit_content", "clip": false, "alignItems": "start"},
  "field-value": {"textGrowth": "fixed-width", "width": "fill_container"}
}
```

Use the descendant addresses read from the actual imported instance. Keep the parent form in flow so later fields follow the resulting height. An intentionally fixed-height multiline field needs its own clipping or visible-overflow design; use the requested behavior rather than applying the single-line treatment to every input.
