# Performance observations

These are preliminary observations, not a completed release benchmark or acceptance of the design's performance targets. The target workload remains 1,000 nodes at 1920×1080, with rendering, process-group memory, interaction latency, and idle CPU measured separately.

## Native backdrop example, 2026-10-23

Environment: macOS arm64, debug build. Input: original `examples/backdrop-panel.pen`, 35 nodes, 1000×680 output, bundled Noto Sans JP, background blur and a shadow, no external assets or Chromium. Each run starts a fresh CLI process; the OS file cache already exists. This is not a controlled cold-start measurement.

Elapsed time uses Python's monotonic clock around a child process. Peak RSS comes from macOS `wait4` resource usage, in bytes. The values cover the CLI child, not the measuring process. All three exports exited successfully.

| Run | Elapsed | Peak RSS |
| --- | --- | --- |
| 1 | 1966.7 ms | 105,709,568 bytes |
| 2 | 1908.4 ms | 105,644,032 bytes |
| 3 | 2039.6 ms | 105,791,488 bytes |

The sample takes roughly two seconds and about 101 MiB in this development build. It does not establish 1,000-node performance, release-build performance, a percentile, idle behavior, or any other OS's results. Chromium and agent runtimes need separate process-group measurements.

## Synthetic native shadow scene, 2026-10-23

The fixed generator [make_native_performance_fixture.py](../scripts/make_native_performance_fixture.py) creates exactly 1,000 stored/expanded nodes: one clipped 1920×1080 frame, 999 rounded rectangles, and one `multiply` outer-shadow effect among those rectangles. It has no text, images, scripts, external assets, or browser/shader runtime. This isolates the path where one backdrop-dependent child previously made ordinary siblings capture the whole parent canvas. Ordinary siblings now avoid that capture, and nested passes retain decoded image caches.

```sh
python3 scripts/make_native_performance_fixture.py --output /tmp/native-1000.pen
lazypencil render /tmp/native-1000.pen --output /tmp/native-1000.png
```

The generator refuses to overwrite an existing output. Fixture bytes have SHA-256 `d0a54ed3c8b3716b3ac5f26fe79e4e130164e8afd128bb4146ef96774ccbaa8b`. Measurements used the macOS arm64 debug CLI, fresh child processes, Python's monotonic clock and `wait4` peak RSS in bytes. The OS file cache was already populated; cold start and host load were not controlled. No compilation or test suite ran during these three observations.

| Run | Elapsed | Peak RSS |
| --- | --- | --- |
| 1 | 1556.97 ms | 152,158,208 bytes |
| 2 | 1496.40 ms | 152,322,048 bytes |
| 3 | 1565.22 ms | 152,092,672 bytes |

All runs exited successfully, returned no layout diagnostics or dynamic artifacts, and produced PNG SHA-256 `71f026838d25811159637859efbe051106860ace17c304b54c051b74b69815a7`. This demonstrates the fixed simple workload in a development build, at about 1.5 seconds and 145 MiB. It is not a before/after timing comparison, a release result, a percentile, or acceptance of the full performance target. The following section records initial release observations for this scene and a text/component/image workload. Interaction/idle behavior and complete process groups still require measurement.

## Linux arm64 release observations, 2026-10-23

Environment: Debian 12 arm64 in Docker Desktop, Linux `7.0.14-linuxkit`, glibc 2.36, Rust 1.94.0, with eight virtual CPUs and 4 GiB assigned to Docker. The application uses the unchanged `release` profile: thin LTO, one codegen unit, stripped debug information, and native CPU Skia. The tested executable has SHA-256 `f7f3c98946d4655741c5cabab5b12f5ede430cef95104a86eb8b87b3273b1686`.

The new [dashboard generator](../scripts/make_dashboard_performance_fixture.py) creates a 1920×1080 document with Japanese/English text, six color variables, a reusable 80-node card, eleven instances with text overrides, rounded clipping, normal shadows, and one local PNG used repeatedly. There are 131 stored nodes and exactly 1,000 expanded nodes. The document SHA-256 is `c55bcf8727afc04d31646ea1a259e17723fe9be03e3cb4fd2dccbf8512764da7`; the asset SHA-256 is `adea05ca47573c3398f5e7d10aebcba7cd6da37abdeae104131e6085cd1e0937`. The generator creates a new directory and refuses to overwrite an existing fixture.

