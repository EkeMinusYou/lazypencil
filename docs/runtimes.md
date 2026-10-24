# Browser runtime installation

Initial-release support is limited to macOS Apple Silicon and Linux x64/arm64. Intel Mac is unsupported; Windows support and additional work are deferred. Existing runtime catalogue entries and earlier Windows/Intel evidence are retained implementation/history, not release support or acceptance requirements. The same three targets are used by normal and manual CI.

Static Skia rendering and script-generated geometry do not require Chromium. Browser nodes and GLSL fills/effects require a separate headless browser installation. Rendering never downloads or updates a runtime.

## Managed installation

```sh
lazypencil runtime setup chromium
lazypencil runtime status
```

Use the same `--data-dir` or `LAZYPENCIL_DATA_DIR` for setup and rendering. The installer stores the runtime under `runtimes/chromium/<version>/<platform>` in that directory and keeps versions separate. It requires no administrator privileges to write its files. Linux may still require system libraries; automated host-library diagnosis remains unfinished.

The current manifest pins Chrome for Testing **156.0.8078.47**, specifically `chrome-headless-shell`, from the [official Chrome for Testing distribution](https://googlechromelabs.github.io/chrome-for-testing/). Platform archives are recorded for macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64. Windows and Intel Mac entries are retained for implementation/history and do not expand initial-release support. Installation and rendering must be verified on the three initial-release targets; archive availability alone does not establish support.

[The manifest](../resources/runtimes/chromium.json) records the exact HTTPS URL, archive byte length and SHA-256, and the name, size, SHA-256, and executable bit of every installed file. These hashes were computed from the versioned official downloads; they are **not publisher-supplied checksums**. Updating a pin requires acquiring the official archives, regenerating and reviewing their file manifests, and running the installation and rendering checks before release.

The installer verifies the archive before bounded extraction, rejects traversal paths, symbolic links and unpinned files, checks the complete installed file set, then activates the staging directory by rename. A verified installation is reused. A modified installation fails with `RUNTIME_INTEGRITY` and is not silently overwritten. `runtime status` reports installation availability and integrity without downloading anything or launching the browser. Installation cancellation and complete cross-platform filesystem-durability checks still require work.

Configured launch and acquisition deadlines also bound partial HTTP/WebSocket reads and blocked sends. Receiving more bytes does not extend the deadline. Cancellation remains active while a response is incomplete, and each subsequent command uses its own current operation budget. Regression tests exercise partial responses and stalled sends separately from real browser rendering.

## Read-only diagnosis

```sh
lazypencil --json runtime status
lazypencil --json doctor
```

Both reports include the Chromium installation status and explicitly mark `startup: "not_checked"`. `available` describes installed files, not a successful browser launch. `host` reports OS/architecture, the required browser sandbox, and `sandbox_profile` / `shared_libraries` as `not_checked`. On Linux it also reports the effective UID and observed `unprivileged_userns_clone`, `apparmor_restrict_unprivileged_userns` and `max_user_namespaces` values. Missing, unreadable, malformed or oversized values are `null`; they are not treated as permission granted or denied. Reads are bounded, and neither command changes OS policy or creates runtime/settings/lock directories.

Page navigation failures retain `PAGE_LOAD_FAILED` and include `details.stage: "page_navigation"`. `details.browser_error` contains only a bounded `net::ERR_` identifier with an uppercase/digit/underscore suffix, or `null` for another response. Arbitrary browser text, URLs and page contents are omitted. This identifies network errors without treating every failed navigation as a selector or runtime-startup failure.

These host facts do not determine which AppArmor profile will apply to a selected executable. A restricted namespace setting can coexist with an allowed browser, as the hosted integration demonstrates. If the browser child exits before CDP is ready, the failure includes `error.details.stage: "browser_startup"` and the same host facts, with a command for obtaining the full report. Automated shared-library dependency inspection, profile verification, and a general startup probe in the product remain unfinished. The new reporting and read-only regression tests await hosted verification.

## Offline archives

```sh
lazypencil runtime setup chromium --archive /path/to/chrome-headless-shell-mac-arm64.zip
```

Supply the exact archive for the current platform and pinned version. Offline setup applies the same byte-length, archive-hash, and per-file checks. It does not accept arbitrary ZIP files or infer compatibility from their names.

## An explicitly selected browser

```sh
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil runtime status
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil render design.pen --output design.png
```

This explicit override is reported as `external-unverified`, with `managed: false` and `integrity_verified: false`. The actual running browser version is included in acquisition metadata. Its installation, supporting libraries, licensing, CDP compatibility, and software WebGL capability are the caller's responsibility. No existing personal browser profile is reused.

## Execution and licensing

Rust connects directly to a loopback Chrome DevTools Protocol socket. Node.js and Puppeteer are not needed for this rendering path. Each process uses a temporary private browser profile. CLI/MCP acquisitions and shader jobs use fresh isolated contexts with downloads denied. The TUI opts into up to eight isolated page contexts tied to its document and nodes; capture refresh preserves their DOM, while URL/device changes, eviction, idle expiry, failure or browser-policy changes can reset them. Downloads remain denied. This new retention path is undergoing hosted verification. Page and shader jobs use separate processes. Browser JavaScript is enabled for page captures.

The shader process explicitly selects `--use-gl=angle --use-angle=swiftshader-webgl --enable-unsafe-swiftshader`. Chromium documents that this software WebGL opt-in has weaker security guarantees than its normal GPU path. It is restricted here to the shader process, which receives captured GLSL and image data and blocks HTTP(S), file, and FTP requests. The ordinary page process does not receive these flags. Chromium's own OS sandbox remains enabled; lazypencil does not add `--no-sandbox`. See [Chromium's SwiftShader documentation](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/gpu/swiftshader.md).

