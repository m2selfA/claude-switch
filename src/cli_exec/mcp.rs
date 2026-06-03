use super::*;
use anyhow::bail;
use std::fs;

use crate::profile::{DiagnosticLevel, McpServerInput, McpServerUpdate};

pub(super) fn handle_mcp_command(manager: &ProfileManager, command: McpCommands) -> Result<()> {
    match command {
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
            sync_shims(manager);
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
            sync_shims(manager);
        }
        McpCommands::Remove { query } => {
            manager.remove_mcp_server(&query)?;
            println!("MCP '{}' removed.", query);
            sync_shims(manager);
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
            sync_shims(manager);
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
            sync_shims(manager);
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
            let content = fs::read_to_string(&input)
                .map_err(|err| anyhow::anyhow!("Failed to read {}: {}", input.display(), err))?;
            let imported = manager.import_mcp_config(&content, replace)?;
            println!(
                "Imported {} MCP server(s) from {}.",
                imported.len(),
                input.display()
            );
            for server in imported {
                println!("  {} ({})", server.name, server.id);
            }
            sync_shims(manager);
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
    }
    Ok(())
}
