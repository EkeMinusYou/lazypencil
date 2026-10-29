use crate::{
    Error, Result,
    operations::{EditRequest, Operation},
    service::{
        apply_request_with_cancellation, document_metadata, document_metadata_with_fonts,
        read_store_nodes_with_cancellation,
    },
    store::{self, Store},
};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "lazypencil",
    version,
    about = "Edit and inspect .pen design documents from the terminal"
)]
pub struct Arguments {
    #[arg(long, global = true, help = "Return structured JSON, including errors")]
    pub json: bool,
    #[arg(
        long,
        global = true,
        env = "LAZYPENCIL_DATA_DIR",
        help = "Override the persistent user data directory"
    )]
    pub data_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create, inspect, or describe versioned project configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Create an empty document without overwriting an existing file.
    Init { file: PathBuf },
    /// Inspect document metadata or a bounded selection of nodes.
    Inspect(Inspect),
    /// Validate structure and references, optionally including calculated layout.
    Validate(Validate),
    /// Read variable definitions and resolved values for a library, node, or theme.
    Variables(Variables),
    /// Apply an atomic batch of structured editing operations.
    Apply(Apply),
    /// Calculate positions, text metrics, and overflow diagnostics.
    Layout(Layout),
    /// Render a design or selected node as a PNG image.
    Render(Render),
    /// Ask an ACP agent to create or edit a design.
    Prompt(crate::prompt::PromptArgs),
    /// View a design, paste reference images, and converse with an ACP agent.
    #[cfg(feature = "tui")]
    Tui(crate::tui::TuiArgs),
    /// Serve one bound document over MCP stdio.
    Mcp {
        #[arg(long)]
        file: PathBuf,
        #[arg(long = "font")]
        fonts: Vec<PathBuf>,
        #[arg(long, hide = true)]
        attachments_manifest: Option<PathBuf>,
        #[arg(long, hide = true)]
        guidance_audit_manifest: Option<PathBuf>,
    },
    /// Read a bundled or trusted project guide (defaults to the workflow guide).
    Guidance {
        id: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// List bundled and trusted project design styles.
    Styles {
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        use_case: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read a style, its tokens, and its editable example.
    Style {
        id: String,
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Search pinned vector icon packs or explicitly install an optional pack.
    Icons {
        #[command(subcommand)]
        command: IconCommand,
    },
    /// Inspect additional fonts and read the bundled font's complete license.
    #[cfg(feature = "rendering")]
    Fonts {
        #[command(subcommand)]
        command: FontCommand,
    },
    /// Create immutable project assets or read referenced code assets.
    Assets {
        #[command(subcommand)]
        command: AssetCommand,
    },
    /// Explicitly install or inspect optional native runtimes.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
    /// List committed edits and their checkpoints, newest first.
    History {
        file: PathBuf,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        before_sequence: Option<u64>,
    },
    /// Browse saved local conversations without starting or authenticating an agent.
    Conversations {
        #[command(subcommand)]
        command: ConversationCommand,
    },
    /// Restore a committed checkpoint as a new edit.
    Restore {
        file: PathBuf,
        #[arg(long)]
        checkpoint: String,
        #[arg(long)]
        expected_revision: String,
        #[arg(long)]
        expected_dependency_revision: Option<String>,
        #[arg(long = "font")]
        fonts: Vec<PathBuf>,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Reconcile an interrupted save without overwriting external changes.
    Recover { file: PathBuf },
    /// Report build and environment information without changing a design.
    Doctor,
}

#[derive(Debug, Subcommand)]
pub enum ConversationCommand {
    /// Review aggregate user retention, or commit exactly the reviewed plan.
    Prune {
        #[arg(long)]
        expected_revision: Option<String>,
    },
    /// Review a local deletion, or commit it using the revision from that review.
    Delete {
        file: PathBuf,
        session: String,
        #[arg(long)]
        expected_revision: Option<String>,
    },
    /// Finish local conversation deletions interrupted after their atomic commit.
    Cleanup {
        #[arg(required_unless_present = "all")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file")]
        all: bool,
    },
    /// Read formatted saved requests, responses and tool status in bounded pages.
    View {
        file: PathBuf,
        session: String,
        turn: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 65536)]
        limit: usize,
    },
    /// List saved conversations for a document in stable UUID order.
    List {
        file: PathBuf,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// List the saved turns of one local conversation.
    Turns {
        file: PathBuf,
        session: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read a byte-bounded JSONL transcript page, including unfinished final records.
    Read {
        file: PathBuf,
        session: String,
        turn: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 65536)]
        limit: usize,
        #[arg(long)]
        expected_revision: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create project or user defaults without overwriting an existing file.
    Init {
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        user: bool,
    },
    /// Inspect document configuration, fonts, and design resources without executing code.
    Show {
        #[arg(required_unless_present = "user")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file")]
        user: bool,
        /// Inspect project agent settings without font or design-resource loading.
        #[arg(long, conflicts_with = "user")]
        executables: bool,
    },
    /// Authorize current project design instructions, or agent settings with --executables.
    Trust {
        file: PathBuf,
        #[arg(long)]
        expected_fingerprint: Option<String>,
        /// Authorize exactly the project agent settings, separately from design instructions.
        #[arg(long)]
        executables: bool,
    },
    /// Revoke local project design and agent-execution grants.
    Untrust { file: PathBuf },
    /// Return the JSON Schema for project or user settings.
    Schema {
        #[arg(long)]
        user: bool,
    },
}

#[derive(Debug, Args)]
pub struct Inspect {
    pub file: PathBuf,
    /// Execute scripts and inspect the calculated tree used by layout/rendering.
    #[arg(long)]
    pub calculated: bool,
    #[arg(long)]
    pub node: Option<String>,
    #[arg(long, value_delimiter = '/', requires = "node")]
    pub instance_path: Vec<String>,
    #[arg(long)]
    pub library_alias: Option<String>,
    #[arg(long)]
    pub expand_instances: bool,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long = "type")]
    pub node_type: Option<String>,
    #[arg(long, default_value_t = 1)]
    pub depth: usize,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub nodes: bool,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum IconCommand {
    /// Read the complete embedded upstream license without installing the pack.
    License { pack: String },
    List {
        #[arg(long)]
        pack: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    Setup {
        pack: String,
        /// Exact pinned upstream archive or font file for offline installation.
        #[arg(long, visible_alias = "input")]
        archive: Option<PathBuf>,
    },
}

#[cfg(feature = "rendering")]
#[derive(Debug, Subcommand)]
pub enum FontCommand {
    /// List every face in explicitly supplied font files, plus the bundled default.
    List {
        /// Resolve the document's project configuration before applying --font.
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long = "font")]
        fonts: Vec<PathBuf>,
    },
    /// Read the bundled Noto Sans JP license without accessing system fonts.
    License,
}

