# Layout and spacing

Establish the main content regions and the dominant reading direction before positioning detail. Use frames with explicit horizontal or vertical layout for ordinary interface structure. Reserve absolute positioning for intentional overlap, badges, illustration layers, or objects that must stay outside normal flow.

Use a short spacing scale consistently: 4, 8, 12, 16, 24, 32, and 48 are a useful starting point. Separate related controls by less space than independent sections. Align text baselines, card edges, and repeated controls. Avoid compensating for a wrong parent layout with unrelated offsets on many children.

Specify important dimensions explicitly. A fill child in the main axis shares the remaining inner space after fixed siblings and gaps. A content-sized parent and fill-sized child can form an unresolved dependency; choose an intrinsic size, an explicit fallback, or a fixed parent constraint. Do not hide the problem with a large arbitrary size.

Auto-growing text does not wrap. For a constrained paragraph, set textGrowth to fixed-width and provide a usable width. Then let its measured height participate in the parent's content height. Use fixed-width-height only when the fixed box is intentional and inspect overflow.

## Compose for the available width

Assign each region a sizing role: fixed dimensions for elements that must retain their size, content dimensions for intrinsically sized elements, and fill for regions that can use the remaining space. Account for the parent's padding and gaps before sharing width. Preserve readable type and the content required for the task.

When a horizontal arrangement becomes crowded, decide which text can wrap and which supporting regions can move below the main content. Use a vertical frame or separate row frames to express that reading order. The layout engine does not automatically wrap children, shrink fixed siblings, or switch direction at breakpoints. Author the structure for the requested width; create additional variants only when they are part of the task. Avoid shrinking all type or removing supplied content to preserve a wide arrangement.

This insertion batch shows the same reading content and supporting visual in a wide row and a narrow stack. The wide paragraph shares space with a fixed-size visual; the narrow paragraph and visual use the available width. Both parents grow to their content height. The colored rectangle stands for a supporting visual; replace it with meaningful artwork when the task calls for it.

```json
[
  {"op":"insert","client_ref":"wide","node":{
    "type":"frame","name":"Wide composition","width":640,"height":"fit_content",
    "layout":"horizontal","padding":24,"gap":24,"fill":"#FFFFFF",
    "children":[
      {"type":"text","name":"Wide reading content","textGrowth":"fixed-width","width":"fill_container","fontFamily":"Noto Sans JP","fontSize":16,"lineHeight":1.5,"fill":"#0F172A","content":"調査では、利用者が必要な情報を見つけやすい構成を優先しました。関連する説明は本文にまとめ、補助的な図は読み順を保って配置します。"},
      {"type":"rectangle","name":"Wide supporting visual","width":160,"height":120,"fill":"#E2E8F0"}
    ]
  }},
  {"op":"insert","client_ref":"narrow","node":{
    "type":"frame","name":"Narrow composition","x":704,"width":280,"height":"fit_content",
    "layout":"vertical","padding":24,"gap":16,"fill":"#FFFFFF",
    "children":[
      {"type":"text","name":"Narrow reading content","textGrowth":"fixed-width","width":"fill_container","fontFamily":"Noto Sans JP","fontSize":16,"lineHeight":1.5,"fill":"#0F172A","content":"調査では、利用者が必要な情報を見つけやすい構成を優先しました。関連する説明は本文にまとめ、補助的な図は読み順を保って配置します。"},
      {"type":"rectangle","name":"Narrow supporting visual","width":"fill_container","height":120,"fill":"#E2E8F0"}
    ]
  }}
]
```

For interface-specific label and control patterns, read `components`; input fields are covered by `forms`.

Inspect get_layout after structural changes. A shadow's extent and a layout box are different measurements. Overlap may be intentional; clipped text or a displaced primary action usually needs correction. Scale the preview after layout rather than changing document coordinates to fit terminal cells.
