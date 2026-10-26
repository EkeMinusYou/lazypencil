# Performance observations

These are preliminary observations, not a completed release benchmark or acceptance of the design's performance targets. The target workload remains 1,000 nodes at 1920×1080, with rendering, process-group memory, interaction latency, and idle CPU measured separately.

The independent Rust task `cargo xtask measure-tui --candidate ARCHIVE --output DIRECTORY` runs on isolated GHA runners. Changes to its code select one representative candidate and run the measurement before the full native matrix. The manual release-evidence workflow opts in with `measure_tui: true` for other environments, after compilation, packaging and other checks have finished. It uses the original 1,000-node dashboard, a fresh data directory for each of 20 startup/input/preview samples, reversible zoom and pan operations, and a final 30-second idle interval. It reuses the packaged executable and the existing Kitty stream observer; it does not link or build the application. The report binds the candidate source, binary and helper, retains individual observations, and rejects changed process membership or identity while comparing idle CPU clocks.

These are PTY input-echo and complete-image-output observations, including observer backpressure. They do not measure physical terminal presentation, controlled cold starts, transient peak RSS, or external agent/emulator resources. The report labels sampled process-tree RSS and includes CPU clock quantization in its idle upper bound.

The hosted measurement also explicitly opts into native preview-worker timing on its own child. The application emits at most 128 content-free records to the separately retained stderr log, only when both the hosted-runner marker and `LAZYPENCIL_PROFILE_PREVIEW=owned-gha-measurement` are present. Each record partitions snapshot preparation, rendering/PNG encoding, dependency verification, PNG decoding, terminal encoding, and metadata work within a monotonic worker interval. The Rust observer rejects missing/extra phases, invalid durations, reordered generations, and logs over 64 KiB. These are completed worker requests before UI delivery; a superseded result can be included. They are not directly correlated with individual transported images and do not measure the UI wakeup, terminal transport, or physical presentation.

The [Linux x64 representative gate at `3ca2595`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40271235125) completed all 20 samples and exact pixel restoration after zoom/pan, plus a 30.0195-second idle interval. Nearest-rank p95 was 24.3032 ms for input echo, 602.6199 ms for first preview, and 701.1244 ms across 80 interaction outputs. The maximum sampled idle RSS was 83,689,472 bytes, with one persistent application process; its CPU upper estimate including clock quantization was 0.33312%.

