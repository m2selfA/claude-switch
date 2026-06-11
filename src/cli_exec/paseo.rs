use anyhow::{Result, bail};
use std::fs;
use std::path::PathBuf;

use crate::profile::{
    PaseoExportOptions, PaseoModelPolicy, PaseoOutputShape, PaseoSecretMode, ProfileManager,
};

use super::*;

pub(super) struct PaseoCliExportArgs<'a> {
    pub(super) profiles: &'a [String],
    pub(super) output: Option<&'a PathBuf>,
    pub(super) providers_only: bool,
    pub(super) full_config: bool,
    pub(super) include_secrets: bool,
    pub(super) with_extras: bool,
    pub(super) strict_model_discovery: bool,
}

pub(super) fn handle_paseo_command(manager: &ProfileManager, command: PaseoCommands) -> Result<()> {
    match command {
        PaseoCommands::Export {
            profiles,
            output,
            providers_only,
            full_config,
            include_secrets,
            with_extras,
            strict_model_discovery,
        } => export_paseo(
            manager,
            PaseoCliExportArgs {
                profiles: &profiles,
                output: output.as_ref(),
                providers_only,
                full_config,
                include_secrets,
                with_extras,
                strict_model_discovery,
            },
        ),
    }
}

pub(super) fn export_paseo(manager: &ProfileManager, args: PaseoCliExportArgs<'_>) -> Result<()> {
    if args.providers_only && args.full_config {
        bail!("--providers-only and --full-config cannot be used together.");
    }

    let options = PaseoExportOptions {
        output_shape: if args.providers_only {
            PaseoOutputShape::ProvidersOnly
        } else if args.full_config {
            PaseoOutputShape::FullConfig
        } else {
            PaseoOutputShape::AgentsFragment
        },
        secret_mode: if args.include_secrets {
            PaseoSecretMode::SelfContained
        } else {
            PaseoSecretMode::Wrapper
        },
        model_policy: PaseoModelPolicy::DiscoverThenFallback,
        include_stored_launch_args: args.with_extras,
        strict_model_discovery: args.strict_model_discovery,
    };
    let exported = manager.export_paseo_config(args.profiles, &options)?;
    write_json_output(&exported.content, args.output)?;
    for warning in &exported.warnings {
        eprintln!("Paseo export warning: {}", warning.message);
    }
    Ok(())
}

fn write_json_output(content: &str, output: Option<&PathBuf>) -> Result<()> {
    if let Some(output) = output {
        if let Some(parent) = output.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, content)?;
    } else {
        println!("{content}");
    }
    Ok(())
}
