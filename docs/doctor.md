# Diagnose a lazypencil installation

Run `lazypencil doctor` before opening a design to check configuration, agent setup
and the local preview pipeline. Checks are automatic; doctor never asks whether an
image is visible and never waits for user confirmation.

```sh
lazypencil doctor
lazypencil doctor --agent codex
lazypencil doctor --agent claude --timeout-seconds 10
lazypencil --json doctor --strict
```

The text report lists OK/WARN/FAIL/SKIP checks with remedies. JSON retains the existing
build/runtime fields and adds `checks`, `summary` and `agents`. A normal invocation
exits successfully when it can produce a report. `--strict` returns 1 when the report
contains errors, while still printing it; warnings do not fail strict mode. Command
errors and cancellation retain their normal exit codes.

## Checks

| Area | Automatic checks |
| --- | --- |
| Build | Version, platform, compiled rendering/TUI features and release metadata |
| Supported platform | OS/CPU release target and minimum macOS version from the release declaration |
| User configuration | Selected config path, syntax and validated preferences; an absent file uses defaults |
| Directories | Data/lock paths and access to existing parent directories, without creating probe files |
| Fonts | File/search-directory availability, loading all configured faces, and the configured default/fallback families |
| ACP agents | Executable resolution, multiple PATH copies, Node.js when needed, ACP v1 initialization and reported version |
| Authentication | Reported login state and agreement with the selected subscription/API route |
| Steering | The same advertised capability and compatible-version checks used by the TUI |
| Preview renderer | A known 128×128 image, PNG encode/decode and five expected pixel samples |
| Terminal images | TUI image encoding/presentation and, when supported, upload/placement receipts |
| Chromium | Optional runtime installation/integrity status; the browser is not launched |

## Agent checks

Both installed adapters are checked by default. `--agent codex` or `--agent claude`
limits the check to one. The selected agent defaults to the user's `agents.default`,
then Codex; its failures are errors. An uninstalled unselected adapter is listed as
optional. User-profile adapter paths and arguments are honored, including paths
relative to the physical configuration file. Project executable settings are not
loaded or executed by doctor.

Each installed adapter is started, initialized over ACP and asked only for the
authentication status it normally pushes. No session is created or loaded, no model
prompt or authenticate request is sent, and no MCP tools or login flow are opened.
The adapter/runtime can maintain its own startup or cache state. Probe processes
are closed afterward. The default initialization/account-reporting deadline is five
seconds per adapter; `--timeout-seconds` accepts 1–30 seconds.

Missing executables, missing Node.js, startup/protocol failures, explicitly reported
signed-out states, authentication-route mismatches and unreported authentication
have different results. An installed executable alone does not establish that AI
requests can run. Model availability, quota and inference are not tested. Adapter
arguments, identity, credentials and vendor metadata are not printed.

## Automatic preview checks

The renderer draws red, green, blue and white quadrants with a black circle in
memory. Doctor encodes the result as PNG, decodes it and checks the dimensions and
expected pixels. No design or image file is written. This also runs in JSON output
and without an interactive terminal when rendering is compiled in.

With interactive stdin/stdout and text output, doctor briefly presents the test image
using the same terminal encoder and presenter as the TUI. Protocol detection is
automatic; `--image-protocol kitty`, `sixel` or `halfblocks` can test a specific path.
The terminal is restored after the test, including on failure.

Direct Kitty graphics can acknowledge both upload and placement. Doctor requests
these receipts and waits at most 500 ms. A negative receipt is a failure; missing
receipts are unconfirmed. Sixel, half-block fallback and other paths without a receipt
are reported as encoded/written with display unconfirmed. A receipt establishes
protocol acceptance, not physical visibility or correct screen composition. Doctor
does not capture the screen or ask the user to validate it.

JSON output and noninteractive invocations skip terminal queries and graphics so
output remains machine-readable. Renderer checks still run. Builds without rendering
or TUI support report the corresponding checks as unavailable/not checked.

## Limits and remedies

Use the reported executable path and PATH order when upgrading an adapter; an older
installation can shadow a newly installed copy. Follow the supplied npm installation
hint or fix a custom adapter path. Sign in through the selected runtime when it
reports a signed-out state. Doctor does not install dependencies or initiate login.

Directory checks inspect existing permissions rather than testing an actual write.
Font availability and actual loading have separate results. Rendering builds load the
configured faces and default/fallback families through the normal font loader;
core builds report font loading as not checked. The supported-platform check reads
the macOS product version locally; an older macOS is an error, while an unknown
version or an unsupported release OS/CPU is a warning. The optional Chromium check
does not certify browser launchability. Use `lazypencil --json runtime status` and the
[runtime reference](runtimes.md) for browser-specific details.