#[derive(Debug, Subcommand)]
pub enum AssetCommand {
    /// Store a validated image, JavaScript source, or GLSL source without changing the document.
    Put {
        file: PathBuf,
        #[arg(long)]
        kind: crate::assets::AssetKind,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        mime_type: Option<String>,
    },
    /// Read a captured script or shader by its dependency hash.
    Read {
        file: PathBuf,
        #[arg(long)]
        hash: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 65536)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum RuntimeCommand {
    /// Install a pinned runtime without changing an existing installation.
    Setup {
        #[arg(value_parser=["chromium"])]
        name: String,
        #[arg(
            long,
            help = "Use a local archive matching the pinned official checksum"
        )]
        archive: Option<PathBuf>,
    },
    /// Verify the installed Chromium files and report their status.
    Status,
    /// Explicitly create or authenticate a dedicated page profile.
    Profile {
        #[command(subcommand)]
        command: BrowserProfileCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum BrowserProfileCommand {
    /// Create private browser data without adopting an existing browser profile.
    Create { name: String },
    /// Show the path of an explicitly created profile, without reading credentials.
    Path { name: String },
    /// Open a supplied full Chromium browser for interactive authentication.
    Login {
        name: String,
        #[arg(long)]
        browser: PathBuf,
        #[arg(long)]
        url: String,
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
        #[arg(long, hide = true)]
        headless: bool,
    },
}

#[derive(Debug, Args)]
pub struct Validate {
    pub file: PathBuf,
    #[arg(long, value_enum, default_value = "structure")]
    pub scope: crate::service::ValidationScope,
    #[arg(long)]
    pub expected_revision: Option<String>,
    #[arg(long)]
    pub expected_dependency_revision: Option<String>,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Debug, Args)]
