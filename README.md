# lazypencil

**Design interfaces with AI, from your terminal.**

lazypencil is an open-source editor for `.pen` files, the design document format
used by Pencil. Describe a screen or a change to Codex or Claude, then review the
result in a terminal preview. Your design stays in a local, editable file.

![lazypencil running in Herdr, with the node tree, a project dashboard preview, and an AI conversation side by side.](docs/images/lazypencil-tui.png)

The terminal workspace in use, showing the editable
[project dashboard example](examples/project-dashboard.pen).

- **Create and refine designs:** start from a prompt, edit an existing document,
  or attach a reference image.
- **Review as you work:** browse the node tree, pan and zoom the preview, check
  layout diagnostics, and restore checkpoints.
- **Use your preferred workflow:** chat inside lazypencil, connect your own AI
  client through MCP, or inspect and render files from the command line.
- **Render locally:** export PNGs with Skia, including Japanese text with bundled
  Noto Sans JP fonts.

[Install](#install) · [Quick start](#quick-start) ·
[MCP](#mcp-and-ai-prompts) · [CLI](#cli) ·
[Documentation](#documentation-and-development)

## Supported environment

**macOS 15.7.9 or later on Apple Silicon.** For image previews, use direct local
Kitty 0.49.2 (Kitty Graphics Protocol) or WezTerm `20240203-110809-5046fc22` (Sixel),
the initial terminal baselines. Other terminals, input methods, tmux, and SSH need
separate compatibility checks. Linux, Windows, and Intel Mac are not supported
release targets. See [terminal support and fallbacks](docs/tui.md#initial-terminal-support).

## Install

Download `lazypencil-0.1.0-aarch64-apple-darwin.tar.gz` from
[GitHub Releases](https://github.com/EkeMinusYou/lazypencil/releases).
See [provenance verification](docs/development.md#distribution-and-build-provenance)
for checking the download, then extract it:

```sh
tar -xzf lazypencil-0.1.0-aarch64-apple-darwin.tar.gz
export PATH="$PWD/lazypencil-0.1.0-aarch64-apple-darwin/bin:$PATH"
lazypencil --version
lazypencil --json doctor
```

Add the extracted `bin` directory to your shell startup configuration to keep it
on `PATH`. Keep the complete extracted directory, including its source and license
materials. The prebuilt package needs no Rust toolchain.

To build the current source instead, follow [Build from source](docs/development.md#build-from-source).

> This README describes the current source. Release v0.1.0 has an older MCP
> interface and different default keys. Check the installed binary's `--help`
> and TUI help with F1; changes are listed in the [changelog](CHANGELOG.md).

## Quick start

### 1. Connect an agent

Install and authenticate a [Codex or Claude ACP adapter](docs/agents.md#install-and-authenticate-an-adapter).
The adapter bridges lazypencil and your AI runtime: Codex uses `codex-acp`, and
Claude uses `claude-agent-acp`. Put the selected executable on `PATH` before
starting. Authentication stays with the external runtime.

### 2. Create your first design

```sh
lazypencil tui new.pen --create --agent codex
```

Use `--agent claude` for Claude. In the prompt panel, describe what you want:

```text
Design a settings screen for a note-taking app, with a sidebar and light theme.
```

Submit using the binding shown in the footer. Review the preview, then ask for
changes such as “Make the sidebar narrower and increase the spacing.” Tab moves
between panels; F1 shows the available controls. `--create` creates a new document
and refuses to overwrite an existing file.

### 3. Continue an existing design

```sh
lazypencil tui new.pen --agent codex
```

Attach a reference with `--image reference.png` at launch, or use `/attach` inside
the TUI. `/history` opens checkpoints and `/logs` opens saved conversations.
See [TUI controls](docs/tui.md) and [image attachments](docs/image-attachments.md).

## MCP and AI prompts

### Use lazypencil from your AI client

Connect lazypencil as a **Model Context Protocol (MCP)** server to let your existing
AI client open, edit, validate, and render `.pen` files. This route uses the
client's AI connection and does not need an ACP adapter.

The registration commands below use the current source's MCP interface:

```sh
# Codex
codex mcp add lazypencil -- lazypencil mcp

# Claude Code
claude mcp add --scope user --transport stdio lazypencil -- lazypencil mcp
```

Use an absolute executable path if your client cannot resolve `lazypencil` on
`PATH`. Register once per client, then ask it to create or open an absolute `.pen`
path. For example: “Open /absolute/path/new.pen, improve the spacing, and render
the result.” See the [MCP reference](docs/mcp.md) for tools and editing operations.

### Send a prompt from the command line

With an ACP adapter configured, you can also run a design request without opening
the TUI:

```sh
lazypencil prompt new.pen --agent codex "Improve the spacing and hierarchy"
lazypencil prompt new.pen --agent claude --image reference.png "Use this visual style"
```

See [agent setup and prompt options](docs/agents.md) for models, sessions, and permissions.

## CLI

Inspect, check, or export an existing document without an AI connection:

```sh
lazypencil inspect new.pen
lazypencil validate new.pen --scope layout
lazypencil layout new.pen
lazypencil render new.pen --output new.png
```

Add `--json` for structured output. `layout` reports measured geometry and text
metrics; `render` exports a PNG. For scripted edits, see the
[editing reference](docs/mcp.md). Try the documents in [examples](examples/) to
explore components, graphics, icons, scripts, and shaders.

## Format support

lazypencil reads and edits `.pen` versions **2.19 and 2.21**, preserving comments
and unrelated content. It implements its own renderer and does not claim complete
Pencil compatibility. See [document semantics](docs/pen-semantics.md),
[graphics support](docs/graphics.md), and [dynamic-content limitations](docs/dynamic-content.md#initial-compatibility-limitations).

Basic editing and native rendering need no Chromium. Browser nodes and GLSL
fills/effects use an optional Chromium runtime:

```sh
lazypencil runtime setup chromium
```

See [runtime setup](docs/runtimes.md) before using these features.

## Documentation and development

| What you want to do | Where to start |
| --- | --- |
| Use the terminal workspace | [TUI](docs/tui.md), [image attachments](docs/image-attachments.md) |
| Configure agents and preferences | [Agent setup](docs/agents.md), [configuration](docs/configuration.md), [shell completion](docs/completions.md) |
| Automate document editing | [MCP and editing reference](docs/mcp.md) |
| Understand document rendering | [`.pen` semantics](docs/pen-semantics.md), [graphics](docs/graphics.md), [fonts](docs/fonts.md), [icons](docs/icons.md) |
| Use scripts, browser nodes, or shaders | [Dynamic content](docs/dynamic-content.md), [runtimes](docs/runtimes.md) |
| Build, test, or contribute | [Development](docs/development.md), [architecture](docs/design.md) |
| Measure performance | [Performance checks](docs/performance.md), [timing fields](docs/performance-logging.md) |

Bug reports and contributions are welcome. Include your version, macOS and terminal
details, reproduction steps, and a minimal `.pen` file when possible in
[an issue](https://github.com/EkeMinusYou/lazypencil/issues). Read the development
guide before opening a pull request.

## License

[MIT](LICENSE). Bundled third-party code, fonts, icons, and separately installed
runtimes retain their own licenses and attribution. See
[redistribution requirements](docs/development.md#licenses-and-third-party-inputs).