The downloaded browser, software renderer, and supporting libraries retain their own licensing. The installer keeps the complete pinned official archive contents, including `ABOUT`, license notices, and supporting resources. lazypencil's MIT license does not relicense this separate runtime. Runtime redistribution and the complete release dependency/license audit must be checked before publishing a packaged release; no browser archive is included in this repository's product binary.

## Run the native integration checks

After building tests with `cargo test --no-run`, provide an installed executable and the matching official archive:

```sh
LAZYPENCIL_TEST_CHROMIUM=/path/to/chrome-headless-shell cargo test --test dynamic_render -- --ignored --test-threads=1
LAZYPENCIL_TEST_CHROMIUM=/path/to/chrome-headless-shell cargo test --test mcp -- --ignored --test-threads=1
LAZYPENCIL_TEST_CHROMIUM_ARCHIVE=/path/to/pinned.zip cargo test --test runtime -- --ignored --test-threads=1
```

These tests launch a real browser and are intentionally separate from ordinary unit tests. The tests cover page pixels, transforms and capture conditions, cache refresh, cancellation and recovery, software shader pixels and extensions, object/background sampling, effect order, texture orientation and opacity, CLI interruption, MCP image content, and installation integrity. In a macOS development sandbox, browser startup may need host permission for Mach services. Do not work around an environment restriction by disabling Chromium's own sandbox.

## Hosted integration without local build trees

The manually dispatched [Chromium integration workflow](../.github/workflows/chromium.yml) delegates to the same CI pipeline for one selected initial-release target: macOS Apple Silicon, Linux x64 or Linux arm64. The native job builds verified distributed source, reuses matching dependencies, removes application outputs, installs/verifies the pinned browser and runs ordinary and real browser checks with the same compiled binaries. Sandboxed pixels, both feature configurations, Clippy, real MCP idle cleanup and PTY tests remain covered. A single-target diagnostic run does not produce the full release source artifact or establish three-target acceptance. Do not dispatch it alongside full CI to repeat browser checks.

```sh
gh workflow run chromium.yml --ref main -f os=ubuntu-24.04
gh workflow run chromium.yml --ref main -f os=macos-15
```

