# Checkpoints and restoration

Each saved edit keeps a checkpoint. Use one to return the document to an earlier
state while retaining a history of the restoration itself.

## Browse and restore

In the TUI, choose **Checkpoints · Browse and restore checkpoints…** from the
command palette (Alt+P). Select an entry, press Enter to review it, then Enter
again to restore. Esc returns without restoring.

From the CLI:

```sh
lazypencil history design.pen --limit 50 --json
lazypencil inspect design.pen --json
lazypencil restore design.pen --checkpoint CHECKPOINT_ID \
  --expected-revision CURRENT_DOCUMENT_REVISION \
  --expected-dependency-revision CURRENT_DEPENDENCY_REVISION
```

Replace the checkpoint ID from history and the current revisions from inspection.
Listings are newest first. Follow `next_before_sequence` to older entries:

```sh
lazypencil history design.pen --before-sequence SEQUENCE --limit 50 --json
```

A page can contain fewer than `limit` entries when results are large; the cursor
tells you whether more remain. MCP provides `get_history` and `restore_checkpoint`.

Restoration creates a new edit and checks the current document and dependency
revisions. External images, code, fonts and settings keep their current contents.
If inputs changed since review, reload and review again before restoring.

## Explicit retention

Checkpoints stay until you review and apply a retention plan. For example, keep
the newest 100 committed edits:

```sh
lazypencil history design.pen --prune --keep 100 --json
lazypencil history design.pen --prune --keep 100 \
  --expected-revision REVIEWED_PLAN_REVISION --json
```

The first command shows candidates and sizes. The second uses that review's
revision to confirm removal. Keep at least one checkpoint when committed history
exists. If the document, history or candidate contents change, review again.

Pruned checkpoints and saved edit results can no longer be restored. The current
document, conversations, drafts, images and project assets remain. Asset files
referenced only by removed history can later be reviewed through
[asset maintenance](mcp.md#project-asset-maintenance).

`candidate_bytes` counts logical file bytes. Shared filesystem storage can make
the actual reclaimed disk space smaller. Unknown files and unpublished work are
retained separately for inspection.

For API callers, request-ID reservations survive pruning. An expired committed
request returns `REQUEST_EXPIRED` (CLI exit 3); a changed payload under that ID
returns `REQUEST_ID_CONFLICT`. Use current editing executables when pruning so
all writers understand these reservations.

## Interrupted cleanup

A plan is committed before its file cleanup. `applied: true` means the decision
was applied; `cleanup_complete: false` and `errors` identify remaining work.
To finish cleanup:

```sh
lazypencil recover design.pen --json
```

Normal history reads and saves also resume committed cleanup. Repeat recovery
if a large pass reaches its 30-second time budget. Unknown files or symbolic links
can stop cleanup for inspection; keep the journal until the error is resolved.
Cancellation after the plan commits leaves its completed removals applied.

A review allows up to 100,000 removals and 64 MiB of request reservations. Keep
more checkpoints to make a smaller batch if those limits are reached.

## Storage and compatibility

Checkpoint files and their index live in the selected user data directory.
Back up the complete directory to preserve history and interrupted-cleanup state.
After pruning, the remaining document snapshots alone cannot reconstruct expired
request-ID reservations.

Document checkpoints and [conversation retention](configuration.md#local-conversation-retention)
are separate. Use document history for design edits, and conversation maintenance
for local prompts, responses and reference attachments.
