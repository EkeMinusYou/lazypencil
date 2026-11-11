# Reference images

Attach screenshots or other images when asking an agent to create or refine a
design. The agent receives the images with your request. Ask it to import an image
as a layer when the image should also become part of the design.

## Attach from the command line

```sh
lazypencil prompt design.pen --image reference.png "Use this reference"
lazypencil prompt design.pen --image before.png --image after.jpg "Use the second image"
lazypencil prompt design.pen --image reference.jpg --image-max-size 1024x768 "Use this reference"
```

`--image` accepts static PNG, JPEG and WebP and can be repeated. Paths resolve
from the directory where you launch lazypencil. The selected agent must support
image input.

## Paste or attach in the TUI

| Action | Result |
| --- | --- |
| Ctrl+V / Alt+V | Read the current host's clipboard |
| `/attach "path with spaces.png"` | Attach a local file |
| `/paste-image 1024x768` | Paste with an explicit send size |
| `/resize-image NUMBER 1024x768` | Resize a pending image |
| `/resize-image NUMBER original` | Restore its original dimensions |
| Attachments arrows | Select and preview pending images |
| Delete / Backspace / `d` in Attachments | Remove the selected pending image |
| `/clear-images` | Clear pending images |

`/attach` and `/paste-image` also accept `original` to override a startup resize
choice. The Attachments panel shows source and send dimensions and their order.
You can submit images with text or on their own.

Images attached while an agent works belong to the next draft. Submitting a draft
captures its text, references and images together; later changes affect the next
request. Image follow-ups run as a later turn, including with an adapter that
supports text steering. Reopening preserves the draft and waits for submission.

Cmd+V is handled by your terminal and may deliver text rather than image data.
Use Ctrl+V or Alt+V for clipboard image input. Over SSH, clipboard commands read
the remote host's clipboard; they do not automatically read your local Mac's
clipboard. For ordinary text paste, use the terminal's usual paste action.

## Choose a send size

`--image-max-size WIDTHxHEIGHT` fits inside that size while preserving aspect
ratio and avoiding upscaling. Orientation is applied and send data is normalized
to PNG. The original file data is kept separately so subsequent resizes start
from the original. Original EXIF metadata is not forwarded to the model.

In the TUI, a startup size also applies to later attachments. Use `original` for
one attachment to keep its dimensions. An older saved attachment without original
bytes must be reattached before it can be resized; the error is
`ATTACHMENT_ORIGINAL_UNAVAILABLE`.

## Image limits

Defaults allow eight images, 20 MiB per source/send image, 50 MiB total source
data, 50 MiB total send data, and 32 × 1024 × 1024 pixels per source image.
Provider limits also apply. Animated files are not accepted.

Choose a smaller send size for an oversized request. If the original exceeds the
source size/pixel limit, resize it before attaching. User-only
[attachment settings](configuration.md#image-attachment-budgets) can adjust the
count and aggregate budgets within their supported ranges.

Changing limits preserves saved images and drafts. Submission can remain blocked
until the pending images fit the new policy; remove or resize them to continue.

## Import a reference into the document

In a lazypencil-owned conversation, MCP exposes attachment metadata through
`get_document`. The agent can import one with `put_asset`:

```json
{"document_id":"DOCUMENT_ID","kind":"image","source":{"attachment_id":"ATTACHMENT_ID"}}
```

Use the returned asset URI in an edit operation. External MCP clients upload
MIME-typed image data instead; see [managed assets](mcp.md#managed-assets).
Mentioning a file path in prompt text alone does not attach its pixels.

Original and send images are stored with the conversation. Imported project
assets are stored with the project. Removing a pending image leaves other saved
references and imported assets intact. See [conversation deletion](agents.md#delete-local-conversation-data)
for removing local logs and images.
