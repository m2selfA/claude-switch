use anyhow::Result;

use super::*;
use crate::cli_output::{
    print_runtime_gc_summary, print_runtime_session, print_runtime_session_list,
};

pub(super) fn handle_process_command(
    manager: &ProfileManager,
    command: ProcessCommands,
) -> Result<()> {
    match command {
        ProcessCommands::List => {
            let _ = manager.garbage_collect_runtime_sessions()?;
            let sessions = manager.list_runtime_sessions()?;
            print_runtime_session_list(&sessions);
        }
        ProcessCommands::Inspect { session_id } => {
            let sessions = manager.list_runtime_sessions()?;
            let session = sessions
                .into_iter()
                .find(|entry| entry.state.session_id == session_id)
                .ok_or_else(|| anyhow::anyhow!("Runtime session '{}' not found.", session_id))?;
            print_runtime_session(&session);
        }
        ProcessCommands::Switch {
            session_id,
            provider,
            key,
            model,
        } => {
            let updated = manager.switch_runtime_session(&session_id, &provider, &key, &model)?;
            println!(
                "Runtime session '{}' switched to provider '{}' key '{}' model '{}'.",
                updated.session_id,
                updated.provider_name.as_deref().unwrap_or(&provider),
                updated.key_name.as_deref().unwrap_or(&key),
                updated.model.as_deref().unwrap_or(&model)
            );
        }
        ProcessCommands::Gc => {
            let summary = manager.garbage_collect_runtime_sessions()?;
            print_runtime_gc_summary(&summary);
        }
    }
    Ok(())
}
