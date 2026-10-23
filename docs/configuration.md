# User and project configuration

User `config.toml` persists font preferences, runtime limits, and per-agent ACP preferences across projects. Project `lazypencil.toml` overrides individual font/runtime fields and adds project design guides, style catalogues, and a default style. CLI, MCP, and TUI use the same captured document settings; CLI and TUI resolve the same user agent profiles. Project instructions require an explicit local user grant. Project executable settings, browser authentication/network/wait policy, asset/library, global/viewing key-binding, and other resource-limit settings remain unfinished. Prompt submission and clipboard shortcuts are user-configurable as described below.

## Start and inspect

```sh
lazypencil config init
lazypencil config init --user
lazypencil config show design.pen
lazypencil config show --user
lazypencil config schema
lazypencil config schema --user
lazypencil fonts list --file design.pen
```

`config init [PATH]` creates `lazypencil.toml` atomically and never overwrites an existing file. A supplied path must have that filename and an existing parent directory. `config show FILE` resolves project settings for that document path without executing scripts, launching a browser/agent, or decoding font glyphs. It can inspect configuration even when the document's contents are invalid. `config schema` returns the JSON Schema for supported settings, including version and array limits. Parsing also enforces byte/path/language limits.

`config init --user` explicitly creates the user configuration and missing parent directories; it also never overwrites an existing file. Ordinary reads do not create those directories. `config show --user` and `config schema --user` need no document and inspect only user settings. The templates leave optional fields commented so a newly initialized project inherits user preferences. User scope accepts `version`, `[fonts]`, `[runtime]`, `[agents]` and `[tui.keys]`; project design instructions are rejected there. Only the user scope accepts agent executables and prompt shortcut settings. See [TUI keys](tui.md#keys-and-commands) for replacement, validation and focus rules. `show --user` reports the effective prompt keys and `schema --user` describes them, including builds without TUI support.

The default user location comes from the platform's configuration directory: `~/Library/Application Support/dev.lazypencil.lazypencil/config.toml` on macOS, `$XDG_CONFIG_HOME/lazypencil/config.toml` (normally `~/.config/lazypencil/config.toml`) on Linux, and `%APPDATA%\\lazypencil\\lazypencil\\config\\config.toml` on Windows. `LAZYPENCIL_CONFIG_DIR` overrides the directory, not the filename. A relative override resolves against the invoking directory. `doctor` reports the selected path; `config show --user` reports the requested/physical paths and source hash. The actual Windows path and native operation still require platform verification.

Each field follows **explicit project value > explicit user value > built-in default**. Arrays replace rather than concatenate; `files = []`, `search_paths = []`, or `fallbacks = []` explicitly stop inheritance for that field. Explicit values equal to built-in defaults still override user settings. `config show FILE` exposes the merged `settings`, per-field `font_origins` (`project`, `user`, or `builtin`), and separate `user` source metadata. File lists retain their declared spelling; `font_files` / `effective_font_files` expose resolved paths and ordering.

For example:

```toml
version = 1

[fonts]
files = ["fonts/MyFamily.ttc"]
search_paths = ["fonts/extra"]
default_family = "Noto Sans JP"
locale = "ja"
fallbacks = ["Noto Sans JP", "My Supplemental Family"]
```

Unknown fields, duplicate keys, unsupported versions, wrong types, invalid font paths, and oversized configuration produce `CONFIGURATION_ERROR`. Syntax/schema errors include a file and byte/line location, without returning the source excerpt or arbitrary input values. An empty file uses version 1 defaults. Unavailable design resources have their own diagnostic, described below. Keep secrets in the external runtimes' authentication stores rather than in project configuration.

## Discovery and path rules

Project discovery starts at the canonical document's directory and walks upward. The nearest `lazypencil.toml` is selected. A `.git` directory or worktree marker stops the search after checking for configuration in that directory. Without either marker, the document directory remains the default project root. Parent project configurations are not merged; omitted font fields inherit the separately selected user file.

File and search-directory paths resolve against the physical parent of the configuration that supplied that field, including when the configuration itself is a symlink. Inherited user paths therefore stay relative to the user file. Absolute local paths are accepted. Home-directory and environment-variable expansions are not performed, and URLs are not font-file paths. Configured external font files can be used without copying them into the project. Canonical configuration and font paths must be UTF-8. The user configuration directory does not extend the document's allowed import/asset roots.

These font paths are explicit data inputs. Project configuration has no executable/argument settings; unsupported tables are rejected. Design trust authorizes instruction use, and does not enable project-selected adapters or arbitrary commands. Trust for future executable settings remains unfinished.

`--data-dir` and `LAZYPENCIL_DATA_DIR` resolve relative paths against the invoking directory before child processes change their working directory. ACP adapters and their bound MCP processes therefore use the same local grants, history, and attachments.

The conversation host also pins the absolute `LAZYPENCIL_CONFIG_DIR` in its bound MCP descriptor. Changing the adapter/MCP working directory to a nested document directory does not select another user file. This does not modify the user's global environment or external agent configuration.

## JavaScript execution limits

Both user and project files accept these data-only settings:

```toml
[runtime.javascript]
timeout_ms = 2000
heap_mib = 64
idle_seconds = 60
layout_timeout_ms = 10000
max_evaluations = 128
max_passes = 8
```

| Field | Accepted range | Meaning |
| --- | --- | --- |
| `timeout_ms` | 1..60,000 ms | One interpreter evaluation, including context setup, compilation and output normalization |
| `heap_mib` | 4..512 MiB | QuickJS heap cap; separate from native-process or complete process-group RSS |
| `idle_seconds` | 0..3,600 s | Reap an unused script worker after its last completed request; zero permits immediate reaping |
| `layout_timeout_ms` | 1..300,000 ms | Budget for script-generated layout calculation, across passes and evaluations |
| `max_evaluations` | 1..4,096 | Maximum actual script executions in one calculation |
| `max_passes` | 1..64 | Maximum fixed-point layout passes |

The values shown are defaults. Each field follows **explicit project > explicit user > built-in**. An empty table inherits; an explicit built-in value overrides a different user preference, and `idle_seconds = 0` is an explicit value. `config show FILE` reports effective settings and the supplying scope in `runtime_origins`, using keys such as `javascript.timeout_ms`. Values outside the ranges, wrong types and unknown fields are configuration errors. Invalid lower-priority declarations are rejected even when another scope would override them.

Each calculation applies its captured settings to the shared script runtime. Changes to either configuration file invalidate retained calculations through ordinary configuration dependencies, even when resulting geometry would match. Long-lived hosts apply the new policy on the next calculation; idle cleanup otherwise uses the last applied policy. Configuration inspection does not start workers. Numeric runtime policy does not select executables or grant project executable trust.

The host sends a versioned policy envelope separately from script inputs and the worker validates it before execution. Script code cannot change its limits through `pencil.input`. The parent watchdog is the effective per-evaluation timeout plus 1,000 ms; an earlier absolute layout deadline also caps worker startup, I/O and execution. Interrupting or catching an execution timeout never makes that evaluation successful. Layout budgets are checked before/after native passes and before publishing a completed calculation; they cannot preempt synchronous native layout operations. Document/font loading before script layout is outside this particular budget.

## Browser, shader and raster-cache limits

Both scopes accept the following defaults, with the same per-field inheritance, validation, `runtime_origins`, and captured dependency rules as JavaScript:

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

| Field | Accepted range | Meaning |
| --- | --- | --- |
| Browser/shader `timeout_ms` | 1..300,000 ms | One acquisition, including launch when needed, tab setup, navigation/evaluation and capture |
| Browser/shader `launch_timeout_ms` | 1..60,000 ms | An additional cap on starting and connecting to that purpose's process |
| Browser/shader `idle_seconds` | 0..3,600 s | Independent idle expiry after completed work; zero permits immediate cleanup |
| Browser `wait_until` | `load`, `network_idle` | Readiness before fonts, image decoding and two animation frames; default `load` |
| Browser `ready_selector` | Empty or CSS selector of at most 512 Unicode characters, without NUL | Optional first matching element in the main document; empty disables this condition |
| Browser `selector_state` | `attached`, `visible` | Require existence or visibility; default `visible`, used only with a nonempty selector |
| Dynamic `render_timeout_ms` | 1..300,000 ms | Shared rendering budget across acquisitions and checked native stages |
| Dynamic `cache_mib` | 0..256 MiB | Maximum total retained encoded raster bytes; zero disables reuse |
| Dynamic `cache_entries` | 0..256 | Maximum retained raster entries; zero disables reuse |

The common render entry applies the captured policy before beginning work, so CLI, MCP and TUI use the same limits. Launch uses the earliest of its own deadline, the operation deadline and the remaining shared render budget. Deadlines are not restarted between CDP commands. The final checks reject expired/cancelled acquisitions and check the shared budget again after native PNG encoding. Native work, runtime-file verification and OS transport calls are cooperative; synchronous work cannot be preempted, and bounded context/process cleanup may finish after a deadline. Document/font loading and script-layout calculation precede this rendering budget.

Changing browser, shader or dynamic policy clears the runtime's raster cache before reuse; unchanged policy preserves it. An image larger than the configured cache capacity is returned without retention. Reaching the count/byte bound evicts the retained cache before insertion. These limits bound encoded cached PNGs, not active Skia surfaces, decoded image memory, or complete process-group RSS. Acquisitions include their effective purpose and dynamic `limits` in metadata. Expiring a process alone preserves cached pixels; cached reads do not extend its last execution time.

`wait_until = "network_idle"` additionally waits for Chromium's `networkIdle` lifecycle event for the current main-frame loader, while still requiring document load. Lifecycle notifications from the previous document, another tab or a child frame cannot satisfy it. Fonts, decoded images and two animation frames remain required afterward. This follows Chromium's lifecycle readiness; it is not a guarantee that application state, future requests, animations, workers or every cross-origin frame have settled. Long-lived or repeated requests can exhaust the existing acquisition/render deadline; the runtime returns an error and disposes the context and process instead of capturing early. No additional deadline is started. The chosen policy appears in acquisition metadata and inherits independently from user/project settings.

For application-specific readiness, set `ready_selector = "#app[data-state='ready']"`. It is queried after load/network readiness and before fonts, image decoding and final animation frames, using the same deadline. `attached` accepts the first matching element even when hidden. `visible` additionally requires a positive-area client rectangle and Chromium's `checkVisibility` with opacity, CSS visibility and skipped-content checks enabled. Hidden ancestors, opacity zero, skipped content and zero-size boxes do not satisfy it. This is a CSS/layout condition, not a test for occlusion or viewport intersection. It does not enter iframes or shadow roots. Selector text is passed as a serialized string, never as JavaScript source. Invalid syntax returns `PAGE_LOAD_FAILED`; an unsatisfied condition times out and disposes the acquisition. See the [CSSOM visibility definition](https://drafts.csswg.org/cssom-view/#dom-element-checkvisibility).

The setting applies to every browser acquisition using that captured configuration. Each field inherits independently; an explicit `ready_selector = ""` clears an inherited selector. The requested zoom and scroll are applied before visibility checks. Scroll is repeated during polling as content appears, with `behavior: instant`, so page CSS smooth scrolling cannot leave an intermediate capture. Metadata records the actual scroll offsets, selector and state; the existing runtime limits and dependency/cache invalidation rules also apply to these fields.

Configuration is data-only: inspection starts no process, and these fields neither select executable files nor install Chromium. Page authentication profiles, request restrictions, animation controls, decoded/native memory budgets, and process-group resource enforcement remain separate unfinished requirements. The limits, real workers, policy reload, Linux interruption and TUI policy recovery passed the hosted Chromium runs recorded in [runtime verification](runtimes.md#hosted-integration-without-local-build-trees). Network-idle and selector conditions, quoted selector data, timeout/cancellation/recovery and actual scroll metadata passed dedicated macOS arm64, Linux x64/arm64 and Windows x64 runs at source `d617d5e`. Unix real MCP idle and TUI checks also passed. Full normal CI at the current source remains pending.

Execution records contain the **configured** `limits`, included in calculation identity. Code/input/output/node/depth and stack guards remain independent: 1 MiB source, 4 MiB protocol frames, 1,000 generated nodes per output, 64 output levels and a 512 KiB JS stack. Browser/shader limits and wider process/cache/asset policies remain separate unfinished work.

Local work is limited to formatting, Python checks and source review; native builds and the remaining full verification run on GHA. A dedicated integration result has the scope listed in its linked record.

## User agent profiles

Add preferences to the selected **user** `config.toml`:

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
# auth_method = "an-adapter-advertised-authentication-method"

[agents.claude]
adapter = "claude-agent-acp"
auth = "subscription"
require_auth_status = true
mode = "default"
```

`prompt` and `tui` resolve these preferences once at invocation, before creating a document or conversation. The selected agent is explicit `--agent`, then `agents.default`, then built-in `codex`. Only that agent's profile supplies defaults; fields never leak from the other profile. Each remaining field follows explicit CLI value, selected user profile value, then built-in behavior. An explicitly supplied value equal to the built-in default still overrides the user profile, including `--timeout 600`, `--auth subscription`, and `--document-tools allow`.

`--require-auth-status` enables strict authentication reporting; `--require-auth-status=false` explicitly overrides a true user default. Disabling this flag does not disable an adapter's advertised authentication-status requirements or permit a conflicting reported route. Authentication, model/mode negotiation, document-tool permissions, and timeout cancellation retain the same behavior as their CLI options. Credentials remain in the external agent's stores. A profile is a set of preferences, not evidence that an adapter is installed or that a model/API account can execute a request.

An explicit `--adapter` replaces both the configured executable and its entire `adapter_args` vector. Other selected-profile fields still apply unless individually overridden. No shell parses the command or arguments, and no adapter is installed automatically. A plain command name searches `PATH`; a relative path such as `./bin/codex-acp` resolves against the physical user configuration directory, including when `config.toml` itself is a symlink. Absolute paths are accepted. The process still runs in the document's directory. Relative **argument strings** are passed unchanged and therefore follow the adapter's own working-directory rules; use absolute script paths when necessary. Home/environment expansion is not performed.

`adapter_args` requires a configured adapter and is bounded to 128 arguments, 4,096 UTF-8 bytes each, and 64 KiB total. Adapter commands/paths are bounded to 4,096 bytes. Model identifiers use 1..512 bytes; mode/authentication-method identifiers use 1..256 bytes without control characters. Timeout uses 1..86,400 seconds. Both profiles are structurally validated even when one is unselected; unused adapter files are not opened. Inspecting a profile, font inventory, or document never launches its adapter.

Changing user preferences affects the next CLI/TUI invocation; an existing connection keeps its resolved settings. Session reload still verifies the agent, requested authentication route, and command/argument fingerprint. It refuses a changed adapter rather than silently switching a saved conversation. Programmatic `Conversation::open` callers supply complete `PromptArgs`; they do not implicitly reapply CLI preferences.

Project `lazypencil.toml` continues to reject `[agents]`. A design-instruction grant does not authorize project-selected executables. Project executable-setting trust remains separate unfinished work. User preferences do not add executable operations to MCP or alter the external agent's own configuration files.

## Project guides and styles

Add explicitly named guides and JSON style files:

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

Guide files contain UTF-8 text, including Markdown when desired. IDs contain 1..64 ASCII letters, digits, `-`, `_`, or `.`; tools expose them as `project:ID`, so a project cannot shadow a built-in ID. Version defaults to `1.0.0`, language to `und`, summary to empty, and `automatic` to true. Project guides can use any language. An automatic guide accompanies the entry workflow; a guide with `automatic = false` is listed and fetched by ID when needed. User instructions take precedence over project rules, followed by the selected style and standard guidance. Capability checks, validation, and save rules still apply.

Style paths resolve against the physical configuration directory. A style's example resolves against that JSON file's physical directory. All guide/style/example targets, including symlink targets, must remain inside the selected project root. They are regular local files, never URLs, commands, or automatic downloads.

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

`id`, `name`, `description`, `tokens` (an object), `instructions`, and `example` are required. Version/language use the guide defaults. `use_cases`, `guidance`, `required_fonts`, and `required_packs` default to empty arrays. `license` is optional; an omitted project license remains unknown, and is never assigned the application's MIT license. Guide references use built-in IDs or complete `project:ID` names. Unknown fields and invalid metadata are rejected.

The example is a separate schema 2.21 `.pen` file. For example, `design/components.pen` can contain:

```json
{
  "version": "2.21",
  "children": [
    {"id": "button", "type": "frame", "reusable": true,
     "name": "Project button", "width": 120, "height": 44,
     "cornerRadius": 8, "fill": "#134E4A"}
  ]
}
```

Selection does not change a document. `prompt` and TUI use `default_style` when no explicit `--style` is supplied. Existing conventions still take priority when editing an existing design. Startup includes tokens, instructions, component IDs and the example hash; the complete example is retrieved through MCP when needed. Component imports use normal `source: {"style_id":"project:brand","node_id":"button"}` operations. Both link and copy retain the example's source directory for assets and references. Linked imports use relative project URIs and preserve a requested example alias; copied components use normal dependency closure and immutable asset publication. Existing copy limitations, including independent script copying, still apply.

## Explicit design trust

```sh
lazypencil config show design.pen
# Review the listed guide, style, and example files.
lazypencil config trust design.pen --expected-fingerprint HASH_FROM_SHOW
lazypencil guidance --file design.pen
lazypencil guidance project:brand --file design.pen
lazypencil styles --file design.pen --use-case dashboard
lazypencil style project:brand --file design.pen
lazypencil config untrust design.pen
```

`config show` exposes IDs, paths, sizes and hashes, without loading guide bodies into AI instructions. `config trust` is an explicit local user action; the optional expected fingerprint rejects changes since review. Grants are stored in the selected user data directory, outside document contents. The review/catalogue `fingerprint` includes the canonical project root, configuration source, resource bytes, and requested-to-canonical path bindings. The separate `trust_fingerprint` pins instruction scope: parsed design settings, the physical configuration source, guide/style bytes, and their path bindings. Changes to these instructions require a new grant. Example libraries remain reference data: editing an example or retargeting its alias changes catalogue/revision hashes without revoking unchanged instructions. Font-only settings and configuration comments also leave instruction trust intact. Grants do not transfer to another project root. Documents under the same selected project root share its grant.

MCP does not grant trust. Untrusted configured instructions cause `PROJECT_NOT_TRUSTED`, and `prompt` refuses to start an adapter turn with them. Agents must not grant trust on the user's behalf. Built-in CLI guides remain available without `--file`. A project with no design instructions needs no grant. Revocation applies to subsequent reads/turns; it does not undo saved edits or erase an agent's earlier context. `config untrust` revokes applicable ancestor grants through the Git boundary, even with broken design resources or a removed nested configuration.

The entry workflow keeps its English body and returns automatic project guides separately, with each guide's own language, version, and hash. CLI/TUI send the same content through their shared conversation host. Turn records retain selected guide/style identities and hashes. Reference-only guides are fetched on demand. MCP clients receive the same catalogue and respect the external client's own instruction hierarchy.

Design resources are bounded to 32 guides (64 KiB each), 256 KiB of automatic guide text, 16 style files (256 KiB each), 2 MiB per example, and 8 MiB of unique captured resource bytes. Styles have bounded metadata, at most 32 use cases and 64 guidance/font/pack entries. Tool responses retain the shared 4 MiB limit. The catalogue currently captures/parses local resources eagerly; startup omits complete example documents from the agent prompt.

Missing, malformed, or out-of-root design resources appear under `configuration.design.error` with `available: false`. AI catalogue reads, style imports, and new prompt turns reject that error. Ordinary document reads and rendering can continue using their existing data and valid font settings; a broken guide does not become an empty set of approved instructions. Invalid TOML or font settings still fail their normal checks.

## Font precedence and fallback

Explicit CLI `--font` files come first, followed by project `files`, project search results, inherited user `files`, and inherited user search results. Only the selected source of each field participates: a project `files` array replaces the user array, while a project `search_paths` array may coexist with inherited user `files` and takes precedence over them. This ordering chooses among competing supplied faces of the same family/style. The bundled Noto Sans JP follows supplied same-named files and takes precedence over the system installation. Named system families are matched only when requested or explicitly included in a fallback chain.

`default_family` applies to text nodes without an explicit `fontFamily`. It does not rewrite stored nodes or override existing explicit font choices. `locale` is passed to SkParagraph and recorded in text metrics. Use a hyphenated language tag such as `ja`, `en-US`, or `zh-Hant`.

When neither scope defines `fallbacks`, the default glyph chain is requested family, Noto Sans JP, then other supplied families in file/collection order. An explicit array replaces the fallback portion after the requested family; `fallbacks = []` disables it. A missing requested family still fails. `fonts list --file FILE`, MCP `list_fonts`, and `get_document.configuration` expose the effective settings. Without `--file`, `fonts list` uses user settings only. CLI `--font` alone adds files; it does not replace the configured fallback policy.

Search directories are traversed recursively in filename order, with hidden entries skipped. Explicit root-directory symlinks and file symlinks are resolved; nested directory symlinks are not followed. Repeated canonical files are deduplicated while preserving first occurrence. Search is currently bounded eager discovery/loading, not a lazy system-font catalogue.

Current configuration limits are 256 KiB per file, 256 explicit/effective font files or fallback families, 32 search roots, 16 nested directory levels, and 4,096 entries / 256 candidate files per search root. File/data/face limits also apply; see [font behavior](fonts.md). Both files are parsed and structurally validated, even if a project overrides a user field. Overridden user file/search lists are not opened. Large font libraries, configurable limits, and full lazy loading remain unfinished.

## Revisions and concurrent changes

Both user/project configuration bytes, effective font bytes, and available design resource bytes become captured project dependencies. They participate in `dependency_revision`, retained calculations, and edit/restore preconditions. Settings are parsed from the exact bytes captured in that snapshot. A comment-only configuration edit therefore changes the dependency revision too. Design trust separately pins its instruction resources; example/font data changes and configuration comments still invalidate affected data snapshots, without granting or revoking instruction trust.

Validation checks discovery bindings and directory membership, as well as file contents. Adding a nearer configuration, removing/adding a font in a search directory, or retargeting configuration/font/directory symlinks invalidates the old snapshot. The TUI checks these dependencies using its existing update path, reloads the font/layout state, and retains the user's draft and attachments. A retained MCP snapshot rejects changed settings rather than returning an image from mixed revisions.

Document saves hold shared locks for project lookup candidates and the selected user configuration, including currently absent files. `config init` / `config init --user` take the matching exclusive lock, so cooperating configuration creation waits until the save releases it. Its lock wait can be cancelled before publication. Ordinary CLI lock waits report once on stderr; MCP and TUI do not print those messages into their protocol or terminal screen. External writers remain subject to the design's existing limit on fully simultaneous non-cooperating writes.

A retry of an already committed edit returns its saved result even if configuration later becomes invalid. A new edit must validate current settings and dependency revisions. Configuration changes themselves are not restored by document checkpoints and are not silently repaired by document editing tools.

## Verification and remaining scope

`tests/config.rs` covers discovery boundaries, nearer settings, deterministic search/deduplication, captured dependencies, unknown/invalid/oversized data, source-safe diagnostics, CLI initialization/show/schema, read-lock coordination, cancellation, symlink changes/cycles, edit replay, and common layout defaults/locale/fallback/CLI precedence. Core configuration tests also run without rendering.

`tests/user_config.rs` additionally checks per-field overlays, explicit built-in values, empty replacement arrays, physical user-path bases, source-safe errors, standalone user inventory, initialization locks/cancellation, committed-edit replay, and project trust remaining valid after user font changes. Actual MCP tests use project and external user fonts without CLI flags, inspect chosen faces and locale, render PNG content, and reject retained calculations and stale edits after user/project settings change. The Unix PTY suite separately changes project and user default families from the original TrueType fixture to the CFF fixture, observes different decoded Kitty pixels automatically, and verifies that the Japanese draft and document bytes remain intact.

`tests/project_guidance.rs` covers project trust, invalidation/revocation, discovery bindings, language separation, styles/defaults, bounded resources, unavailable-instruction diagnostics, and link/copy origins. Real stdio MCP tests exercise the same grants and imports. The deterministic ACP peer also checks that relative user configuration is pinned across working directories. It checks actual prompt content, explicit style precedence, reference-only guide exclusion, blocked turns, example retrieval, and relative user data paths across working directories.

`tests/user_agent_config.rs` exercises actual CLI-to-ACP startup, selected profiles, model/mode/authentication negotiation, explicit adapter argument replacement, strict-authentication overrides, correlated document-tool approvals, live timeout cancellation, saved-session reload/fingerprint rejection, source-safe invalid settings, schema/templates, and physical user-file path bases. Shared argument-provenance tests also cover TUI selection and explicit built-in values. The Unix PTY suite uses a saved Claude profile for two actual turns and confirms that editing the profile does not switch an existing live connection. These use a deterministic local ACP peer rather than paid model requests.

The remaining global configuration sections, executable-setting trust, font compatibility, all target platforms, resource measurements, and release packaging remain required work. This configuration layer does not establish whole-product release readiness.

The standard `sh scripts/ci/test.sh` runs both feature configurations and Clippy with an isolated temporary `LAZYPENCIL_CONFIG_DIR`, avoiding the developer's actual preferences. Focused direct `cargo test` invocations should likewise select an empty temporary configuration directory when reference fixtures require built-in defaults. Tests that exercise user settings set per-child environments or pass explicit library paths; they do not mutate process-global environment variables.
