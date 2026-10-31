# Changelog

## Unreleased

- Accept legacy vertical text alignment `center` as an alias of `middle` in layout and native/backdrop rendering, preserving the stored value.
- Load large macOS system emoji fonts within a separate bounded capture budget, defer system fallbacks until needed, and reuse captures when requested styles select the same installed face.
- Accept `normal` and `bold` text font weights, including string variables, without rewriting document values.
- Prevent repeated preview failures from flooding Conversation or repainting the terminal; show the actual diagnostic in Preview and list supported versions for unsupported documents.
- Use `$XDG_CONFIG_HOME/lazypencil/config.toml` for user configuration on macOS and Linux, falling back to `~/.config/lazypencil/config.toml`. The former macOS configuration location is no longer read or automatically migrated. `LAZYPENCIL_CONFIG_DIR` remains the highest-priority override.

## 0.1.0

Initial release for macOS 15.7.9 or later on Apple Silicon.

- Read and edit `.pen` 2.19 and 2.21 documents through the CLI, MCP server and TUI.
- Preserve comments, unknown fields and unrelated document content during edits.
- Render with Skia CPU and SkParagraph, using bundled Noto Sans JP and optional user fonts.
- Execute JavaScript-generated designs, capture browser nodes and render GLSL fills/effects with a separately installed Chromium runtime.
- Use Codex and Claude subscriptions through ACP adapters, with image attachments and shared design guidance.
- Browse previews with Kitty or Sixel, use Japanese input and Emacs-style prompt editing, and paste images on macOS.
- Generate Bash, Zsh and Fish completion hooks from the installed CLI.
- Include original dependency notices, matching project source and build provenance with binary distributions.

Known compatibility limitations:

- Pencil-specific browser device presets are unsupported. Omitted `deviceId` and basic `desktop`/`phone`/`tablet` modes are supported; they do not select a specific device model or User-Agent.
- Shader fills cannot sample `pencil:object`. Object sampling in shader effects is supported.

Unsupported settings are preserved in the document and produce explicit rendering errors. The release does not claim complete Pencil compatibility. Linux, Windows and Intel Mac are not initial release targets.