```sh
python3 scripts/make_dashboard_performance_fixture.py --output-dir /tmp/dashboard-1000
cargo build --locked --release
target/release/lazypencil --json render /tmp/dashboard-1000/dashboard.pen --output /tmp/dashboard-1000.png
```

Each observation starts a fresh CLI process, renders the entire document, and writes the PNG. A Python parent measures elapsed time with `monotonic_ns` around process creation and `wait4`, and converts Linux `ru_maxrss` from KiB to bytes. Compilation and tests did not run concurrently. The file cache was already populated; cold start and host load were not controlled. These measurements include document loading, layout, fonts, rendering, encoding, and output. They cover the native CLI child, not Docker, the measuring process, TUI, MCP, or an external AI runtime.

| Scene | Run | Elapsed | Peak RSS |
| --- | --- | --- | --- |
| Simple shadow scene | 1 | 2239.494 ms | 106,725,376 bytes |
| Simple shadow scene | 2 | 2104.281 ms | 107,175,936 bytes |
| Simple shadow scene | 3 | 2085.983 ms | 107,225,088 bytes |
| Dashboard | 1 | 1038.252 ms | 85,282,816 bytes |
| Dashboard | 2 | 1043.503 ms | 84,987,904 bytes |
| Dashboard | 3 | 1018.806 ms | 82,857,984 bytes |

All six exports succeeded with 1,000 expanded nodes, no layout diagnostics, no executed scripts, and no dynamic acquisitions. Each scene's three PNG hashes matched. The simple scene retained the earlier PNG hash; the dashboard PNG has SHA-256 `eac15be8a871dca4cafa59de7a157083114bc02e3847a5f1e1bdf8ac18060046` and was visually inspected. The initial dashboard fixture mistakenly used image mode `fill`; the CLI rejected it. Measurements above use the corrected `cover` fixture, without changing the renderer.

The dashboard observations are about one second and 79–81 MiB. The simple shadow scene initially exceeded the provisional two-second initial-preview target on this environment; the correction below addresses the observed image-copy cost. Three runs do not establish a percentile or a worst case, and CLI peak RSS does not establish the 200 MiB process-group target. A container result also does not establish native Linux desktop, macOS release, or Windows performance. The CI release smoke check exercises the same dashboard's geometry, diagnostics, asset count, and PNG integrity without imposing a hardware-dependent timing threshold.

### Releasing parent snapshots before compositing

Separate CLI observations localized the simple scene's delay to export: `inspect` took about 44 ms, structural `validate` 70 ms, `layout` 96 ms, and `render` 2172 ms. These are distinct commands, not instrumented phase timings or values to subtract as an exact profile.

The backdrop compositor kept its reader's snapshot of parent coverage alive while writing the next child into that same coverage surface. This forced Skia to copy the parent pixels on write for ordinary siblings. It now releases the reader before compositing children and strokes. Color/coverage semantics, effects, output dimensions, and resource limits are unchanged.

The following observations used the same fixtures, container, toolchain, release profile, fresh-CLI harness, and populated file cache. Compilation and test execution did not overlap these runs. The executable SHA-256 was `4cc2e897806df257882aeb3beedbc220aa3d6cdd382942527cfcac634a4b7270`. This records the rendering correction before the subsequent terminal-backend change, rather than claiming a completed final-release benchmark.

| Scene | Run | Elapsed | Peak RSS |
| --- | --- | --- | --- |
| Simple shadow scene | 1 | 455.190 ms | 98,037,760 bytes |
| Simple shadow scene | 2 | 308.267 ms | 97,976,320 bytes |
| Simple shadow scene | 3 | 268.729 ms | 97,972,224 bytes |
| Dashboard | 1 | 1910.537 ms | 82,173,952 bytes |
| Dashboard | 2 | 1290.948 ms | 86,310,912 bytes |
| Dashboard | 3 | 1189.074 ms | 82,370,560 bytes |

All six exports succeeded with 1,000 expanded nodes, no diagnostics, and the expected local-asset counts. Both scenes retained exactly their earlier PNG bytes/hashes. The simple scene's observed median fell from 2104.281 ms to 308.267 ms; its peak RSS was about 93.4–93.5 MiB. The dashboard path has no backdrop-dependent shadow and is not evidence of the same speedup; its three observed times varied from about 1.19 to 1.91 seconds. Host load and CPU frequency were not controlled, and these samples establish neither a percentile nor the complete preview/process-group targets.

