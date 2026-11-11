# User and project configuration

Use user settings for your preferred fonts, AI agent, theme and controls. Use
project settings for shared fonts, rendering policy and design guidance.

| File | Purpose |
| --- | --- |
| `~/.config/lazypencil/config.toml` | Your defaults across projects |
| `lazypencil.toml` in a project | Shared settings for documents in that project |

## Start and inspect

```sh
lazypencil config init --user
lazypencil config init
lazypencil config show --user
lazypencil config show design.pen
```

`config init --user` creates the user file and its parent directories. `config init`
creates `lazypencil.toml` in the current directory; `config init PATH` selects an
existing directory's `lazypencil.toml`. Both commands preserve existing files.
Templates leave optional fields commented so defaults continue to apply.

`config show` displays effective settings and their sources. For the full schema,
run `config schema --user` or `config schema`. Unknown fields, invalid values,
duplicate keys and unsupported versions produce `CONFIGURATION_ERROR` with the
file and location to correct. Configuration files are limited to 256 KiB.

For each field, precedence is **explicit project value > explicit user value >
built-in default**. Command-line options take precedence where available. Arrays
replace inherited arrays: `files = []`, for example, clears inherited font files.

TUI controls, attachment budgets and conversation retention belong in user settings.
Projects can set fonts, runtime policy, agent defaults and design resources.
Project-supplied agent executables and design instructions each require a local
trust decision, described below.

## Discovery and path rules

User configuration uses `$XDG_CONFIG_HOME/lazypencil/config.toml` when
`XDG_CONFIG_HOME` is an absolute path, otherwise `~/.config/lazypencil/config.toml`.
`LAZYPENCIL_CONFIG_DIR` overrides that directory. Run `lazypencil doctor` to see
which file is selected if your settings seem to be ignored.

Project discovery starts beside the document and walks up to the nearest
`lazypencil.toml`, stopping at a Git repository or worktree boundary. Only that
project file applies; parent project files are not merged.

File paths are relative to the physical configuration file that defines them,
including when that file is reached through a symlink. Use an absolute path for a
file elsewhere. `~` and environment variables are not expanded inside TOML paths.
Font paths refer to local files, rather than URLs.

`--data-dir` and `LAZYPENCIL_DATA_DIR` select the storage used for conversations,
checkpoints, attachments and runtimes. Relative values resolve from the directory
where you invoke lazypencil. The AI adapter and its MCP connection use the same
resolved storage and user settings.

## Font precedence and fallback

```toml
version = 1

[fonts]
files = ["fonts/MyFamily.ttc"]
search_paths = ["fonts/extra"]
default_family = "Noto Sans JP"
locale = "ja"
fallbacks = ["Noto Sans JP", "My Supplemental Family"]
```

Use `files` for explicit fonts and `search_paths` to discover fonts recursively.
Hidden entries and nested directory symlinks are skipped. Repeated files are
loaded once. A search root allows 16 directory levels, 4,096 entries and 256 font
candidates; configurations allow 32 roots and 256 effective files/fallback families.

Font inputs are ordered as command-line files, project files, project search
results, inherited user files, then inherited user search results. Only the
selected source of each field participates. The bundled Noto Sans JP follows
explicit same-named files and takes precedence over a system installation.

`default_family` applies when a text node omits `fontFamily`. `locale` is a language
tag such as `ja`, `en-US` or `zh-Hant`. An explicit `fallbacks` array replaces the
fallback chain after the requested family; `[]` leaves only that family. When
omitted, fallback is Noto Sans JP followed by other supplied families.

Inspect the result with `lazypencil fonts list --file design.pen`. Missing paths
appear in `font_diagnostics`; repair or remove them before rendering. See
[fonts](fonts.md) for coverage, formats and the project-level
`font_lock = "fonts.lock.json"` option.

## Preview node names

Set the initial visibility of root names in user settings:

```toml
[tui.preview]
show_node_names = true
```

