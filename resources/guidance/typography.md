# Typography and multilingual copy

Use typography to express hierarchy. Choose a few clear roles, such as page title, section title, body, label, and supporting text. Keep their sizes and weights consistent across the document. The standard-ui style starts with 32/24/20 for headings, 16 for body, 14 for supporting copy, and 12 for short labels. These are defaults for that style, not universal rules.

Noto Sans JP is bundled. Use another family only when it is available or the user supplies its font file. A missing font or missing glyph is a real diagnostic; do not silently claim the original typography was reproduced. Extra fonts and different font weights can change wrapping and control sizes.

Read the project's font settings in `get_document.configuration` and call `list_fonts` before selecting supplied fonts. Honor its default family, language, and declared fallback chain when creating text, while preserving explicit families in existing nodes. A collection can contain several families and styles; inspect its face indexes, weights, slants, and axes. Without a configured chain, glyph fallback uses the requested family, Noto Sans JP, then other explicitly supplied families in file order. Missing requested families remain errors. `get_layout` text metrics identify the requested family, locale, and faces actually used by shaped glyph runs, including source/face hashes and synthetic styling. Check these records when mixed scripts or combining marks require fallback. The full emoji and custom variation-axis compatibility audit is unfinished; a successful structural validation does not establish that every requested glyph sequence can be reproduced.

Use real text in the intended language. Do not substitute English for Japanese because the guides are English. Check punctuation, mixed scripts, line breaks, and long labels. Avoid fixed line breaks that only disguise a poor width unless the break is part of the intended copy.

For body text, a line height around 1.5 is a useful starting point; headings can be tighter. Set textGrowth explicitly when width matters. Read measured line count and height from get_layout, then inspect the image for reading rhythm, baseline alignment, and comfortable line length. A visually centered label may require checking the actual font metrics rather than assuming its bounding rectangle describes the glyphs perfectly.

Prefer concise, specific labels. Preserve supplied copy unless editing it is part of the task. Do not turn a dense dashboard into uniformly tiny text to force everything onto one screen.
