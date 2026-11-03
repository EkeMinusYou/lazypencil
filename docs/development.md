# Development checks and completion

Choose the checks and completion condition from the requested behavior before editing.
An ordinary fix is complete after the relevant checks and a coherent commit. Creating a
PR, merging it, installing a local executable and publishing a release have their own
completion conditions. Report which checks ran and which did not.

## Focused development checks

The existing `cargo xtask test` remains the explicit complete developer check: both
feature configurations, Clippy and a normal development build. Use the focused form
for a small change:

```sh
cargo xtask test --lib --filter collapsed_panels
cargo xtask test --test tui --filter real_pty_collapses_panels
cargo xtask test --test user_config --lint
cargo xtask test --core --lib --filter config::
```

`--lib` and repeatable `--test NAME` select targets. `--filter TEXT` applies to every
selected target; run separate commands when their filters differ. A selected target
that executes zero tests fails, including a misspelled filter or a selection consisting
only of ignored tests. These focused commands stop at the first failure. `--lint` checks
only the selected targets. Without `--lib` or `--test`, the original full checks remain.

Both forms isolate `LAZYPENCIL_CONFIG_DIR` and prepare the native test helper. An explicit
`LAZYPENCIL_TEST_HELPER` can reuse an already prepared executable. No test changes the
user's real configuration. Keep target directories stable across iterations. Limit local
Cargo jobs to two and run only one build at a time across active worktrees. Heavy or cold
Skia builds and broad suites belong on hosted runners; focused checks can use a warm
local build when appropriate.

For wording, spacing and colors, start with the affected TUI behavior. Add the relevant
PTY case when layout, focus or input changes. Shared rendering, dependencies, build
recipes and regressions discovered outside that scope justify broader verification.
Finish the implementation before the final check pass. After another edit, repeat the
checks affected by that edit, rather than starting every unrelated suite again.

## Pull requests and integration

CI uses the PR merge result against its base to select a completion contract. A missing
or invalid base requires full verification. Main, manual CI and reusable CI calls keep
the complete graph and the existing verified-baseline behavior.

| Contract | PR changes | Required work |
| --- | --- | --- |
| docs | Repository Markdown prose and fixture README files | Plan and whitespace validation |
| tools | Independent developer-tool inputs | Tool tests, lint and dependency policy on Linux |
| focused | Known presentation/key paths | Library/TUI/agent/configuration checks on Mac |
| focused | Embedded design guidance | Its existing core consumer checks |
| full | Rendering, dependencies, build/CI/distribution inputs, unknown or mixed changes | Original complete verification |

Embedded resources are compiler inputs, even when their file extension is Markdown.
The planner emits `verification` and `verification_reason`. Its presentation route uses
the existing focused native workflow without adding a release build or full consumers.
Do not interpret focused success as a full product verification result.

The stable `Verification` check requires precisely the jobs selected by the contract.
A required failure, cancellation, skip or missing result fails the check. A focused run
cannot issue a complete-source result. The original `Verified source artifact` job and
its complete consumer, source and provenance gates remain on the full route. Where
branch protection is used, select `Verification` as the required PR check so legitimate
skipped full jobs do not block a focused PR.

## Observe one existing CI run

Reuse the same run for the same source and inputs. Watch it with:

```sh
cargo xtask ci watch --run RUN_ID --head FULL_COMMIT_SHA
```

The command verifies the run ID and source identity before invoking one
`gh run watch --exit-status --interval 30 --compact`. GitHub CLI owns polling; the model
does not repeatedly query, sleep and reinterpret unchanged status. A failed/cancelled
run or observation failure returns a failure. It never redispatches or cancels CI.
For PR runs, pass the run's `headSha` reported by GitHub CLI; the PR head,
synthetic merge commit and local HEAD can differ.

Inspect failed job logs after the result is known. A temporary observation failure is
not evidence that a run stopped: reconnect to that same run instead of starting another.
Starting full CI after focused success needs a concrete change-related reason or a
requested integration/release outcome.

## Local executable updates

Batch visual adjustments while using a development executable. When a local installation
is requested, complete edits and related checks, then build the final state once:

```sh
cargo build --locked --release --bin lazypencil
```

Replace the installed executable only with that final release build. Intermediate edits
do not each need another installation or distribution archive. Keep build and install
status separate in the result so the user knows which executable they are running.

The release profile remains optimized for the product. An unoptimized debug executable
can make previews appear slow and is not a substitute for release performance validation.
A separate optimized development profile should be adopted only after comparing warm
edit/build time and actual preview behavior; no unmeasured speed claim changes the
release recipe or existing acceptance thresholds.

## Measure the effect

Use existing process timings and CI run/job/step timestamps. Record implementation to
commit, requested local installation, and CI queue/build/test time separately. Distinguish
cold dependency compilation from a warm rebuild. Count avoidable repeated checks and
release builds, rather than enforcing an arbitrary task deadline. A comparison of model
settings should use similar tasks and should not attribute all unobserved time to model
reasoning or network delay.