The default is `true`. Toggle names for the current session with **View · Toggle
node names** in the command palette. This changes the preview, while exported
images contain only the design.

Set `[tui] theme` to `auto`, `dark`, `light` or `monochrome`. `auto` follows the
terminal background. `tui --theme NAME` selects a theme for one launch. See
[TUI controls](tui.md#keys-and-commands), [panel widths](tui.md#panel-widths) and
[themes](tui.md#ui-themes) for more user preferences.

## User agent profiles

Save defaults for the AI runtimes you use:

```toml
[agents]
default = "codex"

[agents.codex]
adapter = "codex-acp"
adapter_args = []
auth = "subscription"
require_auth_status = true
mode = "read-only"
timeout = 600
document_tools = "allow"
# model = "an-adapter-advertised-model"
# fast = true
# auth_method = "an-adapter-advertised-authentication-method"

[agents.claude]
adapter = "claude-agent-acp"
auth = "subscription"
require_auth_status = true
mode = "default"
```

`prompt` and `tui` select the agent from `--agent`, then the configured default,
then Codex. Only the selected agent's profile applies. Explicit command-line
values override configured values, including an explicit built-in default.

Use the adapter's advertised model, mode and authentication-method identifiers.
`require_auth_status = true` requires a reported authentication route before
submission. `--require-auth-status=false` overrides that preference, while a
conflicting reported route still stops the request. See [agent setup](agents.md).

Codex `fast = true` or `false` chooses Fast mode; `--fast` and `--fast=false` override
it. Omission uses the adapter's inherited setting. If Fast mode is unexpectedly
cleared, pass the choice explicitly; see the upstream
[inheritance issue](https://github.com/agentclientprotocol/codex-acp/issues/285).
Fast mode is available only for Codex and follows the provider's account limits.

An explicit `--adapter` replaces both the configured executable and its argument
list. Plain command names use `PATH`; relative adapter paths resolve beside their
configuration file. Arguments are passed unchanged, so use absolute script paths
when the adapter needs them. The adapter runs in the document's directory.

Adapter paths allow up to 4,096 UTF-8 bytes. `adapter_args` requires an adapter and
allows 128 arguments, 4,096 bytes each and 64 KiB total. Model IDs allow 1–512 bytes;
mode and authentication-method IDs allow 1–256 bytes without control characters.
Timeout accepts 1–86,400 seconds.

Edited defaults apply on the next CLI/TUI launch. To continue a saved conversation,
use its original agent, adapter command and authentication route. Credentials stay
in the external runtime's authentication store.

## Project agent profiles

Project settings accept the same agent tables. Precedence is **command-line >
project > user > built-in**. Replacing an adapter replaces its complete argument
list; inherited paths keep the directory of the file that defined them.

Review the project settings and approve their fingerprint before using them:

```sh
lazypencil config show design.pen --executables
lazypencil config trust design.pen --executables --expected-fingerprint SHA256_FROM_SHOW
```

This approval is separate from design-instruction trust. It covers the reviewed
settings at that project root, including the executable path and arguments.
Review the program and scripts at those paths as well: their contents are not
included in the settings fingerprint.

If the selected project settings need approval, sending returns
`PROJECT_NOT_TRUSTED` and keeps the draft. You can inspect the design before
authorizing them. After changing the agent table, review its new fingerprint and
reopen the CLI/TUI. Font or design-data changes alone keep agent approval intact.

`config untrust design.pen` revokes both agent and design-instruction approvals.
Revocation takes effect on the next attempted turn. MCP tools cannot grant trust.

## Project guides and styles

Guides supply project conventions. Styles add tokens, instructions and an editable
component example. For instance:

```toml
[design]
styles = ["design/brand.json"]
default_style = "project:brand"

[[design.guides]]
id = "brand"
path = "design/brand.txt"
version = "1.0.0"
language = "en"
summary = "Project typography and color roles"
automatic = true
```

Guide files are UTF-8 text and can contain Markdown in any language. IDs allow
1–64 ASCII letters, digits, `-`, `_` or `.` and appear as `project:ID`. Version
defaults to `1.0.0`, language to `und`, summary to empty, and `automatic` to `true`.
Automatic guides accompany new turns; set `automatic = false` for an on-demand guide.

Paths resolve beside the configuration; a style's example resolves beside its JSON
file. All resources and their symlink targets must stay inside the project root.

For `design/brand.json`:

```json
{
  "id": "brand",
  "name": "Project brand",
  "version": "1.0.0",
  "language": "en",
  "description": "A starting point for new project screens",
  "use_cases": ["app-screen", "dashboard"],
  "guidance": ["layout", "project:brand"],
  "tokens": {"accent": "#134E4A", "spacing": [4, 8, 16, 24]},
  "instructions": "Use the project color roles and reuse existing components.",
  "example": "components.pen",
  "required_fonts": [{"family": "Noto Sans JP", "source": "bundled"}]
}
```

Required fields are `id`, `name`, `description`, `tokens`, `instructions` and
`example`. `tokens` is an object. Optional arrays default to empty; `license` is
optional and should describe the project's own terms. Guide references use a
built-in ID or the full `project:ID`.

The example is a separate `.pen` file using a supported version. For example,
`design/components.pen` can contain:

```json
{
  "version": "2.22",
  "children": [
    {"id": "button", "type": "frame", "reusable": true,
     "name": "Project button", "width": 120, "height": 44,
     "cornerRadius": 8, "fill": "#134E4A"}
  ]
}
```

`--style ID` overrides `default_style`. Applying a style provides guidance; the
agent still edits the document through ordinary operations. Components can be
imported from a style with `source: {"style_id":"project:brand","node_id":"button"}`.
See [component imports](mcp.md#importing-components) for link/copy options.

## Explicit design trust

Review project-supplied instructions before making them available to an agent:

```sh
lazypencil config show design.pen
# Review the listed guide, style, and example files.
lazypencil config trust design.pen --expected-fingerprint HASH_FROM_SHOW
lazypencil guidance --file design.pen
lazypencil styles --file design.pen --use-case dashboard
lazypencil style project:brand --file design.pen
```

`config show` lists resource paths, IDs, sizes and hashes. `config trust` approves
the reviewed instructions for this project root and stores the grant in your user
data directory. Changed guide/style instructions require a new review. Changes to
example data, font settings or configuration comments update data revisions without
revoking unchanged instruction approval.

Your request takes precedence over project guidance. Built-in guides remain
available without a project grant. Use `config untrust design.pen` to revoke
approval. Revocation affects future reads and turns; prior conversation context
and saved document edits remain.

Design resources allow up to 32 guides of 64 KiB each, 256 KiB of automatic guide
text, 16 style files of 256 KiB each, 2 MiB per example and 8 MiB in captured
resources. Styles allow up to 32 use cases and 64 guidance/font/pack entries.

Broken resources appear as `configuration.design.error`. Fix them before starting
a new guided turn or importing a style. Ordinary inspection and rendering remain
available when the document and its rendering inputs are valid.

## Image attachment budgets

Set reference-image limits in user settings:

```toml
[attachments]
max_images = 8
max_source_bytes = 20971520
max_send_bytes = 20971520
max_total_source_bytes = 52428800
max_total_send_bytes = 52428800
max_source_pixels = 33554432
```

The values shown are defaults. `max_images` accepts 1–64. Per-image byte limits
accept up to 20 MiB; each aggregate total accepts up to 256 MiB. The source-pixel
limit accepts up to 32 × 1024 × 1024 pixels and is measured before resizing.
Provider limits also apply.

Source bytes measure the original encoding; send bytes measure the normalized PNG.
To reduce send size, use `--image-max-size WIDTHxHEIGHT`, `/attach PATH WIDTHxHEIGHT`,
`/paste-image WIDTHxHEIGHT` or `/resize-image NUMBER WIDTHxHEIGHT`.
See [image attachments](image-attachments.md) for resizing and clipboard controls.

Limits apply when adding and sending images. Tightening a limit preserves existing
images and drafts, but may block submission until you remove or resize the affected
images. Projects cannot change these user limits.

## Local conversation retention

Set a policy in user settings, then explicitly review and apply it:

```toml
[conversations.retention]
max_age_days = 90
max_total_bytes = 1073741824
keep_recent_per_document = 1
```

```sh
lazypencil conversations prune --json
lazypencil conversations prune --expected-revision REVIEWED_SHA256 --json
lazypencil conversations cleanup --all --json
```

Omitted limits keep all conversations. The policy runs only when you confirm a
prune operation. Age uses the most recent local file write, with the cutoff
rounded to the current UTC day's start. The byte limit covers conversations in
all document stores under the selected data directory, including images and logs;
project assets, checkpoints, fonts and runtimes have separate storage.

Review shows candidates, sizes and reasons for keeping protected conversations.
The policy keeps the configured recent count per document, then selects eligible
expired and oldest conversations. Active sessions, unsent drafts, pending images,
queued or unconfirmed requests and incomplete continuity are protected. Setting
`keep_recent_per_document = 0` clears the recent-count minimum while keeping these
protections, so total usage may remain above the limit.

If settings or conversation contents change after review, review again. A batch
can commit some deletions before a later failure; completed deletions stay applied.
Repeat `cleanup --all` for interrupted file removal. Maintenance can operate even
when an original document has moved or disappeared. Each pass has a 30-second
budget and limits of 10,000 records and 100,000 scanned entries.

## Document expansion budget

For a component-heavy design, adjust the budget for expanding instances and
resolving their properties:

```toml
[runtime.document]
expansion_mib = 256
```

The default is 256 MiB; accepted values are whole MiB from 1 to 16,384.
A calculation shares this budget across roots and generated nodes, including
content outside the viewport. `RESOURCE_LIMIT` reports `limit_bytes` when it is
exhausted. This controls expansion work; file-size, dependency and total process
memory limits are separate.

## JavaScript execution limits

```toml
[runtime.javascript]
timeout_ms = 2000
heap_mib = 64
idle_seconds = 60
layout_timeout_ms = 10000
max_evaluations = 128
max_passes = 8
```

| Field | Accepted range | Purpose |
| --- | --- | --- |
| `timeout_ms` | 1–60,000 ms | Time for one script evaluation |
| `heap_mib` | 4–512 MiB | JavaScript heap limit |
| `idle_seconds` | 0–3,600 s | Close an unused script worker after this interval |
| `layout_timeout_ms` | 1–300,000 ms | Time for the complete script-driven layout calculation |
| `max_evaluations` | 1–4,096 | Script executions allowed in a calculation |
| `max_passes` | 1–64 | Layout passes allowed to converge on the result |

The example shows defaults. Both user and project files accept these settings.
A zero idle interval permits immediate worker cleanup. If a calculation hits a
limit, simplify the script/design or increase the relevant value. Timeouts cover
script evaluation and layout work; loading document and font inputs precedes them.

## Browser, shader and raster-cache limits

Install [Chromium](runtimes.md) before using browser nodes or GLSL. Configure its
capture behavior and reusable image cache with these defaults:

```toml
[runtime.browser]
timeout_ms = 30000
launch_timeout_ms = 15000
idle_seconds = 60
wait_until = "load"
ready_selector = ""
selector_state = "visible"

[runtime.shader]
timeout_ms = 15000
launch_timeout_ms = 15000
idle_seconds = 60

[runtime.dynamic]
render_timeout_ms = 45000
cache_mib = 64
cache_entries = 32
```

| Field | Accepted range | Purpose |
| --- | --- | --- |
| Browser/shader `timeout_ms` | 1–300,000 ms | Time for one capture, including launch and navigation |
| Browser/shader `launch_timeout_ms` | 1–60,000 ms | Additional cap on starting the browser process |
| Browser/shader `idle_seconds` | 0–3,600 s | Close an unused process after this interval |
| Browser `wait_until` | `load`, `network_idle` | Page readiness condition |
| Browser `ready_selector` | Empty or a CSS selector of up to 512 Unicode characters, without NUL | Optional element to wait for in the main document |
| Browser `selector_state` | `attached`, `visible` | Require the element to exist or be visible |
| Browser `additional_wait_ms` | 0–60,000 ms | Extra wait after readiness; default 0 |
| Browser `animation_policy` | `running`, `paused` | Native animation playback at capture; default `running` |
| Browser `network_policy` | `online`, `offline` | Network policy; default `online` |
| Browser `image_origins` | Up to 64 exact origins, 4,096 bytes combined | Allowlist for HTTP image acquisition |
| Dynamic `render_timeout_ms` | 1–300,000 ms | Total rendering time across captures and native stages |
| Dynamic `cache_mib` | 0–256 MiB | Reusable encoded images and expanded shader-source data |
| Dynamic `cache_entries` | 0–256 | Maximum cache entries |

Setting a cache limit to zero disables reuse. Changed runtime settings clear
previous captures before reuse. Decoded images, native raster buffers and browser
process memory are separate from this encoded-data cache.

### Wait for page content

The default `load` waits for document load, fonts, decoded images and two animation
frames. For a page that fills in content later, select `network_idle` or set a
readiness marker:

```toml
[runtime.browser]
ready_selector = "#app[data-state='ready']"
selector_state = "visible"
additional_wait_ms = 250
```

`attached` accepts an existing element even when hidden. `visible` also checks its
size and CSS visibility. The selector applies to the main document, outside
iframes and shadow roots. Use an application-owned marker when network idleness
alone cannot tell whether the page is ready.

All waits share the capture/render timeout. `additional_wait_ms` adds a wait after
the readiness conditions, within that timeout. An empty selector clears an
inherited selector. Zoom and scroll apply before visibility checks.

`animation_policy = "paused"` pauses CSS animations, transitions and Web Animations
at the captured state. JavaScript timers, video and animated images can continue;
use application-specific readiness for those.

### Network and authenticated pages

`network_policy = "offline"` blocks HTTP(S) and WebSocket requests during browser
capture, including localhost, workers and child frames. It also blocks explicit
HTTP image acquisition. A project may tighten a user's policy to offline but
cannot relax a user-selected offline policy.

`image_origins = ["https://assets.example.com"]` restricts HTTP image requests and
redirects to those exact scheme/host/port origins. `[]` blocks them all; omission
inherits. Project lists intersect user lists. This setting applies to image
acquisition; browser page requests use `network_policy`.

For authenticated pages, create and log in to a [dedicated profile](runtimes.md#dedicated-authentication-profiles),
then set `runtime.browser.profile = "work"` in user settings. A project can disable
it with `profile = ""`, but cannot select a different named profile.

## Revisions and concurrent changes

Settings, fonts, design resources and an optional font lock contribute to the
document's dependency revision. After any of these change, recalculate before
editing from old measurements. The TUI refreshes its layout while preserving your
draft and attachments; MCP returns a conflict for a stale snapshot.

This includes configuration comments, search-directory contents and changes to
which nearby configuration is selected. `config show` reports effective values
and their origins, including `font_origins` and `runtime_origins`.

A retry of an already committed request returns its saved result. New edits use
the current settings. Document checkpoints restore document edits; configuration
files and font inputs must be managed separately, for example in Git.

## Development checks

For changes to configuration handling, use focused tests for the affected scope:
`cargo xtask test --test config --test user_config --test project_guidance --test user_agent_config`.
See [development](development.md#focused-development-checks) for filters and
isolated settings.
