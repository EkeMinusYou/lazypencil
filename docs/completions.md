# Shell completion

Install `lazypencil` on `PATH`, then enable its completion hook in your shell.
Press Tab to complete commands, options, file paths and built-in design resources.

## Zsh

Place these lines in `~/.zshrc`, after any existing completion initialization:

```zsh
# Omit these two lines if your shell framework already initializes completion.
autoload -Uz compinit
compinit
source <(lazypencil completions zsh)
```

Open a new shell, or run the same lines in the current shell.

## Bash

Add this line to the startup file your Bash session reads, such as `~/.bashrc`
or `~/.bash_profile` for a macOS login shell:

```bash
source <(lazypencil completions bash)
```

The hook supports macOS's system Bash as well as newer Bash versions. It does
not require the separate bash-completion package.

## Fish

Create `~/.config/fish/completions/lazypencil.fish` with this content (use your
configured Fish configuration directory if it differs):

```fish
lazypencil completions fish | source
```

To enable it immediately in an existing session, run that line directly.

## Completion behavior

Examples below show the text to enter before pressing Tab:

```text
lazypencil ren
lazypencil runtime profile login --br
lazypencil prompt design.pen --agent c
lazypencil assets put design.pen --kind sh
lazypencil inspect design.pen --type fr
lazypencil icons setup M
lazypencil guidance sh
lazypencil style
lazypencil render "design assets/日本語"
```

Completion covers the command hierarchy, visible flags and aliases, declared
enum values, bundled guides/styles/icon packs/node types, and paths. Directory
arguments offer directories; file paths preserve spaces and Unicode. Free-form
values, including document node IDs and adapter-advertised model IDs, remain
free-form.

The hook resolves `lazypencil` through `PATH`. A binary in a directory containing
spaces or Unicode works when that directory is on `PATH`. When testing a
different executable, update `PATH` to select it before generating/loading the
hook.

Keep the hook in your shell startup file so it uses the current executable after
an upgrade. `lazypencil completions SHELL --json` returns the shell name and hook
text for tools that manage shell configuration.

## Troubleshooting

If completion stops working after an upgrade, open a new shell. Check that
`command -v lazypencil` selects the executable you intend to use. If you manage
completion settings yourself, avoid exporting `LAZYPENCIL_COMPLETE` or
`_CLAP_COMPLETE_*`: the hook sets them only for its completion requests.

To disable completion, remove the startup hook and open a new shell.
For completion development and tests, see [development](development.md).
