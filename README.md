# lazypencil

A terminal-first editor and renderer for `.pen` design files, written in Rust.

**Development status:** implementation is in progress. The repository is not yet a finished release. See [implementation status](docs/implementation-status.md) for requirements and verification evidence.

The intended product provides a CLI, an MCP server, and a TUI with keyboard navigation and image previews. Claude Code and Codex integration uses ACP adapters. Rendering uses Skia CPU and SkParagraph, with Noto Sans JP as the bundled default font. Chromium is installed separately for browser nodes and WebGL shaders.

The complete scope is recorded in the [design](docs/design.md), [component and layout semantics](docs/pen-semantics.md), [design guidance](docs/design-guidance.md), and [image attachment specification](docs/image-attachments.md).

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

The original chart example combines editable `.pen` layout, Noto Sans JP, and generated geometry. Script nodes use a separate Rust worker with QuickJS-NG 0.16.2 through the pinned `rquickjs` 0.14.0 binding. Headers currently support `@schema 2.21` and six typed input kinds. The engine exposes `pencil.width`, `pencil.height`, and `pencil.input`; output is measured and painted by the same Skia path as ordinary nodes. Generated children remain derived data and carry `generated_by` metadata.

Use `inspect design.pen --calculated --node chart --depth 2` to read generated properties and their measured geometry. MCP `read_nodes` accepts `calculated: true` for a new calculation or `snapshot_id` to reuse the exact tree from layout/render without rerunning scripts. These reads expose resolved variables, themes, and property origins; generated IDs remain read-only. Ordinary stored reads do not execute scripts. See [calculated node reads](docs/mcp.md#calculated-and-generated-nodes).

Header and JavaScript failures report original-source positions and bounded excerpts when available. JSON/MCP results keep these in `error.details.script_diagnostic`, with the captured source hash, path, and owning address. Readable errors also show the location and source lines. Columns count UTF-8 bytes; dynamic-code call sites and unavailable positions are explicit. See [script execution and diagnostics](docs/mcp.md#script-execution).

The interpreter has a 2 second execution limit, a 64 MiB heap limit, and a 1,000 node output limit. A parent watchdog and MCP cancellation terminate a worker that does not finish. Each request gets a fresh JS context, while MCP and TUI reuse the worker process and reap it after 60 seconds without execution. Scripts have no host filesystem, network, DOM, module loader, or timer APIs. Their fixed random sequence has not been compared with Pencil. Earlier script header versions, a complete compatibility audit, configurable policy, and all-platform verification remain unfinished.

Files discovered only in generated content appear in `runtime_dependencies`, separately from the document-declared `dependency_revision`. A retained calculation snapshot captures and checks both sets before rendering. After reviewing a calculation, pass its `runtime_dependency_revision` to `apply` as `--expected-runtime-dependency-revision` to check those files before saving as well. This re-evaluates scripts before taking save locks and checks captured files again before replacing the document. Replaying a committed request does not re-evaluate scripts. See [dynamic execution](docs/dynamic-content.md) for current limits and remaining requirements.

## Browser pages and GLSL

```sh
cargo run -- runtime setup chromium
cargo run -- runtime status
cargo run -- render examples/dynamic-panel.pen --time 0.5 --output dynamic-panel.png
```

The setup command explicitly downloads a pinned Chrome for Testing headless shell and verifies its archive and installed files. Rendering never downloads a runtime. [Runtime installation](docs/runtimes.md) covers offline archives, custom executables, platforms, licensing, and testing.

Browser nodes capture HTTP(S) or HTML data pages with dimensions, zoom, scroll, and basic desktop/phone/tablet modes. Shader fills and effects execute fragment GLSL through a separate software WebGL process, with numeric, color, vector, and image uniforms. Effects can sample the object and previously painted background. `--time` defaults to zero; `--refresh` requests fresh page acquisitions. MCP retains a bounded raster cache and returns acquisition metadata with the actual runtime, inputs, and image hashes.

The original dynamic-panel example combines a local HTML data page, GLSL, and native Japanese text without external page resources. The [uniform study](examples/uniform-study.pen) reuses one shader with flat color arrays, grouped float declarations, and column-major matrices. With Chromium installed, render it using `cargo run -- render examples/uniform-study.pen --output uniform-study.png`. Numeric arrays and GLSL constant-expression dimensions are checked against the active linked WebGL inputs; defaults remain transient zero values. See [declaration rules and limits](docs/dynamic-content.md#宣言と数値入力).

Page and shader processes expire independently after 60 idle seconds; cached images remain available, and explicit refresh can restart the processes. Basic dynamic TUI preview, idle cleanup, and refresh have passed a real PTY test on macOS arm64. Complete Pencil device presets, annotation defaults, sampler arrays and full declaration/vertex compatibility, fill-stage object sampling, persistent authenticated page contexts, configurable capture policy, animation controls, and all-platform verification remain unfinished. These rendering paths do not establish full Pencil compatibility.

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

The CI workflow is configured to build **from verified source archives** using Rust 1.94 on Linux x64/arm64, macOS Intel/arm64, and Windows x64. It checks both feature configurations, formatting, Clippy, source archive integrity, and release-profile renders of the mesh example, a 1,000-node Japanese/component/image dashboard, and the graphics study with a stroke-only decoration comparison. Hosted runners also explicitly prepare the three checksum-pinned original Material fonts and run the native all-name/continuous-weight test and the real MCP weight/snapshot test. Downloaded fonts stay in runner temporary storage and are not added to the source distribution. Workflow syntax is checked with actionlint; a configured job is only platform evidence after it actually executes successfully. Chromium-dependent opt-ins, real subscriptions, and physical clipboards/terminals still require their separately recorded environments. Initial Linux release timing and memory observations, including a parent-image copy correction, are recorded in [performance observations](docs/performance.md).

CI runs for pushes to `main`, pull requests and manual dispatches. It caches Cargo downloads, keeps compilation outputs in the hosted runner's temporary directory, and retains test/release logs and rendered examples as per-platform artifacts for three days. Source artifacts use the same retention period. Inspect results with `gh run view RUN_ID --log-failed`; download only the required platform's artifact, rather than all platforms' output. CI execution requires the workflow and implementation to be committed and pushed; local workflow linting alone does not run GitHub Actions.

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

The cache must contain the original archives named by the standard library's `library/Cargo.lock`, which can differ from the application's lockfile. Registry mode works with a `rust-src` tree without `library/vendor`; installed sources and root distribution legal files still need to be present or supplied with the path overrides. Documentation is detected under `share/doc/rustc` or `share/doc/rust`. Collection has been exercised with Homebrew Rust 1.99.1 on macOS and the official Rust 1.94.0 sources on Linux arm64. A minimal rustup installation may need `rust-src` and separately prepared `COPYRIGHT` / MIT / Apache inputs through `--source-root` and `--legal-root`.

The inventory retains original notices, source comment candidates, publisher manifests, and hashes of the installed library sources and target libraries. Present vendor files are checked against `.cargo-checksum.json`, package identities and declared archive checksums against the library lock. With `--cargo-home`, it verifies the original archive bytes against that lock, hashes every regular source member, and compares present vendor sources with those originals. Differences remain explicit findings with missing/extra/changed paths. Without `--cargo-home`, the original vendor-only mode remains available for a complete vendor tree and reports `registry_archives_verified: false`.

The [pinned toolchain inputs](resources/licenses/toolchain/upstream.json) supply omitted workspace notices for `fortanix-sgx-abi`, `vex-sdk`, and `wasip1`, bound to the original archive, published VCS commit, repository, and Git blob. Published dirty VCS metadata remains an explicit finding. If a copyright document names terms absent from the installation, only matching, pinned SPDX reference terms can fill that document's missing reference. Reference text is labeled separately from package copyright notices; placeholders are not filled with inferred authors. Missing unreviewed terms fail explicitly. The standalone integrity check runs in CI; package applicability is checked against the registry archives during collection.

An MPL-2.0 registry package also retains its complete, unmodified `.crate` under `sources/`, with its path, checksum, license, and source URL in the readable notice and inventory. Keep those sources together with their notices when assembling the eventual distribution. The inventory includes optional/test/platform dependencies and all installed target libraries, so it does not establish exact application linkage or source-to-binary provenance. Keep its findings for the final binary review. These tools run during release preparation; they add no application startup or rendering work.

## License

MIT. Bundled third-party assets and dependencies retain their respective licenses.
