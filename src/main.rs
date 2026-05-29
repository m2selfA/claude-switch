mod cli_args;
mod env_vars;
mod profile;
mod tui;

use anyhow::Result;
use anyhow::bail;
use clap::{Parser, Subcommand};
use profile::{
    ConfigBundleValidation, ConfigImportPlan, ConfigImportSummary, ConfigInspection,
    DiagnosticItem, DiagnosticLevel, DoctorReport, LightweightEnv, McpServerInput, McpServerUpdate,
    McpValidationIssue, ProfileManager, ShimRecoveryPlan, ShimRecoverySummary, StatuslineInfo,
    fetch_models,
};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show registry paths, generated artifact paths, and object counts
    Inspect {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
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
}

#[derive(Subcommand)]
enum ShellCommands {
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
enum ProviderCommands {
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
enum McpCommands {
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let manager = ProfileManager::new()?;

    match cli.command {
        None | Some(Commands::Ui) => {
            let app = tui::App::new(manager)?;
            app.run()?;
        }

        Some(Commands::List) => {
            let profiles = manager.list_profiles()?;
            if profiles.is_empty() {
                println!("No profiles found. Add one with:");
                println!("  cswitch add <name>");
                return Ok(());
            }

            println!(
                "{:<20} {:<12} {:<8} {:<16} LAST USED",
                "NAME", "ALIAS", "KIND", "PROVIDER"
            );
            println!("{}", "─".repeat(80));
            for p in profiles {
                let kind = if p.kind == profile::ProfileKind::Full {
                    "full"
                } else {
                    "lite"
                };
                let alias = p.alias.as_deref().unwrap_or("—");
                let prov_name = if p.kind == profile::ProfileKind::Lightweight {
                    p.provider_id
                        .as_ref()
                        .and_then(|pid| manager.get_provider(pid).ok())
                        .map(|prov| prov.name)
                        .unwrap_or_else(|| "—".to_string())
                } else {
                    "—".to_string()
                };
                let last_used = p
                    .last_used
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or("never".to_string());
                println!(
                    "{:<20} {:<12} {:<8} {:<16} {}",
                    p.name, alias, kind, prov_name, last_used
                );
            }
        }

        Some(Commands::Add {
            name,
            alias,
            force,
            full,
        }) => {
            handle_add(&manager, &name, alias.as_deref(), force, full)?;
            sync_shims(&manager);
        }

        Some(Commands::Remove { name }) => match manager.remove_profile(&name) {
            Ok(_) => {
                println!("Profile '{}' removed.", name);
                sync_shims(&manager);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },

        Some(Commands::Use {
            name,
            no_extras,
            args,
        }) => {
            manager.launch_claude(&name, &args, !no_extras)?;
        }

        Some(Commands::Info { name }) => match manager.get_profile(&name) {
            Ok(p) => {
                let kind = if p.kind == profile::ProfileKind::Full {
                    "full"
                } else {
                    "lightweight"
                };
                println!("Id:        {}", p.id);
                println!("Name:      {}", p.name);
                if let Some(ref a) = p.alias {
                    println!("Alias:     {}", a);
                }
                println!("Kind:      {}", kind);
                println!("Added:     {}", p.added.format("%Y-%m-%d %H:%M UTC"));
                println!(
                    "Last used: {}",
                    p.last_used
                        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or("never".to_string())
                );
                if p.kind == profile::ProfileKind::Full {
                    let dir = manager.profile_dir(&p);
                    println!("Directory: {}", dir.display());
                    println!();
                    println!("Launch:");
                    println!("  CLAUDE_CONFIG_DIR='{}' claude", dir.display());
                } else {
                    if let Some(ref pid) = p.provider_id
                        && let Ok(prov) = manager.get_provider(pid)
                    {
                        println!("Provider:  {} ({})", prov.name, prov.id);
                        if let Some(ref kid) = p.key_id
                            && let Some(k) = prov.keys.get(kid)
                        {
                            println!("Key:       {} ({})", k.name, k.id);
                        }
                    }
                    println!();
                    println!("Environment variables set on launch:");
                    if let Some(ref env) = p.env {
                        if env.auth_token.is_some() {
                            println!("  ANTHROPIC_AUTH_TOKEN=***");
                        }
                        if env.base_url.is_some() {
                            println!(
                                "  ANTHROPIC_BASE_URL={}",
                                env.base_url.as_deref().unwrap_or("")
                            );
                        }
                        if let Some(ref m) = env.default_opus_model {
                            println!("  ANTHROPIC_DEFAULT_OPUS_MODEL={}", m);
                        }
                        if let Some(ref m) = env.default_sonnet_model {
                            println!("  ANTHROPIC_DEFAULT_SONNET_MODEL={}", m);
                        }
                        if let Some(ref m) = env.default_haiku_model {
                            println!("  ANTHROPIC_DEFAULT_HAIKU_MODEL={}", m);
                        }
                        if let Some(ref m) = env.model {
                            println!("  ANTHROPIC_MODEL={}", m);
                        }
                        if let Some(ref m) = env.subagent_model {
                            println!("  CLAUDE_CODE_SUBAGENT_MODEL={}", m);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },

        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            let run_local = local || remote.is_empty();
            let multi_section = run_local && !remote.is_empty() || remote.len() > 1;
            let mut errors: Vec<String> = Vec::new();

            if run_local {
                if verbose {
                    eprintln!("# local alias/shim sync target");
                }
                match manager.generate_aliases() {
                    Ok(report) => {
                        if multi_section {
                            println!("# Local aliases/shims");
                        }
                        println!("{}", report);
                    }
                    Err(err) => {
                        errors.push(format!("local: {err}"));
                    }
                }
            }

            for host in remote {
                if verbose {
                    eprintln!("# remote sync target: {}", host);
                }
                match manager.sync_remote_aliases_with_progress(&host, verbose, |line| {
                    eprintln!("{}", line);
                }) {
                    Ok(report) => {
                        if multi_section {
                            eprintln!("# Remote aliases/shims: {}", host);
                        }
                        eprintln!("{}", report);
                    }
                    Err(err) => {
                        let msg = format!("remote {}: {}", host, err);
                        eprintln!("# ERROR: {}", msg);
                        errors.push(msg);
                    }
                }
            }

            if !errors.is_empty() {
                bail!(
                    "alias sync completed with {} error(s):\n{}",
                    errors.len(),
                    errors
                        .iter()
                        .map(|e| format!("  - {e}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }

        Some(Commands::Doctor { json, strict }) => {
            let report = manager.doctor_report()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_doctor_report(&report);
            }
            if strict && (report.error_count() > 0 || report.warning_count() > 0) {
                bail!(
                    "doctor found {} error(s) and {} warning(s)",
                    report.error_count(),
                    report.warning_count()
                );
            }
        }

        Some(Commands::Config { command }) => match command {
            ConfigCommands::Inspect { json } => {
                let inspection = manager.inspect_config()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&inspection)?);
                } else {
                    print_config_inspection(&inspection);
                }
            }
            ConfigCommands::Export {
                profiles,
                output,
                include_secrets,
            } => {
                let content = manager.export_config_bundle(&profiles, include_secrets)?;
                write_or_print(&content, output.as_ref(), "Config bundle exported")?;
            }
            ConfigCommands::Import {
                input,
                replace,
                dry_run,
                json,
            } => {
                let content = fs::read_to_string(&input).map_err(|err| {
                    anyhow::anyhow!("Failed to read {}: {}", input.display(), err)
                })?;
                if dry_run {
                    let plan = manager.plan_config_bundle_import(&content, replace)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plan)?);
                    } else {
                        print_config_import_plan(&plan, &input);
                    }
                } else {
                    let summary = manager.import_config_bundle(&content, replace)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&summary)?);
                    } else {
                        print_config_import_summary(&summary, &input);
                    }
                    sync_shims(&manager);
                }
            }
            ConfigCommands::Validate {
                input,
                json,
                strict,
            } => {
                let content = fs::read_to_string(&input).map_err(|err| {
                    anyhow::anyhow!("Failed to read {}: {}", input.display(), err)
                })?;
                let validation = manager.validate_config_bundle(&content)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&validation)?);
                } else {
                    print_config_bundle_validation(&validation, &input);
                }
                if validation.error_count() > 0 || (strict && validation.warning_count() > 0) {
                    bail!(
                        "config bundle validation found {} error(s) and {} warning(s)",
                        validation.error_count(),
                        validation.warning_count()
                    );
                }
            }
            ConfigCommands::RecoverShims {
                shim_dir,
                write,
                replace,
                json,
            } => {
                if write {
                    let summary = manager.recover_shims(&shim_dir, replace)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&summary)?);
                    } else {
                        print_shim_recovery_summary(&summary);
                    }
                    sync_shims(&manager);
                } else {
                    let plan = manager.plan_shim_recovery(&shim_dir, replace)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&plan)?);
                    } else {
                        print_shim_recovery_plan(&plan);
                    }
                }
            }
        },

        Some(Commands::Statusline { profile, dir, json }) => {
            let info = manager.statusline_info(profile.as_deref(), dir.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("{}", render_statusline(&info));
            }
        }

        Some(Commands::Shell { command }) => match command {
            ShellCommands::Hook { shell } => {
                println!("{}", render_shell_hook(&shell)?);
            }
            ShellCommands::Current { dir } => {
                if let Some(profile) = manager.resolve_project_profile(&dir)? {
                    println!("{}", profile.alias.as_deref().unwrap_or(&profile.name));
                } else {
                    std::process::exit(1);
                }
            }
        },

        Some(Commands::Provider { command }) => match command {
            ProviderCommands::List => {
                let providers = manager.list_providers()?;
                if providers.is_empty() {
                    println!("No providers found.");
                    println!(
                        "Providers are auto-extracted from lightweight profiles on first load."
                    );
                    return Ok(());
                }
                println!(
                    "{:<14} {:<22} {:<40} {:<8}",
                    "ID", "NAME", "BASE URL", "KEYS"
                );
                println!("{}", "─".repeat(90));
                for prov in providers {
                    println!(
                        "{:<14} {:<22} {:<40} {:<8}",
                        prov.id,
                        prov.name,
                        prov.base_url,
                        prov.keys.len()
                    );
                }
            }

            ProviderCommands::Add { name, url, key } => {
                match manager.add_provider(&name, &url, &key) {
                    Ok(prov) => {
                        println!("Provider '{}' ({}) added.", prov.name, prov.id);
                        sync_shims(&manager);
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            ProviderCommands::Remove { id } => match manager.remove_provider(&id) {
                Ok(_) => {
                    println!("Provider '{}' removed.", id);
                    sync_shims(&manager);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            },

            ProviderCommands::Edit { id, name, url } => {
                let provider = manager.get_provider(&id)?;
                let new_name = name.unwrap_or(provider.name);
                let new_url = url.unwrap_or(provider.base_url);
                manager.update_provider(&id, &new_name, &new_url)?;
                println!("Provider '{}' updated.", id);
                sync_shims(&manager);
            }

            ProviderCommands::Keys { id } => {
                let keys = manager.list_keys(&id)?;
                if keys.is_empty() {
                    println!("No keys found for provider '{}'.", id);
                    return Ok(());
                }
                println!("{:<14} {:<22} API KEY", "KEY ID", "NAME");
                println!("{}", "─".repeat(70));
                for k in keys {
                    let masked = if k.api_key.len() > 12 {
                        format!(
                            "{}...{}",
                            &k.api_key[..6],
                            &k.api_key[k.api_key.len() - 6..]
                        )
                    } else {
                        k.api_key.clone()
                    };
                    println!("{:<14} {:<22} {}", k.id, k.name, masked);
                }
            }

            ProviderCommands::AddKey { id, name, key } => match manager.add_key(&id, &name, &key) {
                Ok(k) => {
                    println!("Key '{}' ({}) added to provider '{}'.", k.name, k.id, id);
                    sync_shims(&manager);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            },

            ProviderCommands::EditKey {
                id,
                key_id,
                name,
                key,
            } => {
                let keys = manager.list_keys(&id)?;
                let existing = keys
                    .iter()
                    .find(|k| k.id == key_id)
                    .ok_or_else(|| anyhow::anyhow!("Key '{}' not found.", key_id))?;
                let new_name = name.unwrap_or(existing.name.clone());
                let new_key = key.unwrap_or(existing.api_key.clone());
                manager.update_key(&id, &key_id, &new_name, &new_key)?;
                println!("Key '{}' updated.", key_id);
                sync_shims(&manager);
            }

            ProviderCommands::RemoveKey { id, key_id } => match manager.remove_key(&id, &key_id) {
                Ok(_) => {
                    println!("Key '{}' removed from provider '{}'.", key_id, id);
                    sync_shims(&manager);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            },

            ProviderCommands::Link {
                profile,
                provider,
                key,
            } => {
                let prov = manager
                    .get_provider(&provider)
                    .map_err(|_| anyhow::anyhow!("Provider '{}' not found.", provider))?;
                if !prov.keys.contains_key(&key) {
                    eprintln!("Error: Key '{}' not found in provider '{}'.", key, provider);
                    std::process::exit(1);
                }
                manager.set_provider(&profile, &provider, &key)?;
                println!(
                    "Profile '{}' linked to provider '{}' with key '{}'.",
                    profile, prov.name, key
                );
                sync_shims(&manager);
            }

            ProviderCommands::Unlink { profile } => {
                manager.unset_provider(&profile)?;
                println!("Provider association removed from profile '{}'.", profile);
                sync_shims(&manager);
            }
        },

        Some(Commands::Mcp { command }) => match command {
            McpCommands::List => {
                let servers = manager.list_mcp_servers()?;
                if servers.is_empty() {
                    println!("No MCP servers found.");
                    println!("Add one with: cswitch mcp add <name> --command <cmd>");
                    return Ok(());
                }
                println!(
                    "{:<14} {:<24} {:<16} {:<8} TARGET",
                    "ID", "NAME", "TYPE", "DISABLED"
                );
                println!("{}", "─".repeat(96));
                for server in servers {
                    let target = server
                        .command
                        .as_deref()
                        .or(server.url.as_deref())
                        .unwrap_or("—");
                    println!(
                        "{:<14} {:<24} {:<16} {:<8} {}",
                        server.id,
                        server.name,
                        server.server_type,
                        server.disabled.unwrap_or(false),
                        target
                    );
                }
            }
            McpCommands::Show { query } => {
                let server = manager.get_mcp_server(&query)?;
                println!("Id:             {}", server.id);
                println!("Name:           {}", server.name);
                println!("Type:           {}", server.server_type);
                if let Some(command) = server.command {
                    println!("Command:        {}", command);
                }
                if !server.args.is_empty() {
                    println!("Args:           {}", server.args.join(" "));
                }
                if !server.env.is_empty() {
                    println!("Env:            {} entrie(s)", server.env.len());
                }
                if let Some(cwd) = server.cwd {
                    println!("Cwd:            {}", cwd);
                }
                if let Some(url) = server.url {
                    println!("Url:            {}", url);
                }
                if !server.headers.is_empty() {
                    println!("Headers:        {} entrie(s)", server.headers.len());
                }
                if server.oauth.is_some() {
                    println!("OAuth:          configured");
                }
                if let Some(headers_helper) = server.headers_helper {
                    println!("Headers helper: {}", headers_helper);
                }
                if let Some(timeout) = server.timeout {
                    println!("Timeout:        {}", timeout);
                }
                if let Some(always_load) = server.always_load {
                    println!("Always load:    {}", always_load);
                }
                if let Some(disabled) = server.disabled {
                    println!("Disabled:       {}", disabled);
                }
            }
            McpCommands::Add {
                name,
                server_type,
                command,
                args,
                env,
                cwd,
                url,
                headers,
                oauth_json,
                headers_helper,
                timeout,
                always_load,
                disabled,
            } => {
                let server = manager.add_mcp_server(McpServerInput {
                    name,
                    server_type,
                    command,
                    args,
                    env: parse_key_values(&env, "--env")?,
                    cwd,
                    url,
                    headers: parse_key_values(&headers, "--header")?,
                    oauth: parse_optional_json(oauth_json.as_deref(), "--oauth-json")?,
                    headers_helper,
                    timeout,
                    always_load,
                    disabled,
                })?;
                println!("MCP '{}' ({}) added.", server.name, server.id);
                sync_shims(&manager);
            }
            McpCommands::Edit {
                query,
                name,
                server_type,
                command,
                clear_command,
                args,
                clear_args,
                env,
                clear_env,
                cwd,
                clear_cwd,
                url,
                clear_url,
                headers,
                clear_headers,
                oauth_json,
                clear_oauth,
                headers_helper,
                clear_headers_helper,
                timeout,
                clear_timeout,
                always_load,
                clear_always_load,
                disabled,
                clear_disabled,
            } => {
                let update = McpServerUpdate {
                    name,
                    server_type,
                    command: optional_field(command, clear_command),
                    args: if clear_args || !args.is_empty() {
                        Some(args)
                    } else {
                        None
                    },
                    env: if clear_env || !env.is_empty() {
                        Some(parse_key_values(&env, "--env")?)
                    } else {
                        None
                    },
                    cwd: optional_field(cwd, clear_cwd),
                    url: optional_field(url, clear_url),
                    headers: if clear_headers || !headers.is_empty() {
                        Some(parse_key_values(&headers, "--header")?)
                    } else {
                        None
                    },
                    oauth: if clear_oauth || oauth_json.is_some() {
                        Some(parse_optional_json(oauth_json.as_deref(), "--oauth-json")?)
                    } else {
                        None
                    },
                    headers_helper: optional_field(headers_helper, clear_headers_helper),
                    timeout: optional_field(timeout, clear_timeout),
                    always_load: optional_field(always_load, clear_always_load),
                    disabled: optional_field(disabled, clear_disabled),
                };
                let server = manager.update_mcp_server(&query, update)?;
                println!("MCP '{}' ({}) updated.", server.name, server.id);
                sync_shims(&manager);
            }
            McpCommands::Remove { query } => {
                manager.remove_mcp_server(&query)?;
                println!("MCP '{}' removed.", query);
                sync_shims(&manager);
            }
            McpCommands::Link {
                profile,
                mcps,
                replace,
            } => {
                if mcps.is_empty() {
                    bail!("Provide at least one MCP id or name.");
                }
                let updated = if replace {
                    manager.set_profile_mcps(&profile, &mcps)?
                } else {
                    manager.add_profile_mcps(&profile, &mcps)?
                };
                println!(
                    "Profile '{}' now has {} MCP server(s).",
                    updated.name,
                    updated.mcp_server_ids.len()
                );
                sync_shims(&manager);
            }
            McpCommands::Unlink { profile, mcps, all } => {
                if !all && mcps.is_empty() {
                    bail!("Provide MCP ids/names or use --all.");
                }
                let updated = manager.remove_profile_mcps(&profile, &mcps, all)?;
                println!(
                    "Profile '{}' now has {} MCP server(s).",
                    updated.name,
                    updated.mcp_server_ids.len()
                );
                sync_shims(&manager);
            }
            McpCommands::Export {
                queries,
                all,
                output,
            } => {
                let content = manager.export_mcp_config(&queries, all)?;
                if let Some(output) = output {
                    write_or_print(&content, Some(&output), "MCP config exported")?;
                } else {
                    println!("{content}");
                }
            }
            McpCommands::Import { input, replace } => {
                let content = fs::read_to_string(&input).map_err(|err| {
                    anyhow::anyhow!("Failed to read {}: {}", input.display(), err)
                })?;
                let imported = manager.import_mcp_config(&content, replace)?;
                println!(
                    "Imported {} MCP server(s) from {}.",
                    imported.len(),
                    input.display()
                );
                for server in imported {
                    println!("  {} ({})", server.name, server.id);
                }
                sync_shims(&manager);
            }
            McpCommands::Validate {
                queries,
                all,
                strict,
            } => {
                let issues = manager.validate_mcp_servers(&queries, all)?;
                print_mcp_validation(&issues);
                let has_errors = issues
                    .iter()
                    .any(|issue| issue.level == DiagnosticLevel::Error);
                let has_warnings = issues
                    .iter()
                    .any(|issue| issue.level == DiagnosticLevel::Warn);
                if has_errors || (strict && has_warnings) {
                    bail!(
                        "MCP validation found {} error(s) and {} warning(s)",
                        issues
                            .iter()
                            .filter(|issue| issue.level == DiagnosticLevel::Error)
                            .count(),
                        issues
                            .iter()
                            .filter(|issue| issue.level == DiagnosticLevel::Warn)
                            .count()
                    );
                }
            }
        },
    }

    Ok(())
}

/// Smart add: creates a lightweight profile (default) or full directory-isolated profile.
fn handle_add(
    manager: &ProfileManager,
    name: &str,
    alias: Option<&str>,
    force: bool,
    full: bool,
) -> Result<()> {
    if full {
        // Full isolation: copy ~/.claude directory
        let result = if force {
            manager.add_profile_force(name, alias)
        } else {
            manager.add_profile(name, alias)
        };
        match result {
            Ok(p) => {
                println!("Profile '{}' added (full isolation).", p.name);
                println!("  Launch with: cswitch use {}", p.name);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Ok(())
    } else {
        // Lightweight flow
        println!(
            "Creating lightweight profile '{}' (env-var based isolation).\n",
            name
        );

        print!("ANTHROPIC_AUTH_TOKEN: ");
        io::stdout().flush()?;
        let mut token = String::new();
        io::stdin().read_line(&mut token)?;
        let token = token.trim().to_string();
        if token.is_empty() {
            eprintln!("Error: ANTHROPIC_AUTH_TOKEN is required.");
            std::process::exit(1);
        }

        print!("ANTHROPIC_BASE_URL [https://api.anthropic.com]: ");
        io::stdout().flush()?;
        let mut base_url = String::new();
        io::stdin().read_line(&mut base_url)?;
        let base_url = base_url.trim().to_string();
        let base_url = if base_url.is_empty() {
            "https://api.anthropic.com".to_string()
        } else {
            base_url
        };

        // Try to fetch models
        println!("\nFetching available models from {}/v1/models...", base_url);
        let models = match fetch_models(&base_url, &token) {
            Ok(m) => {
                println!("Found {} models:", m.len());
                for (i, model) in m.iter().enumerate() {
                    println!("  {}. {}", i + 1, model);
                }
                println!();
                Some(m)
            }
            Err(e) => {
                eprintln!("Warning: could not fetch models: {}", e);
                eprintln!("You can enter model IDs manually.\n");
                None
            }
        };

        fn prompt_model(prompt: &str, models: &Option<Vec<String>>) -> Option<String> {
            print!("{}", prompt);
            io::stdout().flush().ok()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input).ok()?;
            let input = input.trim().to_string();
            if input.is_empty() {
                return None;
            }

            if let Some(models_list) = models
                && let Ok(idx) = input.parse::<usize>()
                && idx >= 1
                && idx <= models_list.len()
            {
                return Some(models_list[idx - 1].clone());
            }
            Some(input)
        }

        println!("Configure model settings (press Enter to skip any):");
        println!("  You can enter a model ID directly, or a number to select from the list above.");

        print!(
            "Append [1m] suffix to model IDs for Claude Code context window recognition? [y/N]: "
        );
        io::stdout().flush()?;
        let mut suffix = String::new();
        io::stdin().read_line(&mut suffix)?;
        let append_1m = suffix.trim().to_lowercase() == "y";

        let default_opus_model = prompt_model("  Default Opus Model []: ", &models);
        let default_sonnet_model = prompt_model("  Default Sonnet Model []: ", &models);
        let default_haiku_model = prompt_model("  Default Haiku Model []: ", &models);
        let model = prompt_model("  Default Model (ANTHROPIC_MODEL) []: ", &models);
        let subagent_model = prompt_model(
            "  Subagent Model (CLAUDE_CODE_SUBAGENT_MODEL) []: ",
            &models,
        );

        fn apply_suffix(val: Option<String>, append: bool) -> Option<String> {
            match (val, append) {
                (Some(v), true) if !v.ends_with("[1m]") => Some(format!("{}[1m]", v)),
                (v, _) => v,
            }
        }

        let env = LightweightEnv {
            auth_token: Some(token),
            base_url: Some(base_url.clone()),
            default_opus_model: apply_suffix(default_opus_model, append_1m),
            default_sonnet_model: apply_suffix(default_sonnet_model, append_1m),
            default_haiku_model: apply_suffix(default_haiku_model, append_1m),
            model: apply_suffix(model, append_1m),
            subagent_model: apply_suffix(subagent_model, append_1m),
            extras: Vec::new(),
        };

        match manager.create_lightweight_profile(name, alias, env) {
            Ok(p) => {
                // Auto-link to provider if matching base_url + token exists
                if let Some((prov, key)) = manager.find_provider_by_url_and_key(
                    &base_url,
                    p.env.as_ref().unwrap().auth_token.as_deref().unwrap_or(""),
                ) {
                    let _ = manager.set_provider(&p.id, &prov.id, &key.id);
                }
                println!("\nLightweight profile '{}' created.", p.name);
                println!("  Launch with: cswitch use {}", p.name);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Ok(())
    }
}

fn parse_key_values(
    entries: &[String],
    flag_name: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for entry in entries {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("{} expects KEY=VALUE, got '{}'.", flag_name, entry))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("{} entry has an empty key.", flag_name);
        }
        map.insert(key.to_string(), value.trim().to_string());
    }
    Ok(map)
}

fn parse_optional_json(input: Option<&str>, flag_name: &str) -> Result<Option<serde_json::Value>> {
    input
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|err| anyhow::anyhow!("{} must be valid JSON: {}", flag_name, err))
        })
        .transpose()
}

fn optional_field<T>(value: Option<T>, clear: bool) -> Option<Option<T>> {
    if clear { Some(None) } else { value.map(Some) }
}

fn print_doctor_report(report: &DoctorReport) {
    println!(
        "claude-switch doctor: {} error(s), {} warning(s)",
        report.error_count(),
        report.warning_count()
    );
    for item in &report.items {
        print_diagnostic_item(item);
    }
}

fn print_diagnostic_item(item: &DiagnosticItem) {
    println!(
        "{:<5} {:<12} {}",
        item.level.as_str().to_uppercase(),
        item.area,
        item.message
    );
    if let Some(hint) = &item.hint {
        println!("      {:<12} hint: {}", "", hint);
    }
}

fn print_config_inspection(inspection: &ConfigInspection) {
    println!(
        "Base dir:                 {}",
        inspection.base_dir.display()
    );
    println!(
        "Registry:                 {}",
        inspection.registry_path.display()
    );
    println!(
        "Profiles dir:             {}",
        inspection.profiles_dir.display()
    );
    println!(
        "Generated root:           {}",
        inspection.generated_root.display()
    );
    println!("Profiles:                 {}", inspection.profiles);
    println!(
        "  lightweight/full:       {}/{}",
        inspection.lightweight_profiles, inspection.full_profiles
    );
    println!("Providers:                {}", inspection.providers);
    println!("Provider keys:            {}", inspection.provider_keys);
    println!("MCP servers:              {}", inspection.mcp_servers);
    println!("Linked MCP refs:          {}", inspection.linked_mcp_refs);
    println!(
        "Generated MCP plugins:    {}",
        inspection.generated_mcp_plugins
    );
    println!(
        "Generated TinyFish dirs:  {}",
        inspection.generated_tinyfish_plugins
    );
    println!("Generated prompts:        {}", inspection.generated_prompts);
    if let Some(dir) = &inspection.cmd_shims_dir {
        println!("CMD shims dir:            {}", dir.display());
    }
    if let Some(dir) = &inspection.shell_shims_dir {
        println!("Shell shims dir:          {}", dir.display());
    }
}

fn print_config_import_summary(summary: &ConfigImportSummary, input: &Path) {
    println!("Config bundle imported from {}.", input.display());
    println!(
        "Profiles:                 {} added, {} updated, {} conflicted",
        summary.profiles_added, summary.profiles_updated, summary.profiles_conflicted
    );
    println!(
        "Providers:                {} added, {} updated, {} conflicted",
        summary.providers_added, summary.providers_updated, summary.providers_conflicted
    );
    println!(
        "MCP servers:              {} added, {} updated, {} conflicted",
        summary.mcp_servers_added, summary.mcp_servers_updated, summary.mcp_servers_conflicted
    );
}

fn print_config_import_plan(plan: &ConfigImportPlan, input: &Path) {
    println!("Config bundle import plan for {}.", input.display());
    println!("Dry run:                  registry will not be modified");
    println!("Secrets included:         {}", plan.secrets_included);
    println!(
        "Profiles:                 {} add, {} update, {} conflict",
        plan.summary.profiles_added,
        plan.summary.profiles_updated,
        plan.summary.profiles_conflicted
    );
    println!(
        "Providers:                {} add, {} update, {} conflict",
        plan.summary.providers_added,
        plan.summary.providers_updated,
        plan.summary.providers_conflicted
    );
    println!(
        "MCP servers:              {} add, {} update, {} conflict",
        plan.summary.mcp_servers_added,
        plan.summary.mcp_servers_updated,
        plan.summary.mcp_servers_conflicted
    );
    print_plan_items("Profiles to add", &plan.profiles_add);
    print_plan_items("Profiles to update", &plan.profiles_update);
    print_plan_items("Profiles with conflicts", &plan.profiles_conflict);
    print_plan_items("Providers to add", &plan.providers_add);
    print_plan_items("Providers to update", &plan.providers_update);
    print_plan_items("Providers with conflicts", &plan.providers_conflict);
    print_plan_items("MCP servers to add", &plan.mcp_servers_add);
    print_plan_items("MCP servers to update", &plan.mcp_servers_update);
    print_plan_items("MCP servers with conflicts", &plan.mcp_servers_conflict);
}

fn print_plan_items(label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!("{label}:");
    for item in items {
        println!("  {item}");
    }
}

fn print_config_bundle_validation(validation: &ConfigBundleValidation, input: &Path) {
    println!("Config bundle:            {}", input.display());
    println!("Schema:                   {}", validation.schema);
    println!(
        "Objects:                  {} profile(s), {} provider(s), {} MCP server(s)",
        validation.profiles, validation.providers, validation.mcp_servers
    );
    println!("Secrets included:         {}", validation.secrets_included);
    println!(
        "Issues:                   {} error(s), {} warning(s)",
        validation.error_count(),
        validation.warning_count()
    );
    for item in &validation.issues {
        print_diagnostic_item(item);
    }
}

fn print_shim_recovery_summary(summary: &ShimRecoverySummary) {
    println!(
        "Shim recovery written from {}.",
        summary.plan.shim_dir.display()
    );
    if let Some(path) = &summary.backup_path {
        println!("Registry backup:          {}", path.display());
    }
    print_shim_recovery_counts(&summary.plan, false);
    print_plan_items("Profiles added", &summary.plan.profiles_add);
    print_plan_items("Profiles updated", &summary.plan.profiles_update);
    print_plan_items("Providers added", &summary.plan.providers_add);
    print_plan_items("Provider keys added", &summary.plan.provider_keys_add);
    print_plan_items("Warnings", &summary.plan.warnings);
}

fn print_shim_recovery_plan(plan: &ShimRecoveryPlan) {
    println!("Shim recovery plan for {}.", plan.shim_dir.display());
    println!("Dry run:                  registry will not be modified");
    print_shim_recovery_counts(plan, true);
    print_plan_items("Profiles to add", &plan.profiles_add);
    print_plan_items("Profiles to update", &plan.profiles_update);
    print_plan_items("Profiles with conflicts", &plan.profiles_conflict);
    print_plan_items("Providers to add", &plan.providers_add);
    print_plan_items("Provider keys to add", &plan.provider_keys_add);
    print_plan_items("Warnings", &plan.warnings);
}

fn print_shim_recovery_counts(plan: &ShimRecoveryPlan, planned: bool) {
    let verb = if planned {
        "add/update/conflict"
    } else {
        "added/updated/conflicted"
    };
    println!("Files scanned:            {}", plan.files_scanned);
    println!("Files recoverable:        {}", plan.files_recoverable);
    println!("Files skipped:            {}", plan.files_skipped);
    println!(
        "Profiles {}:  {}/{}/{}",
        verb, plan.profiles_added, plan.profiles_updated, plan.profiles_conflicted
    );
    println!("Providers added:          {}", plan.providers_added);
    println!("Provider keys added:      {}", plan.provider_keys_added);
    println!("Provider keys reused:     {}", plan.provider_keys_reused);
}

fn print_mcp_validation(issues: &[McpValidationIssue]) {
    if issues.is_empty() {
        println!("MCP validation passed.");
        return;
    }
    for issue in issues {
        println!(
            "{:<5} {:<24} {}",
            issue.level.as_str().to_uppercase(),
            issue.server_name,
            issue.message
        );
        if let Some(hint) = &issue.hint {
            println!("      {:<24} hint: {}", "", hint);
        }
    }
}

fn render_statusline(info: &StatuslineInfo) -> String {
    let Some(profile_name) = &info.profile_name else {
        return "cswitch: no profile".to_string();
    };
    let alias = info
        .profile_alias
        .as_ref()
        .map(|alias| format!(" ({alias})"))
        .unwrap_or_default();
    let kind = info
        .kind
        .as_ref()
        .map(|kind| {
            if *kind == profile::ProfileKind::Lightweight {
                "lite"
            } else {
                "full"
            }
        })
        .unwrap_or("unknown");
    let provider = info.provider_name.as_deref().unwrap_or("inline");
    let key = info.key_name.as_deref().unwrap_or("no-key");
    let project = if info.project_marker { " project" } else { "" };
    format!(
        "cswitch:{project} {profile_name}{alias} [{kind}] provider={provider} key={key} mcp={}",
        info.mcp_servers
    )
}

fn write_or_print(content: &str, output: Option<&PathBuf>, label: &str) -> Result<()> {
    if let Some(output) = output {
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, content)?;
        println!("{label} to {}.", output.display());
    } else {
        println!("{content}");
    }
    Ok(())
}

fn render_shell_hook(shell: &str) -> Result<String> {
    let shell = if shell == "auto" {
        default_shell_name()
    } else {
        shell.to_ascii_lowercase()
    };
    match shell.as_str() {
        "powershell" | "pwsh" => Ok(
            r#"function claude {
    $profile = $null
    $profile = (& cswitch shell current --dir (Get-Location).Path 2>$null)
    if ($LASTEXITCODE -eq 0 -and $profile) {
        & cswitch use $profile -- @args
        return
    }
    $cmd = Get-Command claude.exe -CommandType Application -ErrorAction SilentlyContinue
    if (-not $cmd) {
        throw "claude.exe was not found on PATH"
    }
    & $cmd.Source @args
}"#
            .to_string(),
        ),
        "bash" | "zsh" => Ok(
            r#"# claude-switch project auto-profile hook
claude() {
  local _cswitch_profile
  _cswitch_profile="$(command cswitch shell current --dir "$PWD" 2>/dev/null)" || _cswitch_profile=""
  if [ -n "$_cswitch_profile" ]; then
    command cswitch use "$_cswitch_profile" -- "$@"
  else
    command claude "$@"
  fi
}"#
            .to_string(),
        ),
        "fish" => Ok(
            r#"# claude-switch project auto-profile hook
function claude
    set -l _cswitch_profile (command cswitch shell current --dir "$PWD" 2>/dev/null)
    if test $status -eq 0; and test -n "$_cswitch_profile"
        command cswitch use "$_cswitch_profile" -- $argv
    else
        command claude $argv
    end
end"#
            .to_string(),
        ),
        _ => bail!(
            "Unsupported shell '{}'. Use auto, powershell, bash, zsh, or fish.",
            shell
        ),
    }
}

