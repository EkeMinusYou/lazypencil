//! Exercise completion hooks without loading a document, configuration, or agent.
#[path = "../xtask/src/test_support/command_output.rs"]
#[allow(dead_code)]
mod command_output;

use std::{fs, process::Command, time::Duration};
use tempfile::TempDir;

fn cli(temp: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .current_dir(temp.path())
        .env_remove("LAZYPENCIL_COMPLETE")
        .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("configuration"))
        .env("LAZYPENCIL_DATA_DIR", temp.path().join("untouched-data"))
        .env("LAZYPENCIL_LOCK_DIR", temp.path().join("untouched-locks"));
    command
}

fn checked(command: &mut Command) -> String {
    let output = command_output::output(command, Duration::from_secs(20), 4 * 1024 * 1024).unwrap();
    assert!(!output.timed_out, "Completion request timed out");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn candidates(temp: &TempDir, shell: &str, words: &[&str]) -> Vec<String> {
    let output = checked(
        cli(temp)
            .env("LAZYPENCIL_COMPLETE", shell)
            .env("_CLAP_COMPLETE_INDEX", (words.len() - 1).to_string())
            .env("_CLAP_IFS", "\n")
            .args(["--"])
            .args(words),
    );
    output
        .lines()
        .map(|line| {
            match shell {
                "fish" => line.split('\t').next().unwrap(),
                "zsh" => line.split(':').next().unwrap(),
                _ => line,
            }
            .to_owned()
        })
        .collect()
}

#[test]
fn completion_generation_is_clean_and_queries_cover_nested_commands_values_and_catalogues() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("lazypencil.toml"), "invalid project = [").unwrap();
    fs::create_dir(temp.path().join("configuration")).unwrap();
    fs::write(
        temp.path().join("configuration/config.toml"),
        "invalid user = [",
    )
    .unwrap();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let script = checked(cli(&temp).args(["completions", shell]));
        assert!(script.contains("LAZYPENCIL_COMPLETE"));
        assert!(script.contains("lazypencil"));
        assert!(!script.contains("\"script\":"));
        let encoded = checked(cli(&temp).args(["--json", "completions", shell]));
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["shell"], shell);
        assert_eq!(
            value["script"].as_str().unwrap().trim_end(),
            script.trim_end()
        );
    }
    for shell in ["bash", "zsh", "fish"] {
        for (words, expected) in [
            (vec!["lazypencil", "ren"], vec!["render"]),
            (
                vec!["lazypencil", "runtime", "profile", "lo"],
                vec!["login"],
            ),
            (
                vec!["lazypencil", "runtime", "profile", "login", "--br"],
                vec!["--browser"],
            ),
            (
                vec!["lazypencil", "prompt", "absent.pen", "--agent", ""],
                vec!["claude", "codex"],
            ),
            (
                vec!["lazypencil", "assets", "put", "absent.pen", "--kind", ""],
                vec!["image", "script", "shader"],
            ),
            (
                vec!["lazypencil", "icons", "setup", "M"],
                vec![
                    "Material Symbols Outlined",
                    "Material Symbols Rounded",
                    "Material Symbols Sharp",
                ],
            ),
            (vec!["lazypencil", "guidance", "sh"], vec!["shaders"]),
            (
                vec!["lazypencil", "inspect", "absent.pen", "--type", ""],
                vec!["frame", "text", "browser", "script"],
            ),
        ] {
            let actual = candidates(&temp, shell, &words);
            for expected in expected {
                assert!(
                    actual.iter().any(|candidate| candidate == expected),
                    "{shell}: {words:?}: {actual:?}"
                );
            }
        }
        let styles: serde_json::Value =
            serde_json::from_str(include_str!("../resources/styles/manifest.json")).unwrap();
        let suggestions = candidates(&temp, shell, &["lazypencil", "style", ""]);
        for style in styles["styles"].as_array().unwrap() {
            assert!(
                suggestions
                    .iter()
                    .any(|candidate| Some(candidate.as_str()) == style["id"].as_str())
            );
        }
        let hidden = candidates(&temp, shell, &["lazypencil", "mcp", "--"]);
        assert!(
            !hidden
                .iter()
                .any(|value| value.contains("attachments-manifest")
                    || value.contains("guidance-audit-manifest"))
        );
        #[cfg(feature = "tui")]
        assert!(
            candidates(
                &temp,
                shell,
                &["lazypencil", "tui", "absent.pen", "--image-protocol", "si"]
            )
            .contains(&"sixel".to_owned())
        );
    }
    assert!(!temp.path().join("absent.pen").exists());
    assert!(!temp.path().join("untouched-data").exists());
    assert!(!temp.path().join("untouched-locks").exists());
}

