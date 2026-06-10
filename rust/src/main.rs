mod chat_sync;
mod cli;
mod compression;
mod config;
mod error;
mod provider;
mod session_key;
mod sync;
mod utils;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

use crate::config::FileConfig;
use crate::error::Result;

/// ClaudeSync: Synchronize local files with AI projects.
#[derive(Parser)]
#[command(name = "claudesync", version, about = "ClaudeSync: Synchronize local files with AI projects.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProviderChoice {
    #[value(name = "claude.ai")]
    ClaudeAi,
}

impl ProviderChoice {
    fn as_str(&self) -> &'static str {
        "claude.ai"
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// Manage AI organizations
    Organization {
        #[command(subcommand)]
        command: OrganizationCommands,
    },
    /// Manage AI projects within the active organization
    Project {
        #[command(subcommand)]
        command: ProjectCommands,
    },
    /// Manage claudesync configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Manage and synchronize chats
    Chat {
        #[command(subcommand)]
        command: ChatCommands,
    },
    /// Manage Claude Code web sessions
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Synchronize the project files, optionally including submodules in the parent project
    Push {
        /// Specify the file category to sync
        #[arg(long)]
        category: Option<String>,
        /// Include submodules in the parent project sync
        #[arg(long)]
        uberproject: bool,
        /// Just show what files would be sent
        #[arg(long)]
        dryrun: bool,
    },
    /// Generate a text embedding from the project
    Embedding {
        /// Specify the file category to sync
        #[arg(long)]
        category: Option<String>,
        /// Include submodules in the parent project sync
        #[arg(long)]
        uberproject: bool,
    },
    /// Set up automated synchronization at regular intervals
    Schedule {
        /// Sync interval in minutes
        #[arg(long)]
        interval: Option<u32>,
    },
    /// Upgrade ClaudeSync to the latest version
    Upgrade,
    /// Install completion for the specified shell
    InstallCompletion {
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with an AI provider
    Login {
        /// The provider to use for this project
        #[arg(long, value_enum, default_value = "claude.ai")]
        provider: ProviderChoice,
        /// Directly provide the Claude.ai session key
        #[arg(long, env = "CLAUDE_SESSION_KEY")]
        session_key: Option<String>,
        /// Automatically approve the suggested expiry time
        #[arg(long)]
        auto_approve: bool,
    },
    /// Log out from all AI providers
    Logout,
    /// List all authenticated providers
    Ls,
}

#[derive(Subcommand)]
enum OrganizationCommands {
    /// List all available organizations with required capabilities
    Ls,
    /// Set the active organization
    Set {
        /// ID of the organization to set as active
        #[arg(long)]
        org_id: Option<String>,
        /// Specify the provider for repositories without .claudesync
        #[arg(long, value_enum, default_value = "claude.ai")]
        provider: ProviderChoice,
    },
}

#[derive(Subcommand)]
enum ProjectCommands {
    /// Initialize a new project configuration
    Init {
        /// The name of the project (defaults to current directory name)
        #[arg(long)]
        name: Option<String>,
        /// The project description
        #[arg(long)]
        description: Option<String>,
        /// The local path for the project (defaults to current working directory)
        #[arg(long)]
        local_path: Option<String>,
        /// Create a new remote project on Claude.ai
        #[arg(long)]
        new: bool,
        /// The provider to use for this project
        #[arg(long, value_enum, default_value = "claude.ai")]
        provider: ProviderChoice,
    },
    /// Create a new project (alias for 'init --new')
    Create {
        /// The name of the project (defaults to current directory name)
        #[arg(long)]
        name: Option<String>,
        /// The project description
        #[arg(long)]
        description: Option<String>,
        /// The local path for the project (defaults to current working directory)
        #[arg(long)]
        local_path: Option<String>,
        /// The provider to use for this project
        #[arg(long, value_enum, default_value = "claude.ai")]
        provider: ProviderChoice,
    },
    /// Archive existing projects
    Archive {
        /// Archive all active projects
        #[arg(short = 'a', long = "all")]
        archive_all: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Set the active project for syncing
    Set {
        /// Include submodule projects in the selection
        #[arg(short = 'a', long = "all")]
        show_all: bool,
        /// UUID of the project to set as active (skips interactive selection)
        #[arg(long)]
        project_id: Option<String>,
        /// Specify the provider for repositories without .claudesync
        #[arg(long, value_enum, default_value = "claude.ai")]
        provider: ProviderChoice,
    },
    /// List all projects in the active organization
    Ls {
        /// Include archived projects in the list
        #[arg(short = 'a', long = "all")]
        show_all: bool,
    },
    /// Truncate one or all projects
    Truncate {
        /// Include archived projects
        #[arg(short = 'a', long)]
        include_archived: bool,
        /// Truncate all projects
        #[arg(long = "all")]
        truncate_all: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Manage submodules within the current project
    Submodule {
        #[command(subcommand)]
        command: SubmoduleCommands,
    },
    /// Manage remote project files
    File {
        #[command(subcommand)]
        command: FileCommands,
    },
}

#[derive(Subcommand)]
enum SubmoduleCommands {
    /// List all detected submodules in the current project
    Ls,
    /// Creates new projects for each detected submodule that doesn't already exist remotely
    Create,
}

#[derive(Subcommand)]
enum FileCommands {
    /// List files in the active remote project
    Ls,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a configuration value
    Set { key: String, value: String },
    /// Get a configuration value
    Get { key: String },
    /// List all configuration values
    Ls,
    /// Manage file categories
    Category {
        #[command(subcommand)]
        command: CategoryCommands,
    },
}

#[derive(Subcommand)]
enum CategoryCommands {
    /// Add a new file category
    Add {
        name: String,
        /// Description of the category
        #[arg(long, required = true)]
        description: String,
        /// File patterns for the category
        #[arg(long, required = true)]
        patterns: Vec<String>,
    },
    /// Remove a file category
    Rm { name: String },
    /// Update an existing file category
    Update {
        name: String,
        /// New description for the category
        #[arg(long)]
        description: Option<String>,
        /// New file patterns for the category
        #[arg(long)]
        patterns: Vec<String>,
    },
    /// List all file categories
    Ls,
    /// Set the default category for synchronization
    SetDefault { category: String },
}

#[derive(Subcommand)]
enum ChatCommands {
    /// Synchronize chats and their artifacts from the remote source
    Pull,
    /// List all chats
    Ls,
    /// Delete chat conversations
    Rm {
        /// Delete all chats
        #[arg(short = 'a', long = "all")]
        delete_all: bool,
    },
    /// Initializes a new chat conversation on the active provider
    Init {
        /// Name of the chat conversation
        #[arg(long, default_value = "")]
        name: String,
        /// UUID of the project to associate the chat with
        #[arg(long)]
        project: Option<String>,
    },
    /// Send a message to a specified chat or create a new chat and send the message
    Message {
        /// The message to send
        #[arg(required = true)]
        message: Vec<String>,
        /// UUID of the chat to send the message to
        #[arg(long)]
        chat: Option<String>,
        /// Timezone for the message
        #[arg(long, default_value = "UTC")]
        timezone: String,
        /// Model to use for the conversation (e.g. claude-3-5-haiku-20241022)
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List all web sessions
    Ls {
        /// Show all sessions including archived
        #[arg(short = 'a', long = "all")]
        show_all: bool,
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
    },
    /// Create a new Claude Code web session
    Create {
        /// Session title
        title: Option<String>,
        /// Environment ID (if not provided, will try to use active environment)
        #[arg(short, long)]
        environment_id: Option<String>,
        /// Model to use
        #[arg(short, long, default_value = crate::provider::DEFAULT_SESSION_MODEL)]
        model: String,
        /// Branch name to create (auto-generated if not provided)
        #[arg(short, long)]
        branch: Option<String>,
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
    },
    /// Archive existing sessions
    Archive {
        /// Archive all active sessions
        #[arg(short = 'a', long = "all")]
        archive_all: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Manage Claude Code environments
    Environment {
        #[command(subcommand)]
        command: EnvironmentCommands,
    },
    /// Manage Claude Code repository branches
    Branch {
        #[command(subcommand)]
        command: BranchCommands,
    },
}

#[derive(Subcommand)]
enum EnvironmentCommands {
    /// List all Claude Code environments
    Ls {
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
    },
}

#[derive(Subcommand)]
enum BranchCommands {
    /// List available repositories for Claude Code sessions
    Ls {
        /// Output in JSON format
        #[arg(long = "json")]
        json_output: bool,
        /// Filter repositories by name
        #[arg(short, long)]
        search: Option<String>,
    },
}

fn init_logging(config: &FileConfig) {
    let level = config
        .get_str("log_level")
        .unwrap_or_else(|| "INFO".to_string());
    let filter = match level.to_uppercase().as_str() {
        "DEBUG" => log::LevelFilter::Debug,
        "WARNING" | "WARN" => log::LevelFilter::Warn,
        "ERROR" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };
    env_logger::Builder::new()
        .filter_level(filter)
        .format_timestamp_secs()
        .init();
}

fn run(cli: Cli, config: &mut FileConfig) -> Result<()> {
    match cli.command {
        Commands::Auth { command } => match command {
            AuthCommands::Login {
                provider,
                session_key,
                auto_approve,
            } => cli::auth::login(config, provider.as_str(), session_key, auto_approve),
            AuthCommands::Logout => cli::auth::logout(config),
            AuthCommands::Ls => cli::auth::ls(config),
        },
        Commands::Organization { command } => match command {
            OrganizationCommands::Ls => cli::organization::ls(config),
            OrganizationCommands::Set { org_id, provider } => {
                cli::organization::set(config, org_id, provider.as_str())
            }
        },
        Commands::Project { command } => match command {
            ProjectCommands::Init {
                name,
                description,
                local_path,
                new,
                provider,
            } => cli::project::init(config, name, description, local_path, new, provider.as_str()),
            ProjectCommands::Create {
                name,
                description,
                local_path,
                provider,
            } => cli::project::init(config, name, description, local_path, true, provider.as_str()),
            ProjectCommands::Archive { archive_all, yes } => {
                cli::project::archive(config, archive_all, yes)
            }
            ProjectCommands::Set {
                show_all,
                project_id,
                provider,
            } => cli::project::set(config, show_all, project_id, provider.as_str()),
            ProjectCommands::Ls { show_all } => cli::project::ls(config, show_all),
            ProjectCommands::Truncate {
                include_archived,
                truncate_all,
                yes,
            } => cli::project::truncate(config, include_archived, truncate_all, yes),
            ProjectCommands::Submodule { command } => match command {
                SubmoduleCommands::Ls => cli::submodule::ls(config),
                SubmoduleCommands::Create => cli::submodule::create(config),
            },
            ProjectCommands::File { command } => match command {
                FileCommands::Ls => cli::project::file_ls(config),
            },
        },
        Commands::Config { command } => match command {
            ConfigCommands::Set { key, value } => cli::config_cmd::set(config, &key, &value),
            ConfigCommands::Get { key } => cli::config_cmd::get(config, &key),
            ConfigCommands::Ls => cli::config_cmd::ls(config),
            ConfigCommands::Category { command } => match command {
                CategoryCommands::Add {
                    name,
                    description,
                    patterns,
                } => cli::config_cmd::category_add(config, &name, &description, patterns),
                CategoryCommands::Rm { name } => cli::config_cmd::category_rm(config, &name),
                CategoryCommands::Update {
                    name,
                    description,
                    patterns,
                } => cli::config_cmd::category_update(
                    config,
                    &name,
                    description.as_deref(),
                    if patterns.is_empty() {
                        None
                    } else {
                        Some(patterns)
                    },
                ),
                CategoryCommands::Ls => cli::config_cmd::category_ls(config),
                CategoryCommands::SetDefault { category } => {
                    cli::config_cmd::category_set_default(config, &category)
                }
            },
        },
        Commands::Chat { command } => match command {
            ChatCommands::Pull => cli::chat::pull(config),
            ChatCommands::Ls => cli::chat::ls(config),
            ChatCommands::Rm { delete_all } => cli::chat::rm(config, delete_all),
            ChatCommands::Init { name, project } => cli::chat::init(config, &name, project),
            ChatCommands::Message {
                message,
                chat,
                timezone,
                model,
            } => cli::chat::message(config, &message, chat, &timezone, model),
        },
        Commands::Session { command } => match command {
            SessionCommands::Ls {
                show_all,
                json_output,
            } => cli::session::ls(config, show_all, json_output),
            SessionCommands::Create {
                title,
                environment_id,
                model,
                branch,
                json_output,
            } => cli::session::create(config, title, environment_id, &model, branch, json_output),
            SessionCommands::Archive { archive_all, yes } => {
                cli::session::archive(config, archive_all, yes)
            }
            SessionCommands::Environment { command } => match command {
                EnvironmentCommands::Ls { json_output } => {
                    cli::session::environment_ls(config, json_output)
                }
            },
            SessionCommands::Branch { command } => match command {
                BranchCommands::Ls {
                    json_output,
                    search,
                } => cli::session::branch_ls(config, json_output, search),
            },
        },
        Commands::Push {
            category,
            uberproject,
            dryrun,
        } => cli::push::push(config, category, uberproject, dryrun),
        Commands::Embedding {
            category,
            uberproject,
        } => cli::push::embedding(config, category, uberproject),
        Commands::Schedule { interval } => cli::schedule::schedule(interval),
        Commands::Upgrade => {
            println!(
                "The Rust version of ClaudeSync is upgraded via cargo or your package manager:"
            );
            println!("  cargo install --path .   (from the rust/ directory of the repository)");
            println!("Your session key and configuration are preserved across upgrades.");
            Ok(())
        }
        Commands::InstallCompletion { shell } => {
            let shell = shell
                .or_else(Shell::from_env)
                .unwrap_or(Shell::Bash);
            println!("# Shell is set to '{shell}'");
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "claudesync", &mut std::io::stdout());
            eprintln!("# Completion script written to stdout. Append it to your shell's rc file.");
            Ok(())
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let mut config = match FileConfig::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };
    init_logging(&config);

    if let Err(e) = run(cli, &mut config) {
        // ConfigurationError / ProviderError are reported gently, like the
        // Python `handle_errors` decorator; anything else is a hard failure.
        if e.is_handled() {
            println!("Error: {e}");
        } else {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
