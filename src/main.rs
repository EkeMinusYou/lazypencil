use clap::{CommandFactory, FromArgMatches};
use std::{io::Write, process::ExitCode};

fn main() -> ExitCode {
    // Completion queries finish before worker setup, configuration, or locks.
    clap_complete::CompleteEnv::with_factory(lazypencil::cli::Arguments::command)
        .var(lazypencil::cli::COMPLETE_ENV)
        .complete();
    if let Some(exit) = dispatch_worker(std::env::args_os().nth(1).as_deref()) {
        return exit;
    }
    lazypencil::remote_images::initialize_worker();
    let input: Vec<_> = std::env::args_os().collect();
    #[cfg(feature = "tui")]
    let input = if input.len() == 1 {
        vec![input[0].clone(), "tui".into()]
    } else {
        input
    };
    let parsed = lazypencil::cli::Arguments::command()
        .try_get_matches_from(input)
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
    #[cfg(feature = "tui")]
    let mut selected_document = None;
    #[cfg(feature = "tui")]
    if let lazypencil::cli::Command::Tui(options) = &mut arguments.command
        && matches.subcommand_matches("tui").is_some_and(|matches| {
            matches.value_source("file") == Some(clap::parser::ValueSource::DefaultValue)
        })
    {
        if arguments.json || options.prompt.create || options.prompt.session.is_some() {
            return failure(
                lazypencil::Error::new(
                    "INVALID_ARGUMENT",
                    "File selection requires a terminal and cannot use --json, --create or --session",
                ),
                Some(&arguments),
            );
        }
        let mut startup = options.prompt.clone();
        if let Err(error) = lazypencil::config::agents::resolve_user(
            &mut startup,
            matches.subcommand_matches("tui").expect("TUI matches"),
        ) {
            return failure(error, Some(&arguments));
        }
        match lazypencil::tui::documents::select(
            arguments.data_dir.clone(),
            &startup,
            options.image_protocol,
            options.theme,
        ) {
            Ok(Some(selection)) => {
                options.prompt.file = selection.file.clone();
                selected_document = Some(selection);
            }
            Ok(None) => return ExitCode::SUCCESS,
            Err(error) => return failure(error, Some(&arguments)),
        }
    }
    // Keep uploaded-but-not-yet-referenced assets live until the CLI result
    // has been written, including long-lived TUI, prompt and MCP frontends.
    let uses_assets = !matches!(
        &arguments.command,
        lazypencil::cli::Command::Doctor(_)
            | lazypencil::cli::Command::Completions { .. }
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
            #[cfg(feature = "tui")]
            drop(selected_document.take());
            return failure(error, Some(&arguments));
        }
    };
    if let Err(error) = lazypencil::config::agents::resolve_arguments(&mut arguments, &matches) {
        #[cfg(feature = "tui")]
        drop(selected_document.take());
        return failure(error, Some(&arguments));
    }
    #[cfg(feature = "tui")]
    if let lazypencil::cli::Command::Tui(options) = &arguments.command {
        return match lazypencil::tui::run_with_selection(
            options,
            arguments.data_dir.clone(),
            arguments.json,
            selected_document,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => failure(error, Some(&arguments)),
        };
    }
    if let lazypencil::cli::Command::Mcp {
        roots,
        fonts,
        attachments_manifest,
        guidance_audit_manifest,
        preview_control,
    } = &arguments.command
    {
        return match lazypencil::mcp::serve(lazypencil::mcp::Options {
            data_dir: arguments.data_dir.clone(),
            fonts: fonts.clone(),
            roots: roots.clone(),
            attachments_manifest: attachments_manifest.clone(),
            guidance_audit_manifest: guidance_audit_manifest.clone(),
            preview_control: preview_control.clone(),
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => failure(error, None),
        };
    }
    if let lazypencil::cli::Command::Prompt(options) = &arguments.command {
        return match lazypencil::cancel::install_cli_handler().and_then(|()| {
            lazypencil::prompt::run_cli(options, arguments.data_dir.clone(), arguments.json)
        }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => failure(error, Some(&arguments)),
        };
    }
    let cancellable = match &arguments.command {
        lazypencil::cli::Command::Render(_)
        | lazypencil::cli::Command::Inspect(_)
        | lazypencil::cli::Command::Validate(_)
        | lazypencil::cli::Command::Variables(_)
        | lazypencil::cli::Command::Guidance { .. }
        | lazypencil::cli::Command::Styles { .. }
        | lazypencil::cli::Command::Style { .. }
        | lazypencil::cli::Command::Config { .. }
        | lazypencil::cli::Command::Layout(_)
        | lazypencil::cli::Command::Apply(_)
        | lazypencil::cli::Command::Restore { .. }
        | lazypencil::cli::Command::History { .. }
        | lazypencil::cli::Command::Conversations { .. }
        | lazypencil::cli::Command::Icons { .. }
        | lazypencil::cli::Command::Assets { .. }
        | lazypencil::cli::Command::Runtime { .. } => true,
        lazypencil::cli::Command::Doctor(_) => true,
        #[cfg(feature = "rendering")]
        lazypencil::cli::Command::Fonts { .. } => true,
        _ => false,
    };
    if cancellable && let Err(error) = lazypencil::cancel::install_cli_handler() {
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
        && matches!(arguments.command, lazypencil::cli::Command::Doctor(_))
        && code == 0
    {
        lazypencil::doctor::format_report(&value)
    } else if !arguments.json
        && matches!(
            arguments.command,
            lazypencil::cli::Command::Completions { .. }
        )
        && code == 0
    {
        value["script"].as_str().unwrap_or_default().to_string()
    } else if !arguments.json
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
    let strict_failure = matches!(&arguments.command, lazypencil::cli::Command::Doctor(options) if options.strict)
        && value["summary"]["errors"]
            .as_u64()
            .is_some_and(|errors| errors > 0);
    ExitCode::from(if strict_failure { 1 } else { code })
}

/// Private workers run before application configuration or document leases.
fn dispatch_worker(command: Option<&std::ffi::OsStr>) -> Option<ExitCode> {
    let result = match command?.to_str()? {
        "__http_image_worker" => {
            // Image URLs can carry private information; worker errors stay silent.
            return Some(if lazypencil::remote_images::serve_worker().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        #[cfg(all(unix, feature = "rendering"))]
        "__profile_page_worker" => {
            lazypencil::runtime::profiles::serve_page_worker().map(|()| true)
        }
        "__runtime_download_worker" => lazypencil::runtime::serve_download_worker().map(|()| true),
        #[cfg(any(windows, unix))]
        "__browser_process_worker" => lazypencil::cdp::serve_process_worker(),
        #[cfg(any(windows, unix))]
        "__agent_process_worker" => lazypencil::agent::serve_process_worker(),
        #[cfg(feature = "tui")]
        "__image_worker" => lazypencil::clipboard::serve_worker().map(|()| true),
        "__script_worker" => lazypencil::script::serve_worker().map(|()| true),
        _ => return None,
    };
    Some(match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => failure(error, None),
    })
}

fn failure(error: lazypencil::Error, arguments: Option<&lazypencil::cli::Arguments>) -> ExitCode {
    if let Some(arguments) = arguments.filter(|arguments| arguments.json) {
        let value = if matches!(&arguments.command, lazypencil::cli::Command::Prompt(_)) {
            serde_json::json!({"type":"error","error":error})
        } else {
            serde_json::json!({"error":error})
        };
        let _ = writeln!(std::io::stdout().lock(), "{value}");
    } else {
        eprintln!("{error}");
    }
    ExitCode::from(error.exit_code())
}
