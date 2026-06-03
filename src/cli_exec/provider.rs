use super::*;

pub(super) fn handle_provider_command(
    manager: &ProfileManager,
    command: ProviderCommands,
) -> Result<()> {
    match command {
        ProviderCommands::List => {
            let providers = manager.list_providers()?;
            if providers.is_empty() {
                println!("No providers found.");
                println!("Providers are auto-extracted from lightweight profiles on first load.");
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
        ProviderCommands::Add { name, url, key } => match manager.add_provider(&name, &url, &key) {
            Ok(prov) => {
                println!("Provider '{}' ({}) added.", prov.name, prov.id);
                sync_shims(manager);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        },
        ProviderCommands::Remove { id } => match manager.remove_provider(&id) {
            Ok(_) => {
                println!("Provider '{}' removed.", id);
                sync_shims(manager);
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
            sync_shims(manager);
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
                sync_shims(manager);
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
            sync_shims(manager);
        }
        ProviderCommands::RenameKey { id, key_id, name } => {
            let renamed = manager.rename_key(&id, &key_id, &name)?;
            println!("Key '{}' renamed to '{}'.", key_id, renamed.name);
            sync_shims(manager);
        }
        ProviderCommands::RemoveKey { id, key_id } => match manager.remove_key(&id, &key_id) {
            Ok(_) => {
                println!("Key '{}' removed from provider '{}'.", key_id, id);
                sync_shims(manager);
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
            sync_shims(manager);
        }
        ProviderCommands::Unlink { profile } => {
            manager.unset_provider(&profile)?;
            println!("Provider association removed from profile '{}'.", profile);
            sync_shims(manager);
        }
    }
    Ok(())
}
