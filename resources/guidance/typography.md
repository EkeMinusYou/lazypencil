# Typography and multilingual copy

Use typography to express hierarchy. Choose a few clear roles, such as page title, section title, body, label, and supporting text. Keep their sizes and weights consistent across the document. If a style is selected, read its type tokens rather than applying one universal size scale.

## Text roles and boxes

Choose the text box from its role and available space, rather than estimating dimensions for the current wording:

| Role | Text box |
| --- | --- |
| Unconstrained single-line action label | Content-sized text. Follow `components` for button and icon placement. |
| Heading or description with a constrained width | `textGrowth: "fixed-width"` and a usable width, often `fill_container` within a width-constrained layout parent. Let text height grow when it wraps. |
| Intentionally fixed text box, such as a text avatar | `textGrowth: "fixed-width-height"` with deliberate width and height. For a centered avatar, fill its parent and use `textAlign: "center"` and `textAlignVertical: "middle"`. |

Align the text node with its parent's layout. `textAlign` aligns lines inside the text box; it does not move the box within its parent. For an avatar, a fixed-size parent and a text child filling both dimensions provide a stable box when the character changes. The `standard-ui` style includes an importable `text-avatar` example; read its child before changing the character.

Use standard alignment first. A centered text box does not guarantee that every glyph appears optically centered. Do not turn an offset chosen for one character into a rule for all characters or fonts. Preserve established type styles and intentional optical adjustments when editing an existing design.

Use real text in the intended language. Check punctuation, mixed scripts, line breaks, and long labels. Avoid fixed line breaks that only disguise a poor width unless the break is part of the intended copy.

For body text, a line height around 1.5 is a useful starting point; headings can be tighter. Read measured line count and height from get_layout, then inspect the image for reading rhythm, baseline alignment, and comfortable line length. Prefer concise, specific labels, but preserve supplied copy unless editing it is part of the task. Do not turn a dense dashboard into uniformly tiny text to force everything onto one screen.

## Font selection and availability

Noto Sans JP is bundled. When choosing a family for new text, request get_document with include_configuration:true, read configuration.settings.fonts, and call list_fonts to select an available face. Use the configured default_family, locale, and fallback chain for new text while preserving explicit families in existing nodes. Font collections may contain multiple families and styles; inspect face indexes, weights, slants, and axes. A missing font or missing glyph is a real diagnostic; do not silently claim the original typography was reproduced. Extra fonts and different font weights can change wrapping and control sizes.

## Missing inputs and font locks

If selected font files or search directories are absent, inspect `get_document.configuration.fonts_complete`, `font_diagnostics` and the document's asset diagnostics. Structural edits remain available, but inventory and rendering reject missing selected inputs. Preserve existing family names while the declared inputs are restored or their configuration is explicitly revised, then obtain fresh document/dependency revisions. Document edits do not install fonts or rewrite user settings automatically.

An optional project `font_lock` fixes font data, selection settings, the Skia dependency identity and the bundled pack. `FONT_LOCK_MISMATCH` means the current inputs or a newly selected face differ from that baseline; do not hide it with a substitute or silently regenerate the lock. Inspection and structural edits remain available. The explicit `fonts lock FILE` CLI captures all supplied faces and system faces observed by that document, including generated text. Replacing lock contents requires `--expected-lock-revision` for the baseline being replaced; `fonts check FILE` verifies the configured baseline. A missing lock remains a missing dependency until explicitly created. Browser CSS fonts and terminal text fonts have their own owners and are outside this lock.

## Glyph fallback and inspection

Without a configured chain, glyph fallback uses the requested family, Noto Sans JP, then other explicitly supplied families in file order. Missing requested families remain errors. `get_layout` text metrics identify the requested family, locale, and faces actually used by shaped glyph runs, including source/face hashes and synthetic styling. Check these records when mixed scripts or combining marks require fallback. Inspect the actual rendered glyphs when using emoji sequences or custom variation axes.
