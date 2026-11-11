# lazypencil

**Design interfaces with AI, from your terminal.**

Create and compare UI ideas before asking your agent to implement them. Keep the
editable design alongside your code, use your existing AI client through MCP, or
work in the terminal workspace.

[日本語ではじめる](docs/getting-started.ja.md) ·
[Example gallery](examples/README.md) · [Install](#install) ·
[Choose your workflow](#quick-start)

## Demo

[![Recorded light-mode TUI session showing node selection and fitting, preview panning and zooming, and a successful agent edit reflected in the preview.](docs/images/lazypencil-demo.gif)](docs/media/lazypencil-demo.mp4)

[Watch or download the MP4](docs/media/lazypencil-demo.mp4).

Select a node to fit it in Preview, pan and zoom, then ask Codex to recolor the
visible project cards and watch the changes appear. Recorded in Herdr with the
light UI theme; agent waiting time is shortened.

lazypencil is an open-source editor for `.pen` files, the design document format
used by Pencil. Describe a screen or a change to Codex or Claude, then review the
result in a terminal preview. Your design stays in a local, editable file.

<details>
<summary>Screenshot and editable example</summary>

![lazypencil running in Herdr, with the node tree, a project dashboard preview, and an AI conversation side by side.](docs/images/lazypencil-tui.png)

The terminal workspace in use, showing the editable
[project dashboard example](examples/project-dashboard.pen).

</details>

- **Create and refine designs:** start from a prompt, edit an existing document,
  or attach a reference image.
- **Review as you work:** browse the node tree, pan and zoom the preview, check
  layout diagnostics, and restore checkpoints.
- **Use your preferred workflow:** chat inside lazypencil, connect your own AI
  client through MCP, or inspect and render files from the command line.
- **Render locally:** export PNGs with Skia, including Japanese text with bundled
  Noto Sans JP fonts.

[Install](#install) · [Quick start](#quick-start) · [Demo](#demo) ·
[Features](#features) · [Nodes](#nodes) · [Preview](#preview) ·
[MCP](#mcp-and-ai-prompts) · [CLI](#cli) ·
[Doctor](#check-your-setup) ·
[Documentation](#documentation-and-development)

## Features

### Nodes

Browse the tree, search by name with `/`, and press `f` or Enter to fit a selected
element in Preview.

[![Browse and search the node tree in light mode, then display the selected elements in Preview.](docs/images/lazypencil-nodes.gif)](docs/media/lazypencil-nodes.mp4)

[Watch or download the Nodes video](docs/media/lazypencil-nodes.mp4).

### Preview

Use the arrow keys to pan, `+` / `-` to zoom, and `0` to fit the whole design.
Camera controls change the view while keeping the design itself in place.

[![Pan and zoom the design preview in light mode, then fit the entire dashboard.](docs/images/lazypencil-preview.gif)](docs/media/lazypencil-preview.mp4)

[Watch or download the Preview video](docs/media/lazypencil-preview.mp4).

## Supported environment

**macOS Tahoe 26 or later on Apple Silicon.** For image previews, use direct local
Kitty 0.49.2 (Kitty Graphics Protocol) or WezTerm `20240203-110809-5046fc22` (Sixel),
the initial terminal baselines. Other terminals, input methods, tmux, and SSH need
separate compatibility checks. Linux, Windows, and Intel Mac are not supported
release targets. See [terminal support and fallbacks](docs/tui.md#initial-terminal-support).

## Install

Install or update from the dedicated
[Homebrew tap](https://github.com/EkeMinusYou/homebrew-lazypencil):

```sh
brew install --cask ekeminusyou/lazypencil/lazypencil
brew upgrade --cask lazypencil
```

For manual installation:

Download `lazypencil-0.2.5-aarch64-apple-darwin.tar.gz` from
[GitHub Releases](https://github.com/EkeMinusYou/lazypencil/releases).
See [provenance verification](docs/development.md#distribution-and-build-provenance)
for checking the download, then extract it:

```sh
tar -xzf lazypencil-0.2.5-aarch64-apple-darwin.tar.gz
export PATH="$PWD/lazypencil-0.2.5-aarch64-apple-darwin/bin:$PATH"
lazypencil --version
lazypencil --json doctor
```

Add the extracted `bin` directory to your shell startup configuration to keep it
on `PATH`. Keep the complete extracted directory, including its source and license
materials. The prebuilt package needs no Rust toolchain.

To build the current source instead, follow [Build from source](docs/development.md#build-from-source).

### Check your setup

```sh
lazypencil doctor
```

Doctor checks the supported OS/CPU and macOS version, user configuration, data/lock
directory access, configured font-file availability and loading,
the optional Chromium installation, and installed Codex/Claude ACP adapters. It starts
adapters to check ACP connectivity, reported versions, login state and mid-turn input
support. It also renders and verifies a small test image; interactive text output
automatically tests terminal image transport without asking for confirmation.
It never sends a model prompt, creates an agent session, starts a login flow,
installs dependencies or edits a design. Each problem includes a suggested next step.

Use `lazypencil doctor --agent codex` (or `claude`) to check one agent,
`--timeout-seconds 10` for a slower adapter startup, and `--json` for structured output.
Doctor prints the complete report even when a dependency is missing. Add `--strict`
to exit with status 1 if it reports errors; warnings about optional features do not fail.
See the [doctor reference](docs/doctor.md) for check coverage and preview limitations.

## Quick start

Choose the route that matches your setup. All routes use the same local `.pen`
files; AI usage and authentication belong to your chosen provider/client.

| What you want to try | Start here | What you need |
| --- | --- | --- |
| See a finished design and export a PNG | [Try without AI](#try-without-ai) | lazypencil; no AI login or terminal image support |
| Design with an AI client you already use | [Connect through MCP](#use-lazypencil-from-your-ai-client) | An AI client with MCP support; no ACP adapter |
| Chat and review the design in one terminal | [Use the TUI](#use-the-terminal-workspace) | A supported terminal and a Codex or Claude ACP adapter |

### Try without AI

First get an editable dashboard as `demo.pen` in a new working directory:

- **If your build lists `demo` in `lazypencil --help`:** run `lazypencil demo`.
  This command is new in the current source.
- **For releases without `demo` (including 0.2.5):** download the
  [raw dashboard example](https://raw.githubusercontent.com/EkeMinusYou/lazypencil/main/examples/project-dashboard.pen)
  and save it as `demo.pen`.

`demo` creates `demo.pen` in the current directory and refuses to overwrite an
existing file. It needs no repository checkout, AI connection, or downloads at run
time. Use `lazypencil demo another-name.pen` to choose another destination in an
existing directory. PNG export uses the local renderer and bundled fonts.

Then render the file and open the image:

```sh
lazypencil render demo.pen --output demo.png
open demo.png
```

To explore it in a supported terminal, run `lazypencil tui demo.pen`. To edit it
with your existing AI client, [connect MCP](#use-lazypencil-from-your-ai-client)
and ask it to open the file's absolute path. The
[example gallery](examples/README.md) includes screenshots, editable files and
prompts for a dashboard, Japanese settings screen and content page.

### Use the terminal workspace

#### 1. Connect an agent

Install one of the separate [Codex or Claude ACP adapters](docs/agents.md#install-and-authenticate-an-adapter):

```sh
# Codex
npm install -g @agentclientprotocol/codex-acp@latest

# Or Claude
npm install -g @agentclientprotocol/claude-agent-acp@latest
```

Authenticate using the selected runtime's instructions, then run `lazypencil doctor`.
The adapter bridges lazypencil and your AI runtime: Codex uses `codex-acp`, and
Claude uses `claude-agent-acp`. Put the selected executable on `PATH` before
starting. Authentication stays with the external runtime.

#### 2. Create your first design

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

#### 3. Continue an existing design

```sh
lazypencil tui new.pen --agent codex
```

Attach a reference with `--image reference.png` at launch, or use `/attach` inside
the TUI. Type `/` to complete conversation commands: `/logs` opens the current
conversation log and `/conversations` browses saved conversations. The command palette
(`Alt+P`) lists all workspace actions, including checkpoint review and restoration.
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

lazypencil reads and edits `.pen` versions **2.19, 2.21, and 2.22**, preserving comments
and unrelated content. New empty documents use **2.22**; existing documents,
copied examples and imported user libraries keep their versions. It implements its
own renderer and does not claim complete Pencil compatibility. See [document semantics](docs/pen-semantics.md),
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
| Get started in Japanese | [日本語の導入ガイド](docs/getting-started.ja.md) |
| Explore and reuse finished screens | [Example gallery](examples/README.md) |
| Use the terminal workspace | [TUI](docs/tui.md), [image attachments](docs/image-attachments.md) |
| Configure agents and preferences | [Agent setup](docs/agents.md), [configuration](docs/configuration.md), [shell completion](docs/completions.md) |
| Automate document editing | [MCP and editing reference](docs/mcp.md) |
| Browse, restore or prune editing history | [Checkpoints and retention](docs/checkpoints.md) |
| Understand document rendering | [`.pen` semantics](docs/pen-semantics.md), [graphics](docs/graphics.md), [fonts](docs/fonts.md), [icons](docs/icons.md) |
| Use scripts, browser nodes, or shaders | [Dynamic content](docs/dynamic-content.md), [runtimes](docs/runtimes.md) |
| Build, test, or contribute | [Development](docs/development.md), [architecture](docs/design.md) |
| Measure performance | [Performance checks](docs/performance.md), [timing fields](docs/performance-logging.md) |

Bug reports and contributions are welcome. Include your version, macOS and terminal
details, reproduction steps, and a minimal `.pen` file when possible in
[an issue](https://github.com/EkeMinusYou/lazypencil/issues/new/choose). Setup-help
and environment-request forms are available there too. Read the development
guide before opening a pull request.

## License

[MIT](LICENSE). Bundled third-party code, fonts, icons, and separately installed
runtimes retain their own licenses and attribution. See
[redistribution requirements](docs/development.md#licenses-and-third-party-inputs).