## Optional icon-font observation, 2026-10-24

The original [icon families study](../examples/icon-families.pen) uses 52 nodes at 1120×704, bundled Noto Sans JP, Lucide, installed Feather/Phosphor, and all three installed original Material variable fonts at weights 100, 400 and 700. It executes no scripts or Chromium. One fresh macOS arm64 release CLI process with a populated filesystem cache took 0.874 seconds and reached 239,026,176 bytes (227.95 MiB) peak RSS. Python's monotonic clock measured the child invocation; Darwin `getrusage(RUSAGE_CHILDREN)` supplied the child peak RSS in bytes. The measuring process launched only that application child.

The executable SHA-256 was `e7c3c6562143a14e0bd463fd3026d085f99aa7f8c21b778620662ff5095167d0`. All 52 nodes rendered without diagnostics or dynamic acquisitions. PNG SHA-256 `6843803c9e92cada4d785396f61cde2ebcb33f2a5e359990f0954179e9e7f694` matched the separately verified debug and release images byte-for-byte. The complete three TTFs total 34,898,544 bytes and are optional installed assets, not embedded executable data.

This single observation is not the 1,000-node acceptance workload, a percentile, or a controlled cold start; the Linux verification container was also active. The observed RSS exceeds the provisional 200 MiB process-group target. Shared immutable pack bytes and the bounded native-instance cache are implemented, but original-font capture, native font allocations and complete process-group memory still need profiling and optimization before accepting that target. Font-rich documents and the original baseline both remain required performance work.

The same final example was also rendered by a fresh Linux arm64 release CLI in the Debian 12 container, after its verification build had exited. An offline CLI setup installed all five optional packs from the original archives/TTFs. One render took about 0.575 seconds with 142,728 KiB (139.38 MiB) peak RSS from that child's `wait4` record; a 20 ms polling interval is included in elapsed time. The executable SHA-256 was `d8e8102279474f2198743374d105931a1e8ce410a7c8a1302dbdd86878b6316f`. It produced 52 nodes without diagnostics or dynamic acquisitions, and the image was inspected. Linux PNG SHA-256 is `b8b40182ab0e6b32c8efe5188c7688635ae49ae89b483fe7253c013217b5a166`.

These two single observations do not isolate the cause of the memory difference or establish cross-platform performance acceptance. Both use the same example/source font bytes, but different platform text/font backends and execution environments. Platform-specific native-font allocation profiling, controlled repeated runs and the original 1,000-node/process-group workload remain necessary.

## Streaming captured-asset validation, 2026-10-24

A diagnostic run localized additional memory growth to dependency validation: the application read complete installed fonts again while keeping their captured originals and native font data alive. Repeated checks could increase the high-water mark even after each temporary vector was dropped. Validation now computes SHA-256 through a fixed 64 KiB buffer, checks cancellation between reads, and retains regular-file and size checks. Loading an already live icon pack similarly validates its current file before sharing the captured bytes. Native Skia data retains its ownership; no borrowed font slice was introduced.

The macOS arm64 release CLI rendered the same 52-node icon families study three times before and after the change. Each invocation started a fresh child; `wait4` provided peak RSS in bytes and Python's monotonic clock measured elapsed time. The filesystem cache was populated and host load was not controlled. No build ran during these render measurements. Original font bytes and example SHA-256 `310b4a3661b246afbcb2a9da9ec2511e14685716456c6ced1d1fecdae0bd1ce0` were unchanged.

| Build | Run | Elapsed | Peak RSS |
| --- | --- | --- | --- |
| Before | 1 | 1061.01 ms | 260,538,368 bytes |
| Before | 2 | 384.00 ms | 262,537,216 bytes |
| Before | 3 | 357.86 ms | 262,553,600 bytes |
| After | 1 | 565.85 ms | 151,699,456 bytes |
| After | 2 | 317.41 ms | 151,601,152 bytes |
| After | 3 | 299.47 ms | 151,552,000 bytes |

