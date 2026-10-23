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

The platform choices also include `ubuntu-24.04-arm`, `macos-15-intel` and `windows-2025`. Runs retain runtime identity/integrity reports, test logs and a hash-checked combined example PNG for three days. They restore dependency caches without saving replacements, and keep build trees/browser downloads on the hosted runner. A dispatch or successful cache restore does not establish a passed integration check; inspect the completed run and its evidence. These new hosted integration runs remain to be verified.

Linux jobs first probe the verified binary with an isolated `about:blank` profile, a bounded lifetime and core dumps disabled. The artifacts retain its stderr, shared-library resolution and namespace-related kernel settings, even on failure. This probe does not alter OS policy. Ubuntu's user-namespace restrictions can affect downloaded Chromium builds; [Chromium's documentation](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md) explains the distinction and administrator-controlled options. A generic SIGTRAP alone does not prove that cause.
