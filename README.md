# lazypencil

A terminal-first editor and renderer for `.pen` design files, written in Rust.

**Development status:** implementation is in progress. The repository is not yet a finished release. See [implementation status](docs/implementation-status.md) for requirements and verification evidence.

**Initial release platform:** macOS Apple Silicon (`aarch64-apple-darwin`) only. Linux x64/arm64 and Windows support are deferred; their existing code, runtime catalogues and diagnostic fixtures are retained. Intel Mac remains unsupported. Minimum macOS and guaranteed terminals still need acceptance. Earlier cross-platform results are historical evidence, not initial-release commitments.

The intended product provides a CLI, an MCP server, and a TUI with keyboard navigation and image previews. Claude Code and Codex integration uses ACP adapters. Rendering uses Skia CPU and SkParagraph, with Noto Sans JP as the bundled default font. Chromium is installed separately for browser nodes and WebGL shaders.

The complete scope is recorded in the [design](docs/design.md), [component and layout semantics](docs/pen-semantics.md), [design guidance](docs/design-guidance.md), and [image attachment specification](docs/image-attachments.md).

The initial release is complete only when the [design acceptance conditions](docs/design.md#検証と受け入れ条件) are met on macOS Apple Silicon, including CLI/MCP/TUI, all three dynamic-content types, both AI integrations, performance, Kitty/Sixel and clipboard behavior, and distributable binaries with complete licensing materials. Only the OS/CPU scope is reduced; the functional and quality criteria remain.

## Try the current CLI

The editable example uses the bundled font and requires no browser or AI service:

```sh
cargo run -- inspect examples/welcome.pen
cargo run -- layout examples/welcome.pen
cargo run -- render examples/welcome.pen --output welcome.png
```

`render` writes a PNG; `layout` reports positions, text metrics, a snapshot ID, and overflow diagnostics. `--json` returns structured results and errors. Additional font files can be supplied with repeated `--font` options on `inspect`, `variables`, `validate`, `apply`, `layout`, `render`, and `restore`.

`variables design.pen --name ink` reads a raw variable definition and its resolved value with source documents. Select a declared `--library-alias`, a `--node` and inherited `--instance-path`, or an explicit `--property` namespace; repeated `--theme AXIS=VALUE` options are hypothetical queries. `validate design.pen --scope layout` reports measured diagnostics from the normal script/layout pipeline. Its error counts include later diagnostic pages; warnings return success and errors return exit code 4. MCP can reuse the returned calculation for validation, geometry, and PNG output. These checks do not establish paint compatibility or a completed visual review. See [variable queries](docs/mcp.md#variables-and-themes) and [validation scopes](docs/mcp.md#validation-scopes).

Use `fonts list --font /path/to/family.ttc` to inspect every supplied face, style, index, source/face hash, and variable axis. TTF, OTF, TTC, and OTC inputs share the same loading path. Text metrics report the faces actually used after shaping, including Japanese and supplied-font fallback. `fonts license` reads the complete bundled Noto Sans JP license. See [font behavior and remaining work](docs/fonts.md).

Argument errors also honor `--json`, including invalid numbers, unknown options, and missing required arguments. They return `error.code: "INVALID_ARGUMENT"` on stdout and exit with status 2. Help and version remain readable text with status 0. A `--json` string after the `--` separator is positional input rather than a request for JSON output.

The [graphics example](examples/graphics-study.pen) demonstrates Japanese text decorations and combined glyph/decoration outlines, per-side rectangular borders, image crops, and normal inner shadows. Run `cargo run -- render examples/graphics-study.pen --output graphics-study.png`. See [graphics implementation](docs/graphics.md) for verified behavior and remaining compatibility work.

The [backdrop example](examples/backdrop-panel.pen) demonstrates native background blur with sharp foreground text. It needs no browser installation. `render --node NODE_ID` crops the composited document to that node's visual bounds; `render --region X Y WIDTH HEIGHT` selects an explicit crop. Both retain scene context and sample filter inputs outside the requested viewport.

The [shadow study](examples/shadow-blends.pen) compares all eighteen native color modes on outer and inner shadows. Run `cargo run -- render examples/shadow-blends.pen --output shadow-blends.png`. Translucent shadow, paint, and parent opacity use source-over alpha, including linear burn/dodge. See [shadow and paint blending](docs/graphics.md#shadow-and-paint-blending) for behavior and remaining compatibility work.

The [mesh study](examples/mesh-study.pen) demonstrates native color grids with moved points and Bézier handles, a hero surface, Japanese/Latin text, and a mesh border. Run `cargo run -- render examples/mesh-study.pen --output mesh-study.png`. It needs no browser or external assets. See [mesh behavior and limits](docs/graphics.md#mesh-gradients), including the documented defaults and auto-handle rules that still require Pencil compatibility comparison.

The [icon study](examples/icon-study.pen) uses native Lucide vector geometry with Japanese labels, gradients, and transparency. Search with `icons list --name settings`. Lucide is bundled; Feather, Phosphor Regular, and Material Symbols Outlined/Rounded/Sharp are pinned optional packs installed explicitly with `icons setup PACK`. Material accepts real continuous font weights from 100 to 700, including fractional values. `icons license PACK` reads the complete embedded license without installing it. After optional setup, [the icon families study](examples/icon-families.pen) compares all six libraries and three Material weights without a browser. See [icon packs](docs/icons.md) for quoted setup commands, offline inputs, provenance, and current verification scope.

Save shared fonts with `config init --user`, then edit the reported `config.toml`. Project `lazypencil.toml` overrides individual fields: font files/search directories, default family, fallback chain, and language. Omitted fields inherit user settings; explicit arrays replace them, including empty arrays. `config init` creates a project template without overwriting an existing file. `config show --user` / `config show design.pen` expose sources and effective settings; `config schema --user` / `config schema` describe the supported formats. `fonts list` uses user settings and `fonts list --file design.pen` adds project overrides. CLI, MCP, and TUI capture the same data. User `[agents]` selects a default agent; `[agents.codex]` and `[agents.claude]` persist adapter, model, mode, authentication, timeout and document-tool preferences for both `prompt` and TUI. Explicit CLI options take precedence. Project agent overrides require a separate grant: review `config show design.pen --executables`, then run `config trust design.pen --executables`. Design-instruction grants do not authorize executables. TUI preview and draft recovery remain available before authorization. See [configuration](docs/configuration.md) for platform paths, profiles and trust scope, and `LAZYPENCIL_CONFIG_DIR`, and the [project example](examples/lazypencil.example.toml). The project-agent CLI and real-PTY regressions passed on all three platforms in the earlier cross-platform scope at `3bfa2f1`; other unimplemented configuration sections remain outside this claim.

Project design instructions use `[design]` in `lazypencil.toml`. Review `config show design.pen`, then explicitly run `config trust design.pen`. Use `guidance --file design.pen`, `styles --file design.pen`, and `style project:ID --file design.pen` for the same catalogue as MCP and the CLI/TUI conversation host. Default styles and automatic guides are passed to the agent, while complete examples are fetched when needed. Changed instructions require a new grant; ordinary example-library updates remain revision-checked data. See [project guides and styles](docs/configuration.md#project-guides-and-styles).

To edit a file, read its current `revision` and `dependency_revision` with `inspect`, then pass an operation array to `apply`:

```json
[
  {"op": "update", "target": "heading", "set": {"content": "My project"}}
]
```

```sh
cargo run -- apply design.pen --operations edits.json --expected-revision REVISION --expected-dependency-revision DEPENDENCY_REVISION --request-id edit-1
```

Use `--dry-run` to validate a batch without saving. Reusing the same request ID and payload returns the saved result. `history`, `restore`, and `recover` expose checkpoints and interrupted-save recovery. The data directory can be set with `--data-dir` or `LAZYPENCIL_DATA_DIR`; document locks remain in a shared per-user cache directory. `doctor` reports both locations.

Ctrl+C cancels edits while waiting for locks and at checkpoints during batch and component preparation. Native layout and text measurement also check cancellation while expanding nodes, solving dimensions, placing content, collecting glyph outlines and finalizing a snapshot. Rendering checks before allocating its working surface. Cancellation is cooperative around individual native calls; interrupted calculations do not return partial snapshots. MCP edits use the request's cancellation token. A cancellation before document replacement preserves the original document; complete unreferenced managed assets can remain. Replacement already in progress finishes its journal commit. See [cancellation and retry behavior](docs/mcp.md#snapshots-limits-and-cancellation).

Inherited edits use `{"node_id":"instance","instance_path":["nested-instance","label"]}` as the target. Override reset and replacement, slot children, theme axes, and import declarations have explicit operations. `detach_instance` handles local, external, and inherited instance targets while preserving nested references and variable origins; external scripts retain their execution/input scopes through versioned lazypencil metadata, with Pencil round-trip rendering unverified. Inspect an inherited node with `inspect design.pen --node instance --instance-path nested-instance/label`; use `--library-alias ui` for declared library definitions or `--expand-instances` for expanded trees. `import_component` supports linked instances and static dependency copies from project libraries and bundled styles, with explicit name collision handling. The [MCP guide](docs/mcp.md) describes addresses, capabilities, and remaining gaps.

Use `assets put design.pen --kind image --input reference.jpg` to create a checked, normalized PNG, or `--kind script` / `--kind shader` for UTF-8 code. The returned URI points to an immutable file under `assets/lazypencil` beside the document. Referencing it is a separate editing batch; storing code does not execute it.

Image fills and shader image inputs can use HTTP(S) URLs. A project snapshot keeps the acquired image bytes in memory; standalone copies retain them locally while preserving shared string values and themes. `runtime.browser.network_policy = "offline"` prevents acquisition, and copied local images remain available offline. See [HTTP image limits and capture conditions](docs/dynamic-content.md#http画像の取得と保持).

Embedded `data:image/png`, `data:image/jpeg`, and `data:image/webp` URLs use the same static-image validation, with base64 or percent-encoded bytes and a 20 MiB decoded source limit. They work offline and retain their original URL spelling when copied or detached, without writing a separate managed image file. See [embedded image handling](docs/dynamic-content.md#埋め込み画像).

Asset uploads and component imports coordinate with dependency readers through the destination's writer lock, including absent files. `assets put` and MCP uploads can be cancelled while waiting for that lock and before publication. Existing content is rechecked after waiting and is never overwritten. See [managed asset behavior](docs/mcp.md#managed-assets).

Review unused project assets with `assets prune design.pen --project-root /path/to/project --json`. Repeat with `--expected-revision REVIEWED_SHA256` to apply. Include all sibling documents sharing assets in the chosen root and select their retained history with `--data-dir` when needed. Review never deletes files; active frontends block maintenance. Uncertain references retain assets, and interrupted cleanup is reviewed by the same command. See [asset maintenance](docs/mcp.md#project-asset-maintenance) for scope and recovery conditions.

Missing declared local image, JS, and GLSL files can be diagnosed and repaired through ordinary revision-checked editing batches, including partial repairs and checkpoint restores. `inspect` and edit results report `assets_complete` and `asset_diagnostics`; layout and rendering reject known missing files. Missing-library diagnostics and revision-checked import rebinding passed hosted core/default verification; unresolved component references must be repaired in the same batch. Configured font files/search directories have missing-input diagnostics that preserve structural editing, while font inventory and rendering still require every selected input. Missing-library and font recovery passed the [complete hosted verification](https://github.com/EkeMinusYou/lazypencil/actions/runs/40935920526), including real MCP and CLI tests; restoring a font or revising its configuration remains an explicit filesystem/configuration action. See [missing asset repair](docs/mcp.md#repairing-missing-assets).

CLI, MCP, TUI and the three dynamic-content paths are implemented within the documented limits. See [current feature gaps and release checks](docs/implementation-status.md#remaining-work). Unsupported node/fill types fail explicitly. Validation supports structural and measured-layout scopes; neither alone certifies all rendering behavior.

## Script-generated designs

```sh
cargo run -- render examples/script-chart.pen --output script-chart.png
cargo run -- guidance scripts
```

The original chart example combines editable `.pen` layout, Noto Sans JP, and generated geometry. Script nodes use a separate Rust worker with QuickJS-NG 0.16.2 through the pinned `rquickjs` 0.14.0 binding. Headers accept explicit `@schema 2.19`, `@schema 2.21` and legacy `@schema 2.11`, with six typed input kinds; header compatibility does not enable 2.11 document files. The engine exposes `pencil.width`, `pencil.height`, and `pencil.input`; output is measured and painted by the same Skia path as ordinary nodes. Generated children remain derived data and carry `generated_by` metadata.

Use `inspect design.pen --calculated --node chart --depth 2` to read generated properties and their measured geometry. MCP `read_nodes` accepts `calculated: true` for a new calculation or `snapshot_id` to reuse the exact tree from layout/render without rerunning scripts. These reads expose resolved variables, themes, and property origins; generated IDs remain read-only. Ordinary stored reads do not execute scripts. See [calculated node reads](docs/mcp.md#calculated-and-generated-nodes).

Header and JavaScript failures report original-source positions and bounded excerpts when available. JSON/MCP results keep these in `error.details.script_diagnostic`, with the captured source hash, path, and owning address. Readable errors also show the location and source lines. Columns count UTF-8 bytes; dynamic-code call sites and unavailable positions are explicit. See [script execution and diagnostics](docs/mcp.md#script-execution).

The interpreter defaults to a 2-second evaluation, 64 MiB heap, 1,000 output nodes and 60-second idle reap interval. Captured user/project JavaScript policy is shared by CLI/MCP/TUI. Fresh contexts have no host filesystem, network, DOM, module loader or timers; cancellation and parent/layout deadlines bound work. Random values use the reviewed Mulberry32 algorithm with seed 0 and independently computed expectations; retired Pencil output/observation files are not a current oracle. Script-header schemas 2.11/2.19/2.21 are accepted, independently of the 2.19/2.21 document codecs. Complete generated-node/input compatibility and the remaining explicit copied contexts are described in [dynamic-content semantics](docs/dynamic-content.md).

Missing or null script inputs use declared or type-specific defaults without changing stored values. Stale saved controls stay preserved but are excluded from runtime lookup. Actual-worker and both-schema rendering cases passed the completed three-target gates. See [input behavior](docs/dynamic-content.md#入力の既定値と保存済みの旧項目).

Files discovered only in generated content appear in `runtime_dependencies`, separately from the document-declared `dependency_revision`. A retained calculation snapshot captures and checks both sets before rendering. After reviewing a calculation, pass its `runtime_dependency_revision` to `apply` as `--expected-runtime-dependency-revision` to check those files before saving as well. This re-evaluates scripts before taking save locks and checks captured files again before replacing the document. Replaying a committed request does not re-evaluate scripts. See [dynamic execution](docs/dynamic-content.md) for current limits and remaining requirements.

## Browser pages and GLSL

Independent script copies capture declared assets, component/variable/theme bindings and local browser resources. Generated browser scope/override composition, URL ownership and recopy passed both schemas, including 162 namespace images per target in the completed baseline. Explicit component reuse includes definitions contained inside a copied parent, with separate placement IDs and explicit descendant-path mappings. Stored and runtime-authored replacement children preserve their addresses through reuse, source removal and recopy. Generated script scopes support explicit or inherited source URIs and independently guarded inherited inputs, retaining their original spelling through copy and recopy. Custom generated bindings translate logical names, declared libraries and hash-guarded ref-input maps through captured namespaces. Subsequent complete Mac gates verified explicit input-context composition, generation-specific input restoration and code-owner preservation under later overrides, including source removal and recopy. Older captures missing the required provenance require recopying from their original source. See [copy semantics](docs/mcp.md#importing-components) and [verified context behavior](docs/dynamic-content.md).

```sh
cargo run -- runtime setup chromium
cargo run -- runtime status
cargo run -- render examples/dynamic-panel.pen --time 0.5 --output dynamic-panel.png
```

The setup command explicitly downloads a pinned Chrome for Testing headless shell and verifies its archive and installed files. Rendering never downloads a runtime. [Runtime installation](docs/runtimes.md) covers offline archives, custom executables, platforms, licensing, and testing.

Browser nodes capture HTTP(S) or HTML data pages with dimensions, zoom, scroll, and basic desktop/phone/tablet modes. Shader fills and effects execute fragment GLSL through a separate software WebGL process, with numeric, color, vector, and image uniforms. Effects can sample the object and previously painted background. `--time` defaults to zero; `--refresh` requests fresh page acquisitions. MCP retains a bounded raster cache and returns acquisition metadata with the actual runtime, inputs, and image hashes. The shader preprocessor is bundled from a source-bound GHA build; using lazypencil or building the native application does not require Node or a Wasm toolchain. Shader coordinate and resolution handling at enlarged output is described in the [dynamic-content specification](docs/dynamic-content.md#宣言と数値入力).

The original dynamic-panel example combines a local HTML data page, GLSL, and native Japanese text without external page resources. The [uniform study](examples/uniform-study.pen) reuses one shader with flat color arrays, grouped float declarations, and column-major matrices. With Chromium installed, render it using `cargo run -- render examples/uniform-study.pen --output uniform-study.png`. Numeric arrays and GLSL constant-expression dimensions are checked against the active linked WebGL inputs; defaults remain transient zero values. One-dimensional sampler2D arrays accept lists of captured image URLs, including per-element string variables with aliases and themes. The runtime binds the active image prefix and uses transparent defaults for unset elements. Fixed-image lists and local relative string sampler variables can be copied independently while preserving shared text strings, aliases and themes through ordinary imported variable libraries. Absolute local paths, local file URLs, HTTP(S) images and conflicting path projections use portable image bindings while preserving the original text, aliases and themes. Embedded static-image data URLs retain their original strings and payloads. These paths passed complete Mac verification, including source removal and offline recopy. See [declaration rules and limits](docs/dynamic-content.md#宣言と数値入力).

Page and shader processes expire independently after 60 idle seconds by default; cached images remain available and refresh restarts processes. Captured settings configure launch/acquisition/idle/cache limits. Earlier three-target gates cover policy reload, retained TUI DOM, playback/time controls, additional waits, selectors, animation and offline-network behavior. Dedicated authentication profiles and visible login are verified. Shared profile acquisition by retained preview and a separate bound MCP client passed complete Mac gates, including isolation, cancellation/death, captured resources and network-policy changes. Physical-terminal and vendor-account acceptance remain separate. See [runtime behavior](docs/runtimes.md), [capture policies](docs/dynamic-content.md#取得条件と再現性) and [remaining scope](docs/implementation-status.md#remaining-work).

Local file pages require explicit resource roots within the existing project read boundaries. `set_browser_resources` declares an original file directory URL and a resource directory; `pack_browser_resources` saves those captured inputs as portable assets. The common render path serves captured files only, retains original URLs and invalidates stale rasters and retained DOM when resources change. Arbitrary host file reads are rejected. See [local resources](docs/dynamic-content.md#ローカル素材の固定と持ち運び) for the metadata format and remaining native-file compatibility work.

## MCP and design guidance

```sh
cargo run -- mcp --file /absolute/path/design.pen
cargo run -- guidance
cargo run -- guidance operations
cargo run -- styles
cargo run -- style standard-ui --json
```

The stdio MCP server uses the same editing and rendering services as the CLI. It returns PNG image content and can reuse a calculation snapshot between layout inspection and rendering. See the [MCP guide](docs/mcp.md) for supported tools, protocol version, limits, and current gaps.

Fifteen original English guides cover the workflow, real editing operations, layout, typography, color, components, icons, scripts, native effects, browser pages, shaders, verification, app screens, dashboards, and content pages. The three bundled styles are `standard-ui`, `dense-workspace`, and `content-page`. Their editable examples are in [examples](examples/), and all guidance and styles have versions and content hashes. English guidance preserves the user's requested response and design language.

## AI prompts

With an ACP adapter explicitly installed and its runtime authenticated:

```sh
cargo run -- prompt new.pen --create --agent codex "日本語の設定画面を作って"
cargo run -- prompt new.pen --agent claude --image reference.png "Improve this design"
cargo run -- prompt new.pen --image reference.jpg --image-max-size 1024x768 "Use this reference"
```

The host supports ACP v1, image blocks, document-bound MCP, native session reload, permissions, and cancellation. It checks adapter-reported authentication against the requested subscription/API route and cancels a conflicting change. `--model` and `--mode` select advertised options; document-tool preauthorization does not approve shell or unrelated file operations. `--json prompt` streams NDJSON.

Turn logs keep the initial guide/style identities and metadata for additional resources returned by the owned MCP server. `design_resource` events carry IDs, versions and hashes without duplicating guide bodies or examples; they are emitted at turn end, including failed or cancelled turns. See [recording behavior and limits](docs/agents.md#design-resource-records).

Codex and Claude completed image-backed subscription workflows on macOS arm64, including registered image import, edits, layout, and PNG previews. [The editable settings example](examples/agent-settings.pen) was generated through the Codex workflow; its asset URI was made portable to reference the original bundled image. Separate Codex API-key workflows verified a reviewed project guide and an initial image attachment, actual MCP edits, native editable text read from the image, layout and matching MCP/CLI PNG output. Claude API, remaining guaranteed-terminal combinations and final release acceptance still need evidence. Native clipboard/PTY integration and ordinary TUI controls have automated coverage. These observations do not establish aesthetic equivalence with Pencil or pixel-perfect reference reconstruction. See [agent integration](docs/agents.md) for installation, limits, evidence, and constraints.

Use `--session LOCAL_UUID` to continue a saved conversation. Unsupported native loading uses an explicitly reported new session and bounded local context. A load error stays visible; explicit `--restart-session` preserves local records and starts the documented recovery path. Historical images/prompts are not automatically resubmitted. Restart, cancellation and transcript-recovery cases passed both feature configurations on all three targets. See [conversation details](docs/agents.md#conversations-and-events).

Saved local records can be inspected without starting or authenticating an agent:

```sh
lazypencil conversations list design.pen --json
lazypencil conversations turns design.pen LOCAL_UUID --json
lazypencil conversations read design.pen LOCAL_UUID TURN_UUID --json
lazypencil conversations view design.pen LOCAL_UUID TURN_UUID
lazypencil conversations prune --json
```

Lists, formatted messages and complete raw transcripts are paged, with bounded reads and revision-checked continuation. The TUI provides the same saved-log browsing through `/logs`, `/conversations` and its command palette. Browsing does not restore model memory. Explicit `u` selects a compatible saved conversation while idle; `N` or **New empty conversation** starts empty and preserves the previous draft/images. Selection passed all three targets in the earlier cross-platform scope at `b387074` (CI `40134426833`). `conversations delete FILE UUID` reviews local deletion; repeat with `--expected-revision REVIEWED_SHA256` to commit. TUI `d` reviews and `y` confirms the same operation. Initial deletion passed all three targets at `4a51d23` (CI `40139304446`). Busy or changed data is protected, and project assets/provider history remain. Interrupted cleanup can be completed with `conversations cleanup FILE`, or `cleanup --all` after original files moved. User retention settings control reviewed `conversations prune` across all document stores; defaults keep all conversations, and unsent drafts/images are always protected. These retention and minimum-terminal refinements passed the completed three-target gates. See [saved records](docs/agents.md#read-saved-conversations-without-an-agent), [local deletion](docs/agents.md#delete-local-conversation-data) and [retention configuration](docs/configuration.md#local-conversation-retention).

## Terminal preview and prompts

```sh
lazypencil tui design.pen
lazypencil tui new.pen --create --agent claude --image reference.png
lazypencil tui design.pen --image-protocol sixel
```

The default build includes a terminal viewer with pan/zoom, node selection, Japanese multiline input, attachment review/removal, checkpoint restoration, and a persistent ACP conversation. Use Tab to change panels, Alt+Enter or Ctrl+S to send, Ctrl+V or `/paste-image` to paste, and `/attach <quoted path>` for files. Ctrl+P opens the command palette; `/history` opens a paged checkpoint list with review and guarded restoration. Kitty and Sixel have protocol-level PTY tests; halfblocks and external PNG viewing provide fallbacks. Prompt and viewing/global keys can be configured in user settings. Native clipboard→Ctrl+V→image-worker→deterministic ACP checks passed on all three targets. Physical-terminal/macSKK behavior and final environment/release acceptance remain separate. See [terminal behavior and verification](docs/tui.md).

## Development

Keep heavy Rust, Skia and Docker builds on GitHub Actions. Documentation updates need source inspection, formatting and workflow linting, not an application or xtask build.

Development, CI, distribution, license verification, fixtures and measurement live in the independent [Rust xtask workspace](xtask/Cargo.toml), which does not depend on the app or Skia. Its [notice inventory](xtask/notices/RUST-NOTICES.txt) covers 135 packages including the workspace package. App integration tests use the native `lazypencil-test-helper`. Maintained Python/Shell helpers are removed; upstream native build tools and JavaScript browser/ABI inputs remain separate dependencies.

```sh
cargo xtask --help
cargo xtask test
```

Run heavy build/test commands on hosted runners. `cargo xtask test` prepares native helpers, isolates user settings and runs both feature configurations and Clippy. Workflows set up environments and call Rust tasks.

CI starts core verification, one default-feature build, and the isolated release build concurrently on macOS Apple Silicon. The default build performs Cargo-owned checks and publishes one source/compiler/hash-bound executable catalog. Normal, pixel/font, terminal, browser/runtime and Material tests consume that same catalog on separate Mac runners without native compilation or Cargo dependency-cache downloads. All transfers retain the same receipt; runtime receives only its 11 scheduled test targets and the application. The normal and pixel consumers download the same uploaded bundle. Each bundle is one tar/Zstandard stream with a 128 MiB matching window, so statically linked code can compress across executable boundaries; Actions does not recompress individual files. Consumers accept only receipt-listed regular members before the original byte and compiler checks and reject any Cargo execution. The pixel consumer owns the complete font/project/graphics/mesh invocations and the real-emoji checks, with their existing filters, thread counts and independent deadlines. The signed-source gate requires all five consumers and runs the complete shared image checks after they succeed. The full-target lint replaces the identical focused subset. The default builder also leaves core namespace compilation to the complete core job; standalone focus and rendering-only attachment configurations retain their own builds. Lightweight planning/audit/source-packaging jobs may use Linux runners; they do not build or certify a Linux application. Deferred Linux native jobs run only when explicitly requested through diagnostic workflows. Standalone focus workflows retain verified archive handoff. Compatible checkpoints use actual compiler dep-info to invalidate affected units while preserving unrelated production/test outputs. Cargo always checks freshness. Missing inputs, changed recipes/modes or incompatible environments reject reuse. Separate mapped-release checkpoints let test-only changes preserve the executable and dependencies.

Runtime-only browser installer/catalogue inputs are separated from compiler inputs, with real-Cargo freshness and compiled-consumer rejection controls. Expanding eligibility causes a one-time cache namespace transition. Tool, native-dependency, integration and release checkpoints have bounded retention; identical complete focused payloads are not also saved as application-cache copies.

The Mac core runner also owns the complete default-feature Clippy check and normal development build, so test consumers can start without waiting for lint. Both configurations remain mandatory at the final gate. Selected reproductions first compile only their required integration binaries and library; unrelated selected targets remain in the later complete build. Optional-feature boundaries, feature-switching conversation/core reproductions and standalone focus retain their existing builds. A compiler-reported early catalog starts reproductions while Cargo compiles the remaining targets with the same test graph; both operations are joined, including on failure. Terminal and color-font reproductions use immutable catalog binaries when available, avoiding another Cargo lock wait. The builder keeps the test graph active during overlap; the independent normal development build cannot replace its CLI executable.

The complete TUI, dynamic_render and runtime_config targets run on a mandatory terminal consumer because their short launch/script deadline cases are CPU-sensitive. Standalone default routes retain their quiet interval before other tests. Library units, general integrations and script_scope occupy three bounded lanes on the normal host; graphics, font/project/mesh and emoji checks use three lanes on their own host. The headed Chrome login/storage case runs once before other runtime producers and is excluded from the later readiness invocation; the original browser-suite budget includes both phases. Remaining browser readiness cases use two test threads while other browser targets remain serial within each lane and Material styles run serially on their independent consumer. These groups reuse the source/hash-verified executable catalog. They own separate temporary files, logs, settings, automatic ports and process trees. All started groups are collected and all results checked after failure. MCP and TUI lifecycle checks run concurrently with separate settings paths and their original per-command deadlines. Runtime ownership checks follow that runner's producers; the shared image gate follows all consumers and rejects mixed catalogs or colliding evidence names. Normal and pixel tests retain their original active test budget, including Cargo checks completed by the builder. The release runner measures after its compilation and smoke checks, without concurrent test processes on that runner. Core and default executable catalogs have distinct cache contexts; only the release lane publishes mapped release outputs. OS, feature and debug/release purposes retain separate evidence; matching names alone do not establish duplication.

Hosted tests optimize only the lazypencil package at level 1 while explicitly retaining debug assertions and overflow checks. Normal development builds, dependencies, Skia and release settings retain their original profiles. Application cache identities bind these exact Cargo options; release and dependency identities remain stable. The first test-profile build is measured separately from reuse with matching compiler inputs.

Repeated bundled-font digest work is shared only for immutable embedded bytes. External font loads retain fresh input hashes, and extracted collection faces retain their independent identities.

Use existing run/job/step timestamps and `ci-timings.jsonl` for queue, build and test durations. Preserve useful live CI; inspect completed logs and selected small artifacts. Reuse valid unchanged-condition evidence instead of rerunning it for documentation changes.

For a controlled application rebuild measurement, dispatch CI with `rebuild_application=true`. It bypasses application, integration and release checkpoints while retaining dependency caches, Cargo freshness checks, every test and the signed-source gate. The default is false; report this condition separately from complete application-cache reuse and from rebuilding dependencies.

Consumers download the same run's small, hash-checked xtask and test-helper binaries and invoke xtask directly, avoiding another Cargo tool build tree and tool recompilation. Every executable bundle retains the identical complete catalog receipt; terminal and Material bundles contain only their scheduled targets and the application. Packing verifies compiler bytes directly into tar rather than syncing another executable tree, with complete and subset archives prepared in two joined lanes. The final gate requires all five consumer receipts and the original shared image checks.

The latest complete Mac run, [ac6df19](https://github.com/EkeMinusYou/lazypencil/actions/runs/41000322063), passed every native, audit, image and signed-source gate in **8m47s**. It reused 304 checked compiler outputs from the preceding application source; Cargo reported all 408 units fresh, including 50 application units. The normal consumer retained all 771 preceding named invocations, including ignored cases. This verifies the helper-change/reused-application condition. The preceding application-change run took **15m25s**; cold dependencies, application rebuilds and runner allocation are separate conditions. Historical timings, including the former cross-platform scope, remain in [performance observations](docs/performance.md) and [implementation status](docs/implementation-status.md).

## Source distributions and CI

Complete source archives preserve the vendored Skia patch, both Cargo workspaces/lockfiles, owned sources, resources, examples, tests and original notices. Downloaded native checkouts, caches and local data are excluded. `cargo package` drops local patches/nested packages, so registry publication is not the supported distribution route for this tree.

```sh
cargo xtask dist source --output dist/lazypencil-source.tar.gz
cargo xtask dist verify-source dist/lazypencil-source.tar.gz --extract /tmp/lazypencil-source
```

Creation requires a clean checkout; `--allow-dirty` explicitly creates a development snapshot. Verification checks complete membership, sizes, SHA-256, modes, source identity, local Cargo patches and extraction boundaries. Existing outputs are preserved. Final CI creates and signs source only after every required native/audit job succeeds.

Binary candidates contain the matching complete source, executable, original project/dependency notices, corresponding-source materials and a hash/mode manifest:

```sh
cargo xtask dist binary --executable /build/release/lazypencil \
  --source-archive /build/native-source.tar.gz --evidence /build/release-evidence \
  --output /tmp/lazypencil-candidate.tar.gz
cargo xtask dist verify-binary /tmp/lazypencil-candidate.tar.gz --extract /tmp/lazypencil-install
```

The [release-evidence workflow](.github/workflows/release-evidence.yml) reuses valid mapped release outputs, runs Cargo freshness checks, gathers original notices/build/linkage evidence, packages the candidate and smokes the installed binary with source/build trees hidden. Packaging and independent source/notice/pixel checks passed on all three targets. `release_ready: false` remains until minimum-environment, applicability and final publication review is complete.

Signed GitHub build provenance is implemented for final source and manual candidates. Verify the archive against its reviewed source/signer commit:

```sh
gh attestation verify /path/to/lazypencil-candidate.tar.gz \
  --repo EkeMinusYou/lazypencil \
  --signer-workflow EkeMinusYou/lazypencil/.github/workflows/release-evidence.yml \
  --source-digest EXPECTED_COMMIT_SHA --signer-digest EXPECTED_COMMIT_SHA \
  --source-ref refs/heads/main --deny-self-hosted-runners
```

For source archives select `.github/workflows/ci.yml` as signer workflow. Build provenance, OS signing/notarization and release approval are separate.

The retained deferred-Linux real-terminal diagnostic uses Kitty/xterm under owned Xvfb/software OpenGL. Four explicit/automatic Kitty/Sixel modes and twelve phases cover actual pixels, Japanese committed text, cursor geometry, zoom/pan/resize/restoration and normal exit. Debug and release checks have different purposes. The initial-release Mac CI uses native PTY checks: its VM failed Kitty 0.49.2 OpenGL initialization before app startup. The explicit `macos_terminal_gui` release-evidence input selects a **billable GPU macos-15-xlarge runner** for the preserved native Mac window test; this route has not run. Neither PTY nor Linux images certify physical macSKK preedit/candidate windows. See [terminal verification](docs/tui.md#verification-and-remaining-scope).

Manual runtime/process/release workflows are diagnostic entry points, not reasons to duplicate a useful full CI. Artifacts expire after their recorded retention period; source/run/hash bindings identify evidence without promising permanent downloads.

### Licenses and build evidence

The application [Rust inventory](resources/licenses/rust/index.json) covers 391 locked packages; tool notices are independent. Original blobs, archive/source hashes and reviewed supplements accompany readable notices. The `option-ext` MPL-2.0 allowance retains its exact source archive and terms. Noto Sans JP retains OFL-1.1; optional packs and external Chromium/ACP adapters retain their licenses.

```sh
cargo xtask notices rust verify
cargo fetch --locked
cargo xtask notices rust verify --cargo-home /path/to/cargo-home
cargo deny --all-features --locked check licenses sources advisories
```

Hosted policy pins cargo-deny 0.20.2. Collection requires checksum-verified original crate archives and reviewed upstream inputs, never generic replacement terms. The [tool-notices workflow](.github/workflows/tool-notices.yml) uses the existing Rust collector to regenerate and independently reverify proposed tool notices; normal CI still rejects stale bundles.

Native evidence covers actual Skia/QuickJS/wrapper and asset inputs, original notices, corresponding-source materials, Rust standard-library/toolchain data, direct load declarations and the matching raw linker map. Mac candidates retain embedded FreeType's original notices. `link-map-inputs.json` is implemented and recomputed during packaging. It is an inclusive archive-member list that may include discarded sections; exact live-code/license applicability still needs final review.

Three-target native gates retain 42 release-render observations and same-binary linkage reports. Prior Linux binaries required glibc symbols through 2.39; a prior Mac binary declared minimum OS 11.0. These declarations do not prove minimum-OS execution or resolve every transitive system library. Minimum macOS/guaranteed terminals and final Mac-binary publication remain open. Linux ABI results are retained for later support.

Current source excludes retired Pencil-exported reference PNGs and derived JSON; tests use owned inputs and independently calculated expectations. Old Git history and past CI artifacts retain the retired files; their public-release handling is unresolved. See [permission/history status](docs/design-guidance.md#pencil-の-skill-とライセンス).

## License

MIT. Bundled third-party assets and dependencies retain their respective licenses.
