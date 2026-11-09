# Typography and multilingual copy

Use typography to express hierarchy. Choose a few clear roles, such as page title, section title, body, label, and supporting text. Keep their sizes and weights consistent across the document. The standard-ui style starts with 32/24/20 for headings, 16 for body, 14 for supporting copy, and 12 for short labels. These are defaults for that style, not universal rules.

## Text roles and boxes

Choose the text box from its role and available space, rather than estimating dimensions for the current wording:

| Role | Text box |
| --- | --- |
| Unconstrained single-line action label | Content-sized text. Follow `components` for button and icon placement. |
| Heading or description with a constrained width | `textGrowth: "fixed-width"` and a usable width, often `fill_container` within a width-constrained layout parent. Let text height grow when it wraps. |
| Single character inside a text avatar | `textGrowth: "fixed-width-height"` with both dimensions set by the avatar, `textAlign: "center"`, and `textAlignVertical: "middle"`. |

Align the text node with its parent's layout. `textAlign` aligns lines inside the text box; it does not move the box within its parent. For an avatar, a fixed-size parent and a text child filling both dimensions provide a stable box when the character changes. The `standard-ui` style includes an importable `text-avatar` example; read its child before changing the character.

Use standard alignment first. A centered text box does not guarantee that every glyph appears optically centered. Do not turn an offset chosen for one character into a rule for all characters or fonts. Preserve established type styles and intentional optical adjustments when editing an existing design.

Use real text in the intended language. Do not substitute English for Japanese because the guides are English. Check punctuation, mixed scripts, line breaks, and long labels. Avoid fixed line breaks that only disguise a poor width unless the break is part of the intended copy.

For body text, a line height around 1.5 is a useful starting point; headings can be tighter. Read measured line count and height from get_layout, then inspect the image for reading rhythm, baseline alignment, and comfortable line length. Prefer concise, specific labels, but preserve supplied copy unless editing it is part of the task. Do not turn a dense dashboard into uniformly tiny text to force everything onto one screen.

## Font selection and availability

Noto Sans JP is bundled. Use another family only when it is available or the user supplies its font file. A missing font or missing glyph is a real diagnostic; do not silently claim the original typography was reproduced. Extra fonts and different font weights can change wrapping and control sizes.

If selected font files or search directories are absent, inspect `get_document.configuration.fonts_complete`, `font_diagnostics` and the document's asset diagnostics. Structural edits remain available, but inventory and rendering reject missing selected inputs. Preserve existing family names while the declared inputs are restored or their configuration is explicitly revised, then obtain fresh document/dependency revisions. Document edits do not install fonts or rewrite user settings automatically.

An optional project `font_lock` fixes font data, selection settings, the Skia dependency identity and the bundled pack. `FONT_LOCK_MISMATCH` means the current inputs or a newly selected face differ from that baseline; do not hide it with a substitute or silently regenerate the lock. Inspection and structural edits remain available. The explicit `fonts lock FILE` CLI captures all supplied faces and system faces observed by that document, including generated text. Changed contents require the reviewed `--expected-lock-revision`; `fonts check FILE` verifies the configured baseline. A missing lock remains a missing dependency until explicitly created. Browser CSS fonts and terminal text fonts have their own owners and are outside this lock.

Read the project's font settings in `get_document.configuration` and call `list_fonts` before selecting supplied fonts. Honor its default family, language, and declared fallback chain when creating text, while preserving explicit families in existing nodes. A collection can contain several families and styles; inspect its face indexes, weights, slants, and axes. Without a configured chain, glyph fallback uses the requested family, Noto Sans JP, then other explicitly supplied families in file order. Missing requested families remain errors. `get_layout` text metrics identify the requested family, locale, and faces actually used by shaped glyph runs, including source/face hashes and synthetic styling. Check these records when mixed scripts or combining marks require fallback. The full emoji and custom variation-axis compatibility audit is unfinished; a successful structural validation does not establish that every requested glyph sequence can be reproduced.
