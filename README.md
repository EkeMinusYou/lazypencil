# lazypencil

A terminal-first editor, renderer, and MCP server for `.pen` design files, written in Rust.

- Read and edit 2.19 and 2.21 documents while preserving comments and unrelated content.
- Render native graphics and Japanese text with Skia CPU, SkParagraph, and bundled Noto Sans JP.
- Run script-generated designs, browser nodes, and GLSL fills/effects.
- Use Codex or Claude through ACP, or connect an external agent through MCP.
- Browse previews, attach images, and manage conversations from the terminal.

## Supported environment

The released platform is **macOS 15.7.9 or later on Apple Silicon**. The initial
terminal baseline is direct local Kitty 0.49.2 with Kitty Graphics Protocol and
WezTerm `20240203-110809-5046fc22` with Sixel. Other terminal/input-method combinations,
tmux, and SSH require their own compatibility checks. Linux and Windows code is
retained for future support; Intel Mac is unsupported.

Pencil-specific browser device presets and shader-fill `pencil:object` sampling
are unsupported. Basic `desktop`/`phone`/`tablet` modes and object sampling in
shader effects are supported. Unsupported combinations produce explicit errors.
The project does not claim complete Pencil compatibility; see
[dynamic-content limitations](docs/dynamic-content.md#initial-compatibility-limitations).

## Install

Download the Apple Silicon archive from [GitHub Releases](https://github.com/EkeMinusYou/lazypencil/releases).
Verify its [build provenance](docs/development.md#distribution-and-build-provenance),
then extract the complete directory:

```sh
tar -xzf lazypencil-0.1.0-aarch64-apple-darwin.tar.gz
export PATH="$PWD/lazypencil-0.1.0-aarch64-apple-darwin/bin:$PATH"
lazypencil --version
lazypencil --json doctor
```

Add the extracted `bin` directory to your shell startup configuration. Keep its
matching source, release declaration, and license files together. Basic editing
and native rendering require neither Rust nor Chromium. Browser nodes and GLSL
need `lazypencil runtime setup chromium`; AI prompts need an installed and
authenticated [ACP adapter](docs/agents.md#install-and-authenticate-an-adapter).

The examples below describe the current source. Post-release changes are listed
under **Unreleased** in the [changelog](CHANGELOG.md); an older installed binary
may expose a different interface. Use its `--help` to check available options.

## CLI

```sh
lazypencil inspect examples/welcome.pen
lazypencil validate examples/welcome.pen --scope layout
lazypencil layout examples/welcome.pen
lazypencil render examples/welcome.pen --output welcome.png
```

`--json` returns structured results/errors; `prompt --json` streams NDJSON.
`layout` reports measured geometry, text metrics, and diagnostics. Layout or
structural validation complements a rendered visual check; neither certifies
complete graphics compatibility.

Edits use an atomic operation array. Read the current `revision` and
`dependency_revision` with `inspect`, then apply a file such as:

```json
[{"op":"update","target":"heading","set":{"content":"My project"}}]
```

```sh
lazypencil apply design.pen --operations edits.json \
  --expected-revision REVISION --expected-dependency-revision DEPENDENCY_REVISION \
  --request-id edit-1
```

`--dry-run` validates without saving. Retry an ambiguous result with the same
request ID and payload. `history`, `restore`, and `recover` expose checkpoints and
interrupted-save recovery. See the [editing and query reference](docs/mcp.md).
Editable examples for graphics, meshes, icons, scripts, browser nodes, and shaders
are in [examples](examples/).

## Configuration

```sh
lazypencil config init --user
lazypencil config show --user
lazypencil config init
lazypencil config show design.pen
```

On macOS/Linux, user settings use `$XDG_CONFIG_HOME/lazypencil/config.toml`, falling
back to `~/.config/lazypencil/config.toml`. `LAZYPENCIL_CONFIG_DIR` overrides the
directory. The former macOS location is no longer read or migrated. Project
`lazypencil.toml` overrides individual fields; arrays replace inherited arrays.
Project design instructions and executable settings require separate user grants.
See [configuration](docs/configuration.md) for the complete settings and trust rules.

Enable Bash, Zsh, or Fish Tab completion with `lazypencil completions SHELL`; see
[shell setup](docs/completions.md).

## MCP and AI prompts

Register one file-independent MCP server per client:

```sh
codex mcp add lazypencil -- lazypencil mcp
claude mcp add --scope user --transport stdio lazypencil -- lazypencil mcp
```

Use an absolute executable path if the client cannot resolve your `PATH`. Ask the
agent to open or create an absolute `.pen` path. The server returns a connection-local
`document_id`, required by subsequent document tools. `mcp --file` is no longer
accepted. An external MCP client supplies its own AI and needs no ACP adapter.

For lazypencil's own conversations, install/authenticate an [ACP adapter](docs/agents.md):

```sh
lazypencil prompt new.pen --create --agent codex "日本語の設定画面を作って"
lazypencil prompt new.pen --agent claude --image reference.png "Improve this design"
```

`--model`, `--mode`, `--auth`, and `--session` control the connection. Credentials
stay with the external runtime. Document-tool approval does not authorize arbitrary
shell operations. Image attachments are reference inputs; importing them into the
design is a separate edit. Built-in English guides and three editable styles are
available through `guidance`, `styles`, and `style`; they preserve the user's language.

## Terminal UI

```sh
lazypencil
lazypencil tui design.pen
lazypencil tui new.pen --create --agent claude --image reference.png
```

With no file, a bounded recursive picker opens over the empty workspace. Adapter
transport preparation runs in the background; document sessions and previews start
after selection. A bare file argument such as `lazypencil design.pen` is unsupported.

Use Tab to switch panels, Enter or Ctrl+S to submit, Alt+Enter for a newline,
Ctrl+V to paste, and `/attach <quoted path>` for image files. Preview supports
pan/zoom/fit; `/history` browses checkpoints and `/logs` browses saved conversations.
The prompt stays editable during a turn and queues text follow-ups. F1 shows the
effective keys. See [TUI controls](docs/tui.md) and [image attachments](docs/image-attachments.md).

## Documentation and development

| Topic | Reference |
| --- | --- |
| Build, contribute, test, and package | [Development](docs/development.md) |
| Core architecture and invariants | [Design](docs/design.md) |
| Components, variables, and layout | [`.pen` semantics](docs/pen-semantics.md) |
| MCP tools, editing, assets, and snapshots | [MCP reference](docs/mcp.md) |
| Configuration, profiles, and trust | [Configuration](docs/configuration.md) |
| Agent connections and saved conversations | [ACP agents](docs/agents.md) |
| Terminal controls and protocols | [TUI](docs/tui.md) |
| Fonts and reproducibility | [Fonts](docs/fonts.md) |
| Native paints and compositing | [Graphics](docs/graphics.md) |
| Icon packs and provenance | [Icons](docs/icons.md) |
| Scripts, browser pages, and GLSL | [Dynamic content](docs/dynamic-content.md) |
| Chromium installation and profiles | [Runtimes](docs/runtimes.md) |
| Reproducible performance checks | [Performance](docs/performance.md) |
| Conversation/tool timing fields | [Timing](docs/performance-logging.md) |

## License

[MIT](LICENSE). Bundled third-party code, fonts, icons, and separately installed
runtimes retain their own licenses and attribution. Keep the original notices and
corresponding-source materials when redistributing binaries; see
[distribution requirements](docs/development.md#licenses-and-third-party-inputs).
