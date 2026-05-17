mod cli_args;
mod env_vars;
mod profile;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use profile::{LightweightEnv, ProfileManager, fetch_models};
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

    /// Print shell aliases for all profiles, or sync remote lightweight shims only with --remote
    Aliases {
        /// Also sync self-contained shims to a remote host reachable via ssh/scp/sftp
        #[arg(long)]
        remote: Option<String>,
        /// Show detailed remote sync progress
        #[arg(short, long)]
        verbose: bool,
    },

    /// Manage shared providers and provider/profile links
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
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

        Some(Commands::Aliases { remote, verbose }) => {
            if let Some(host) = remote {
                if verbose {
                    eprintln!("# remote sync target: {}", host);
                }
                let report = manager.sync_remote_aliases_with_progress(&host, verbose, |line| {
                    eprintln!("{}", line);
                })?;
                eprintln!("{}", report);
            } else {
                println!("{}", manager.generate_aliases()?);
            }
        }

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
            Some(Commands::Aliases { remote, verbose }) => {
                assert_eq!(remote.as_deref(), Some("devbox"));
                assert!(verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }

    #[test]
    fn aliases_verbose_only_parses() {
        let cli = Cli::try_parse_from(["cswitch", "aliases", "--verbose"]).unwrap();
        match cli.command {
            Some(Commands::Aliases { remote, verbose }) => {
                assert!(remote.is_none());
                assert!(verbose);
            }
            _ => panic!("unexpected aliases parse result"),
        }
    }
}
