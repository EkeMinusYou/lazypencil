# Image attachments

Images are reference inputs to a conversation, not automatically inserted design
layers. CLI, TUI, and the MCP attachment grant use the same immutable attachment data.

## CLI and send size

```sh
lazypencil prompt design.pen --image reference.png "Use this reference"
lazypencil prompt design.pen --image before.png --image after.jpg "Use the second image"
lazypencil prompt design.pen --image reference.jpg --image-max-size 1024x768 "Use this reference"
```

`--image` is repeatable. Paths resolve against the invoking directory. Static PNG,
JPEG, and WebP are accepted; animation, misleading MIME/container data, and oversized
inputs fail explicitly. Orientation is applied and send data is normalized to PNG.
Original encoded bytes remain saved separately, without forwarding original EXIF data
to the model. No image is silently omitted or resized.

Explicit `--image-max-size WIDTHxHEIGHT` fits without upscaling, preserving aspect and
resampling alpha correctly. TUI startup applies it to later inputs too. Every resize
starts from the saved original rather than repeatedly shrinking an already resized PNG.
Older attachments lacking original bytes remain readable, but resizing them needs the
original file to be reattached (`ATTACHMENT_ORIGINAL_UNAVAILABLE`).

## TUI input

| Action | Behavior |
| --- | --- |
| Ctrl+V / Alt+V | Explicitly read this host's clipboard |
| `/paste-image [WIDTHxHEIGHT\|original]` | Paste with an optional send size |
| `/attach <quoted path> [WIDTHxHEIGHT\|original]` | Attach a file |
| `/resize-image NUMBER WIDTHxHEIGHT` | Replace one pending send image from its saved original |
| `/resize-image NUMBER original` | Restore its original dimensions |
| Attachments arrows | Select and preview pending images |
| Delete / Backspace / `d` in Attachments | Remove the selected pending attachment |
| `/clear-images` | Remove pending images |

`original` overrides a startup resize choice for one input. The UI distinguishes source
and send dimensions and retains order. Image-only prompts are allowed. Conversion and
immutable storage run outside the input loop; Ctrl+C cancels without losing adopted
images or the draft. Submission/image edits wait while an image operation is active.

Images can be pasted, attached, resized and removed while an agent is working or the
send queue is paused. They belong to the next draft. Submitting captures its text,
references and images together; later draft edits cannot change a queued request.
Image-bearing follow-ups run as subsequent turns, even when the adapter supports
text steering. Reopening retains draft and queued images without submitting them.

Clipboard priority is bitmap, recognized image-file list, then text. Ordinary bracketed
text paste stays text. Cmd+V belongs to the terminal and does not guarantee bitmap data.
SSH uses the remote host's clipboard, not automatic access to the local PC. Terminal
image-protocol support is independent of OS clipboard availability. The supported
platform/terminal baseline is listed in the [README](../README.md#supported-environment).

## Budgets and failure

Defaults are eight images, 20 MiB per source/send image, separate 50 MiB source/send
totals, and 32 Mi pixels per source (32 × 1024 × 1024). User-only
[attachment settings](configuration.md#image-attachment-budgets) can change the count
through 64 and aggregate totals through 256 MiB, while native per-image safety limits
remain. Provider limits also apply.

Policy is checked at input and again before sending. Tightening it never deletes
originals or drafts; remove or explicitly resize pending images to repair a request.
Text paste remains available when image capacity is exhausted. Failed/unsupported
image submission never falls back to a text-only request silently.

## ACP, MCP, and storage

The host requires negotiated ACP image support and sends ordered MIME/Base64 image
blocks with attachment IDs/display names. A path mentioned in text alone is not an
image block. Transcripts replace payloads with size/hash metadata.

The owned MCP connection receives a private document/conversation-bound manifest.
`get_document` exposes its registered images, and `put_asset` can explicitly import
one with `{"kind":"image","source":{"attachment_id":"ID"}}`. The returned URI
can then be referenced by a normal atomic edit. External MCP clients instead upload
approved image bytes; arbitrary user-data discovery is not exposed.

Original/send images live in the user conversation store. Draft recovery preserves
them; closing/revoking a connection expires its grant while saved images remain.
Removing a pending attachment never deletes another retained reference or an imported
project asset. [Conversation deletion and retention](agents.md#delete-local-conversation-data)
are explicit reviewed operations, separate from project asset maintenance.
