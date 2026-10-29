use clap::{CommandFactory, FromArgMatches};
use std::{io::Write, process::ExitCode};

fn main() -> ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__http_image_worker")
    {
        return match lazypencil::remote_images::serve_worker() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
    #[cfg(all(unix, feature = "rendering"))]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__profile_page_worker")
    {
        return match lazypencil::runtime::profiles::serve_page_worker() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__runtime_download_worker")
    {
        return match lazypencil::runtime::serve_download_worker() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    #[cfg(any(windows, unix))]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__browser_process_worker")
    {
        return match lazypencil::cdp::serve_process_worker() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    #[cfg(any(windows, unix))]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__agent_process_worker")
    {
        return match lazypencil::agent::serve_process_worker() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    #[cfg(feature = "tui")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "__image_worker")
    {
        return match lazypencil::clipboard::serve_worker() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "__script_worker")
    {
        return match lazypencil::script::serve_worker() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    lazypencil::remote_images::initialize_worker();
    let parsed = lazypencil::cli::Arguments::command()
        .try_get_matches()
        .and_then(|matches| {
            lazypencil::cli::Arguments::from_arg_matches(&matches)
                .map(|arguments| (arguments, matches))
        });
    let (mut arguments, matches) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            let json = std::env::args_os()
                .skip(1)
                .take_while(|argument| argument != "--")
                .any(|argument| argument == "--json");
            if json && error.use_stderr() {
                let failure = lazypencil::Error::new("INVALID_ARGUMENT", error.to_string())
                    .details(serde_json::json!({"kind":format!("{:?}",error.kind())}));
                let _ = writeln!(
                    std::io::stdout().lock(),
                    "{}",
                    serde_json::json!({"error":failure})
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    // Keep uploaded-but-not-yet-referenced assets live until the CLI result
    // has been written, including long-lived TUI, prompt and MCP frontends.
    let uses_assets = !matches!(
        &arguments.command,
        lazypencil::cli::Command::Doctor
            | lazypencil::cli::Command::Assets {
                command: lazypencil::cli::AssetCommand::Prune { .. }
            }
            | lazypencil::cli::Command::Runtime {
                command: lazypencil::cli::RuntimeCommand::Status
            }
    );
    let _asset_activity = match uses_assets
        .then(lazypencil::store::AssetActivity::acquire)
        .transpose()
    {
        Ok(activity) => activity,
        Err(error) => {
            if arguments.json {
                let value = if matches!(arguments.command, lazypencil::cli::Command::Prompt(_)) {
                    serde_json::json!({"type":"error","error":error})
                } else {
                    serde_json::json!({"error":error})
                };
                let _ = writeln!(std::io::stdout().lock(), "{value}");
            } else {
                eprintln!("{error}");
            }
            return ExitCode::from(error.exit_code());
        }
    };
    if let Err(error) = lazypencil::config::agents::resolve_arguments(&mut arguments, &matches) {
        if arguments.json {
            let value = if matches!(arguments.command, lazypencil::cli::Command::Prompt(_)) {
                serde_json::json!({"type":"error","error":error})
            } else {
                serde_json::json!({"error":error})
            };
            let _ = writeln!(std::io::stdout().lock(), "{value}");
        } else {
            eprintln!("{error}");
        }
        return ExitCode::from(error.exit_code());
    }
    #[cfg(feature = "tui")]
    if let lazypencil::cli::Command::Tui(options) = &arguments.command {
        return match lazypencil::tui::run(options, arguments.data_dir.clone(), arguments.json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                if arguments.json {
                    let _ = writeln!(
                        std::io::stdout().lock(),
                        "{}",
                        serde_json::json!({"error":error})
                    );
                } else {
                    eprintln!("{error}");
                }
                ExitCode::from(error.exit_code())
            }
        };
    }
    if let lazypencil::cli::Command::Mcp {
        file,
        fonts,
        attachments_manifest,
        guidance_audit_manifest,
    } = &arguments.command
    {
        return match lazypencil::store::Store::open(file, arguments.data_dir.clone()).and_then(
            |store| {
                let access = attachments_manifest
                    .as_ref()
                    .map(|path| lazypencil::attachments::AttachmentAccess::open(&store, path))
                    .transpose()?;
                let usage = guidance_audit_manifest
                    .as_ref()
                    .map(|path| lazypencil::guidance::usage::UsageAccess::open(&store, path))
                    .transpose()?;
                lazypencil::mcp::serve_with_context(store, fonts.clone(), access, usage)
            },
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::from(error.exit_code())
            }
        };
    }
    if let lazypencil::cli::Command::Prompt(options) = &arguments.command {
        return match lazypencil::cancel::install_cli_handler().and_then(|()| {
            lazypencil::prompt::run_cli(options, arguments.data_dir.clone(), arguments.json)
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                if arguments.json {
                    let _ = writeln!(
                        std::io::stdout().lock(),
                        "{}",
                        serde_json::json!({"type":"error","error":error})
                    );
                } else {
                    eprintln!("{error}");
                }
                ExitCode::from(error.exit_code())
            }
        };
    }
    if matches!(
        arguments.command,
        lazypencil::cli::Command::Render(_)
            | lazypencil::cli::Command::Config { .. }
            | lazypencil::cli::Command::Layout(_)
            | lazypencil::cli::Command::Apply(_)
            | lazypencil::cli::Command::Restore { .. }
            | lazypencil::cli::Command::History { .. }
            | lazypencil::cli::Command::Conversations { .. }
            | lazypencil::cli::Command::Icons { .. }
            | lazypencil::cli::Command::Assets { .. }
            | lazypencil::cli::Command::Runtime { .. }
    ) && let Err(error) = lazypencil::cancel::install_cli_handler()
    {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let result = lazypencil::cli::run(&arguments);
    let result = match result {
        Ok(completed) if lazypencil::cancel::token().is_cancelled() => Err(lazypencil::Error::new(
            "CANCELLED",
            "Command interrupted after completing work",
        )
        .details(serde_json::json!({"completed_result":completed}))),
        result => result,
    };
    let (value, code) = match result {
        Ok(value) => (value, 0),
        Err(error) => {
            if !arguments.json {
                eprintln!("{error}");
            }
            let code = error.exit_code();
            (serde_json::json!({"error":error}), code)
        }
    };
    if code != 0 && !arguments.json {
        return ExitCode::from(code);
    }
    let output = if !arguments.json
        && matches!(arguments.command, lazypencil::cli::Command::Guidance { .. })
        && code == 0
    {
        value["body"].as_str().unwrap_or_default().to_string()
    } else if !arguments.json
        && code == 0
        && matches!(
            arguments.command,
            lazypencil::cli::Command::Conversations {
                command: lazypencil::cli::ConversationCommand::View { .. }
            }
        )
    {
        if let Some(cursor) = value["next_cursor"].as_str() {
            eprintln!("Continue with --cursor {cursor}");
        }
        value["text"].as_str().unwrap_or_default().to_string()
    } else {
        match serde_json::to_string_pretty(&value) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("Cannot encode result: {error}");
                return ExitCode::FAILURE;
            }
        }
    };
    if let Err(error) = writeln!(std::io::stdout().lock(), "{output}")
        && error.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("Cannot write result: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::from(code)
}