pub struct Variables {
    pub file: PathBuf,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub library_alias: Option<String>,
    #[arg(long)]
    pub node: Option<String>,
    #[arg(long, value_delimiter = '/', requires = "node")]
    pub instance_path: Vec<String>,
    #[arg(long, requires = "node")]
    pub property: Option<String>,
    /// Hypothetical theme overrides, as AXIS=VALUE; may be repeated.
    #[arg(long = "theme")]
    pub themes: Vec<String>,
    #[arg(long)]
    pub expected_revision: Option<String>,
    #[arg(long)]
    pub expected_dependency_revision: Option<String>,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Debug, Args)]
pub struct Apply {
    pub file: PathBuf,
    #[arg(long)]
    pub operations: PathBuf,
    #[arg(long)]
    pub expected_revision: String,
    #[arg(long)]
    pub expected_dependency_revision: Option<String>,
    #[arg(long)]
    pub expected_runtime_dependency_revision: Option<String>,
    #[arg(long)]
    pub request_id: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct Layout {
    pub file: PathBuf,
    #[arg(long)]
    pub expected_revision: Option<String>,
    #[arg(long)]
    pub expected_dependency_revision: Option<String>,
    #[arg(long = "font", help = "Load an additional font file; may be repeated")]
    pub fonts: Vec<PathBuf>,
    #[arg(long)]
    pub node: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct Render {
    pub file: PathBuf,
    #[arg(short, long)]
    pub output: PathBuf,
    #[arg(long, default_value_t = 1.0)]
    pub scale: f32,
    #[arg(long, default_value_t = 0.0)]
    pub time: f64,
    #[arg(long)]
    pub refresh: bool,
    #[arg(
        long,
        help = "Use a node's visual bounds as a crop of the composited document"
    )]
    pub node: Option<String>,
    #[arg(long,num_args=4,value_names=["X","Y","WIDTH","HEIGHT"],allow_hyphen_values=true)]
    pub region: Vec<f32>,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
    #[arg(long)]
    pub expected_revision: Option<String>,
    #[arg(long)]
    pub expected_dependency_revision: Option<String>,
    #[arg(long, help = "Hex background color; defaults to transparent")]
    pub background: Option<String>,
}

