# lazypencil

A terminal-first editor and renderer for `.pen` design files, written in Rust.

**Development status:** implementation is in progress. The repository is not yet a finished release. See [implementation status](docs/implementation-status.md) for requirements and verification evidence.

**Initial release platforms:** macOS Apple Silicon (`aarch64-apple-darwin`), Linux x64 (`x86_64-unknown-linux-gnu`) and Linux arm64 (`aarch64-unknown-linux-gnu`); Linux requires glibc. Intel Macs are unsupported. Windows support is deferred; existing Windows code and fixtures are retained, with further work postponed. Minimum OS/distribution versions and supported terminals must still be established. Earlier Windows/Intel results below are historical evidence, not current support commitments.

The intended product provides a CLI, an MCP server, and a TUI with keyboard navigation and image previews. Claude Code and Codex integration uses ACP adapters. Rendering uses Skia CPU and SkParagraph, with Noto Sans JP as the bundled default font. Chromium is installed separately for browser nodes and WebGL shaders.

The complete scope is recorded in the [design](docs/design.md), [component and layout semantics](docs/pen-semantics.md), [design guidance](docs/design-guidance.md), and [image attachment specification](docs/image-attachments.md).

The initial release is complete only when the [design acceptance conditions](docs/design.md#検証と受け入れ条件) are met on all three targets, including CLI/MCP/TUI, dynamic content, both AI integrations, performance, terminal/clipboard behavior, and distributable binaries with complete licensing materials. Passing hosted CI alone does not complete those conditions. Only the platform scope has changed.

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

Save shared fonts with `config init --user`, then edit the reported `config.toml`. Project `lazypencil.toml` overrides individual fields: font files/search directories, default family, fallback chain, and language. Omitted fields inherit user settings; explicit arrays replace them, including empty arrays. `config init` creates a project template without overwriting an existing file. `config show --user` / `config show design.pen` expose sources and effective settings; `config schema --user` / `config schema` describe the supported formats. `fonts list` uses user settings and `fonts list --file design.pen` adds project overrides. CLI, MCP, and TUI capture the same data. User `[agents]` selects a default agent; `[agents.codex]` and `[agents.claude]` persist adapter, model, mode, authentication, timeout and document-tool preferences for both `prompt` and TUI. Explicit CLI options take precedence. See [configuration](docs/configuration.md) for platform paths, user agent profiles, and `LAZYPENCIL_CONFIG_DIR`, and the [project example](examples/lazypencil.example.toml). Project guides/styles require an explicit local user grant; the remaining configuration sections and project executable trust remain unfinished.

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

Ctrl+C cancels edits while waiting for locks and at checkpoints during batch and component preparation. MCP edits use the request's cancellation token. A cancellation before document replacement preserves the original document; complete unreferenced managed assets can remain. Replacement already in progress finishes its journal commit. See [cancellation and retry behavior](docs/mcp.md#snapshots-limits-and-cancellation).

Inherited edits use `{"node_id":"instance","instance_path":["nested-instance","label"]}` as the target. Override reset and replacement, slot children, theme axes, and import declarations have explicit operations. `detach_instance` handles local, external, and inherited instance targets while preserving nested references and variable origins; external scripts retain their execution/input scopes through versioned lazypencil metadata, with Pencil round-trip rendering unverified. Inspect an inherited node with `inspect design.pen --node instance --instance-path nested-instance/label`; use `--library-alias ui` for declared library definitions or `--expand-instances` for expanded trees. `import_component` supports linked instances and static dependency copies from project libraries and bundled styles, with explicit name collision handling. The [MCP guide](docs/mcp.md) describes addresses, capabilities, and remaining gaps.

Use `assets put design.pen --kind image --input reference.jpg` to create a checked, normalized PNG, or `--kind script` / `--kind shader` for UTF-8 code. The returned URI points to an immutable file under `assets/lazypencil` beside the document. Referencing it is a separate editing batch; storing code does not execute it.

Asset uploads and component imports coordinate with dependency readers through the destination's writer lock, including absent files. `assets put` and MCP uploads can be cancelled while waiting for that lock and before publication. Existing content is rechecked after waiting and is never overwritten. See [managed asset behavior](docs/mcp.md#managed-assets).

Missing declared local image, JS, and GLSL files can be diagnosed and repaired through ordinary revision-checked editing batches, including partial repairs and checkpoint restores. `inspect` and edit results report `assets_complete` and `asset_diagnostics`; layout and rendering reject known missing files. Missing libraries, configured font files, and runtime-discovered resources require additional handling. See [missing asset repair](docs/mcp.md#repairing-missing-assets).

These commands are an implementation in progress. Complete ACP/TUI behavior, dynamic-content compatibility, and some graphics and MCP features remain unfinished. Unsupported node and fill types produce explicit rendering errors. `validate` currently checks structure, references, variables, and themes; successful validation does not establish that every graphics feature or script can be rendered.

## Script-generated designs

```sh
cargo run -- render examples/script-chart.pen --output script-chart.png
cargo run -- guidance scripts
```

The original chart example combines editable `.pen` layout, Noto Sans JP, and generated geometry. Script nodes use a separate Rust worker with QuickJS-NG 0.16.2 through the pinned `rquickjs` 0.14.0 binding. Headers accept explicit `@schema 2.19`, `@schema 2.21` and legacy `@schema 2.11`, with six typed input kinds; header compatibility does not enable 2.11 document files. The engine exposes `pencil.width`, `pencil.height`, and `pencil.input`; output is measured and painted by the same Skia path as ordinary nodes. Generated children remain derived data and carry `generated_by` metadata.

Use `inspect design.pen --calculated --node chart --depth 2` to read generated properties and their measured geometry. MCP `read_nodes` accepts `calculated: true` for a new calculation or `snapshot_id` to reuse the exact tree from layout/render without rerunning scripts. These reads expose resolved variables, themes, and property origins; generated IDs remain read-only. Ordinary stored reads do not execute scripts. See [calculated node reads](docs/mcp.md#calculated-and-generated-nodes).

Header and JavaScript failures report original-source positions and bounded excerpts when available. JSON/MCP results keep these in `error.details.script_diagnostic`, with the captured source hash, path, and owning address. Readable errors also show the location and source lines. Columns count UTF-8 bytes; dynamic-code call sites and unavailable positions are explicit. See [script execution and diagnostics](docs/mcp.md#script-execution).

The interpreter defaults to a 2 second execution limit, a 64 MiB heap limit, and a fixed 1,000 node output limit. User/project `[runtime.javascript]` settings configure evaluation/heap/layout/idle limits; CLI, MCP, and TUI share captured settings and execution metadata. A parent watchdog, absolute layout deadline and cancellation terminate unfinished work. Each request gets a fresh JS context; the default idle reap interval is 60 seconds. Scripts have no host filesystem, network, DOM, module loader, or timer APIs. Random numbers use Mulberry32 with seed 0, matching 37 observed outputs through draw 65,536 from the original native CLI 0.3.9 corpus. At source `e6a1bfb`, real QuickJS and both-codec images passed on all three initial targets, with every saved RGBA channel independently matching the native reference. The author's original CC0 dedication is retained. Complete generated-node/input compatibility and final product acceptance remain unfinished. See [native reference scope](docs/dynamic-content.md) and [execution limits](docs/configuration.md#javascript-execution-limits).

Missing or null script inputs use their declared or type-specific defaults without changing the file. Stale saved controls remain in the document and are excluded from runtime input and variable lookup. A new 22-case native corpus covers these rules; its actual worker/render regression is under hosted verification. See [input behavior](docs/dynamic-content.md#入力の既定値と保存済みの旧項目).

Files discovered only in generated content appear in `runtime_dependencies`, separately from the document-declared `dependency_revision`. A retained calculation snapshot captures and checks both sets before rendering. After reviewing a calculation, pass its `runtime_dependency_revision` to `apply` as `--expected-runtime-dependency-revision` to check those files before saving as well. This re-evaluates scripts before taking save locks and checks captured files again before replacing the document. Replaying a committed request does not re-evaluate scripts. See [dynamic execution](docs/dynamic-content.md) for current limits and remaining requirements.

## Browser pages and GLSL

```sh
cargo run -- runtime setup chromium
cargo run -- runtime status
cargo run -- render examples/dynamic-panel.pen --time 0.5 --output dynamic-panel.png
```

The setup command explicitly downloads a pinned Chrome for Testing headless shell and verifies its archive and installed files. Rendering never downloads a runtime. [Runtime installation](docs/runtimes.md) covers offline archives, custom executables, platforms, licensing, and testing.

Browser nodes capture HTTP(S) or HTML data pages with dimensions, zoom, scroll, and basic desktop/phone/tablet modes. Shader fills and effects execute fragment GLSL through a separate software WebGL process, with numeric, color, vector, and image uniforms. Effects can sample the object and previously painted background. `--time` defaults to zero; `--refresh` requests fresh page acquisitions. MCP retains a bounded raster cache and returns acquisition metadata with the actual runtime, inputs, and image hashes. The shader preprocessor is bundled from a source-bound GHA build; using lazypencil or building the native application does not require Node or a Wasm toolchain. Shader coordinate and resolution handling at enlarged output is described in the [dynamic-content specification](docs/dynamic-content.md#宣言と数値入力).

The original dynamic-panel example combines a local HTML data page, GLSL, and native Japanese text without external page resources. The [uniform study](examples/uniform-study.pen) reuses one shader with flat color arrays, grouped float declarations, and column-major matrices. With Chromium installed, render it using `cargo run -- render examples/uniform-study.pen --output uniform-study.png`. Numeric arrays and GLSL constant-expression dimensions are checked against the active linked WebGL inputs; defaults remain transient zero values. One-dimensional sampler2D arrays accept lists of captured image URLs, including per-element string variables with aliases and themes. The runtime binds the active image prefix and uses transparent defaults for unset elements. Fixed-image lists and local relative string sampler variables can be copied independently while preserving shared text strings, aliases and themes through ordinary imported variable libraries. Absolute string paths and conflicting path projections remain unsupported. See [declaration rules and limits](docs/dynamic-content.md#宣言と数値入力).

Page and shader processes expire independently after 60 idle seconds by default; cached images remain available, and explicit refresh can restart the processes. User/project `[runtime.browser]`, `[runtime.shader]` and `[runtime.dynamic]` configure acquisition/launch deadlines, idle expiry and encoded-image cache limits through the common render entry. Configuration, real policy reload, dynamic pixels, MCP and idle/dynamic TUI checks passed a hosted macOS arm64 run; Linux startup and expanded regression coverage remain under verification. See [limits and scope](docs/configuration.md#browser-shader-and-raster-cache-limits) and [hosted evidence](docs/runtimes.md#hosted-integration-without-local-build-trees). Complete Pencil device presets, remaining annotations, macro/conditional declarations and full vertex compatibility, fill-stage object sampling, persistent authenticated page contexts, configurable capture policy, animation controls, and all-platform verification remain unfinished. These rendering paths do not establish full Pencil compatibility.

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
```

The host supports ACP v1, image blocks, document-bound MCP, native session reload, permissions, and cancellation. It checks adapter-reported authentication against the requested subscription/API route and cancels a conflicting change. `--model` and `--mode` select advertised options; document-tool preauthorization does not approve shell or unrelated file operations. `--json prompt` streams NDJSON.

Turn logs keep the initial guide/style identities and metadata for additional resources returned by the owned MCP server. `design_resource` events carry IDs, versions and hashes without duplicating guide bodies or examples; they are emitted at turn end, including failed or cancelled turns. See [recording behavior and limits](docs/agents.md#design-resource-records).

Codex and Claude completed image-backed subscription workflows on macOS arm64, including registered image import, edits, layout, and PNG previews. [The editable settings example](examples/agent-settings.pen) was generated through the Codex workflow; its asset URI was made portable to reference the original bundled image. This demonstrates the workflow, not aesthetic equivalence with Pencil. Actual API/provider workflows, complete clipboard/TUI behavior, other platforms, and release validation remain unfinished. See [agent integration](docs/agents.md) for installation, limits, evidence, and constraints.

Use `--session LOCAL_UUID` to continue a saved conversation. If native reload is unsupported, the host reports a new native session and supplies current document context with bounded local history. A reload error remains visible; use `--session LOCAL_UUID --restart-session` with an explicit request to start anew while preserving local records. Historical prompts and images are not automatically resubmitted. These shared CLI/TUI recovery paths await hosted verification; see [conversation details](docs/agents.md#conversations-and-events).

## Terminal preview and prompts

```sh
lazypencil tui design.pen
lazypencil tui new.pen --create --agent claude --image reference.png
lazypencil tui design.pen --image-protocol sixel
```

The default build includes a terminal viewer with pan/zoom, node selection, Japanese multiline input, attachment review/removal, checkpoint restoration, and a persistent ACP conversation. Use Tab to change panels, Alt+Enter or Ctrl+S to send, Ctrl+V or `/paste-image` to paste, and `/attach <quoted path>` for files. Ctrl+P opens the command palette; `/history` opens a paged checkpoint list with review and guarded restoration. Kitty and Sixel have protocol-level PTY tests; halfblocks and external PNG viewing provide fallbacks. Native clipboard and physical-terminal compatibility, configurable keys, and the remaining platform/release work are unfinished. See [terminal behavior and verification](docs/tui.md).

## Development

Run full Rust, Skia and Docker builds on GitHub Actions by default. Local work should focus on source changes, formatting, workflow linting and lightweight Python checks. Avoid creating local build trees or repeated extracted source snapshots for routine verification. Keep only the logs and selected artifacts needed for the current investigation.

Rust 1.94 or newer and a C compiler for QuickJS-NG are required to build from source. Skia uses prebuilt libraries where supported; otherwise its upstream build tools are required.

```sh
sh scripts/ci/test.sh
cargo clippy --all-targets -- -D warnings
```

Development verification includes macOS arm64 and a Debian 12 arm64 container using the declared Rust 1.94 minimum. Exact test/build conditions and remaining platform/release work are recorded in [implementation status](docs/implementation-status.md). The vendored Skia binding patch and its upstream source are documented in [LAZYPENCIL_PATCH.md](vendor/skia-bindings/LAZYPENCIL_PATCH.md).

## Source distributions and CI

Use the source distribution tool to retain the local Skia binding patch and the original Cargo manifests:

```sh
python3 scripts/dist.py source --output dist/lazypencil-source.tar.gz
python3 scripts/dist.py verify-source dist/lazypencil-source.tar.gz --extract /tmp/lazypencil-source
cd /tmp/lazypencil-source
cargo build --locked
```

Python 3.11 or newer is required by the distribution tool. Archive creation requires a clean Git checkout; `--allow-dirty` explicitly creates a local development snapshot with its dirty provenance recorded. The extraction destination must not already exist. The archive includes project sources, the vendored binding patch, fonts/icons and their existing license files, examples, tests, and documentation. Downloaded Skia checkouts and build caches are excluded. Dependencies and native Skia sources still need to be obtained during a first build; this is not an offline dependency vendor bundle.

Release sources are selected from Git's tracked files within the distribution's source directories. A development snapshot also includes visible untracked files, respecting Git ignore rules; ignored local data is excluded from both modes. Without Git, only the explicit development mode is available and collects the documented directories from the filesystem.

`SOURCE-MANIFEST.json` records file sizes, SHA-256 hashes, executable modes, package identity, and Git provenance. The verifier checks all listed members, rejects unlisted files, links, escaping or colliding paths, enforces byte/count limits, and checks that local Cargo patches are present before extraction. Repeated captures of unchanged inputs produce the same archive bytes. The complete archive is published locally without overwriting an existing file. The manifest establishes content integrity; release signing remains unfinished.

`cargo package` removes Cargo patch sections and excludes nested packages, so its generated crate is not the supported distribution route for the current patched tree. Registry publication needs separate verification. See the [Cargo packaging behavior](https://doc.rust-lang.org/cargo/commands/cargo-package.html) and the current [implementation status](docs/implementation-status.md).

The CI workflow is configured to build **from verified source archives** using Rust 1.94 on macOS Apple Silicon and Linux x64/arm64, the three initial-release targets. It checks both feature configurations, formatting, Clippy, source archive integrity, and release-profile renders of the mesh example, a 1,000-node Japanese/component/image dashboard, and the graphics study with a stroke-only decoration comparison. The final source job rejects missing, skipped, failed or cancelled native/audit results before creating an archive; success of the focused job alone is insufficient. Hosted runners also explicitly prepare the three checksum-pinned original Material fonts and run the native all-name/continuous-weight test and the real MCP weight/snapshot test. Downloaded fonts stay in runner temporary storage and are not added to the source distribution. Workflow syntax is checked with actionlint; a configured job is only platform evidence after it actually executes successfully. Sandboxed Chromium opt-ins run in the same hosted jobs after the ordinary tests, reusing their binaries. Real subscriptions and physical clipboards/terminals still require their separately recorded environments. Initial Linux release timing and memory observations, including a parent-image copy correction, are recorded in [performance observations](docs/performance.md).

CI runs for pushes to `main`, pull requests and manual dispatches. A focused gate first tests the affected targets on one supported representative OS (Linux x64 by default, macOS Apple Silicon for macOS-specific changes). It compares with the last successful full release source gate so superseded, unverified changes remain in scope. Only a successful focus starts the three-platform matrix and dependency audit. New commits replace obsolete runs for the same branch and platform scope. The browser checks share the native job's compiled dependencies and application rather than repeating the independent Chromium build. Real 60-second idle cleanup, sandboxed pixel checks, PTY input, both feature configurations, Material checks, release benchmarks and source verification remain final gates.

Use `gh workflow run ci.yml --ref main` for the complete three-platform release gate. `-f focus_os=macos-15` overrides the first platform for a macOS issue; `-f os=macos-15 -f focus_os=macos-15` limits a diagnostic run to Apple Silicon macOS. A single-platform run does not produce the full release source artifact or establish final acceptance. All hosted workflow entry points offer only `macos-15` (Apple Silicon), `ubuntu-24.04` (x64) and `ubuntu-24.04-arm`. The manual `chromium.yml` entry point delegates to this pipeline; do not dispatch it alongside full CI to repeat browser checks.

CI caches Cargo downloads, keeps compilation outputs in the hosted runner's temporary directory, and retains logs and rendered examples for three days. Build and test steps are separate, and response waits and test phases have explicit bounds. Use existing run/job/step timestamps to compare dependency waits, runner queues, builds and tests. Download only the needed platform's artifact. Completed job logs can be read with `gh api repos/OWNER/REPO/actions/jobs/JOB_ID/logs`; local workflow linting alone does not run GitHub Actions.

Hosted dependency builds, downloaded Skia sources and original Material fonts can be reused under a cache key bound to the platform, runner image, toolchain/build configuration, lockfile, vendored binding patch and icon catalogue. The application package is cleaned in both profiles before verification and before a compatible dependency cache is saved. A successfully compiled dependency set can be retained even when a later regression test fails; the failed run remains failed. Partial rendering builds use a checkpoint key, allowing a later successful full run to save the complete dependency set under its final key. Core-only focused checks use a separate cache namespace without Skia. Old render PNGs/JSON are removed too, so cached application code or images cannot substitute for the source being tested. Cache availability is optional; build and test checks still run. Artifact names include the run attempt so retries retain distinct evidence. A reviewed migration for the Windows-only direct `windows-sys` feature addition at `4a8d082` may restore the exact previous dependency key. It is restricted to the audited input tree, minimum compiler identity and baseline build flags; a new primary key is still used, Cargo rebuilds changed Rust feature sets, and application outputs are always pruned. No dependency version, checksum or native source changed in that migration. The Wasm/notice-only attribute additions at `e9c90da` have a second audited migration: all 67 existing native-input paths retain identical Git attributes and dependency bytes. It restores only the exact prior v2 key for the same compiler, runner image and baseline flags, while retaining the new primary key and pruning application outputs.

The manual `Agent process integration` workflow first reproduces embedded-helper startup and actual bound MCP edits with a 10-second response limit, then runs the complete core suite and core all-target Clippy without building Skia. It retains logs and a source-bound report. Use `gh workflow run process.yml --ref main -f os=ubuntu-24.04` (or `ubuntu-24.04-arm` / `macos-15`). No model request or subscription credential is needed. Windows lifecycle implementation and fixtures remain in the repository for future support; Windows is outside the initial-release CI and acceptance gate.

The release smoke script also records repeated native-child measurements for the two fixed 1,000-node workloads in `release-benchmark.json`: timing, per-child peak memory/CPU, executable/input hashes and matching render hashes. It uses the built executable directly and preserves only the small JSON report. This new hosted integration awaits verification; it does not establish hardware-independent performance acceptance. See [measurement scope and observations](docs/performance.md#repeatable-native-child-measurements).

Dependency policy uses cargo-deny 0.20.2:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --all-features --locked check licenses sources advisories
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

The policy checks all declared features, registry sources, SPDX expressions, and RustSec advisories. The unchanged `option-ext` dependency has a version-specific MPL-2.0 allowance. Its complete original source archive, matching license, and [source notice](resources/licenses/SOURCE-NOTICES.txt) are included under `resources/licenses`; an automated check matches them against `Cargo.lock`. Keep these source/notice assets with binary distributions.

The [Rust notices](resources/licenses/rust/RUST-NOTICES.txt) and [machine-readable inventory](resources/licenses/rust/index.json) cover all 388 packages in the current lockfile, including optional, build, and platform dependencies. Original notice bytes, source locations, archive checksums, and pinned upstream supplements are retained alongside the readable notices. Published author declarations are identified as declarations; they are not substituted for copyright notices. Reference MIT terms are explicitly distinguished from package notices. The readable file normalizes whitespace, while the original blobs retain their exact upstream bytes through Git.

```sh
python3 scripts/license_notices.py verify
cargo fetch --locked
python3 scripts/license_notices.py verify --cargo-home "${CARGO_HOME:-$HOME/.cargo}"
python3 scripts/license_notices.py collect --cargo-home "${CARGO_HOME:-$HOME/.cargo}" \
  --upstream resources/licenses/rust/upstream.json --output /tmp/lazypencil-new-rust-notices
```

The first check verifies the checked-in bundle, current lockfile, and local package inputs. The second also compares every registry package with its checksum-verified original archive. Collection is offline and requires those archives plus the curated upstream inputs already present; the output directory must not exist. New dependencies without a primary notice require reviewed upstream provenance rather than an automatic generic license substitution. Review and verify the new bundle before replacing the checked-in inventory. CI is configured to run both verification levels.

This is a source-package inventory, not a list of code linked into a specific binary. Passing the policy and notice checks does not complete the native Skia/QuickJS or Rust standard-library audit. Native/toolchain attributions, per-binary notice assembly, binary archives, minimum OS validation, signing, and release publishing remain unfinished. No Chromium or ACP adapter is added to the source archive.

The manual `Release build evidence` workflow builds a release executable from the verified source distribution on a selected hosted platform. It reuses the normal dependency cache, cleans application outputs, and captures [Cargo JSON artifact and build-script records](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages). `scripts/release_evidence.py` pairs each native artifact with its build-script package identity, uses the reported output directory, hashes the executable and selected native archives, and collects and reconstructs the Skia and QuickJS notice inventories. It checks distributed source files and build outputs before and after collection, rejects unlisted source inputs and conflicting archive aliases, and verifies a copy of the checked-in Rust notice bundle. The full collector, including QuickJS and corresponding-source materials, passed on [macOS arm64 at source ec6d326](https://github.com/EkeMinusYou/lazypencil/actions/runs/40026880547), [Windows x64 at source 5a9698e](https://github.com/EkeMinusYou/lazypencil/actions/runs/40028843720) and [Linux x64 at source e99b62e](https://github.com/EkeMinusYou/lazypencil/actions/runs/40028057056). Each downloaded artifact's 325 file sizes/hashes and complete member list were verified. Other platforms remain under verification.

Its three-day artifact contains `build.json`, a file-hash manifest, the source manifest, original project notices, `licenses/` with the checked-in Rust notices and corresponding-source materials, and `skia/` and `quickjs/` input/notice evidence. The complete option-ext source archive and MPL terms retain the relative paths in their original notice. The artifact records compiler and runner identities, selected native features/link declarations, and remaining review. It excludes the executable and arbitrary build-script environment variables. The report always sets `release_ready: false`: Cargo declarations and Ninja inputs do not prove final linkage, and Rust standard-library/system notices, complete native/asset review, signed provenance, packaging and publication remain separate requirements. Hashes provide content integrity, not build authenticity or legal approval.

QuickJS input tracing is pinned to the reviewed rquickjs-sys 0.14.0 archive and build-script hashes. It verifies the selected installed manifest/build script, all nineteen copied C/header inputs (four compiled C sources), the bundled Rust FFI bindings, and the generated target-selection macro against the observed compiler host. It retains the original QuickJS license and copyright/license comment candidates from the complete selected C/header files. Newly generated bindings, package/build-script changes and targets outside the collector's reviewed target set require review. That set includes historical Windows/Intel fixtures and is broader than the three initial-release targets. `--cargo-home` selects an existing offline registry cache; no downloads or compilation are performed by the collector. Original-archive copy fixtures passed for all five target triples. The three hosted results additionally verify actual native build inputs and retain fifteen source comment candidates each.

The collector also assembles `assets/` with the original Noto Sans JP OFL/publisher metadata and six icon packs' license/provenance materials. It checks fifteen reviewed literal font/icon include declarations in Rust sources and twenty asset inputs against the existing font manifest and icon catalogue pins. New, repeated, missing or computed include declarations require review; changed payloads, notices and provenance descriptors fail. The seventeen-file bundle retains original notice bytes, including Phosphor's CRLF, and does not duplicate the embedded font or icon payloads. Optional downloaded fonts/geometry and user/project/system assets are outside this inventory. Source declarations establish an input inventory; final linker inclusion and full applicability remain separate review. This extension passed at source `331c118` on [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40031829807) and [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40031833313); both expanded 342-file artifacts and original notice hashes were verified.

The earlier Skia/QuickJS/corresponding-source stage also passed at source `1355cfd` on [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40030972290) and [Intel macOS](https://github.com/EkeMinusYou/lazypencil/actions/runs/40030975630). Each 325-file artifact was verified, completing that earlier stage on all five hosted targets. These results predate the asset and standard-library attachments described separately here.

For a source-built Skia, collect audit evidence from the actual native build directory:

```sh
python3 scripts/skia_notice_inventory.py collect --source vendor/skia-bindings/skia \
  --build /path/to/skia-bindings-build/out/skia --upstream resources/licenses/native/upstream.json \
  --output /tmp/lazypencil-skia-inventory
python3 scripts/skia_notice_inventory.py verify --source vendor/skia-bindings/skia \
  --build /path/to/skia-bindings-build/out/skia --upstream resources/licenses/native/upstream.json \
  --output /tmp/lazypencil-skia-inventory
python3 scripts/skia_notice_inventory.py verify-upstream --upstream resources/licenses/native/upstream.json
```

This requires Ninja and a completed native build with its recorded header dependencies. It reads the graph for the five rendering/text libraries, hashes source and generated inputs, retains original local notice bytes and source comment candidates, and reports references needing further review. New native components, missing inputs, links in native inputs, unbuilt libraries, and stale inventories fail explicitly. System header symlinks remain excluded system inputs. Verification reconstructs the inventory from the build and source bytes rather than trusting a self-consistent manifest.

The pinned [native reference inputs](resources/licenses/native/upstream.json) retain the original Chromium license referenced by Chromium-origin code in Skia's zlib fork. The January 2024 repository snapshot predates the pinned zlib commit and has the same license blob as the current upstream reference. Applying it requires matching `DEPS`, referencing source hashes, repository/revision URL, notice SHA-256, and Git blob. The original source copyright comments remain separate. Omitting `--upstream` preserves the unresolved-reference finding. The standalone check needs no native build; CI also runs it in extracted source distributions.

The Windows checkout's nine referenced zlib inputs use CRLF. Each reviewed working-tree variant pins its actual byte hash and original Git blob, independently recalculated from zlib commit `646b7f569718921d7d4b5b8e22572ff6c76f2596`. Variant acceptance additionally requires exclusively CRLF newlines, the original LF byte hash and Git blob after conversion, and the same repository/revision in the observed Skia `DEPS`. Original LF pins remain valid and every inventory records the actual source hash and accepted form. Mixed newlines, bare CR, changed content, undeclared variants, and mismatched origins fail explicitly. This preserves attribution evidence across checkout line endings; complete legal review remains required.

Recorded headers can include stale or unused objects, and notice candidates deliberately include nested upstream test/build notices. This evidence is for review; it is not the final set of licenses linked into the application. Local source hashes and the retained `DEPS` file do not authenticate an upstream release or prove an unmodified checkout. Complete comment/license review, system libraries, Rust toolchain, QuickJS, assets, and runtime packaging still require separate work. The inventory command does not compile code or change the native source tree.

Collect the separately installed Rust standard-library evidence with the compiler used for the build:

```sh
python3 scripts/toolchain_notice_inventory.py collect --cargo-home /path/to/stdlib-crate-cache \
  --upstream resources/licenses/toolchain/upstream.json --output /tmp/lazypencil-toolchain-inventory
python3 scripts/toolchain_notice_inventory.py verify --cargo-home /path/to/stdlib-crate-cache \
  --upstream resources/licenses/toolchain/upstream.json --output /tmp/lazypencil-toolchain-inventory
python3 scripts/toolchain_notice_inventory.py verify-upstream --upstream resources/licenses/toolchain/upstream.json
```

This offline tool requires installed Rust sources, the distribution's `COPYRIGHT` / MIT / Apache files, and `COPYRIGHT-library.html` with its referenced license texts. It observes `rustc --version --verbose`, the sysroot, and installed target libraries; `--rustc` and `--target` select a compiler and installed target. Separate installations can use `--source-root`, `--legal-root`, and `--doc-root`. Missing inputs fail explicitly; nothing is downloaded or installed.

The manual `Standard-library notice evidence` workflow prepares these inputs on a selected hosted platform without compiling the application. Its runner-only helper pins Rust 1.94.0's full compiler commit and official source-distribution checksum, selectively extracts library/legal inputs, verifies original registry archives against the library lock, and reconstructs the installed target-library inventory. The approximately 274 MB source download stays on the runner; its archive and prepared source tree are removed after collection. Only the small original notice/provenance bundle is uploaded. [Linux x64 at source 466d90c](https://github.com/EkeMinusYou/lazypencil/actions/runs/40032401890) passed with 2,274 source inputs, 40 registry packages and 55 installed target libraries; all 100 downloaded evidence members were verified. The original vex-sdk publisher's dirty VCS metadata remains an explicit finding.

Release evidence now prepares the standard-library bundle before compilation and attaches it through `--toolchain-evidence`. Attachment requires the exact observed compiler/version/commit/target and byte hashes of every inventoried installed target library, plus original bundle integrity and complete membership. It rechecks the copied attachment, retaining original findings and corresponding sources. This joins installed library inputs to the observed build; exact linker inclusion, source-to-binary reproducibility and complete legal review remain separate. [Linux x64 at source 0acce40](https://github.com/EkeMinusYou/lazypencil/actions/runs/40033309721) passed the actual release build and attachment. Its 442-file artifact was checked for exact membership, sizes and hashes, including the recorded compiler/run identities and nested standard-library bundle hashes (55 installed libraries, 40 registry packages). At source `24414fe`, the same attachment also passed on [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40035653283), [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40035646909), and [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40035650051). Each 442-file artifact and its compiler/run/library binding were verified (55, 59 and 44 installed libraries respectively). [Intel macOS at the same source](https://github.com/EkeMinusYou/lazypencil/actions/runs/40035656900) also passed, with its 442-file artifact and 59 installed-library hashes verified. The combined notice pipeline has therefore been exercised on all five hosted targets; `release_ready: false` and the remaining linkage/system/attribution/publication work still apply.

Standalone preparation also passed at source `0acce40` on [Windows x64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40033312900) and [macOS arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40033316361). Each verified 100-file bundle contains 2,274 library source inputs and 40 registry packages, with 44 and 59 installed target-library hashes respectively. Findings are retained; matching the same version alone is insufficient for attachment.

At source `6e1fce2`, [Linux arm64](https://github.com/EkeMinusYou/lazypencil/actions/runs/40033700360) and [Intel macOS](https://github.com/EkeMinusYou/lazypencil/actions/runs/40033703894) also passed standalone collection. Both 100-file bundles were checked for exact membership, sizes and hashes, with 2,274 source inputs and 40 registry packages each, and 55 / 59 installed target-library hashes respectively. This completes standalone collection on all five hosted targets; the application-build attachment remains a separate verification.

The cache must contain the original archives named by the standard library's `library/Cargo.lock`, which can differ from the application's lockfile. Registry mode works with a `rust-src` tree without `library/vendor`; installed sources and root distribution legal files still need to be present or supplied with the path overrides. Documentation is detected under `share/doc/rustc` or `share/doc/rust`. Collection has been exercised with Homebrew Rust 1.99.1 on macOS and the official Rust 1.94.0 sources on Linux arm64. A minimal rustup installation may need `rust-src` and separately prepared `COPYRIGHT` / MIT / Apache inputs through `--source-root` and `--legal-root`.

The inventory retains original notices, source comment candidates, publisher manifests, and hashes of the installed library sources and target libraries. Present vendor files are checked against `.cargo-checksum.json`, package identities and declared archive checksums against the library lock. With `--cargo-home`, it verifies the original archive bytes against that lock, hashes every regular source member, and compares present vendor sources with those originals. Differences remain explicit findings with missing/extra/changed paths. Without `--cargo-home`, the original vendor-only mode remains available for a complete vendor tree and reports `registry_archives_verified: false`.

The [pinned toolchain inputs](resources/licenses/toolchain/upstream.json) supply omitted workspace notices for `fortanix-sgx-abi`, `vex-sdk`, and `wasip1`, bound to the original archive, published VCS commit, repository, and Git blob. Published dirty VCS metadata remains an explicit finding. If a copyright document names terms absent from the installation, only matching, pinned SPDX reference terms can fill that document's missing reference. Reference text is labeled separately from package copyright notices; placeholders are not filled with inferred authors. Missing unreviewed terms fail explicitly. The standalone integrity check runs in CI; package applicability is checked against the registry archives during collection.

An MPL-2.0 registry package also retains its complete, unmodified `.crate` under `sources/`, with its path, checksum, license, and source URL in the readable notice and inventory. Keep those sources together with their notices when assembling the eventual distribution. The inventory includes optional/test/platform dependencies and all installed target libraries, so it does not establish exact application linkage or source-to-binary provenance. Keep its findings for the final binary review. These tools run during release preparation; they add no application startup or rendering work.

## License

MIT. Bundled third-party assets and dependencies retain their respective licenses.
