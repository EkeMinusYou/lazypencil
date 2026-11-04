# Performance measurement

Use release executables and the maintained Rust observers for repeatable measurements.
Keep raw observations and reports in a temporary directory or CI artifacts, not in this
repository. The [timing reference](performance-logging.md) separately describes the
metadata emitted during ordinary conversations and MCP calls.

## Native rendering

Build the application once for the final measured state, using the shared target cache:

```sh
export CARGO_TARGET_DIR="$HOME/Library/Caches/lazypencil/target"
cargo build --locked --release --bin lazypencil
cargo xtask measure --binary "$CARGO_TARGET_DIR/release/lazypencil" \
  --output /tmp/lazypencil-native-measurement.json --samples 20
```

The task measures the original shadow and 1,000-node dashboard fixtures, separating
first and repeated child-process runs. It verifies dimensions, node counts, diagnostics,
asset counts, PNG hashes, and absence of unexpected dynamic/script work. OS caches
and host load remain part of the environment; a first process is not proof of an
OS-reboot-cold filesystem.

## Local preview measurements

Build the observer in release mode before measuring so its parsing/decompression does
not add debug-build backpressure. On Unix:

```sh
export CARGO_TARGET_DIR="$HOME/Library/Caches/lazypencil/target"
cargo build --locked --release --jobs 2 --manifest-path xtask/Cargo.toml --bin xtask
cargo xtask fixtures performance dashboard --output-dir /tmp/lazypencil-dashboard
"$CARGO_TARGET_DIR/release/xtask" measure-preview \
  --binary "$CARGO_TARGET_DIR/release/lazypencil" \
  --document /tmp/lazypencil-dashboard/dashboard.pen \
  --output /tmp/lazypencil-preview-measurement --label dashboard \
  --geometry 54,160,1920,1080 --samples 20 --all-directions --bursts \
  --kitty-transfer zlib
```

Choose a new output directory. The observer owns its PTY/session, selects an inert
adapter, isolates configuration, and preserves document/executable hashes. Geometry
is `rows,columns,pixel_width,pixel_height`; use identical geometry for comparisons.
The default `--focus-tabs 0` retains the initial Preview focus for an existing file.
Use an explicit value when measuring a different initial focus or panel cycle.
Use `--first-action zoom-out`, `pan-right`, `pan-left`, `pan-down`, or `pan-up`
to isolate cold interactions across fresh processes; vertical actions also require
`--all-directions`. Every round still checks inverse camera pairs against the initial pixels.

`--kitty-transfer zlib` simulates direct Kitty; `png` exercises the unprobed compatible
transport. Both are lossless. `--reference-report PATH` additionally checks matching
document, geometry, protocol, samples, and decoded image hashes. `--median-limit-ms N`
requires each operation's complete-image median to meet that value. Profile/capture
runs are separate and cannot use the median acceptance option.
`--sync-median-limit-ms N` instead bounds receipt of the synchronized-update end
for every operation, including observer parsing/decompression backpressure. This is
the input-response reference; it does not acknowledge terminal drawing or scanout.
Add `--sweep-steps N` to measure N consecutive steps per direction before reversing.
This includes new zoom levels and camera positions; the synchronized median limit
also applies to each sweep operation. Choose a count that does not reach zoom limits.
A reference run must contain the same sweep workload and matching restored pixels.

Retain cold zooms, outliers, all requested samples, burst round trips, and normal exit.
Report complete-image receipt, temporary camera feedback, synchronized-update completion,
and physical display timing separately. A PTY observer cannot establish physical
screen presentation or every terminal/IME combination. Changing observer overhead,
host load, document content, or output geometry invalidates an isolated renderer-speed
comparison.

## Hosted latency and resource checks

Routine push/PR verification does not automatically run numeric TUI performance
acceptance. Request it explicitly on the supported Mac target:

```sh
gh workflow run ci.yml --ref main -f measure_tui=true
```

The release lane uses the verified packaged candidate. The observer runs twenty
startups and four interactions per startup, keeps the original idle intervals, and
measures bound MCP/script processes separately from identified adapter processes.
`measure-tui --candidate ARCHIVE --output DIRECTORY --chromium-resources --enforce-targets`
provides the same assertions when invoked explicitly on the supported Mac host.

| Observation | Enforced bound |
| --- | --- |
| Input ready, each startup | At most 500 ms |
| First preview, each startup | At most 2,000 ms |
| First available feedback/complete frame, 80 interactions | Nearest-rank p95 at most 100 ms |
| Ordinary bound application RSS | At most 200 MiB |
| Chromium-inclusive application RSS | At most 1,536 MiB |
| Ordinary idle application CPU | Below 1%, over at least 30 seconds |
| Chromium-inclusive idle application CPU | Below 2%, over at least 30 seconds |

Sampling bounds are observations, not hard allocation quotas or worst-case latency
guarantees. Chromium memory has its own budget. Keep process identity, source, compiler,
OS/runner image, workload, and observer version with results. A failed timing run must
not be reported as accepted or repaired by excluding observations.

Functional/pixel/runtime/terminal checks and release packaging remain required under
the selected CI contract even when measurement is disabled. Use existing run/job/step
timestamps to separate queue, compilation, execution, and cache effects; do not infer
end-to-end speedups from a changed schedule alone.