fn default_shell_name() -> String {
    #[cfg(target_os = "windows")]
    {
        "powershell".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL")
            .ok()
            .and_then(|value| {
                std::path::Path::new(&value)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_ascii_lowercase())
            })
            .filter(|name| matches!(name.as_str(), "bash" | "zsh" | "fish"))
            .unwrap_or_else(|| "bash".to_string())
    }
}

#[cfg(target_os = "windows")]
fn sync_shims(manager: &profile::ProfileManager) {
    if let Err(e) = manager.sync_cmd_aliases() {
        eprintln!("Note: failed to sync CMD aliases: {}", e);
    }
}

#[cfg(not(target_os = "windows"))]
fn sync_shims(manager: &profile::ProfileManager) {
    // Sync shell scripts if ~/.varusers/bin exists
    if let Err(e) = manager.sync_sh_scripts() {
        eprintln!("Note: failed to sync shell scripts: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_provider_commands() {
        let cli = Cli::try_parse_from([
            "cswitch",
            "provider",
            "add",
            "Example",
            "--url",
            "https://api.example.invalid",
            "--key",
            "sk-test-generated-key-777777777777777777777777",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Provider {
                command: ProviderCommands::Add { name, url, key },
            }) => {
                assert_eq!(name, "Example");
                assert_eq!(url, "https://api.example.invalid");
                assert_eq!(key, "sk-test-generated-key-777777777777777777777777");
            }
            _ => panic!("unexpected command parse result"),
        }
    }

    #[test]
    fn old_flat_provider_commands_no_longer_parse() {
        assert!(
            Cli::try_parse_from([
                "cswitch",
                "provider-add",
                "Example",
                "--url",
                "https://api.example.invalid",
                "--key",
                "sk-test-generated-key-888888888888888888888888",
            ])
            .is_err()
        );

        assert!(Cli::try_parse_from(["cswitch", "providers"]).is_err());
    }

    #[test]
    fn aliases_remote_option_parses() {
        let cli =
            Cli::try_parse_from(["cswitch", "aliases", "--remote", "devbox", "--verbose"]).unwrap();
        match cli.command {
            Some(Commands::Aliases {
                local,
                remote,
                verbose,
            }) => {
                assert!(!local);
                assert_eq!(remote, vec!["devbox"]);
                assert!(verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn aliases_verbose_only_parses() {
        let cli = Cli::try_parse_from(["cswitch", "aliases", "--verbose"]).unwrap();
        match cli.command {
            Some(Commands::Aliases {
                local,
                remote,
                verbose,
            }) => {
                assert!(!local);
                assert!(remote.is_empty());
                assert!(verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn aliases_multiple_remote_options_parse() {
        let cli = Cli::try_parse_from([
            "cswitch", "aliases", "--remote", "host1", "--remote", "host2",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Aliases {
                local,
                remote,
                verbose,
            }) => {
                assert!(!local);
                assert_eq!(remote, vec!["host1", "host2"]);
                assert!(!verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn aliases_local_option_parses() {
        let cli = Cli::try_parse_from(["cswitch", "aliases", "--local"]).unwrap();
        match cli.command {
            Some(Commands::Aliases {
                local,
                remote,
                verbose,
            }) => {
                assert!(local);
                assert!(remote.is_empty());
                assert!(!verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn aliases_local_and_remote_options_parse() {
        let cli = Cli::try_parse_from([
            "cswitch",
            "aliases",
            "--local",
            "--remote",
            "host1",
            "--remote",
            "host2",
            "--verbose",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Aliases {
                local,
                remote,
                verbose,
            }) => {
                assert!(local);
                assert_eq!(remote, vec!["host1", "host2"]);
                assert!(verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn parses_nested_mcp_add_command() {
        let cli = Cli::try_parse_from([
            "cswitch",
            "mcp",
            "add",
            "github",
            "--type",
            "stdio",
            "--command",
            "npx",
            "--arg",
            "-y",
            "--arg",
            "@modelcontextprotocol/server-github",
            "--env",
            "GITHUB_TOKEN=${GITHUB_TOKEN}",
            "--always-load",
            "false",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Mcp {
                command:
                    McpCommands::Add {
                        name,
                        server_type,
                        command,
                        args,
                        env,
                        always_load,
                        ..
                    },
            }) => {
                assert_eq!(name, "github");
                assert_eq!(server_type, "stdio");
                assert_eq!(command.as_deref(), Some("npx"));
                assert_eq!(args, vec!["-y", "@modelcontextprotocol/server-github"]);
                assert_eq!(env, vec!["GITHUB_TOKEN=${GITHUB_TOKEN}"]);
                assert_eq!(always_load, Some(false));
            }
            _ => panic!("unexpected mcp add parse result"),
        }
    }

    #[test]
    fn parses_nested_mcp_link_command() {
        let cli = Cli::try_parse_from([
            "cswitch",
            "mcp",
            "link",
            "work",
            "github",
            "filesystem",
            "--replace",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Mcp {
                command:
                    McpCommands::Link {
                        profile,
                        mcps,
                        replace,
                    },
            }) => {
                assert_eq!(profile, "work");
                assert_eq!(mcps, vec!["github", "filesystem"]);
                assert!(replace);
            }
            _ => panic!("unexpected mcp link parse result"),
        }
    }

    #[test]
    fn parses_diagnostics_and_shell_commands() {
        let cli = Cli::try_parse_from(["cswitch", "doctor", "--json", "--strict"]).unwrap();
        match cli.command {
            Some(Commands::Doctor { json, strict }) => {
                assert!(json);
                assert!(strict);
            }
            _ => panic!("unexpected doctor parse result"),
        }

        let cli = Cli::try_parse_from(["cswitch", "config", "inspect", "--json"]).unwrap();
        match cli.command {
            Some(Commands::Config {
                command: ConfigCommands::Inspect { json },
            }) => assert!(json),
            _ => panic!("unexpected config inspect parse result"),
        }

        let cli = Cli::try_parse_from([
            "cswitch",
            "config",
            "export",
            "--profile",
            "work",
            "--output",
            "bundle.json",
            "--include-secrets",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::Export {
                        profiles,
                        output,
                        include_secrets,
                    },
            }) => {
                assert_eq!(profiles, vec!["work"]);
                assert_eq!(output, Some(PathBuf::from("bundle.json")));
                assert!(include_secrets);
            }
            _ => panic!("unexpected config export parse result"),
        }

        let cli = Cli::try_parse_from(["cswitch", "config", "import", "bundle.json", "--replace"])
            .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::Import {
                        input,
                        replace,
                        dry_run,
                        json,
                    },
            }) => {
                assert_eq!(input, PathBuf::from("bundle.json"));
                assert!(replace);
                assert!(!dry_run);
                assert!(!json);
            }
            _ => panic!("unexpected config import parse result"),
        }

        let cli = Cli::try_parse_from([
            "cswitch",
            "config",
            "import",
            "bundle.json",
            "--replace",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::Import {
                        input,
                        replace,
                        dry_run,
                        json,
                    },
            }) => {
                assert_eq!(input, PathBuf::from("bundle.json"));
                assert!(replace);
                assert!(!dry_run);
                assert!(json);
            }
            _ => panic!("unexpected config import json parse result"),
        }

        let cli = Cli::try_parse_from([
            "cswitch",
            "config",
            "import",
            "bundle.json",
            "--dry-run",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::Import {
                        input,
                        replace,
                        dry_run,
                        json,
                    },
            }) => {
                assert_eq!(input, PathBuf::from("bundle.json"));
                assert!(!replace);
                assert!(dry_run);
                assert!(json);
            }
            _ => panic!("unexpected config import dry-run parse result"),
        }

        let cli = Cli::try_parse_from(["cswitch", "config", "validate", "bundle.json", "--strict"])
            .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::Validate {
                        input,
                        json,
                        strict,
                    },
            }) => {
                assert_eq!(input, PathBuf::from("bundle.json"));
                assert!(!json);
                assert!(strict);
            }
            _ => panic!("unexpected config validate parse result"),
        }

        let cli = Cli::try_parse_from([
            "cswitch",
            "config",
            "recover-shims",
            "shims",
            "--write",
            "--replace",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Config {
                command:
                    ConfigCommands::RecoverShims {
                        shim_dir,
                        write,
                        replace,
                        json,
                    },
            }) => {
                assert_eq!(shim_dir, PathBuf::from("shims"));
                assert!(write);
                assert!(replace);
                assert!(json);
            }
            _ => panic!("unexpected recover-shims parse result"),
        }

        let cli =
            Cli::try_parse_from(["cswitch", "statusline", "--profile", "work", "--json"]).unwrap();
        match cli.command {
            Some(Commands::Statusline { profile, dir, json }) => {
                assert_eq!(profile.as_deref(), Some("work"));
                assert!(dir.is_none());
                assert!(json);
            }
            _ => panic!("unexpected statusline parse result"),
        }

        let cli = Cli::try_parse_from(["cswitch", "shell", "hook", "--shell", "bash"]).unwrap();
        match cli.command {
            Some(Commands::Shell {
                command: ShellCommands::Hook { shell },
            }) => assert_eq!(shell, "bash"),
            _ => panic!("unexpected shell hook parse result"),
        }
    }

    #[test]
    fn parses_mcp_export_import_validate_commands() {
        let cli =
            Cli::try_parse_from(["cswitch", "mcp", "export", "github", "--output", "mcp.json"])
                .unwrap();
        match cli.command {
            Some(Commands::Mcp {
                command:
                    McpCommands::Export {
                        queries,
                        all,
                        output,
                    },
            }) => {
                assert_eq!(queries, vec!["github"]);
                assert!(!all);
                assert_eq!(output, Some(PathBuf::from("mcp.json")));
            }
            _ => panic!("unexpected mcp export parse result"),
        }

        let cli =
            Cli::try_parse_from(["cswitch", "mcp", "import", "mcp.json", "--replace"]).unwrap();
        match cli.command {
            Some(Commands::Mcp {
                command: McpCommands::Import { input, replace },
            }) => {
                assert_eq!(input, PathBuf::from("mcp.json"));
                assert!(replace);
            }
            _ => panic!("unexpected mcp import parse result"),
        }

        let cli = Cli::try_parse_from(["cswitch", "mcp", "validate", "--all", "--strict"]).unwrap();
        match cli.command {
            Some(Commands::Mcp {
                command:
                    McpCommands::Validate {
                        queries,
                        all,
                        strict,
                    },
            }) => {
                assert!(queries.is_empty());
                assert!(all);
                assert!(strict);
            }
            _ => panic!("unexpected mcp validate parse result"),
        }
    }
}