#[test]
fn completion_paths_preserve_spaces_unicode_and_directory_hints() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("design assets")).unwrap();
    fs::write(
        temp.path().join("design assets/日本語 sample.pen"),
        "not a document",
    )
    .unwrap();
    fs::create_dir(temp.path().join("project folder")).unwrap();
    fs::write(temp.path().join("project file"), "not a directory").unwrap();
    for shell in ["bash", "zsh", "fish"] {
        assert!(
            candidates(&temp, shell, &["lazypencil", "inspect", "design assets/日"])
                .contains(&"design assets/日本語 sample.pen".to_owned())
        );
        let directories = candidates(&temp, shell, &["lazypencil", "--data-dir", "project"]);
        assert!(
            directories.contains(&"project folder/".to_owned()),
            "{shell}: {directories:?}"
        );
        assert!(!directories.contains(&"project file".to_owned()));
    }
}

#[cfg(unix)]
#[test]
fn generated_hooks_register_and_execute_in_bash_zsh_and_fish() {
    let temp = TempDir::new().unwrap();
    let bin = temp.path().join("binary 日本語 directory");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_lazypencil"), bin.join("lazypencil")).unwrap();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    for (shell, program, script) in [
        (
            "bash",
            "/bin/bash",
            r#"source "$1"; shift; COMP_WORDS=("$@"); COMP_CWORD=$((${#COMP_WORDS[@]}-1)); COMP_TYPE=9; _clap_complete_lazypencil lazypencil "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD-1]}"; printf '%s\n' "${COMPREPLY[@]}""#,
        ),
        (
            "zsh",
            "/bin/zsh",
            // Only ZLE's final display is replaced; registration and the real
            // executable's completion query both run in the isolated shell.
            r#"autoload -Uz compinit; compinit -D; source "$1"; shift; [[ ${_comps[lazypencil]} == _clap_dynamic_completer_lazypencil ]] || exit 3; _describe() { print -rl -- "${other[@]%%:*}" "${dirs[@]%%:*}"; }; words=("$@"); CURRENT=${#words}; _clap_dynamic_completer_lazypencil"#,
        ),
        (
            "fish",
            "fish",
            r#"source $argv[1]; complete --do-complete 'lazypencil runtime profile login --br'"#,
        ),
    ] {
        let hook = temp.path().join(format!("completion.{shell}"));
        fs::write(&hook, checked(cli(&temp).args(["completions", shell]))).unwrap();
        let mut command = Command::new(program);
        if shell == "bash" {
            command.args(["--noprofile", "--norc"]);
        } else if shell == "zsh" {
            command.arg("-f");
        } else {
            command.arg("--no-config");
        }
        command
            .current_dir(temp.path())
            .env("PATH", &path)
            .args(["-c", script, "--"])
            .arg(&hook)
            .args(["lazypencil", "runtime", "profile", "login", "--br"]);
        let result = checked(&mut command);
        assert!(
            result
                .lines()
                .any(|line| line.split('\t').next() == Some("--browser")),
            "{shell}: {result}"
        );
    }
}
