# Checkpoints and retention

Every committed edit keeps a checkpoint in the selected user data directory.
Restoring one creates another edit; it does not rewrite the original checkpoint.
The `.pen` document remains authoritative. Referenced assets use their current
contents when restoring. See [document and dependency preconditions](mcp.md).

## Browse and restore

```sh
lazypencil history design.pen --limit 50 --json
lazypencil history design.pen --before-sequence SEQUENCE --limit 50 --json
lazypencil restore design.pen --checkpoint CHECKPOINT_ID \
  --expected-revision CURRENT_DOCUMENT_REVISION
```

Listings are newest first. Follow `next_before_sequence` until it is null.
`limit` is a maximum: pages also have a shared response byte budget, so a page
with large edit results can contain fewer entries while still returning a cursor.
CLI, MCP and the TUI checkpoint browser use the same bounded pages. MCP reads
through `get_history` and restores through `restore_checkpoint`; the TUI uses
**Checkpoints · Browse and restore checkpoints…** in the command palette (`Alt+P`).

## Explicit retention

History is kept until the user explicitly reviews and commits a retention plan:

```sh
# Review removal of older checkpoints, keeping the newest 100 committed edits.
lazypencil history design.pen --prune --keep 100 --json
# Copy revision from that review to commit exactly that plan.
lazypencil history design.pen --prune --keep 100 \
  --expected-revision REVIEWED_PLAN_REVISION --json
```

At least one committed checkpoint must be kept when committed history exists.
Older committed results and aborted preparation records are candidates. Review
reports counts, logical file bytes and at most 200 candidate IDs; it never removes
checkpoints. Changes to the document, history or candidate contents invalidate
the reviewed revision, including same-length metadata edits with a preserved mtime.
No opening, prompt, save, or conversation-retention setting silently prunes edits.
Unpublished preparations without metadata and unrecognized directory entries are
retained and reported separately; the command does not guess ownership of them.
Retention is a local CLI/library operation and is not exposed as an MCP tool.

Commit removes the selected checkpoints and their saved edit results. They can no
longer be restored. The current document, conversations, drafts, images and project
assets are preserved. Assets referenced only by pruned history can become eligible
for the separate reviewed [asset collection](mcp.md#managed-assets).
Newly saved checkpoints share identical consecutive snapshot storage where the
filesystem supports it, with a private-copy fallback. `candidate_bytes` counts
logical file bytes, not guaranteed filesystem space reclaimed from shared files.

Request-ID reservations survive retention. Retrying an expired committed request
returns `REQUEST_EXPIRED` (CLI exit 3) and never executes that request again.
A different payload with the same ID still returns `REQUEST_ID_CONFLICT`. Results
for retained committed requests remain replayable, and aborted requests remain
retryable with their original payload. Upgrade editing executables before pruning:
older versions do not understand expired-request reservations.

## Interrupted cleanup

A durable journal commits retention before file cleanup. After that point,
`applied: true` remains true even when cancellation or cleanup fails;
`cleanup_complete: false` and `errors` report the outstanding cleanup. A late CLI
interrupt can put that completed result under `error.details.completed_result`.
Use the normal recovery command to finish:

```sh
lazypencil recover design.pen --json
```

Normal history access and saving also resume previously committed cleanup under
the document lock. Progress is persisted so repeated recovery advances through a
large plan instead of restarting its cleanup. A cleanup pass has a 30-second
cooperative deadline; repeat recovery if it needs another pass. Unknown files or
symbolic links stop cleanup and preserve the journal for inspection. Do not delete
the journal to bypass an unresolved error. Partially removed payloads remain
recoverable even when their checkpoint metadata has already been removed.

Review allows at most 100,000 removals and 64 MiB of reservation records per plan,
with a 30-second cooperative deadline. Keep more checkpoints to make smaller
batches. Snapshot hashing and record scanning check cancellation; individual
filesystem operations and JSON parsing complete before their next checkpoint.

## Storage and compatibility

Private `history-index` records provide direct request lookup, sequence cursors,
and recovery pointers. Existing checkpoint directories keep their original layout
and are indexed once on first use, one metadata payload at a time. New checkpoint
directories from an older cooperating writer are discovered when the history
directory changes. Ordinary reads and saves validate the current tail, pending
recovery, and the records they actually use; they do not audit every old result.

Keep the complete user data directory in backups, including the index and cleanup
state. After pruning, expired-request reservations cannot be reconstructed from
the remaining checkpoint snapshots. Stored history is not tamper-proof against its
filesystem owner.
