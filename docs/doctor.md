# Diagnose setup problems

Run `lazypencil doctor` when an agent will not connect, fonts are missing, or the
preview will not display. Each reported problem includes a suggested next step.

```sh
lazypencil doctor
```

To check only the agent you use, add `--agent codex` or `--agent claude`. For a
slow-starting adapter, increase its timeout:

```sh
lazypencil doctor --agent claude --timeout-seconds 10
```

## Read the report

| Result | Meaning |
| --- | --- |
| OK | The check passed |
| WARN | Review the advice; an optional feature or an unconfirmed check may need attention |
| FAIL | Follow the suggested fix before using the affected feature |
| SKIP | The check does not apply or could not run in this environment |

The report covers your installed version, supported platform, settings, directory
access, fonts, AI adapters and login state, image rendering, terminal graphics,
and the optional Chromium runtime.

A missing ACP adapter matters for TUI chat and the `prompt` command. MCP clients
use their own AI connection, and PNG export can be used without an adapter.
Chromium is needed only for [browser nodes and GLSL](runtimes.md).

## Fix common problems

| Problem | What to do |
| --- | --- |
| Adapter not found | Install the selected [ACP adapter](agents.md#install-and-authenticate-an-adapter), then check its executable is on `PATH` |
| An old adapter is selected | Use the executable path printed in the report to find and update the copy that is actually running |
| Login missing or authentication route differs | Sign in through your AI runtime, or select the intended [subscription/API mode](agents.md#send-a-prompt) |
| Adapter times out | Retry with `--timeout-seconds 10`; accepted values are 1–30 seconds |
| A font is missing | Correct the font path in your [configuration](configuration.md) or install the required font |
| Terminal images fail | Try Kitty or WezTerm, use `--image-protocol halfblocks`, or [open the preview externally](tui.md#terminal-image-previews) |
| Chromium is missing | Run `lazypencil runtime setup chromium` when you need browser nodes or GLSL |

## Preview checks

In an interactive terminal, doctor briefly displays a test image. To try a
specific image protocol, use `--image-protocol kitty`, `sixel` or `halfblocks`.
PNG rendering is also checked when output is redirected or JSON is requested.

Some terminals acknowledge image delivery; others cannot report whether an image
was displayed. An unconfirmed display result is a reason to try the preview, not
an indication that the design cannot be rendered. If the image looks wrong, use
PNG export to check the design in another app.

## What doctor checks

Doctor starts installed adapters to check connectivity and their reported login
state. It sends no model prompt, so quota and model access are established only
when you send a request. Authentication stays with your AI runtime.

The command checks existing settings and files, leaves designs unchanged, and
suggests installation or login steps for you to run. The Chromium status check
checks the installed files; see [runtime troubleshooting](runtimes.md#read-only-diagnosis)
for browser launch failures.

## Use in scripts

```sh
lazypencil --json doctor --strict
```

`--json` returns a structured report with `checks`, `summary` and `agents`.
`--strict` exits with status 1 when the report contains errors; warnings alone
keep status 0. Without `--strict`, a successfully produced report exits with
status 0 even when individual checks fail. Command errors still return an error
status. JSON output skips terminal graphics and terminal queries.