pub fn run(arguments: &Arguments) -> Result<Value> {
    let open = |path: &Path| Store::open(path, arguments.data_dir.clone());
    match &arguments.command {
        Command::Config { command } => match command {
            ConfigCommand::Init { user: true, .. } => crate::config::initialize_user(),
            ConfigCommand::Init { path, .. } => {
                crate::config::initialize(path.as_deref().unwrap_or(Path::new("lazypencil.toml")))
            }
            ConfigCommand::Show { user: true, .. } => {
                let configuration = crate::config::UserSnapshot::load()?;
                configuration.verify_unchanged()?;
                Ok(configuration.summary())
            }
            ConfigCommand::Show {
                file,
                executables: true,
                ..
            } => {
                let store =
                    open(file.as_ref().ok_or_else(|| {
                        Error::new("INVALID_ARGUMENT", "Supply a document file")
                    })?)?;
                let configuration = crate::config::agent_execution::Project::load(store.path())?;
                let value = configuration.summary(store.data_dir())?;
                configuration.verify_unchanged()?;
                Ok(value)
            }
            ConfigCommand::Show { file, .. } => {
                let store = open(file.as_ref().ok_or_else(|| {
                    Error::new("INVALID_ARGUMENT", "Supply a document file or --user")
                })?)?;
                let configuration = crate::config::Snapshot::load(store.path())?;
                configuration.verify_unchanged()?;
                let mut value = configuration.summary();
                value["design"]["trusted"] = json!(
                    configuration
                        .design_catalog()
                        .and_then(|design| design.trusted(store.data_dir()))
                        .unwrap_or(false)
                );
                value["agent_execution"] =
                    configuration.agent_execution_metadata(store.data_dir())?;
                Ok(value)
            }
            ConfigCommand::Trust {
                file,
                expected_fingerprint,
                executables: true,
            } => {
                let store = open(file)?;
                let configuration = crate::config::agent_execution::Project::load(store.path())?;
                let value = configuration
                    .trust_agent_execution(store.data_dir(), expected_fingerprint.as_deref())?;
                configuration.verify_unchanged()?;
                Ok(value)
            }
            ConfigCommand::Trust {
                file,
                expected_fingerprint,
                ..
            } => {
                let store = open(file)?;
                let configuration = crate::config::Snapshot::load(store.path())?;
                configuration.verify_unchanged()?;
                let value = configuration.design_catalog()?.set_trust(
                    store.data_dir(),
                    expected_fingerprint.as_deref(),
                    true,
                )?;
                configuration.verify_unchanged()?;
                Ok(value)
            }
            ConfigCommand::Untrust { file } => {
                let store = open(file)?;
                crate::config::revoke_project_trust(store.path(), store.data_dir())
            }
            ConfigCommand::Schema { user: false } => {
                serde_json::to_value(schemars::schema_for!(crate::config::ProjectConfig))
                    .map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))
            }
            ConfigCommand::Schema { user: true } => {
                serde_json::to_value(schemars::schema_for!(crate::config::UserConfig))
                    .map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))
            }
        },
        #[cfg(feature = "rendering")]
        Command::Fonts { command } => match command {
            FontCommand::List { fonts, file } => {
                let mut metadata = json!({});
                let (fonts, inventory) = if let Some(file) = file {
                    let mut project = crate::project::Project::load(&open(file)?)?;
                    project.add_font_files(fonts)?;
                    let mut fonts = project.load_fonts()?;
                    let inventory = fonts.inventory()?;
                    project.verify_unchanged()?;
                    metadata = json!({"revision":project.root_document().revision(),"dependency_revision":project.dependency_revision(),"configuration":project.configuration()});
                    (fonts, inventory)
                } else {
                    let configuration = crate::config::UserSnapshot::load()?;
                    let mut paths = fonts.clone();
                    paths.extend(configuration.font_files.iter().cloned());
                    let mut loaded = crate::text::Fonts::new(&paths)?;
                    loaded.configure(&configuration.settings.fonts)?;
                    let inventory = loaded.inventory()?;
                    configuration.verify_unchanged()?;
                    metadata["configuration"] = configuration.summary();
                    (loaded, inventory)
                };
                metadata["fonts"] = json!(inventory);
                metadata["fallback_families"] = json!(fonts.fallback_families());
                metadata["default_family"] = json!(fonts.default_family());
                metadata["locale"] = json!(fonts.locale());
                Ok(metadata)
            }
            FontCommand::License => {
                let text = include_str!("../resources/fonts/OFL.txt");
                Ok(
                    json!({"family":crate::text::DEFAULT_FONT_FAMILY,"license":"OFL-1.1","hash":crate::document::hash(text.as_bytes()),"text":text,"manifest":serde_json::from_str::<Value>(include_str!("../resources/fonts/manifest.json")).expect("pinned font manifest")}),
                )
            }
        },
        Command::Icons { command } => {
            let data = store::data_directory(arguments.data_dir.clone())?;
            match command {
                IconCommand::License { pack } => crate::icons::license(pack),
                IconCommand::List {
                    pack,
                    name,
                    limit,
                    cursor,
                } => crate::icons::list(
                    &data,
                    &crate::icons::ListRequest {
                        pack: pack.clone(),
                        name: name.clone(),
                        limit: *limit,
                        cursor: cursor.clone(),
                    },
                ),
                IconCommand::Setup { pack, archive } => {
                    crate::icons::setup(&data, pack, archive.as_deref())
                }
            }
        }
        #[cfg(feature = "tui")]
        Command::Tui(_) => Err(Error::new(
            "INVALID_ARGUMENT",
            "Use the interactive TUI entry point",
        )),
        Command::Prompt(_) => Err(Error::new(
            "INVALID_ARGUMENT",
            "Use the streaming prompt entry point",
        )),
        Command::Init { file } => {
            let store = Store::init(file, arguments.data_dir.clone())?;
            document_metadata(&store)
        }
        Command::Inspect(options) => {
            let store = open(&options.file)?;
            if options.calculated
                || options.nodes
                || options.node.is_some()
                || options.name.is_some()
                || options.node_type.is_some()
                || options.cursor.is_some()
                || options.library_alias.is_some()
                || options.expand_instances
            {
                read_store_nodes_with_cancellation(
                    &store,
                    &options.fonts,
                    &crate::service::ReadNodesRequest {
                        calculated: options.calculated,
                        snapshot_id: None,
                        node_id: options.node.clone(),
                        instance_path: options.instance_path.clone(),
                        library_alias: options.library_alias.clone(),
                        expand_instances: options.expand_instances,
                        name: options.name.clone(),
                        kind: options.node_type.clone(),
                        depth: options.depth,
                        limit: options.limit,
                        cursor: options.cursor.clone(),
                    },
                    Some(&crate::cancel::token()),
                )
            } else {
                document_metadata_with_fonts(&store, &options.fonts)
            }
        }
        Command::Validate(options) => {
            let result = crate::service::validate_document_with_options(
                &open(&options.file)?,
                &options.fonts,
                &crate::service::ValidateRequest {
                    scope: options.scope,
                    expected_revision: options.expected_revision.clone(),
                    expected_dependency_revision: options.expected_dependency_revision.clone(),
                    snapshot_id: None,
                    limit: options.limit,
                    cursor: options.cursor.clone(),
                },
                Some(&crate::cancel::token()),
            )?;
            if result["valid"] == false {
                return Err(
                    Error::new("INVALID_DOCUMENT", "Layout validation found errors")
                        .details(result),
                );
            }
            Ok(result)
        }
        Command::Variables(options) => {
            let mut theme = std::collections::BTreeMap::new();
            for assignment in &options.themes {
                let (axis, value) = assignment
                    .split_once('=')
                    .filter(|(axis, value)| !axis.is_empty() && !value.is_empty())
                    .ok_or_else(|| {
                        Error::new("INVALID_ARGUMENT", "A theme selection requires AXIS=VALUE")
                    })?;
                if theme.insert(axis.to_string(), value.to_string()).is_some() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "A theme axis may be specified only once",
                    ));
                }
            }
            crate::service::get_variables(
                &open(&options.file)?,
                &options.fonts,
                &crate::service::VariablesRequest {
                    name: options.name.clone(),
                    library_alias: options.library_alias.clone(),
                    node_id: options.node.clone(),
                    instance_path: options.instance_path.clone(),
                    property: options.property.clone(),
                    theme,
                    expected_revision: options.expected_revision.clone(),
                    expected_dependency_revision: options.expected_dependency_revision.clone(),
                    limit: options.limit,
                    cursor: options.cursor.clone(),
                },
                Some(&crate::cancel::token()),
            )
        }
        Command::Apply(options) => {
            let source = store::read_text(&options.operations)?;
            let operations: Vec<Operation> = serde_json::from_str(&source).map_err(|e| {
                Error::new("INVALID_ARGUMENT", format!("Invalid operation array: {e}"))
            })?;
            let request = EditRequest {
                request_id: options
                    .request_id
                    .clone()
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                expected_revision: options.expected_revision.clone(),
                expected_dependency_revision: options.expected_dependency_revision.clone(),
                expected_runtime_dependency_revision: options
                    .expected_runtime_dependency_revision
                    .clone(),
                operations,
                dry_run: options.dry_run,
            };
            apply_request_with_cancellation(
                &open(&options.file)?,
                &request,
                &options.fonts,
                Some(&crate::cancel::token()),
            )
        }
        Command::Layout(options) => run_layout(&open(&options.file)?, options),
        Command::Render(options) => run_render(&open(&options.file)?, options),
        Command::Mcp { .. } => Err(Error::new(
            "INVALID_ARGUMENT",
            "MCP uses the streaming stdio entry point",
        )),
        Command::Guidance { id, file } => match file {
            Some(file) => crate::service::project_guidance(&open(file)?, id.as_deref()),
            None => crate::guidance::get_guidance(id.as_deref()),
        },
        Command::Styles {
            file,
            use_case,
            limit,
            cursor,
        } => match file {
            Some(file) => crate::service::project_styles(
                &open(file)?,
                use_case.as_deref(),
                *limit,
                cursor.as_deref(),
            ),
            None => crate::guidance::list_styles(use_case.as_deref(), *limit, cursor.as_deref()),
        },
        Command::Style { id, file } => match file {
            Some(file) => crate::service::project_style(&open(file)?, id),
            None => crate::guidance::get_style(id),
        },
        Command::Assets { command } => match command {
            AssetCommand::Put {
                file,
                kind,
                input,
                mime_type,
            } => {
                let asset = crate::assets::put_file_with_cancellation(
                    &open(file)?,
                    *kind,
                    input,
                    mime_type.as_deref(),
                    Some(&crate::cancel::token()),
                )?;
                serde_json::to_value(asset).map_err(|e| Error::new("INTERNAL_ERROR", e.to_string()))
            }
            AssetCommand::Read {
                file,
                hash,
                offset,
                limit,
            } => {
                let project = crate::project::Project::load(&open(file)?)?;
                crate::assets::read_asset(
                    &project,
                    &crate::assets::ReadAssetRequest {
                        hash: hash.clone(),
                        offset: *offset,
                        limit: *limit,
                    },
                )
            }
        },
        Command::Conversations { command } => {
            let cancel = crate::cancel::token();
            match command {
                ConversationCommand::Prune { expected_revision } => {
                    let snapshot = crate::config::ConversationSnapshot::load()?;
                    let data = crate::store::data_directory(arguments.data_dir.clone())?;
                    if let Some(revision) = expected_revision {
                        crate::conversation::management::retention::apply_checked(
                            &data,
                            &snapshot.settings.retention,
                            revision,
                            Some(&cancel),
                            || snapshot.verify_unchanged(),
                        )
                    } else {
                        let result = crate::conversation::management::retention::review(
                            &data,
                            &snapshot.settings.retention,
                            Some(&cancel),
                        )?;
                        snapshot.verify_unchanged()?;
                        Ok(result)
                    }
                }
                ConversationCommand::Delete {
                    file,
                    session,
                    expected_revision,
                } => {
                    let store = open(file)?;
                    if let Some(revision) = expected_revision {
                        crate::conversation::management::delete(
                            &store,
                            session,
                            revision,
                            Some(&cancel),
                        )
                    } else {
                        crate::conversation::management::review_delete(
                            &store,
                            session,
                            Some(&cancel),
                        )
                    }
                }
                ConversationCommand::Cleanup { file, all } => {
                    if *all {
                        crate::conversation::management::retention::cleanup_all(
                            &crate::store::data_directory(arguments.data_dir.clone())?,
                            Some(&cancel),
                        )
                    } else {
                        crate::conversation::management::cleanup_deleted(
                            &open(file.as_ref().expect("selected document"))?,
                            Some(&cancel),
                        )
                    }
                }
                ConversationCommand::View {
                    file,
                    session,
                    turn,
                    cursor,
                    limit,
                } => crate::conversation::history::view(
                    &open(file)?,
                    session,
                    turn,
                    cursor.as_deref(),
                    *limit,
                    Some(&cancel),
                ),
                ConversationCommand::List {
                    file,
                    limit,
                    cursor,
                } => crate::conversation::history::list(
                    &open(file)?,
                    None,
                    *limit,
                    cursor.as_deref(),
                    Some(&cancel),
                ),
                ConversationCommand::Turns {
                    file,
                    session,
                    limit,
                    cursor,
                } => crate::conversation::history::list(
                    &open(file)?,
                    Some(session),
                    *limit,
                    cursor.as_deref(),
                    Some(&cancel),
                ),
                ConversationCommand::Read {
                    file,
                    session,
                    turn,
                    offset,
                    limit,
                    expected_revision,
                } => crate::conversation::history::read(
                    &open(file)?,
                    session,
                    turn,
                    *offset,
                    *limit,
                    expected_revision.as_deref(),
                    Some(&cancel),
                ),
            }
        }
        Command::History {
            file,
            limit,
            before_sequence,
        } => {
            if !(1..=1000).contains(limit) {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "History limit must be 1..1000",
                ));
            }
            let store = open(file)?;
            let entries = store.history_with_cancellation(
                *limit,
                *before_sequence,
                Some(&crate::cancel::token()),
            )?;
            let next = if entries.len() == *limit {
                entries.last().map(|e| e.sequence)
            } else {
                None
            };
            Ok(
                json!({"revision":store.read()?.revision(),"entries":entries,"next_before_sequence":next}),
            )
        }
        Command::Restore {
            file,
            checkpoint,
            expected_revision,
            expected_dependency_revision,
            fonts,
            request_id,
        } => crate::service::restore_checkpoint_with_cancellation(
            &open(file)?,
            &crate::service::RestoreRequest {
                request_id: &request_id
                    .clone()
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                expected_revision,
                expected_dependency_revision: expected_dependency_revision.as_deref(),
                checkpoint,
            },
            fonts,
            Some(&crate::cancel::token()),
        ),
        Command::Runtime { command } => {
            let data = store::data_directory(arguments.data_dir.clone())?;
            match command {
                RuntimeCommand::Setup { archive, .. } => {
                    let executable = std::env::current_exe()
                        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
                    crate::runtime::setup_with_cancellation(
                        &data,
                        archive.as_deref(),
                        Some(&crate::cancel::token()),
                        Some(&executable),
                    )
                }
                RuntimeCommand::Status => Ok(crate::runtime::status(&data)),
                RuntimeCommand::Profile { command } => match command {
                    BrowserProfileCommand::Create { name } => {
                        crate::runtime::profiles::Profile::create(&data, name)?;
                        Ok(json!({"profile":name,"created":true}))
                    }
                    BrowserProfileCommand::Path { name } => {
                        let profile = crate::runtime::profiles::Profile::open(&data, name)?;
                        Ok(json!({"profile":name,"directory":profile.directory()}))
                    }
                    BrowserProfileCommand::Login {
                        name,
                        browser,
                        url,
                        timeout_seconds,
                        headless,
                    } => {
                        crate::runtime::profiles::login(
                            &data,
                            name,
                            browser,
                            url,
                            *timeout_seconds,
                            *headless,
                        )?;
                        Ok(json!({"profile":name,"browser_closed":true}))
                    }
                },
            }
        }
        Command::Recover { file } => {
            let store = open(file)?;
            Ok(json!({"recovered":store.recover()?,"revision":store.read()?.revision()}))
        }
        Command::Doctor => {
            let data = store::data_directory(arguments.data_dir.clone())?;
            Ok(json!({
            "version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "document_version":crate::document::DOCUMENT_VERSION,
            "supported_document_versions":crate::document::SUPPORTED_DOCUMENT_VERSIONS,
            "data_directory":data,
            "lock_directory":store::default_lock_dir()?,
            "user_configuration":crate::config::user_config_path().map(|path|json!({"path":path})).unwrap_or_else(|error|json!({"error":error})),
            "skia_compiled":cfg!(feature="rendering"),
            "chromium":crate::runtime::status(&data),
            "release_ready":false,
            }))
        }
    }
}

