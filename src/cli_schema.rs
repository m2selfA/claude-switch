use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cswitch",
    about = "Multi-account profile manager for Claude Code",
    long_about = "Manage multiple Claude Code accounts using per-profile env vars or isolated directories.",
    version,
    after_help = "\
Quick start:
  cswitch add work           Create a lightweight profile (env vars)
  cswitch add --full work    Create a full directory-isolated profile
  cswitch use work           Launch Claude with a profile
  cswitch list               Show all profiles
  cswitch                    Open interactive TUI (press ? for help)"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Open the interactive TUI (default when no command given)
    Ui,

    /// List all saved profiles
    List,

    /// Add a new profile — lightweight (env vars) by default, or full with --full
    Add {
        /// Profile name (any characters, including Chinese)
        name: String,
        /// Short CLI-friendly alias (alphanumeric, hyphens, underscores)
        #[arg(short, long)]
        alias: Option<String>,
        /// Overwrite if profile already exists
        #[arg(short, long)]
        force: bool,
        /// Use full directory isolation instead of lightweight env-var isolation
        #[arg(long)]
        full: bool,
    },

    /// Remove a saved profile
    Remove {
        /// Profile name to remove
        name: String,
    },

    /// Launch Claude Code with a specific profile
    Use {
        /// Profile name to use
        name: String,
        /// Skip profile's stored launch args (e.g. --dangerously-skip-permissions)
        #[arg(long = "no-extras")]
        no_extras: bool,
        /// Local gateway mode for localhost/LAN self-hosted lightweight profiles: auto, search-fetch, fetch-only, or gateway-only
        #[arg(long = "local-gateway-mode")]
        local_gateway_mode: Option<String>,
        /// Additional passthrough args passed directly to claude (use -- to separate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show details for a specific profile
    Info {
        /// Profile name
        name: String,
    },

    /// Print/sync shell aliases and shims locally and/or on remote hosts
    Aliases {
        /// Generate/sync local aliases and shims (implied when neither --local nor --remote are given)
        #[arg(long)]
        local: bool,

        /// Sync self-contained shims to a remote host via sftp; repeatable
        #[arg(long)]
        remote: Vec<String>,

        /// Show detailed alias/shim sync progress
        #[arg(short, long)]
        verbose: bool,
    },

    /// Run local registry, generated-artifact, and runtime diagnostics
    Doctor {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Exit non-zero when warnings or errors are present
        #[arg(long)]
        strict: bool,
    },

    /// Inspect claude-switch configuration paths and counts
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Print a compact current-profile summary for shell prompts and status bars
    Statusline {
        /// Profile name, alias, or id to summarize
        #[arg(short, long)]
        profile: Option<String>,
        /// Directory used to resolve project profile markers
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Print shell integration snippets and resolve project profile markers
    Shell {
        #[command(subcommand)]
        command: ShellCommands,
    },

    /// Manage shared providers and provider/profile links
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },

    /// Manage MCP servers and lightweight-profile MCP links
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },

    /// Inspect and switch running lightweight Claude sessions
    Process {
        #[command(subcommand)]
        command: ProcessCommands,
    },

    #[command(hide = true)]
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommands,
    },

    #[command(hide = true)]
    Shim {
        #[command(subcommand)]
        command: ShimCommands,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommands {
    /// Show registry paths, generated artifact paths, and object counts
    Inspect {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Show or update global claude-switch settings
    Settings {
        #[command(subcommand)]
        command: ConfigSettingsCommands,
    },

    /// Export profiles, providers, and MCP registry entries as a portable bundle
    Export {
        /// Profile names, aliases, or ids to include; omitted means all profiles
        #[arg(long = "profile")]
        profiles: Vec<String>,
        /// Output JSON file; omitted prints to stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Include API tokens and provider keys in the export
        #[arg(long)]
        include_secrets: bool,
    },

    /// Import a portable config bundle produced by `cswitch config export`
    Import {
        /// Input bundle JSON file
        input: PathBuf,
        /// Replace same-id entries instead of failing
        #[arg(long)]
        replace: bool,
        /// Show the import plan without writing the registry
        #[arg(long)]
        dry_run: bool,
        /// Emit machine-readable JSON for --dry-run
        #[arg(long)]
        json: bool,
    },

    /// Validate a config bundle without importing it
    Validate {
        /// Input bundle JSON file
        input: PathBuf,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Exit non-zero when warnings are present; errors always fail
        #[arg(long)]
        strict: bool,
    },

    /// Recover registry entries from generated claude-* shims
    RecoverShims {
        /// Directory containing generated claude-* shim files
        shim_dir: PathBuf,
        /// Write recovered entries into registry.json; omitted means preview only
        #[arg(long)]
        write: bool,
        /// Replace existing profiles with matching name or alias
        #[arg(long)]
        replace: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Migrate token-based Claude settings auth to apiKeyHelper
    MigrateAuth {
        /// Write changes to settings files; omitted means preview only
        #[arg(long)]
        write: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Also migrate ~/.claude/settings.json on remote hosts; repeatable
        #[arg(long)]
        remote: Vec<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigSettingsCommands {
    /// Show persisted global settings
    Show {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Update persisted global settings
    Set {
        /// Allow runtime hot-switch for localhost/LAN self-hosted APIs
        #[arg(long)]
        allow_local_runtime_hot_switch: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ShellCommands {
    /// Print a shell wrapper that auto-selects profiles from project markers
    Hook {
        /// Shell name: auto, powershell, bash, zsh, or fish
        #[arg(long, default_value = "auto")]
        shell: String,
    },

    /// Print the profile selected by .cswitch-profile or .claudeprofile
    Current {
        /// Directory or file path used as the project lookup starting point
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProviderCommands {
    /// List all shared providers
    List,

    /// Add a shared provider (base_url + api_key)
    Add {
        /// Human-readable name for this provider
        name: String,
        /// API base URL
        #[arg(short, long)]
        url: String,
        /// API key (ANTHROPIC_AUTH_TOKEN)
        #[arg(short, long)]
        key: String,
    },

    /// Remove a shared provider
    Remove {
        /// Provider ID to remove
        id: String,
    },

    /// Edit a provider's name or base URL
    Edit {
        /// Provider ID
        id: String,
        /// New name
        #[arg(short, long)]
        name: Option<String>,
        /// New base URL
        #[arg(short, long)]
        url: Option<String>,
    },

    /// List keys for a provider
    Keys {
        /// Provider ID
        id: String,
    },

    /// Add a key to a provider
    AddKey {
        /// Provider ID
        id: String,
        /// Key name
        #[arg(short, long)]
        name: String,
        /// API key value
        #[arg(short, long)]
        key: String,
    },

    /// Edit a key's name or token
    EditKey {
        /// Provider ID
        id: String,
        /// Key ID
        key_id: String,
        /// New key name
        #[arg(short, long)]
        name: Option<String>,
        /// New API key value
        #[arg(short, long)]
        key: Option<String>,
    },

    /// Rename a key without changing its token
    RenameKey {
        /// Provider ID
        id: String,
        /// Key ID
        key_id: String,
        /// New key name
        #[arg(short, long)]
        name: String,
    },

    /// Remove a key from a provider
    RemoveKey {
        /// Provider ID
        id: String,
        /// Key ID to remove
        key_id: String,
    },

    /// Link a profile to a provider and key
    Link {
        /// Profile name or alias to link
        profile: String,
        /// Provider ID
        #[arg(short, long)]
        provider: String,
        /// Key ID within the provider
        #[arg(short, long)]
        key: String,
    },

    /// Remove provider/key association from a profile
    Unlink {
        /// Profile name or alias to unlink
        profile: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum McpCommands {
    /// List all MCP servers
    List,

    /// Show one MCP server
    Show {
        /// MCP id or name
        query: String,
    },

    /// Add an MCP server
    Add {
        /// MCP server name
        name: String,
        /// MCP type: stdio, http, streamable-http, or sse
        #[arg(long = "type", default_value = "stdio")]
        server_type: String,
        /// stdio command
        #[arg(long)]
        command: Option<String>,
        /// stdio argument; repeatable
        #[arg(long = "arg", allow_hyphen_values = true)]
        args: Vec<String>,
        /// Environment entry KEY=VALUE; repeatable
        #[arg(long = "env")]
        env: Vec<String>,
        /// stdio working directory
        #[arg(long)]
        cwd: Option<String>,
        /// Remote MCP URL
        #[arg(long)]
        url: Option<String>,
        /// HTTP header KEY=VALUE; repeatable
        #[arg(long = "header")]
        headers: Vec<String>,
        /// OAuth object as JSON
        #[arg(long = "oauth-json")]
        oauth_json: Option<String>,
        /// Command that returns dynamic headers JSON
        #[arg(long = "headers-helper")]
        headers_helper: Option<String>,
        /// MCP call timeout in milliseconds
        #[arg(long)]
        timeout: Option<u64>,
        /// Whether Claude loads tools at startup
        #[arg(long = "always-load")]
        always_load: Option<bool>,
        /// Temporarily disable this server
        #[arg(long)]
        disabled: Option<bool>,
    },

    /// Edit an MCP server
    Edit {
        /// MCP id or name
        query: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "type")]
        server_type: Option<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "clear-command")]
        clear_command: bool,
        #[arg(long = "arg", allow_hyphen_values = true)]
        args: Vec<String>,
        #[arg(long = "clear-args")]
        clear_args: bool,
        #[arg(long = "env")]
        env: Vec<String>,
        #[arg(long = "clear-env")]
        clear_env: bool,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long = "clear-cwd")]
        clear_cwd: bool,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "clear-url")]
        clear_url: bool,
        #[arg(long = "header")]
        headers: Vec<String>,
        #[arg(long = "clear-headers")]
        clear_headers: bool,
        #[arg(long = "oauth-json")]
        oauth_json: Option<String>,
        #[arg(long = "clear-oauth")]
        clear_oauth: bool,
        #[arg(long = "headers-helper")]
        headers_helper: Option<String>,
        #[arg(long = "clear-headers-helper")]
        clear_headers_helper: bool,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(long = "clear-timeout")]
        clear_timeout: bool,
        #[arg(long = "always-load")]
        always_load: Option<bool>,
        #[arg(long = "clear-always-load")]
        clear_always_load: bool,
        #[arg(long)]
        disabled: Option<bool>,
        #[arg(long = "clear-disabled")]
        clear_disabled: bool,
    },

    /// Remove an MCP server
    Remove {
        /// MCP id or name
        query: String,
    },

    /// Link MCP servers to a lightweight profile
    Link {
        /// Profile name, alias, or id
        profile: String,
        /// MCP ids or names
        mcps: Vec<String>,
        /// Replace the profile MCP selection instead of appending
        #[arg(long)]
        replace: bool,
    },

    /// Unlink MCP servers from a lightweight profile
    Unlink {
        /// Profile name, alias, or id
        profile: String,
        /// MCP ids or names
        mcps: Vec<String>,
        /// Remove all selected MCP servers
        #[arg(long)]
        all: bool,
    },

    /// Export saved MCP servers as Claude-compatible mcp.json content
    Export {
        /// MCP ids or names; omitted means all servers
        queries: Vec<String>,
        /// Export all servers explicitly
        #[arg(long)]
        all: bool,
        /// Write JSON to this file instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Import MCP servers from a Claude-compatible mcp.json file
    Import {
        /// Input JSON file containing mcpServers
        input: PathBuf,
        /// Update same-name MCP servers instead of failing
        #[arg(long)]
        replace: bool,
    },

    /// Validate saved MCP server entries
    Validate {
        /// MCP ids or names; omitted means all servers
        queries: Vec<String>,
        /// Validate all servers explicitly
        #[arg(long)]
        all: bool,
        /// Exit non-zero when warnings are present; errors always fail
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProcessCommands {
    /// List runtime-managed Claude sessions
    List,

    /// Show one runtime-managed Claude session
    Inspect {
        /// Session id
        session_id: String,
    },

    /// Switch a running Claude session to a provider/key/model
    Switch {
        /// Session id
        session_id: String,
        /// Provider id
        #[arg(short, long)]
        provider: String,
        /// Key id
        #[arg(short, long)]
        key: String,
        /// Model id to set as ANTHROPIC_MODEL
        #[arg(short, long)]
        model: String,
    },

    /// Remove stale runtime session directories
    Gc,
}

#[derive(Subcommand)]
pub(crate) enum RuntimeCommands {
    /// Print the auth token for one runtime session
    Auth {
        /// Session id
        session_id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ShimCommands {
    /// Probe runtime shim-launch support
    Launch {
        /// Probe only; do not launch anything
        #[arg(long)]
        probe: bool,
        /// Profile id
        #[arg(long = "profile-id")]
        profile_id: Option<String>,
        /// Skip stored launch args
        #[arg(long = "no-extras")]
        no_extras: bool,
        /// Local gateway mode for localhost/LAN self-hosted lightweight profiles
        #[arg(long = "local-gateway-mode")]
        local_gateway_mode: Option<String>,
        /// Additional passthrough args passed directly to claude (use -- to separate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}
