# Browser runtime installation

Static Skia rendering and script-generated geometry require no Chromium. Browser
nodes and GLSL fills/effects need a separate browser. The supported release platform
is macOS Tahoe 26 or later on Apple Silicon; retained Linux/Windows/Intel catalogue
entries do not expand that support.

## Managed installation

```sh
lazypencil runtime setup chromium
lazypencil runtime status
```

Use the same `--data-dir` / `LAZYPENCIL_DATA_DIR` for setup and rendering. Installation
uses `runtimes/chromium/<version>/<platform>` in that user data directory and needs no
administrator privileges to write its files. Rendering never downloads or updates it.

The [manifest](../resources/runtimes/chromium.json) pins Chrome for Testing
`chrome-headless-shell`, with exact HTTPS archive URLs, lengths, SHA-256, and installed
file hashes/modes. The hashes are computed from official downloads, not presented as
publisher-supplied checksums. Updating a pin requires acquiring/reviewing the official
archives and checking installation and rendering.

Setup verifies before bounded extraction, rejects traversal/links/unlisted files,
checks the installed tree, synchronizes data and directories, and activates by rename.
Valid installations are reused; modified ones fail with `RUNTIME_INTEGRITY` and are
not silently overwritten. Status verifies files without launching a browser.

Ctrl+C cancels lock waits, download, hashing, and extraction. Owned network workers
observe host death and bounded cleanup. Normal cancellation removes that attempt's
temporary archive/staging directory; an abrupt crash can leave inactive entries.
Once activation starts, synchronization finishes with a complete installed tree.
These are OS/storage-dependent durability rules, not a power-loss guarantee.

## Read-only diagnosis

```sh
lazypencil --json runtime status
lazypencil --json doctor
```

`available` means the selected files are present, not that startup succeeded. Reports
mark startup, sandbox-profile inspection, and shared-library inspection as `not_checked`.
Retained Linux diagnostics include observed namespace/AppArmor-related host settings;
unknown values are null, not inferred permission. Reads do not create runtime/settings
or lock directories or modify OS policy.

A failed launch includes `details.stage: "browser_startup"`; navigation failures retain
`PAGE_LOAD_FAILED` with `details.stage: "page_navigation"`. A bounded `net::ERR_` code
can identify the navigation failure without returning arbitrary page text or URLs.
Automatic startup/library/profile probing is not implemented. Do not disable Chromium's
sandbox to turn an environment failure into success.

## Offline archives

```sh
lazypencil runtime setup chromium --archive /path/to/chrome-headless-shell-mac-arm64.zip
```

Supply the exact current platform/version archive. Offline setup uses the same archive
and installed-file checks; a matching filename alone is insufficient.

## An explicitly selected browser

```sh
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil runtime status
LAZYPENCIL_CHROMIUM=/absolute/path/to/chrome-headless-shell lazypencil render design.pen --output design.png
```

The override is reported as `external-unverified`, `managed: false`, and
`integrity_verified: false`. Actual browser identity appears in acquisitions. Its
installation, libraries, licensing, CDP compatibility, and software WebGL support are
the caller's responsibility. No existing personal browser profile is adopted.

## Dedicated authentication profiles

```sh
lazypencil runtime profile create work
export LAZYPENCIL_CHROMIUM=/absolute/path/to/full-chromium
lazypencil runtime profile login work \
  --browser /absolute/path/to/full-chromium --url https://example.com/login
```

Login needs a visible full browser. Use that same canonical executable for subsequent
acquisitions; matching major versions do not make full-browser/headless-shell stores
interchangeable. First use binds the profile to a hash of its canonical executable
path, under its lease. A symlink to that path is accepted. Moving to another browser
path requires a new profile/login; updates at the same path remain browser-owned.

Close the owned login tab/window or browser to return. Ctrl+C and `--timeout-seconds`
(default 600, range 1–3,600) close it through normal cleanup. Select the profile in
**user** configuration:

```toml
[runtime.browser]
profile = "work"
```

A project can disable it with `profile = ""`, but cannot select a nonempty name.
Names use 1–64 lowercase ASCII letters/digits/hyphens, starting with a letter/digit.
`runtime profile path work` returns its private directory for management.

Persistent cookies, localStorage, IndexedDB, and Cache Storage live under
`browser-profiles/<name>` in the selected data directory. They are not imported into
`.pen` files or conversations, profile names are omitted from acquisition metadata,
and shader jobs never use them. Host cleanup preserves persistent storage.

One guardian-owned browser leases a profile. On macOS a private local page worker
serializes acquisitions from retained TUI previews and bound MCP frontends, scoping
retained tabs, cancellation, resources, and disconnect cleanup per frontend. It verifies
peer ownership, profile identity, browser path, and supervisor identity. Each frontend
retains at most eight pages, with at most sixteen connected frontends. Independent
browser processes cannot use the same store simultaneously; explicit login still needs
its exclusive lease. Network-policy changes can restart the page browser and reset DOM
while preserving disk authentication data.

## Execution and licensing

Rust uses CDP directly; this rendering path needs neither Node.js nor Puppeteer.
Anonymous new acquisitions use fresh isolated contexts with downloads denied. TUI
retention and explicit profiles change only the documented page-state lifetime.
Page and shader jobs use separate processes.

Only the shader process selects `--use-gl=angle --use-angle=swiftshader-webgl
--enable-unsafe-swiftshader`. This software WebGL opt-in has weaker security guarantees
than Chromium's normal GPU path; it receives captured GLSL/images and blocks direct
HTTP(S), file and FTP acquisition. The page process has no such flags. Chromium's own
OS sandbox remains enabled; `--no-sandbox` is not added.

The downloaded runtime keeps its complete pinned official contents, notices and
supporting resources. Its licenses remain separate from lazypencil's MIT license.
The product binary does not bundle a browser archive. Redistribution requires the
applicable original notices; see [distribution](development.md#licenses-and-third-party-inputs).

## Integration checks

Real-browser tests are opt-in and require a working sandboxed executable:

```sh
LAZYPENCIL_CONFIG_DIR="$(mktemp -d)" \
LAZYPENCIL_TEST_CHROMIUM=/path/to/chrome-headless-shell \
  cargo test --locked --test dynamic_render -- --ignored --test-threads=1
```

Installer opt-ins use `LAZYPENCIL_TEST_CHROMIUM_ARCHIVE` with the exact pinned ZIP.
MCP, browser-readiness, uniform, and terminal tests have their own explicit filters;
prepare their helper/settings as described in [development](development.md).

The [Chromium workflow](../.github/workflows/chromium.yml) delegates to the existing
CI pipeline, using the supported Mac default. Reuse an existing matching full run
instead of dispatching overlapping browser verification. Explicit deferred-platform
diagnostics and their host-library/sandbox setup do not certify a supported release.