The only platform choices are `ubuntu-24.04` (Linux x64), `ubuntu-24.04-arm` (Linux arm64) and `macos-15` (Apple Silicon). Runs retain runtime identity/integrity reports, test logs and a hash-checked combined example PNG for three days. They use the shared native dependency-cache policy and keep build trees/browser downloads on the hosted runner. A dispatch or successful cache restore does not establish a passed integration check; inspect the completed run and its evidence.

The native dependency cache identity uses committed dependency/build-script inputs, the actual Rust compiler identity, declared compilation flags, platform and runner image. Application outputs are pruned before verification. Scheduling-only workflow edits do not invalidate those dependencies. Migration from the four previously saved v1 platform caches is enabled only for the audited input fingerprint, Rust 1.94.0 commit and compilation flags; other inputs use a fresh v2 key. Cache identification/restoration is optional, while source verification and tests remain required. Build-affecting environment must be declared before identification; changes to this cache policy require a key-version review. Hosted reuse of the new keys and migration remains to be verified.

Linux jobs probe the verified binary with an isolated `about:blank` profile, a bounded lifetime and core dumps disabled. The artifacts retain its stderr, shared-library resolution and namespace-related kernel settings, even on failure. Run `40008545887` reported `No usable sandbox!`, resolved shared libraries, and `apparmor_restrict_unprivileged_userns=1`.

On a hosted Linux runner with that restriction enabled, the workflow loads an AppArmor profile granting `userns` to the exact canonical path of the already verified browser. The generated path is restricted to this job's managed runtime directory, contains no wildcard, and is retained with a hashed policy report. The workflow checks that the global restriction remains `1`; it neither changes sysctl settings nor disables Chromium's sandbox. This targeted hosted allowance follows [Chromium's AppArmor guidance](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md). Run `40009150116` then passed sandboxed startup, fifteen dynamic-render tests and all ten numeric-uniform tests; its separate CLI interruption failure remains under investigation. The product installer does not load privileged OS profiles. On a similarly restricted user system, an administrator must review the corresponding exact browser path and OS policy before enabling namespace sandboxing; the browser remains unavailable until a supported sandbox can start.

[macOS arm64 run 40007811724](https://github.com/EkeMinusYou/lazypencil/actions/runs/40007811724), at commit `9a76e66104c388e0ea1656ba3be5d4cfc60bac8f`, passed seven configuration tests, all sixteen dynamic-render tests, actual MCP image/refresh and 60-second idle checks, and the dynamic TUI PTY test with its real idle interval. The retrieved 1000×640 combined PNG has SHA-256 `4d1ca7eb7e29ceec82c6ef5556c40466bdfe40c9ae84cc924ffb7b68332b51ae`; its dimensions, report hash, two acquisition limits and diagnostic-free result were rechecked, and the image was visually inspected. This run precedes the expanded JS-worker, shader-uniform and normal-TUI coverage above. Linux's corresponding run failed during real Chromium startup; other platforms and the expanded checks remain pending.

Both [Linux x64 run 40010090321](https://github.com/EkeMinusYou/lazypencil/actions/runs/40010090321) and [macOS arm64 run 40010094357](https://github.com/EkeMinusYou/lazypencil/actions/runs/40010094357), at source `c9dd6cb`, passed the expanded 92-test scope: all 39 library tests, seven configuration tests, twelve real JS-worker tests, sixteen dynamic renders, ten uniform tests, actual MCP image and idle checks, and all six TUI tests including the twelve normal PTY cases. Linux CLI interruption returned 130 after the CDP retry fix; the TUI policy-only timeout/recovery assertions passed on both systems. The subsequent host-diagnostic changes, remaining platforms, full normal CI and release gates still need their own results.

At source `36d1269`, the expanded scope including read-only host diagnostics and usage-journal contention/cancellation passed on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40014476233) (97 tests), [Linux x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40014480311) (99), [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40014488317) (99), and [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40014484065) (84). Windows omits the Unix PTY and real idle-cleanup checks; it includes the real MCP image check. Each run restored its audited legacy native cache. These results precede the new network-idle waiting policy and do not replace full normal CI, physical-terminal tests or release verification.

Source `8f14b13` additionally passed the new network-idle lifecycle/configuration/real-fetch regression on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40015918054) (101 tests), [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40015927331) (103), and [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40015922773) (88). The real fetch test covers delayed green pixels, policy-driven reacquisition, cache reuse, timeout and recovery, cancellation, and unchanged document bytes. Unix MCP idle and TUI checks also passed. This is dedicated integration evidence; full normal CI, Clippy/release at that source and Linux x64's new wait mode still require their results.

[Linux x64 run 40017390797](https://github.com/EkeMinusYou/lazypencil/actions/runs/40017390797), at source `f2b9b84`, passed that expanded scope (103 tests), including the new network-idle regression, actual MCP idle cleanup and all TUI tests. Its package installation completed in 18 seconds using the temporary official-Ubuntu sources; the previous normal job had exhausted its 20-minute limit downloading packages from the Azure mirror. The package list and archive-signature checks were preserved. The subsequent selector-readiness changes and full normal CI remain separate pending verification.

Selector readiness at source `01d559d` passed the complete dedicated scope on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40019388943) (107 tests), [Linux x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40019393874) (109), and [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40019397974) (109). This includes configuration, four real selector tests and Unix MCP/TUI cleanup. Windows passed three selector tests but failed HTTP recovery after the cancellation/reaping assertions had passed. Its follow-up adds bounded HTTP-fixture waiting and network-code diagnostics; that correction and the actual-window scroll metadata regression require new hosted results. The downloaded macOS report/PNG retained default selector settings and matched the previously inspected image byte for byte.

The follow-up at source `d617d5e` passed on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40021082049) (109 tests), [Linux x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40021085896) (111), [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40021090172) (111), and [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40021078150) (96). This verifies same-HTTP-URL recovery after cancellation, delayed-header/idle-shutdown fixtures, bounded navigation-code diagnostics, and actual window scroll metadata despite same-named page variables. Unix MCP idle/TUI tests also passed. The original failed Windows navigation had discarded its network code, so its exact cause remains unproven.

