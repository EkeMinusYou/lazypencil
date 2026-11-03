# Development checks and completion

Choose the checks and completion condition from the requested behavior before editing.
An ordinary fix is complete after the relevant checks and a coherent commit. Creating a
PR, merging it, installing a local executable and publishing a release have their own
completion conditions. Report which checks ran and which did not.

## Build from source

Use Rust 1.94 or later and a native C/C++ toolchain. On the supported Mac host,
install Xcode Command Line Tools. Building Skia from source also uses Python, Ninja,
and its upstream GN/dependency tooling; warm verified native archives can avoid that
work. The [vendored build README](../vendor/skia-bindings/README.md) describes native
configuration. Ordinary builds use the checked-in shader component and need no Node
or Wasm toolchain.

Choose a writable, stable target cache before building. For macOS:

```sh
export CARGO_TARGET_DIR="$HOME/Library/Caches/lazypencil/target"
cargo build --locked --jobs 2
```

On Linux developer hosts, use a fixed cache such as
`${XDG_CACHE_HOME:-$HOME/.cache}/lazypencil/target`. The environment variable overrides
checkout-specific Cargo configuration. Reuse one target per toolchain; avoid per-task
directories. Limit local builds to two jobs and run one build at a time across worktrees.
`--no-default-features` builds the portable core; `--no-default-features --features rendering`
adds image export without the terminal UI. A successful portable-core check does not
certify a supported Linux application release.

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
`gh run watch --exit-status --interval 30 --compact`. GitHub CLI watches progress without redispatching the workflow. A failed/cancelled
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
Use [performance measurements](performance.md) when evaluating optimized output.

## Test fixtures and documentation

Use original, redistributable inputs and independent expected values for regression
fixtures. Renderer output is not its own expected-value oracle. Do not add proprietary
Pencil exports, extracted prompts/skills, credentials, or user documents to fixtures.
Third-party archives/fonts retain their exact upstream notices and pinned provenance.
See the [render corpus](../tests/fixtures/render-corpus/README.md) and
[font fixtures](../tests/fixtures/fonts/README.md).

Keep maintained user/developer contracts and concise release notes in the repository.
Task diaries, audit reports, verification timelines, installation histories, and one-off
measurement outputs belong in conversation, temporary directories, or CI artifacts.
Do not turn a test report into an unconditional compatibility or performance claim.
Reusable fixtures and required license/provenance records remain source inputs.

## Distribution and build provenance

Source archives require a clean checkout; `--allow-dirty` explicitly creates a development
snapshot. The independent packager verifies membership, file hashes/modes, local Cargo
patches, path boundaries, and source identity:

```sh
cargo xtask dist source --output /tmp/lazypencil-source.tar.gz
cargo xtask dist verify-source /tmp/lazypencil-source.tar.gz --extract /tmp/lazypencil-source
cargo xtask dist binary --executable /build/release/lazypencil \
  --source-archive /build/native-source.tar.gz --evidence /build/release-evidence \
  --output /tmp/lazypencil-candidate.tar.gz
cargo xtask dist verify-binary /tmp/lazypencil-candidate.tar.gz --extract /tmp/lazypencil-install
```

Binary packages retain the matching source, executable, release declaration, original
notices, corresponding-source materials, and manifest. The
[release-evidence workflow](../.github/workflows/release-evidence.yml) gathers actual
build/linkage inputs, packages the candidate, and smokes it with build trees hidden.
`release_ready` describes the declared supported build/evidence conditions; it does not
replace signature verification or imply Apple code signing/notarization.

Verify GitHub provenance against the reviewed source and signer commit:

```sh
gh attestation verify /path/to/lazypencil-candidate.tar.gz \
  --repo EkeMinusYou/lazypencil \
  --signer-workflow EkeMinusYou/lazypencil/.github/workflows/release-evidence.yml \
  --source-digest EXPECTED_COMMIT_SHA --signer-digest EXPECTED_COMMIT_SHA \
  --source-ref refs/heads/main --deny-self-hosted-runners
```

For source archives select `.github/workflows/ci.yml` as signer workflow. Match the
archive's actual source/ref and review them independently; an arbitrary successful
workflow or a matching filename is insufficient. Keep verification/install/publish
completion separate. CI artifact retention is finite.

## Licenses and third-party inputs

Application code and original project documentation use [MIT](../LICENSE). Bundled
fonts/icons, native dependencies, the toolchain, and separately installed runtimes keep
their own licenses. Preserve original notices, copyright, asset provenance, and applicable
corresponding-source materials when redistributing. Do not delete generated notice
inventories as if they were transient task reports.

The application [Rust inventory](../resources/licenses/rust/index.json), tool
[inventory](../xtask/notices/index.json), pinned asset catalogues, and reviewed native
input policies are separate. Verify changes using their maintained collectors:

```sh
cargo xtask notices rust verify
cargo xtask notices rust verify --root xtask --bundle xtask/notices
cargo fetch --locked
cargo xtask notices rust verify --cargo-home /path/to/cargo-home
cargo deny --all-features --locked check licenses sources advisories
```

Collection uses checksum-verified original archives and reviewed upstream sources, not
generic replacement terms. Source-dependent native/toolchain/link-map evidence is built
and checked by the release workflow. Asset licences and runtime installation are described
in [icons](icons.md), [fonts](fonts.md), and [runtimes](runtimes.md). Dependency changes
must update the corresponding source pins and inventories before distribution.
