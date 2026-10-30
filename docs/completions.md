# Shell completion

Install `lazypencil` on `PATH`, then enable its completion hook in your shell.
Hooks use the installed executable's CLI definition, so nested subcommands,
options and their values stay aligned with the installed build. TUI panel keys
are independent of shell completion.

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
free-form. Generating hooks or requesting candidates does not open a document,
load project/user configuration, start an agent/Chromium, acquire document
locks, or create the application data directory.

The hook resolves `lazypencil` through `PATH`. A binary in a directory containing
spaces or Unicode works when that directory is on `PATH`. When testing a
different executable, update `PATH` to select it before generating/loading the
hook.

Generate the hook on shell startup as shown above. Completion uses the pinned
clap dynamic protocol, which upstream marks unstable; refreshing the hook on
startup keeps it matched with an upgraded executable. `lazypencil completions
SHELL --json` returns the shell name and hook text for tooling. Upstream also
provides `powershell` and `elvish` hook generators; the initial Mac interactive
checks focus on Bash, Zsh and Fish.

`LAZYPENCIL_COMPLETE` and `_CLAP_COMPLETE_*` are internal, per-request variables
used by the hooks. Do not export them globally. To disable completion, remove
the startup hook and open a new shell.

## Verification

`tests/completions.rs` checks generation/JSON output, the completion protocol for
all three primary shells, deep command nesting, values from the owned bundled
catalogues, hidden-argument exclusion and paths with spaces/Unicode. Separate
shell processes source the generated hooks and call the real executable; Zsh's
ZLE-only final presentation is replaced by a capture function in that test.
Every process has a deadline and bounded captured output. Validation results
are recorded in the existing [implementation status](implementation-status.md).
