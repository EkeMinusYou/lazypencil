# Layout and spacing

Establish the main content regions and the dominant reading direction before positioning detail. Use frames with explicit horizontal or vertical layout for ordinary interface structure. Reserve absolute positioning for intentional overlap, badges, illustration layers, or objects that must stay outside normal flow.

Use a short spacing scale consistently: 4, 8, 12, 16, 24, 32, and 48 are a useful starting point. Separate related controls by less space than independent sections. Align text baselines, card edges, and repeated controls. Avoid compensating for a wrong parent layout with unrelated offsets on many children.

Specify important dimensions explicitly. A fill child in the main axis shares the remaining inner space after fixed siblings and gaps. A content-sized parent and fill-sized child can form an unresolved dependency; choose an intrinsic size, an explicit fallback, or a fixed parent constraint. Do not hide the problem with a large arbitrary size.

Auto-growing text does not wrap. For a constrained paragraph, set textGrowth to fixed-width and provide a usable width. Then let its measured height participate in the parent's content height. Use fixed-width-height only when the fixed box is intentional and inspect overflow.

Inspect get_layout after structural changes. A shadow's extent and a layout box are different measurements. Overlap may be intentional; clipped text or a displaced primary action usually needs correction. Scale the preview after layout rather than changing document coordinates to fit terminal cells.
