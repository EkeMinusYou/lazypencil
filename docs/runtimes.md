# Install Chromium for browser content and GLSL

Browser nodes and GLSL fills/effects use a separate Chromium runtime. Install it
when you start using those features:

```sh
lazypencil runtime setup chromium
lazypencil runtime status
```

Ordinary shapes, text, icons and script-generated geometry render locally without
this runtime.

## Managed installation

Setup installs the selected Chrome for Testing `chrome-headless-shell` in your
lazypencil data directory. It verifies the download and installed files and reuses
an existing valid installation. Use the same `--data-dir` or `LAZYPENCIL_DATA_DIR`
for setup and rendering.

Rendering leaves runtime installation to `runtime setup`; it does not download a
missing browser in the middle of an export. Ctrl+C cancels setup. If setup is
interrupted, rerun it to complete the installation.

## Read-only diagnosis

```sh
lazypencil runtime status
lazypencil doctor
```

Status reports whether the selected runtime files are available and intact. To
check that the browser can actually start, render a document with a browser node
or GLSL. Follow the reported stage when troubleshooting:

| Result | What to check |
| --- | --- |
| Runtime unavailable | Run `runtime setup chromium` with the rendering command's data directory |
| `RUNTIME_INTEGRITY` | Installed files differ from the expected runtime; inspect the reported path before repairing the installation |
| `browser_startup` | Check the selected executable and its ability to start on this system |
| `PAGE_LOAD_FAILED`, `page_navigation` | Check the page URL, access requirements and [capture/network settings](configuration.md#browser-shader-and-raster-cache-limits) |

Keep Chromium's sandbox enabled when diagnosing startup problems.
For structured output, add `--json` to `runtime status` or `doctor`.

## Offline archives

```sh
lazypencil runtime setup chromium --archive /path/to/chrome-headless-shell-mac-arm64.zip
```

Use the archive for the platform and version in the
[runtime manifest](../resources/runtimes/chromium.json). Offline setup performs
the same integrity checks as an online installation.

## An explicitly selected browser

To use another compatible executable, set `LAZYPENCIL_CHROMIUM`:

```sh
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil runtime status
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil render design.pen --output design.png
```

The report identifies it as `external-unverified`. You manage that installation,
its updates and its compatibility with the required browser features. The
managed runtime is the simplest starting point for ordinary captures and GLSL.

## Dedicated authentication profiles

For pages that need a login, create a lazypencil profile and sign in using a
visible full Chromium browser:

```sh
lazypencil runtime profile create work
export LAZYPENCIL_CHROMIUM=/absolute/path/to/full-chromium
lazypencil runtime profile login work \
  --browser /absolute/path/to/full-chromium --url https://example.com/login
```

Close the login tab/window to return. Ctrl+C also closes the login browser.
`--timeout-seconds` defaults to 600 and accepts 1–3,600 seconds.

Select the profile in your user configuration:

```toml
[runtime.browser]
profile = "work"
```

Use the same browser executable path for login and subsequent captures. Switching
to another path requires a new profile and login, even if the browser versions
match. Updates at the same path continue to use the profile.

Profile names use 1–64 lowercase ASCII letters, digits or hyphens, beginning with
a letter or digit. `runtime profile path work` returns its storage directory.
Cookies, local storage and related browser data remain there across sessions.
They are separate from `.pen` files and conversations.

A project can disable the selected profile with `profile = ""`; choosing a named
profile is a user setting. Close an active login browser before capturing pages
with that profile. Concurrent lazypencil previews share the managed profile browser;
separate browser processes cannot open its storage at the same time.
Network-policy changes can reset open pages while preserving stored login data.

## Execution and licensing

Anonymous captures use isolated contexts; authenticated captures use the selected
profile. Browser pages and GLSL run separately. GLSL uses software WebGL, and its
image inputs are supplied through the document's captured assets. See
[dynamic content](dynamic-content.md) for supported features and limitations.

The downloaded runtime retains its original Chromium files and notices. Its
licenses are separate from lazypencil's MIT license. If redistributing it, retain
the applicable notices; see [distribution](development.md#licenses-and-third-party-inputs).

## Integration checks

Runtime and browser contributors can run opt-in integration tests using
`LAZYPENCIL_TEST_CHROMIUM` and `LAZYPENCIL_TEST_CHROMIUM_ARCHIVE` with the matching
executable and pinned archive. See [development](development.md#focused-development-checks)
and [dynamic rendering tests](../tests/dynamic_render.rs) for specific filters.