Median child peak RSS fell from 250.38 MiB to 144.58 MiB, approximately 42%. All six PNGs matched byte-for-byte, with SHA-256 `6843803c9e92cada4d785396f61cde2ebcb33f2a5e359990f0954179e9e7f694`, 52 nodes and no diagnostics or dynamic acquisitions. The before executable was `e7c3c6562143a14e0bd463fd3026d085f99aa7f8c21b778620662ff5095167d0`; the after executable was `8f32282214f3f3405a16b2ac7ada505edcfb2f63a3e30b016329022aa68001cc`.

This confirms the improvement for this font-rich static example, not acceptance of the 1,000-node workload, cold-start/percentile targets or complete process groups. The Linux verification job passed both normal test configurations but was stopped at the user's request to move builds to GitHub Actions before its remaining checks and release build finished. No new Linux release-memory result is claimed. Future full builds and cross-platform performance work should run on hosted CI rather than recreate the local build trees.

## Repeatable native-child measurements

[measure_native_performance.py](../scripts/measure_native_performance.py) measures the unchanged shadow and dashboard fixtures above without invoking Cargo. It starts a fresh CLI child for every export, isolates user settings and the data directory, and supplies an explicit local project configuration. Each scene has a separate first invocation followed by 20 repeated invocations by default. Every export must have exactly 1,000 expanded nodes at 1920×1080, no diagnostics, no scripts/dynamic acquisitions, the expected asset count, and a PNG whose dimensions and SHA-256 agree with its render report. Repeated PNG hashes must match within each scene. The executable is hashed before and after measurement.

```sh
python3 scripts/measure_native_performance.py \
  --binary /path/to/lazypencil \
  --output native-benchmark.json --samples 20
```

The output includes individual samples, the first invocation, and repeated-sample minimum/median/nearest-rank p95/maximum for elapsed time, peak RSS and user/system CPU time. A 20-sample p95 is the nineteenth ordered sample, an observed tail estimate rather than a population guarantee. The script does not flush OS caches, so the first invocation is **not** a controlled cold start. Elapsed time covers process creation through exit observation; the completion polling interval is 1 ms, and resource-query overhead after observing exit is excluded.