The same source and respective full-matrix executables also completed the unchanged measurement on [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40274589137) and [macOS Apple Silicon](https://github.com/EkeMinusYou/lazypencil/actions/runs/40274590752). Input/first-preview/interaction p95 values were 17.7469/603.9525/705.8302 ms on Linux arm64 and 100.2544/816.4183/774.2919 ms on macOS. Maximum sampled idle RSS was 80,990,208 and 112,984,064 bytes, respectively; idle CPU upper estimates were 0.29999% and 0.29979%, each with one persistent process over at least 30 seconds. All 20 startups, 80 interactions, fixture/session identities, statistics, idle counters, and 24 diagnostic PNGs per platform were independently checked, including exact zoom/pan restoration. These shared-host runs do not control CPU load or OS caches.

Interaction output exceeds the provisional 100 ms target on all three environments. The [instrumented Linux x64 observer at `521ce9f`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40275035324) measured p95 706.7318 ms with the identical application executable. Across 80 interactions, mean observer wall time before complete output receipt was 38.39 ms in the VT screen parser, 122.31 ms in the Kitty decoder, and 372.81 ms in polling. Final-chunk parsing takes additional time after the recorded receipt timestamp. These values identify observer backpressure but cannot be subtracted to derive physical or application-only latency. The tool workspace now optimizes its VT/Base64 dependencies for a repeat measurement; this is not an application speedup or completed acceptance. Complete performance acceptance remains open.

The [native worker measurement at `c355f19`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40281460362) retained five completed worker records in each of 20 samples. Across those 100 records, rendering/PNG generation averaged 352.47 ms (p95 509.02 ms), compared with 1.80 ms for PNG decoding, 1.54 ms for terminal encoding, 1.76 ms for post-render verification and 1.10 ms for metadata. Snapshot preparation averaged 18.06 ms including initial layout; the worker total averaged 376.73 ms. The end-to-end interaction-output p95 remained 703.93 ms. Raw stderr records, generation order, phase membership and durations were independently checked against the report, allowing only floating-point parse/re-serialization ULP differences. Worker records remain separate from individual wire observations and physical presentation.

Both text color/mask layers now receive allocation hints around the text box and measured ink, with one device pixel of antialias support transformed back into local coordinates. The existing drawing geometry and mask operation are retained; [Skia documents layer bounds as a size hint](https://api.skia.org/classSkCanvas.html). An unbounded Skia color/mask reference checks every output pixel for Japanese/Latin italic text, decorations, clipping, rotation, reflection and fractional scales. The first [focused run at `2965d25`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40287105731) stopped before the full matrix: its reference painted infinite color instead of preserving the original antialiased box/ink fill path. The corrected reference includes that original coverage operation; no pixel tolerance or test condition was relaxed. All nine scale/rotation/reflection conditions passed in the [Linux x64 focused gate at `2123963`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40288059187). The reference runs before longer native reproductions. Renderer changes select both native graphics/render regressions and the unchanged 1,000-node TUI measurement before expanding to all platforms.

That gate completed the same 20 startups and 80 interactions. Input/first-preview/interaction p95 were 18.0232/443.2373/490.0317 ms. Across 100 native worker records, rendering/PNG generation averaged 257.4990 ms (p95 361.4286 ms), and the worker total averaged 278.2135 ms (p95 368.7306 ms). Idle sampled RSS peaked at 83,591,168 bytes, with one persistent process and a CPU upper estimate of 0.29993% over 30.0068 seconds. All 24 diagnostic PNGs/17,712,000 pixels, exact zoom/pan restoration, sample/session identities, idle counters and raw native profiles were independently checked. These observations are lower than `c355f19` but do not isolate shared-runner variation. The 100 ms target, physical presentation and complete performance acceptance remain open; the three-platform full gate completed successfully, but the additional original-image comparison below blocks acceptance of this renderer change.

An additional Linux comparison found 7,407 changed pixels in the original mesh example's shadowed text subtree, with a maximum difference of one per RGBA channel. Its dashboard/graphics exports and both benchmark scene hashes were unchanged. This is not accepted as an unchanged compositor. The next renderer candidate retains unbounded text layers inside opacity/effect layers and offscreen passes. Its early regression adds transparent-parent conditions (18 native text comparisons in total) and all 936,000 decoded RGBA pixels of the original mesh example, bound to the respective `c355f19` platform artifacts. No Pencil outputs or additional reference PNG files are used. The subsequent `aafc705` hosted verification below covers this fallback; the `2123963` timing is not a result for the corrected renderer.

The [corrected Linux x64 gate at `aafc705`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40292925670) passed all 18 conditions and the original mesh RGBA hash. Its unchanged 20-startup/80-interaction measurement gave input/first-preview/interaction p95 of 23.1338/573.7373/633.6027 ms. Across 100 worker records, rendering/PNG generation averaged 330.3978 ms (p95 459.1955 ms). Independent checks covered all raw profiles, statistics, fixture/session identities, 24 PNGs/17,712,000 pixels, exact viewport restoration and the 30.0185-second idle observation (one process, peak sampled RSS 84,107,264 bytes, CPU upper estimate 0.33313%). Preserving the original shadowed pixels does not retain the intermediate 490 ms interaction observation. The 100 ms target remains unmet. The completed full matrix passed the original-pixel regression on all three platforms; independent comparisons also found all nine static PNGs and six benchmark scene hashes unchanged from `c355f19`.

The completed `c355f19` workflow took 41 minutes. In its macOS full job, ignored browser/shader targets took 322.234 seconds sequentially: browser readiness 142.14 s, dynamic rendering 62.77 s, samplers 36.56 s and shader uniforms 73.58 s. The verified scheduler splits the same seven targets into two lanes, keeping each target's tests serial. It executes the previously compiled debug test binaries after checking their source commit, location and SHA-256, so the lanes do not request another Cargo build or wait for its build lock. Browser readiness owns separate retained filenames from the dynamic/shader/sampler lane; documents, browser profiles and process groups are private to each fixture. Both lanes share the original 1,200-second suite deadline, continue after individual target failures and join before the separate ownership/destruction regression and required-pixel validation. Individual logs/timings and the existing combined browser log are retained. Completed duration and coverage observations follow below.

At `2123963`, the focused application checkpoint saved successfully under its run/retry key. The completed Linux full job restored 157 files produced by that same commit; the release compile/freshness command completed in 0.233 seconds (Cargo reported 0.19 s). This establishes actual same-environment reuse after the checkpoint collision fix; it is not an overall workflow duration result.

The scheduler verified at `6b8d65d` starts release compilation alongside the two browser-test lanes, using at most three concurrent lanes in that stage. In the completed `2123963` macOS log, the browser suite and actual release compilation were still sequential; release compilation took 5m03s. Both now join before MCP/lifecycle/Material checks, and smoke tests retain Cargo freshness validation afterwards. The browser suite keeps its 1,200-second budget and the compiler its 3,600-second budget; the combined Actions step permits the compiler's existing budget. No workload, samples or timeout are shortened. The full matrices at `6b8d65d` and `cb437c2` passed with this overlap; a cache-fresh compiler check does not demonstrate a cold compilation speedup.

That candidate also gives renderer-only edits a narrower representative gate: all normal library, graphics, blend, mesh, render, dynamic-render and shader-input tests, the early original-pixel reproduction, and exact real-browser composition plus shader contract pixels. Mixed changes to other application subsystems retain the broader dynamic gate. The unchanged TUI workload remains selected, and every full-platform normal/ignored/clipboard/lifecycle/Material/release check remains required. This removes unrelated browser/import repetitions from the representative gate rather than changing the final verification scope. Target fixtures and exact command/skip regressions guard this distinction. The pure scheduling changes selected a CI/candidate representative gate; an application edit exercising the narrower renderer-only gate has not yet been measured.

The `aafc705` workflow completed all three platforms, audit and source validation in 43m01s, so the overall target remains unmet. Its macOS browser step decreased from 275 s at `2123963` to 143 s; Linux x64 completed the same 43 browser/ownership cases with simultaneous readiness and dynamic-render starts. Linux's normal-test command spent 442.392 s in tests while Cargo freshness took only 0.24 s: no test rebuild occurred there. Long normal targets, including script scope (115.79 s versus 73.31 s in the preceding Linux run), also varied with runner throughput. These results distinguish actual overlap from queue/build savings and do not establish a controlled whole-workflow speedup. The subsequent candidate above addresses the remaining sequential release/browser stage and broad representative scope.

The CI-selection regression now belongs to `xtask/tests/ci.rs`, alongside the other scheduling checks. Changes confined to pure measurement/unit tests do not select a native 20-startup measurement of unchanged application/observer code; those Rust tests still run on every required platform. Changes to the measurement implementation, native observer, renderer or production TUI source continue to select the full measurement. `src/tui/tests.rs` is an explicitly test-only module. This avoids repeated empirical measurements after editing only scheduling expectations, while retaining the unchanged sample count and final native gates.

The [completed CI at `6b8d65d`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40298317636) took 26m54s. After moving the scheduling-only regression out of the native measurement trigger, [`cb437c2`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40302309021) completed in **19m20s** (09:47:40–10:07:00 UTC). The representative job waited 2 s; full-platform jobs waited 2–4 s after that gate. All three platforms reused 1,211 source/input/hash-validated files produced at `aafc705`; release Cargo freshness checks took 0.14 s on Linux x64, 0.16 s on Linux arm64 and 1.41 s on macOS. These checks are distinct from step/workflow elapsed time. Every platform passed 144 tool tests and Clippy, 99 normal/core/reproduction summaries (1,159 passing tests on each Linux platform; 1,149 on macOS), 43 browser cases and seven MCP/lifecycle/Material cases. Final source/provenance verification also passed. The hosted clipboard gate remains required. No sample count, final gate or timeout was reduced. The unchanged-application/cache-hit conditions explain the scope of this result; it does not predict cold builds or isolate shared-runner variation.

The repeated `6b8d65d` measurement used the identical Linux application executable as `aafc705`. Its 20 startups, 80 interactions, 24 PNGs/17,712,000 pixels, exact restoration and 100 raw native records were independently checked. Input/first-preview/interaction p95 were 22.8428/583.2854/644.0471 ms; render/PNG generation averaged 330.7325 ms (p95 459.9231 ms). This confirms that CI scheduling changes did not meet the application's 100 ms response target.

The [split-worker focused measurement at `f838fda`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40308860285) retained 100 raw records from the unchanged 20 startups and 80 interactions. Version 2 separates the render setup/rasterization/asset-check/crop interval (mean 295.9062 ms, p95 422.7308 ms) from final PNG encoding (mean 32.3618 ms, p95 33.4976 ms). Worker total averaged 351.9393 ms; interaction-output p95 was 629.1022 ms. Raw stderr/report membership and durations, all 24 diagnostic PNGs/17,712,000 pixels, exact viewport restoration and the idle counters were independently checked. Every diagnostic PNG and all 20 fixture avatars were also byte-identical to `aafc705`. The new observation therefore points to raster work as the larger application interval; it does not meet the 100 ms target or isolate physical presentation.

The next renderer candidate keeps the device clip origin when hinting text layers inside filtered/transparent parents and offscreen passes, and reduces only the opposite allocation edges. The previous tight-origin candidate changed original shadowed pixels, so the unchanged 18 native comparisons and 936,000-pixel mesh hash remain the early gate with zero tolerance. No new speedup or original-pixel equality is claimed for these origin-preserving hints before hosted verification; the same 20/80 workload and final three-platform gates remain required.

The [Linux x64 focused gate at `0e6e8d1`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40314468303) passed the unchanged 18 native text comparisons and original mesh hash with origin-preserving text hints. Its 20-startup/80-interaction measurement retained 100 raw records: the raster/setup/asset-check/crop mean was 280.4242 ms (p95 402.6222 ms), final PNG encoding averaged 32.1280 ms, and worker total averaged 336.3614 ms. Interaction-output p95 was 611.3700 ms, versus 629.1022 ms at `f838fda`. All 24 diagnostic PNGs/17,712,000 pixels, restoration, statistics and raw native records were independently verified; those PNGs and all 20 fixture avatars remained byte-identical to `f838fda` and therefore `aafc705`. Idle observation covered 30.0032 s, one persistent process, maximum sampled RSS 84,570,112 bytes and CPU upper estimate 0.36663%. These shared-host observations do not isolate the code change from runner variation. The small observed decrease does not meet the 100 ms target; complete performance acceptance remains open.

The following allocation candidate also hints opacity/filter node layers from their visual bounds while retaining the device origin, sharing the same projection helper with text layers. Its independent unbounded reference expands to 36 text conditions by adding a smaller parent translated within a larger canvas. All original conditions and the mesh hash remain required, with zero pixel tolerance. This parent-layer candidate has not yet been verified on hosted runners.

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

The fixed fixture [shadow.pen](../tests/fixtures/performance/shadow.pen) creates exactly 1,000 stored/expanded nodes: one clipped 1920×1080 frame, 999 rounded rectangles, and one `multiply` outer-shadow effect among those rectangles. It has no text, images, scripts, external assets, or browser/shader runtime. This isolates the path where one backdrop-dependent child previously made ordinary siblings capture the whole parent canvas. Ordinary siblings now avoid that capture, and nested passes retain decoded image caches.

```sh
cargo xtask fixtures performance shadow --output-dir /tmp/native-1000
lazypencil render /tmp/native-1000/shadow.pen --output /tmp/native-1000.png
```

The Rust fixture task refuses to overwrite an existing directory. Historical observations below retain their original measurement method. Fixture bytes have SHA-256 `d0a54ed3c8b3716b3ac5f26fe79e4e130164e8afd128bb4146ef96774ccbaa8b`. Measurements used the macOS arm64 debug CLI, fresh child processes, Python's monotonic clock and `wait4` peak RSS in bytes. The OS file cache was already populated; cold start and host load were not controlled. No compilation or test suite ran during these three observations.

| Run | Elapsed | Peak RSS |
| --- | --- | --- |
| 1 | 1556.97 ms | 152,158,208 bytes |
| 2 | 1496.40 ms | 152,322,048 bytes |
| 3 | 1565.22 ms | 152,092,672 bytes |

All runs exited successfully, returned no layout diagnostics or dynamic artifacts, and produced PNG SHA-256 `71f026838d25811159637859efbe051106860ace17c304b54c051b74b69815a7`. This demonstrates the fixed simple workload in a development build, at about 1.5 seconds and 145 MiB. It is not a before/after timing comparison, a release result, a percentile, or acceptance of the full performance target. The following section records initial release observations for this scene and a text/component/image workload. Interaction/idle behavior and complete process groups still require measurement.

## Linux arm64 release observations, 2026-10-23

Environment: Debian 12 arm64 in Docker Desktop, Linux `7.0.14-linuxkit`, glibc 2.36, Rust 1.94.0, with eight virtual CPUs and 4 GiB assigned to Docker. The application uses the unchanged `release` profile: thin LTO, one codegen unit, stripped debug information, and native CPU Skia. The tested executable has SHA-256 `f7f3c98946d4655741c5cabab5b12f5ede430cef95104a86eb8b87b3273b1686`.

The fixed [dashboard fixture](../xtask/src/fixtures/mod.rs) creates a 1920×1080 document with Japanese/English text, six color variables, a reusable 80-node card, eleven instances with text overrides, rounded clipping, normal shadows, and one local PNG used repeatedly. There are 131 stored nodes and exactly 1,000 expanded nodes. The document SHA-256 is `c55bcf8727afc04d31646ea1a259e17723fe9be03e3cb4fd2dccbf8512764da7`; the asset SHA-256 is `adea05ca47573c3398f5e7d10aebcba7cd6da37abdeae104131e6085cd1e0937`. The Rust fixture task creates a new directory and refuses to overwrite an existing fixture.

```sh
cargo xtask fixtures performance dashboard --output-dir /tmp/dashboard-1000
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

[performance.rs](../xtask/src/performance.rs) measures the unchanged shadow and dashboard fixtures above without invoking Cargo. It starts a fresh CLI child for every export, isolates user settings and the data directory, and supplies an explicit local project configuration. Each scene has a separate first invocation followed by 20 repeated invocations by default. Every export must have exactly 1,000 expanded nodes at 1920×1080, no diagnostics, no scripts/dynamic acquisitions, the expected asset count, and a PNG whose dimensions and SHA-256 agree with its render report. Repeated PNG hashes must match within each scene. The executable is hashed before and after measurement.

```sh
cargo xtask measure \
  --binary /path/to/lazypencil \
  --output native-benchmark.json --samples 20
```

The output includes individual samples, the first invocation, and repeated-sample minimum/median/nearest-rank p95/maximum for elapsed time, peak RSS and user/system CPU time. A 20-sample p95 is the nineteenth ordered sample, an observed tail estimate rather than a population guarantee. The Rust measurement task does not flush OS caches, so the first invocation is **not** a controlled cold start. Elapsed time covers process creation through exit observation; the completion polling interval is 1 ms, and resource-query overhead after observing exit is excluded.

On macOS/Linux, Rust calls the OS `wait4` interface through `libc` for per-child resource usage; Darwin peak RSS is recorded in bytes and Linux KiB is converted to bytes. Windows uses [`GetProcessMemoryInfo`](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo) / `PeakWorkingSetSize` in bytes and [`GetProcessTimes`](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes) CPU durations. The method is retained in each sample. These OS memory counters have their respective platform semantics; this is not a simultaneous process-group measurement. Current counter regressions are Rust integration tests in `xtask/tests/process.rs`. The historical measurements below identify their own source revisions and tooling; they do not establish current platform acceptance.

Child deadlines, bounded stdout/stderr and PNG size, exit status, workload validation, and immutable executable/pixels are required for a successful report. Reports are published atomically without overwriting another report. Temporary fixtures, logs and PNGs are removed; only the small JSON report remains. Rust integration tests now exercise real child counters independently of the previous child's high-water mark, timeout reaping, child failure/output limits, fixture hashes and settings isolation, percentile calculation, changed binaries/pixels, malformed render evidence, and publication failure cleanup.

The hosted Rust release smoke task runs this measurement against the already built executable and retains `release-benchmark.json` with the platform artifacts. Dependency-cache pruning removes that report too. Completed arm64, Windows x64, and Intel macOS measurements are recorded below with their source revisions. No hardware-dependent timing threshold is asserted by this smoke check.

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

### Hosted Intel macOS release build, 2026-10-24

The macOS 15 Intel job in [run 40014323380](https://github.com/EkeMinusYou/lazypencil/actions/runs/40014323380), at source `36d12699fb69527304d5131d72f7154cfc73668f`, passed both test configurations, Clippy, both Material opt-ins, release smoke and 42 benchmark exports. Its retained report identifies Darwin 24.6.0, x86_64, four logical CPUs and runner image `20260824.0482.1`. The executable SHA-256 is `08e40cd1b0fab950a2f99e196fd75774c93e73b4a168c4e2374dfc67c1719b61`. The overall CI run separately failed Linux x64 dependency installation.

| Scene | First invocation | Repeated median | Repeated p95 | Repeated maximum | Maximum repeated peak RSS |
| --- | --- | --- | --- | --- | --- |
| Simple shadow scene | 663.87 ms | 623.88 ms | 751.08 ms | 766.02 ms | 114,331,648 bytes |
| Dashboard | 1420.07 ms | 1426.66 ms | 1577.48 ms | 1974.17 ms | 109,617,152 bytes |

Each scene retained twenty matching repeated exports after its first invocation: shadow `d29ee020a70d0e8eafa61ca8dd6d8c317388bd88a41164cbcba0bd43f2580034`, dashboard `aa18c4fb3d2de0e746942703f469d1c2559e62e1b350284610b2f2b33376ede4`. The three separate release-smoke PNGs also matched their reports and were visually inspected. This shared-host observation has uncontrolled filesystem cache and host load; it does not establish a cold-start or interaction target, cross-hardware regression, or complete process-group resource usage.

## Same-source hosted release observations, 2026-10-24

[Run 40047491096](https://github.com/EkeMinusYou/lazypencil/actions/runs/40047491096) succeeded on all five native platforms at source `1c26faf09d47e57fbd54e4c27250947c869e1dcf`. Each release executable exported the fixed shadow and dashboard scenes at 1,000 expanded nodes and 1920×1080, with zero diagnostics, script executions or dynamic acquisitions. Each scene has one first invocation and twenty repeated fresh CLI children. All twenty-one PNGs matched within each platform/scene. Artifact ZIP digests, report/image hashes and all 42 exports per platform were verified; the table's summaries were independently recalculated from the raw repeated samples.

Elapsed time includes process creation, document loading, layout, rendering, PNG encoding and writing. It uses the parent's monotonic clock and a 1 ms completion poll. Peak RSS comes from `wait4` on Unix and the retained child process handle's `PeakWorkingSetSize` on Windows. The maximum below is the largest per-child high-water mark among the twenty repeats, not simultaneous process-group memory. Filesystem cache and hosted CPU load were uncontrolled. The first invocation is recorded separately and is not called a cold-cache measurement. No local build was performed for this observation.

| Platform | Scene | Repeated median (ms) | Repeated p95, nearest rank (ms) | Maximum repeated peak RSS (MiB) |
| --- | --- | --- | --- | --- |
| linux-x64 | shadow | 206.4 | 208.5 | 95.8 |
| linux-x64 | dashboard | 720.4 | 737.4 | 83.4 |
| linux-arm | shadow | 204.9 | 207.1 | 93.7 |
| linux-arm | dashboard | 1264.9 | 1276.0 | 77.3 |
| macos-arm | shadow | 218.5 | 256.4 | 137.5 |
| macos-arm | dashboard | 1314.7 | 1713.7 | 130.1 |
| intel | shadow | 435.5 | 454.3 | 109.3 |
| intel | dashboard | 947.8 | 995.5 | 104.7 |
| windows | shadow | 344.0 | 375.0 | 89.1 |
| windows | dashboard | 1094.0 | 1125.0 | 78.8 |

Runner identities: Linux x64 `20261012.334.1` and arm64 `20261012.149.1` used four logical CPUs and kernel `6.17.0-1022-azure`; macOS arm64 `20260907.0337.1` used three logical CPUs, Intel `20260824.0482.1` four, both Darwin 24.6.0; Windows x64 `20261021.278.1` used four logical CPUs on Windows Server 2025. Retained evidence artifacts are respectively `12542428685`, `12542349537`, `12542743195`, `12543092816`, and `12542454823`.

These are static CLI observations at that exact source, before Windows agent job supervision and TUI time seeking. They do not measure interactive latency, authenticated agents, dynamic runtime descendants, idle CPU, physical terminals or final release acceptance. Platform differences are not isolated hardware regressions.

## Remaining work

Background-aware layers preserve color alpha and replacement coverage, which can require several working rasters. Explicit scratch reservations are bounded and fail rather than dropping effects. This resource guard is separate from measured RSS.

The renderer skips nodes outside the working viewport, including unrelated browser/shader jobs. Overscan is still conservative across the document, and intermediate layers can use complete node bounds. Tighten dependency regions and intermediate-layer culling, then measure controlled cold/warm starts across the target environments on the final release build. TUI pan/zoom, image transport, runtime idle expiry, and full process-group memory remain part of the original acceptance scope.
