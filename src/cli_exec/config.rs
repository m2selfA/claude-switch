use super::*;
use anyhow::bail;
use std::fs;

use crate::cli_output::{print_auth_migration_plan, print_auth_migration_summary};

pub(super) fn handle_config_command(
    manager: &ProfileManager,
    command: ConfigCommands,
) -> Result<()> {
    match command {
        ConfigCommands::Inspect { json } => {
            let inspection = manager.inspect_config()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inspection)?);
            } else {
                print_config_inspection(&inspection);
            }
        }
        ConfigCommands::Settings { command } => match command {
            ConfigSettingsCommands::Show { json } => {
                let settings = manager.global_settings()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&settings)?);
                } else {
                    print_global_settings(&settings);
                }
            }
            ConfigSettingsCommands::Set {
                allow_local_runtime_hot_switch,
                json,
            } => {
                manager.set_allow_local_runtime_hot_switch(allow_local_runtime_hot_switch)?;
                let settings = manager.global_settings()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&settings)?);
                } else {
                    print_global_settings(&settings);
                }
            }
        },
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
            let content = fs::read_to_string(&input)
                .map_err(|err| anyhow::anyhow!("Failed to read {}: {}", input.display(), err))?;
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
                sync_shims(manager);
            }
        }
        ConfigCommands::Validate {
            input,
            json,
            strict,
        } => {
            let content = fs::read_to_string(&input)
                .map_err(|err| anyhow::anyhow!("Failed to read {}: {}", input.display(), err))?;
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
                sync_shims(manager);
            } else {
                let plan = manager.plan_shim_recovery(&shim_dir, replace)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plan)?);
                } else {
                    print_shim_recovery_plan(&plan);
                }
            }
        }
        ConfigCommands::MigrateAuth {
            write,
            json,
            remote,
        } => {
            if write {
                let summary = manager.migrate_auth(&remote)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                } else {
                    print_auth_migration_summary(&summary);
                }
            } else {
                let plan = manager.plan_auth_migration(&remote)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&plan)?);
                } else {
                    print_auth_migration_plan(&plan);
                }
            }
        }
    }
    Ok(())
}
