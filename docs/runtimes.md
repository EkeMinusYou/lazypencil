# Browser runtime installation

Static Skia rendering and script-generated geometry do not require Chromium. Browser nodes and GLSL fills/effects require a separate headless browser installation. Rendering never downloads or updates a runtime.

## Managed installation

```sh
lazypencil runtime setup chromium
lazypencil runtime status
```

Use the same `--data-dir` or `LAZYPENCIL_DATA_DIR` for setup and rendering. The installer stores the runtime under `runtimes/chromium/<version>/<platform>` in that directory and keeps versions separate. It requires no administrator privileges to write its files. Linux may still require system libraries; automated host-library diagnosis remains unfinished.

The current manifest pins Chrome for Testing **156.0.8078.47**, specifically `chrome-headless-shell`, from the [official Chrome for Testing distribution](https://googlechromelabs.github.io/chrome-for-testing/). Platform archives are recorded for macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64. The installer and real rendering have been exercised on macOS arm64. Artifact availability is not verification that lazypencil works on the other platforms; those release checks remain required.

[The manifest](../resources/runtimes/chromium.json) records the exact HTTPS URL, archive byte length and SHA-256, and the name, size, SHA-256, and executable bit of every installed file. These hashes were computed from the versioned official downloads; they are **not publisher-supplied checksums**. Updating a pin requires acquiring the official archives, regenerating and reviewing their file manifests, and running the installation and rendering checks before release.

The installer verifies the archive before bounded extraction, rejects traversal paths, symbolic links and unpinned files, checks the complete installed file set, then activates the staging directory by rename. A verified installation is reused. A modified installation fails with `RUNTIME_INTEGRITY` and is not silently overwritten. `runtime status` reports availability and integrity without downloading anything or launching the browser. Installation cancellation and complete cross-platform filesystem-durability checks still require work.

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

Rust connects directly to a loopback Chrome DevTools Protocol socket. Node.js and Puppeteer are not needed for this rendering path. Each process uses a temporary private browser profile; each acquisition uses a fresh isolated context with downloads denied. Page and shader jobs use separate processes. Browser JavaScript is enabled for page captures.

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

The manually dispatched [Chromium integration workflow](../.github/workflows/chromium.yml) builds the verified source distribution on one selected hosted platform. It restores the normal CI job's matching dependency cache, removes application outputs before rebuilding, explicitly installs/verifies the pinned browser, and runs runtime-policy/JS-worker, real page/shader, numeric-array/matrix and MCP image tests. Unix jobs also run the real MCP idle and all normal/dynamic TUI checks; these Unix-only checks are skipped explicitly on Windows. Chromium's OS sandbox stays enabled. Normal CI remains the separate gate for both feature configurations, Clippy, dependency policy and release checks.

```sh
gh workflow run chromium.yml --ref main -f os=ubuntu-24.04
gh workflow run chromium.yml --ref main -f os=macos-15
```

The platform choices also include `ubuntu-24.04-arm`, `macos-15-intel` and `windows-2025`. Runs retain runtime identity/integrity reports, test logs and a hash-checked combined example PNG for three days. They restore dependency caches without saving replacements, and keep build trees/browser downloads on the hosted runner. A dispatch or successful cache restore does not establish a passed integration check; inspect the completed run and its evidence.

Linux jobs probe the verified binary with an isolated `about:blank` profile, a bounded lifetime and core dumps disabled. The artifacts retain its stderr, shared-library resolution and namespace-related kernel settings, even on failure. Run `40008545887` reported `No usable sandbox!`, resolved shared libraries, and `apparmor_restrict_unprivileged_userns=1`.

On a hosted Linux runner with that restriction enabled, the workflow now loads an AppArmor profile granting `userns` to the exact canonical path of the already verified browser. The generated path is restricted to this job's managed runtime directory, contains no wildcard, and is retained with a hashed policy report. The workflow checks that the global restriction remains `1`; it neither changes sysctl settings nor disables Chromium's sandbox. This targeted hosted allowance follows [Chromium's AppArmor guidance](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md) and still awaits the next run's result. The product installer does not load privileged OS profiles. On a similarly restricted user system, an administrator must review the corresponding exact browser path and OS policy before enabling namespace sandboxing; the browser remains unavailable until a supported sandbox can start.

[macOS arm64 run 40007811724](https://github.com/EkeMinusYou/lazypencil/actions/runs/40007811724), at commit `9a76e66104c388e0ea1656ba3be5d4cfc60bac8f`, passed seven configuration tests, all sixteen dynamic-render tests, actual MCP image/refresh and 60-second idle checks, and the dynamic TUI PTY test with its real idle interval. The retrieved 1000×640 combined PNG has SHA-256 `4d1ca7eb7e29ceec82c6ef5556c40466bdfe40c9ae84cc924ffb7b68332b51ae`; its dimensions, report hash, two acquisition limits and diagnostic-free result were rechecked, and the image was visually inspected. This run precedes the expanded JS-worker, shader-uniform and normal-TUI coverage above. Linux's corresponding run failed during real Chromium startup; other platforms and the expanded checks remain pending.
