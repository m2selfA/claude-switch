use super::*;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use clap::Parser;
use std::io::{self, Write};

mod config;
mod mcp;
mod process;
mod provider;

use crate::cli_output::{
    print_config_bundle_validation, print_config_import_plan, print_config_import_summary,
    print_config_inspection, print_doctor_report, print_global_settings, print_mcp_validation,
    print_shim_recovery_plan, print_shim_recovery_summary, render_statusline, write_or_print,
};
use crate::cli_parse::{parse_key_values, parse_optional_json, render_shell_hook};
use crate::profile::{
    LaunchOptions, LightweightEnv, LocalGatewayToolMode, ProfileManager, RequestedLocalGatewayMode,
    fetch_models,
};

fn parse_local_gateway_mode(raw: Option<&str>) -> Result<RequestedLocalGatewayMode> {
    let Some(raw) = raw else {
        return Ok(RequestedLocalGatewayMode::Omitted);
    };
    LocalGatewayToolMode::parse_cli(raw)
        .map(RequestedLocalGatewayMode::Explicit)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid --local-gateway-mode '{}'. Use auto, search-fetch, fetch-only, or gateway-only.",
                raw
            )
        })
}

pub(crate) fn handle_add(
    manager: &ProfileManager,
    name: &str,
    alias: Option<&str>,
    force: bool,
    full: bool,
) -> Result<()> {
    if full {
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
        return Ok(());
    }

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

    print!("Append [1m] suffix to model IDs for Claude Code context window recognition? [y/N]: ");
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

pub(crate) fn sync_shims(manager: &ProfileManager) {
    #[cfg(target_os = "windows")]
    if let Err(e) = manager.sync_cmd_aliases() {
        eprintln!("Note: failed to sync CMD aliases: {}", e);
    }

    #[cfg(not(target_os = "windows"))]
    if let Err(e) = manager.sync_sh_scripts() {
        eprintln!("Note: failed to sync shell scripts: {}", e);
    }
}

fn optional_field<T>(value: Option<T>, clear: bool) -> Option<Option<T>> {
    if clear { Some(None) } else { value.map(Some) }
}

pub(crate) fn run() -> Result<()> {
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
            local_gateway_mode,
            args,
        }) => {
            manager.launch_claude(
                &name,
                &args,
                LaunchOptions {
                    use_stored_args: !no_extras,
                    local_gateway_mode: parse_local_gateway_mode(local_gateway_mode.as_deref())?,
                },
            )?;
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

        Some(Commands::Config { command }) => config::handle_config_command(&manager, command)?,

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

        Some(Commands::Provider { command }) => {
            provider::handle_provider_command(&manager, command)?
        }

        Some(Commands::Mcp { command }) => mcp::handle_mcp_command(&manager, command)?,

        Some(Commands::Process { command }) => process::handle_process_command(&manager, command)?,

        Some(Commands::Runtime { command }) => match command {
            RuntimeCommands::Auth { session_id } => {
                let state = manager.load_runtime_session(&session_id)?;
                print!("{}", state.auth_token);
            }
        },

        Some(Commands::Shim { command }) => match command {
            ShimCommands::Launch {
                probe,
                profile_id,
                no_extras,
                local_gateway_mode,
                args,
            } => {
                if probe {
                    return Ok(());
                }
                let profile_id = profile_id.context("Missing --profile-id for shim launch")?;
                manager.launch_claude(
                    &profile_id,
                    &args,
                    LaunchOptions {
                        use_stored_args: !no_extras,
                        local_gateway_mode: parse_local_gateway_mode(
                            local_gateway_mode.as_deref(),
                        )?,
                    },
                )?;
            }
        },
    }

    Ok(())
}