#[cfg(feature = "rendering")]
fn run_layout(store: &Store, options: &Layout) -> Result<Value> {
    if !(1..=1000).contains(&options.limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Layout limit must be 1..1000",
        ));
    }
    let (_, snapshot) = crate::service::layout_snapshot(
        store,
        &options.fonts,
        options.expected_revision.as_deref(),
        options.expected_dependency_revision.as_deref(),
    )?;
    let mut records = snapshot.records();
    if let Some(id) = &options.node {
        records.retain(|r| &r.id == id);
        if records.is_empty() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                format!("No resolved node {id:?}"),
            ));
        }
    }
    let total = records.len();
    let mut result = snapshot.summary();
    result["nodes"] = json!(records.into_iter().take(options.limit).collect::<Vec<_>>());
    result["total_matches"] = json!(total);
    result["truncated"] = json!(total > options.limit);
    if result.to_string().len() > 4 * 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Layout response exceeds 4 MiB; reduce the limit",
        ));
    }
    Ok(result)
}

#[cfg(feature = "rendering")]
fn run_render(store: &Store, options: &Render) -> Result<Value> {
    if options.output.to_str().is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "The output path must be UTF-8",
        ));
    }
    let (project, snapshot) = crate::service::layout_snapshot(
        store,
        &options.fonts,
        options.expected_revision.as_deref(),
        options.expected_dependency_revision.as_deref(),
    )?;
    let render_options = crate::render::RenderOptions {
        scale: options.scale,
        time: options.time,
        refresh: options.refresh,
        refresh_pages: false,
        node: options.node.clone(),
        region: match options.region.as_slice() {
            [] => None,
            [x, y, width, height] => Some(crate::geometry::Rect::new(*x, *y, *width, *height)),
            _ => {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Render region requires X Y WIDTH HEIGHT",
                ));
            }
        },
        background: options
            .background
            .as_deref()
            .map(crate::color::parse)
            .transpose()?
            .unwrap_or(skia_safe::Color::TRANSPARENT),
    };
    let mut runtime = crate::dynamic::DynamicRuntime::new(store.data_dir());
    let rendered = crate::render::png_with_runtime(
        &project,
        &snapshot,
        &render_options,
        &mut runtime,
        Some(crate::cancel::token()),
    )?;
    if crate::cancel::token().is_cancelled() {
        return Err(Error::new(
            "CANCELLED",
            "Render interrupted before saving its output",
        ));
    }
    store::require_revision(&store.read()?, &snapshot.revision)?;
    if options
        .output
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case("png"))
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "PNG output must use a .png extension",
        ));
    }
    let output_parent = options
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output_parent = crate::paths::canonical(output_parent)?;
    let output = output_parent.join(
        options
            .output
            .file_name()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Missing output filename"))?,
    );
    let actual = if output.exists() {
        crate::paths::canonical(&output)?
    } else {
        output.clone()
    };
    if actual == store.path()
        || project
            .dependencies()
            .into_iter()
            .chain(project.runtime_dependencies())
            .any(|d| d.path == actual)
        || rendered.assets.iter().any(|a| a.path == actual)
        || options
            .fonts
            .iter()
            .any(|p| crate::paths::canonical(p).is_ok_and(|p| p == actual))
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Render output must not overwrite a document or one of its input assets",
        ));
    }
    store::atomic_replace(&output, &rendered.png)?;
    let mut result = snapshot.summary();
    result["output"] = json!(output);
    result["width"] = json!(rendered.width);
    result["height"] = json!(rendered.height);
    result["region"] = json!(rendered.region);
    result["assets"] = json!(rendered.assets);
    result["dynamic_artifacts"] = json!(rendered.dynamic_artifacts);
    result["png_hash"] = json!(crate::document::hash(&rendered.png));
    Ok(result)
}

#[cfg(not(feature = "rendering"))]
fn run_layout(_store: &Store, _options: &Layout) -> Result<Value> {
    Err(Error::new(
        "UNSUPPORTED_FEATURE",
        "This binary was built without rendering support",
    ))
}
#[cfg(not(feature = "rendering"))]
fn run_render(_store: &Store, _options: &Render) -> Result<Value> {
    Err(Error::new(
        "UNSUPPORTED_FEATURE",
        "This binary was built without rendering support",
    ))
}
