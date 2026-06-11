use super::*;
use std::io::{self, IsTerminal, Write};

pub(super) fn handle_plugin_command(
    manager: &ProfileManager,
    command: PluginCommands,
) -> Result<()> {
    match command {
        PluginCommands::Marketplace { command } => match command {
            PluginMarketplaceCommands::List => {
                let marketplaces = manager.list_plugin_marketplaces()?;
                if marketplaces.is_empty() {
                    println!("No plugin marketplaces configured.");
                    println!("Add one with: cswitch plugin marketplace add <owner/repo>");
                    return Ok(());
                }
                println!("{:<24} {:<10} SOURCE", "NAME", "KIND");
                println!("{}", "─".repeat(88));
                for marketplace in marketplaces {
                    let source = marketplace
                        .canonical_url
                        .as_deref()
                        .unwrap_or(&marketplace.locator);
                    println!(
                        "{:<24} {:<10} {}",
                        marketplace.name,
                        format!("{:?}", marketplace.source_kind).to_lowercase(),
                        source
                    );
                }
            }
            PluginMarketplaceCommands::Add { locator, replace } => {
                let (marketplace, items) = manager.add_plugin_marketplace(&locator, replace)?;
                println!("Plugin marketplace '{}' added.", marketplace.name);
                print_catalog_items(&items);
                maybe_prompt_and_install(manager, &items)?;
            }
            PluginMarketplaceCommands::Update { query } => {
                let (marketplace, items) = manager.update_plugin_marketplace(&query)?;
                println!("Plugin marketplace '{}' updated.", marketplace.name);
                print_catalog_items(&items);
            }
            PluginMarketplaceCommands::Remove { query } => {
                manager.remove_plugin_marketplace(&query)?;
                println!("Plugin marketplace '{}' removed.", query);
            }
        },
        PluginCommands::List => {
            let plugins = manager.list_installed_plugins()?;
            if plugins.is_empty() {
                println!("No hosted plugins installed.");
                println!("Install one with: cswitch plugin install <plugin@marketplace>");
                return Ok(());
            }
            println!("{:<32} {:<10} {:<10} VERSION", "ID", "TYPE", "DEFAULT");
            println!("{}", "─".repeat(96));
            for plugin in plugins {
                println!(
                    "{:<32} {:<10} {:<10} {}",
                    plugin.id,
                    if plugin.explicit {
                        "explicit"
                    } else {
                        "dependency"
                    },
                    plugin.default_enabled.unwrap_or(false),
                    plugin.version.as_deref().unwrap_or("—")
                );
            }
        }
        PluginCommands::Show { query } => {
            let details = manager.installed_plugin_details(&query)?;
            println!("Id:             {}", details.installed.id);
            println!("Marketplace:    {}", details.installed.marketplace_name);
            println!("Plugin:         {}", details.installed.plugin_name);
            println!(
                "Version:        {}",
                details.installed.version.as_deref().unwrap_or("—")
            );
            println!(
                "Type:           {}",
                if details.installed.explicit {
                    "explicit"
                } else {
                    "dependency"
                }
            );
            println!(
                "Default enable: {}",
                details.installed.default_enabled.unwrap_or(false)
            );
            println!("Install root:   {}", details.install_root.display());
            println!("Exists:         {}", details.exists);
            if let Some(url) = &details.installed.source_url {
                println!("Source URL:     {}", url);
            }
            if let Some(reference) = &details.installed.source_ref {
                println!("Source ref:     {}", reference);
            }
            if let Some(sha) = &details.installed.source_sha {
                println!("Source SHA:     {}", sha);
            }
            if !details.installed.dependencies.is_empty() {
                println!(
                    "Dependencies:   {}",
                    details.installed.dependencies.join(", ")
                );
            }
            if !details.linked_profiles.is_empty() {
                println!("Profiles:       {}", details.linked_profiles.join(", "));
            }
        }
        PluginCommands::Install {
            query,
            marketplace,
            force: _,
        } => {
            let candidates = manager
                .resolve_hosted_plugin_candidates(query.as_deref(), marketplace.as_deref())?;
            if candidates.is_empty() {
                bail!("No hosted plugin matched the requested query.");
            }
            let chosen = select_catalog_item(&candidates)?;
            let installed = manager.install_hosted_plugin(&chosen.id, true)?;
            println!("Hosted plugin '{}' installed.", installed.id);
        }
        PluginCommands::Update { query } => {
            if let Some(query) = query {
                let updated = manager.update_installed_plugin(&query)?;
                println!("Hosted plugin '{}' updated.", updated.id);
            } else {
                let updated = manager.update_all_installed_plugins()?;
                println!("Updated {} hosted plugin(s).", updated.len());
                for plugin in updated {
                    println!("  {}", plugin.id);
                }
            }
        }
        PluginCommands::Uninstall { query, prune } => {
            manager.uninstall_installed_plugin(&query, prune)?;
            println!("Hosted plugin '{}' removed.", query);
        }
        PluginCommands::Prune => {
            let removed = manager.prune_installed_plugins()?;
            if removed.is_empty() {
                println!("No orphaned dependency installs found.");
            } else {
                println!("Pruned {} orphaned dependency install(s).", removed.len());
                for plugin_id in removed {
                    println!("  {}", plugin_id);
                }
            }
        }
        PluginCommands::Link {
            profile,
            plugins,
            replace,
        } => {
            if plugins.is_empty() {
                bail!("Provide at least one installed plugin id or name.");
            }
            let updated = if replace {
                manager.set_profile_plugins(&profile, &plugins)?
            } else {
                manager.add_profile_plugins(&profile, &plugins)?
            };
            println!(
                "Profile '{}' now has {} hosted plugin(s).",
                updated.name,
                updated.plugin_ids.len()
            );
            sync_shims(manager);
        }
        PluginCommands::Unlink {
            profile,
            plugins,
            all,
        } => {
            if !all && plugins.is_empty() {
                bail!("Provide hosted plugin ids/names or use --all.");
            }
            let updated = manager.remove_profile_plugins(&profile, &plugins, all)?;
            println!(
                "Profile '{}' now has {} hosted plugin(s).",
                updated.name,
                updated.plugin_ids.len()
            );
            sync_shims(manager);
        }
    }
    Ok(())
}

