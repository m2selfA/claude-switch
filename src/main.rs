mod cli_args;
mod env_vars;
mod profile;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use profile::{fetch_models, LightweightEnv, ProfileManager};
use std::io::{self, Write};

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
        /// Enable profile's stored launch args (e.g. --dangerously-skip-permissions)
        #[arg(short = 'e', long = "extra")]
        extra: bool,
        /// Additional passthrough args passed directly to claude (use -- to separate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Show details for a specific profile
    Info {
        /// Profile name
        name: String,
    },

    /// Print shell aliases for all profiles
    Aliases,
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

            println!("{:<20} {:<12} {:<8} {}", "NAME", "ALIAS", "KIND", "LAST USED");
            println!("{}", "─".repeat(70));
            for p in profiles {
                let kind = if p.kind == profile::ProfileKind::Full { "full" } else { "lite" };
                let alias = p.alias.as_deref().unwrap_or("—");
                let last_used = p
                    .last_used
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or("never".to_string());
                println!("{:<20} {:<12} {:<8} {}", p.name, alias, kind, last_used);
            }
        }

        Some(Commands::Add { name, alias, force, full }) => {
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

        Some(Commands::Use { name, extra, args }) => {
            manager.launch_claude(&name, &args, extra)?;
        }

        Some(Commands::Info { name }) => match manager.get_profile(&name) {
            Ok(p) => {
                let kind = if p.kind == profile::ProfileKind::Full { "full" } else { "lightweight" };
                println!("Id:        {}", p.id);
                println!("Name:      {}", p.name);
                if let Some(ref a) = p.alias { println!("Alias:     {}", a); }
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
                    println!();
                    println!("Environment variables set on launch:");
                    if let Some(ref env) = p.env {
                        if env.auth_token.is_some() { println!("  ANTHROPIC_AUTH_TOKEN=***"); }
                        if env.base_url.is_some() { println!("  ANTHROPIC_BASE_URL={}", env.base_url.as_deref().unwrap_or("")); }
                        if let Some(ref m) = env.default_opus_model { println!("  ANTHROPIC_DEFAULT_OPUS_MODEL={}", m); }
                        if let Some(ref m) = env.default_sonnet_model { println!("  ANTHROPIC_DEFAULT_SONNET_MODEL={}", m); }
                        if let Some(ref m) = env.default_haiku_model { println!("  ANTHROPIC_DEFAULT_HAIKU_MODEL={}", m); }
                        if let Some(ref m) = env.model { println!("  ANTHROPIC_MODEL={}", m); }
                        if let Some(ref m) = env.subagent_model { println!("  CLAUDE_CODE_SUBAGENT_MODEL={}", m); }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },

        Some(Commands::Aliases) => {
            println!("{}", manager.generate_aliases()?);
        }
    }

    Ok(())
}

/// Smart add: creates a lightweight profile (default) or full directory-isolated profile.
fn handle_add(manager: &ProfileManager, name: &str, alias: Option<&str>, force: bool, full: bool) -> Result<()> {
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
        println!("Creating lightweight profile '{}' (env-var based isolation).\n", name);

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
            if input.is_empty() { return None; }

            if let Some(models_list) = models {
                if let Ok(idx) = input.parse::<usize>() {
                    if idx >= 1 && idx <= models_list.len() {
                        return Some(models_list[idx - 1].clone());
                    }
                }
            }
            Some(input)
        }

        println!("Configure model settings (press Enter to skip any):");
        println!("  You can enter a model ID directly, or a number to select from the list above.");

        print!("Append [1m] suffix to model IDs for Claude Code context window recognition? [y/N]: ");
        io::stdout().flush()?;
        let mut suffix = String::new();
        io::stdin().read_line(&mut suffix)?;
        let append_1m = suffix.trim().to_lowercase() == "y";

        let default_opus_model = prompt_model("  Default Opus Model []: ", &models);
        let default_sonnet_model = prompt_model("  Default Sonnet Model []: ", &models);
        let default_haiku_model = prompt_model("  Default Haiku Model []: ", &models);
        let model = prompt_model("  Default Model (ANTHROPIC_MODEL) []: ", &models);
        let subagent_model = prompt_model("  Subagent Model (CLAUDE_CODE_SUBAGENT_MODEL) []: ", &models);

        fn apply_suffix(val: Option<String>, append: bool) -> Option<String> {
            match (val, append) {
                (Some(v), true) if !v.ends_with("[1m]") => Some(format!("{}[1m]", v)),
                (v, _) => v,
            }
        }

        let env = LightweightEnv {
            auth_token: Some(token),
            base_url: Some(base_url),
            default_opus_model: apply_suffix(default_opus_model, append_1m),
            default_sonnet_model: apply_suffix(default_sonnet_model, append_1m),
            default_haiku_model: apply_suffix(default_haiku_model, append_1m),
            model: apply_suffix(model, append_1m),
            subagent_model: apply_suffix(subagent_model, append_1m),
            extras: Vec::new(),
        };

        match manager.create_lightweight_profile(name, alias, env) {
            Ok(p) => {
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