All four downloaded 1000×640 PNGs matched their reports, had no diagnostics and retained the expected selector/state/limits and verified browser identity. Each matched its own earlier platform baseline byte for byte; the baselines for macOS, Windows and Linux arm64 had also been visually inspected. The runs restored v2 native archives on macOS/Windows/Linux arm64 and the audited v1 archive on Linux x64. The source-scoped normal CI and subsequent Intel result are recorded below and in the implementation status; physical-terminal coverage remains pending.

At source `5a29847`, [Intel macOS run 40027340811](https://github.com/EkeMinusYou/lazypencil/actions/runs/40027340811) passed all 109 executed tests, including actual MCP image content, sixty-second MCP idle cleanup and all six Unix TUI tests. A separate runtime-archive opt-in remained excluded. This uses a selector-cleared warm capture before timing selector cancellation and sends the fixture probe only for the target `#never` query; the three-second observation and five-second acquisition limits remain unchanged. The prior cold-start attempt at source `74d4a3d` did not observe that query in time, and did not establish the direct cause. The corrected run observed the actual query before cancellation, verified owned-process cleanup and rendered successfully afterward. The v2 Intel dependency-cache restore was confirmed from completed logs.

The downloaded Intel artifact's ZIP hash and 1000×640 PNG/report hash matched, with no diagnostics and two acquisitions. It retained the effective default selector/state/limits, a verified managed runtime and actual `HeadlessChrome/156.0.8078.47` acquisition identity. The image was also visually inspected; SHA-256 `b10c0cf5f90f371eeca13b9f45f5a0a3cd2570f453df4f2258306ea5d6f5328d` is the first retained Intel baseline. These hosted tests do not establish physical terminal/clipboard behavior, minimum OS support, or full design compatibility.
