# Layout and spacing

Establish the main content regions and the dominant reading direction before positioning detail. Use frames with explicit horizontal or vertical layout for ordinary interface structure. Reserve absolute positioning for intentional overlap, badges, illustration layers, or objects that must stay outside normal flow.

Use a short spacing scale consistently: 4, 8, 12, 16, 24, 32, and 48 are a useful starting point. Separate related controls by less space than independent sections. Align text baselines, card edges, and repeated controls. Avoid compensating for a wrong parent layout with unrelated offsets on many children.

Specify important dimensions explicitly. A fill child in the main axis shares the remaining inner space after fixed siblings and gaps. A content-sized parent and fill-sized child can form an unresolved dependency; choose an intrinsic size, an explicit fallback, or a fixed parent constraint. Do not hide the problem with a large arbitrary size.

Auto-growing text does not wrap. For a constrained paragraph, set textGrowth to fixed-width and provide a usable width. Then let its measured height participate in the parent's content height. Use fixed-width-height only when the fixed box is intentional and inspect overflow.

## Control labels and alignment

Use textGrowth: "auto" for an unconstrained single-line label and omit its width and height. The parent control may still have a fixed width, such as equally sized actions; a content-sized label does not require a content-sized button. Use a constrained text box when long names must wrap or fit an established width. Preserve the user's or existing design's alignment conventions.

- For an ordinary button, use horizontal layout, a small gap, symmetric horizontal padding, and center alignment on both axes. Center the icon and label as one group, rather than centering only the text inside a leftover text box.
- For breadcrumbs, use a content-sized horizontal row with explicit gaps between labels and separators. Let a changed label move the following separator and item through layout, rather than recalculating their x coordinates.
- For a compact sort action, keep the label and chevron together in a content-sized horizontal row with a gap. A wide select field has a different role: its constrained label may fill the available space and keep the chevron at the far edge. Do not apply one alignment rule to both.

For example, this button derives its width from its icon, label, gap, and padding. Setting a fixed width on the frame would keep the same group centered without giving the label a guessed width:

```json
{
  "type": "frame", "name": "Add member", "layout": "horizontal",
  "width": "fit_content", "height": 44, "padding": [0, 16], "gap": 8,
  "justifyContent": "center", "alignItems": "center", "fill": "#2563EB",
  "children": [
    {"type": "icon", "name": "Add member icon", "library": "lucide", "icon": "user-plus", "width": 16, "height": 16, "fill": "#FFFFFF"},
    {"type": "text", "name": "Label", "content": "メンバーを追加", "textGrowth": "auto", "fontFamily": "Noto Sans JP", "fontSize": 14, "fill": "#FFFFFF"}
  ]
}
```

Adapt the colors, type, and spacing to the document. The `standard-ui` style includes editable, importable examples of these three control patterns; see `components` for their IDs.

Inspect get_layout after structural changes. A shadow's extent and a layout box are different measurements. Overlap may be intentional; clipped text or a displaced primary action usually needs correction. Scale the preview after layout rather than changing document coordinates to fit terminal cells.