On macOS/Linux, per-child [`wait4`](https://docs.python.org/3/library/os.html#os.wait4) supplies resource usage; Darwin peak RSS is recorded in bytes and Linux KiB is converted to bytes. Windows uses [`GetProcessMemoryInfo`](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo) / `PeakWorkingSetSize` in bytes and [`GetProcessTimes`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes) CPU durations. The method is retained in each sample. These OS memory counters have their respective platform semantics; this is not a simultaneous process-group measurement. The real-child counter tests passed with the full 80-test Python suite on all five hosted environments in [run 40000148383](https://github.com/EkeMinusYou/lazypencil/actions/runs/40000148383). Native release measurements from its successful macOS arm64 and Linux arm64 jobs are recorded below; other platforms remain pending.

Child deadlines, bounded stdout/stderr and PNG size, exit status, workload validation, and immutable executable/pixels are required for a successful report. Reports are published atomically without overwriting another report. Temporary fixtures, logs and PNGs are removed; only the small JSON report remains. Seven Python tests exercise real child counters independently of the previous child's high-water mark, timeout reaping, child failure/output limits, fixture hashes and settings isolation, percentile calculation, changed binaries/pixels, malformed render evidence, and publication failure cleanup.

The hosted release smoke script runs this helper against the already built executable and retains `release-benchmark.json` with the platform artifacts. Dependency-cache pruning removes that report too. Both arm64 jobs completed this integration; Windows and x64 release measurements remain pending. No hardware-dependent timing threshold is asserted by this smoke check.

### Retained macOS executable, 2026-10-24

The helper completed 42 exports using the existing macOS arm64 release executable `8f32282214f3f3405a16b2ac7ada505edcfb2f63a3e30b016329022aa68001cc`, with no local build. This executable predates the current hosted TUI retry changes. Host load and filesystem caches were not controlled. Values below summarize the 20 repeated fresh children per scene, keeping the first invocation separate.

| Scene | First invocation | Repeated median | Repeated p95 | Repeated maximum | Maximum repeated peak RSS |
| --- | --- | --- | --- | --- | --- |
| Simple shadow scene | 173.22 ms | 160.84 ms | 165.26 ms | 168.81 ms | 133,136,384 bytes |
| Dashboard | 823.30 ms | 824.94 ms | 907.03 ms | 923.45 ms | 123,322,368 bytes |

All 21 PNGs per scene matched: shadow `71f026838d25811159637859efbe051106860ace17c304b54c051b74b69815a7`, dashboard `0f65ce77b7de905bb3d03255db3ac60d70debb040a264d849f3c2c56072d69f8`. This validates the measurement path on a retained native executable, not the latest source build or full performance acceptance. Cold start, complete process groups, idle CPU, TUI startup and pan/zoom remain required.

### Hosted arm64 release builds, 2026-10-24

[Run 40000148383](https://github.com/EkeMinusYou/lazypencil/actions/runs/40000148383) built commit `955098a42d3417afdd849789a047239beed37e56` and completed 42 exports on each of macOS arm64 and Linux arm64. The retrieved `release-benchmark.json` artifacts identify the source, executable hashes, runner images and individual samples. Both jobs also passed the normal test configurations, Clippy, Material opt-in tests and release smoke checks. The overall run has a separate Linux x64 input-cancellation failure and is not recorded as an overall pass.

| Environment / scene | First invocation | Repeated median | Repeated p95 | Repeated maximum | Maximum repeated peak RSS |
| --- | --- | --- | --- | --- | --- |
| macOS arm64 / shadow | 240.65 ms | 319.05 ms | 447.51 ms | 467.97 ms | 141,754,368 bytes |
| macOS arm64 / dashboard | 1664.77 ms | 1764.17 ms | 2058.10 ms | 2224.42 ms | 142,983,168 bytes |
| Linux arm64 / shadow | 206.28 ms | 206.04 ms | 207.07 ms | 207.22 ms | 98,062,336 bytes |
| Linux arm64 / dashboard | 1273.75 ms | 1271.65 ms | 1284.11 ms | 1294.74 ms | 87,465,984 bytes |

All 21 PNGs matched within each environment/scene. Shadow matched the retained executable's hash. Dashboard hashes differ across environments: macOS `6eb5bf6a9870598763ae4886c6fe5dcf4587ce6abf3a1f6f6a59b86780fee962`, Linux `26fafa62af3c56516016934a722d25a116fb506eeddeb671dc85c7b7ed3baaca`. The benchmark asserts stability within a run, not identical platform font rendering. These shared-host timings have uncontrolled cache and host load, and do not establish controlled cold starts, TUI performance or full performance acceptance.

### Hosted Windows release build, 2026-10-24

The Windows x64 job in [run 40000148383](https://github.com/EkeMinusYou/lazypencil/actions/runs/40000148383), at the same `955098a42d3417afdd849789a047239beed37e56` source, completed both normal test configurations, Clippy, the two Material opt-ins, release smoke and all 42 benchmark exports. Its retrieved report uses the actual child process handle for `PeakWorkingSetSize` and process CPU counters. Each scene has twenty repeated fresh children after the separately recorded first invocation.

| Scene | First invocation | Repeated median | Repeated p95 | Repeated maximum | Maximum repeated peak RSS |
| --- | --- | --- | --- | --- | --- |
| Simple shadow scene | 359 ms | 328 ms | 359 ms | 360 ms | 93,446,144 bytes |
| Dashboard | 906 ms | 907 ms | 922 ms | 938 ms | 82,513,920 bytes |

All twenty-one PNGs matched within each scene. Windows hashes are shadow `d29ee020a70d0e8eafa61ca8dd6d8c317388bd88a41164cbcba0bd43f2580034` and dashboard `e90e28ab3b879043a8cde381dced403f63d33154bd0585909ceadca85567fdf3`. Both differ from the other platforms above; the cause of those pixel differences is not established here. The same uncontrolled-host/cache and incomplete-process-group limits apply. This does not establish physical Windows terminal/clipboard behavior or complete performance acceptance.

## Remaining work

Background-aware layers preserve color alpha and replacement coverage, which can require several working rasters. Explicit scratch reservations are bounded and fail rather than dropping effects. This resource guard is separate from measured RSS.

The renderer skips nodes outside the working viewport, including unrelated browser/shader jobs. Overscan is still conservative across the document, and intermediate layers can use complete node bounds. Tighten dependency regions and intermediate-layer culling, then measure controlled cold/warm starts across the target environments on the final release build. TUI pan/zoom, image transport, runtime idle expiry, and full process-group memory remain part of the original acceptance scope.