fn print_catalog_items(items: &[crate::profile::HostedPluginCatalogItem]) {
    if items.is_empty() {
        println!("No plugins found in this marketplace.");
        return;
    }
    println!("Plugins:");
    for item in items {
        println!(
            "  {}{}",
            item.id,
            item.description
                .as_deref()
                .map(|description| format!(" — {}", description))
                .unwrap_or_default()
        );
    }
}

fn maybe_prompt_and_install(
    manager: &ProfileManager,
    items: &[crate::profile::HostedPluginCatalogItem],
) -> Result<()> {
    if items.is_empty() || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Ok(());
    }
    println!();
    print!("Install one now? Enter number or press Enter to skip: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let idx = trimmed
        .parse::<usize>()
        .with_context(|| format!("Invalid selection '{}'.", trimmed))?;
    if idx == 0 || idx > items.len() {
        bail!("Selection {} is out of range.", idx);
    }
    let installed = manager.install_hosted_plugin(&items[idx - 1].id, true)?;
    println!("Hosted plugin '{}' installed.", installed.id);
    Ok(())
}

fn select_catalog_item(
    items: &[crate::profile::HostedPluginCatalogItem],
) -> Result<crate::profile::HostedPluginCatalogItem> {
    match items {
        [] => bail!("No hosted plugins matched."),
        [single] => Ok(single.clone()),
        many if io::stdin().is_terminal() && io::stdout().is_terminal() => {
            println!("Multiple hosted plugins matched:");
            for (idx, item) in many.iter().enumerate() {
                println!(
                    "  {}. {}{}",
                    idx + 1,
                    item.id,
                    item.description
                        .as_deref()
                        .map(|description| format!(" — {}", description))
                        .unwrap_or_default()
                );
            }
            print!("Select plugin number: ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let idx = input
                .trim()
                .parse::<usize>()
                .context("Invalid plugin selection")?;
            if idx == 0 || idx > many.len() {
                bail!("Selection {} is out of range.", idx);
            }
            Ok(many[idx - 1].clone())
        }
        _ => bail!("Plugin query is ambiguous. Use plugin@marketplace."),
    }
}
