use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use crate::profile::{
    AuthMigrationPlan, AuthMigrationSummary, ConfigBundleValidation, ConfigImportPlan,
    ConfigImportSummary, ConfigInspection, DiagnosticItem, DoctorReport, GlobalSettings,
    McpValidationIssue, RuntimeGcSummary, RuntimeSessionInfo, ShimRecoveryPlan,
    ShimRecoverySummary, StatuslineInfo,
};

pub(crate) fn print_doctor_report(report: &DoctorReport) {
    println!(
        "claude-switch doctor: {} error(s), {} warning(s)",
        report.error_count(),
        report.warning_count()
    );
    for item in &report.items {
        print_diagnostic_item(item);
    }
}

pub(crate) fn print_diagnostic_item(item: &DiagnosticItem) {
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

pub(crate) fn print_config_inspection(inspection: &ConfigInspection) {
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
    println!(
        "Runtime root:             {}",
        inspection.runtime_root.display()
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
    println!("Runtime sessions:         {}", inspection.runtime_sessions);
    println!(
        "  active/stale:           {}/{}",
        inspection.active_runtime_sessions, inspection.stale_runtime_sessions
    );
    println!(
        "Legacy local override:   {}",
        inspection.allow_local_runtime_hot_switch
    );
    if let Some(dir) = &inspection.cmd_shims_dir {
        println!("CMD shims dir:            {}", dir.display());
    }
    if let Some(dir) = &inspection.shell_shims_dir {
        println!("Shell shims dir:          {}", dir.display());
    }
}

pub(crate) fn print_global_settings(settings: &GlobalSettings) {
    println!(
        "Legacy local override:   {}",
        settings.allow_local_runtime_hot_switch
    );
    println!(
        "Note: local/self-hosted lite profiles always bypass runtime sessions and use an inline apiKeyHelper."
    );
}

pub(crate) fn print_config_import_summary(summary: &ConfigImportSummary, input: &Path) {
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

pub(crate) fn print_config_import_plan(plan: &ConfigImportPlan, input: &Path) {
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

pub(crate) fn print_plan_items(label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!("{label}:");
    for item in items {
        println!("  {item}");
    }
}

pub(crate) fn print_config_bundle_validation(validation: &ConfigBundleValidation, input: &Path) {
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

pub(crate) fn print_shim_recovery_summary(summary: &ShimRecoverySummary) {
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

pub(crate) fn print_shim_recovery_plan(plan: &ShimRecoveryPlan) {
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

pub(crate) fn print_shim_recovery_counts(plan: &ShimRecoveryPlan, planned: bool) {
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

pub(crate) fn print_auth_migration_plan(plan: &AuthMigrationPlan) {
    println!("Auth migration plan");
    print_auth_migration_counts(plan);
    print_plan_items("Settings files to update", &plan.files_to_update);
    print_plan_items("Helpers to overwrite", &plan.helper_overwrite);
    print_plan_items("Warnings", &plan.warnings);
}

pub(crate) fn print_auth_migration_summary(summary: &AuthMigrationSummary) {
    println!("Auth migration complete.");
    print_auth_migration_counts(&summary.plan);
    print_plan_items("Settings files updated", &summary.plan.files_to_update);
    print_plan_items("Backups written", &summary.backup_paths);
    print_plan_items("Warnings", &summary.plan.warnings);
}

pub(crate) fn print_auth_migration_counts(plan: &AuthMigrationPlan) {
    println!("Local files scanned:      {}", plan.local_files_scanned);
    println!("Remote files scanned:     {}", plan.remote_files_scanned);
    println!("Files to update:          {}", plan.files_to_update_count);
    println!("Files already ok:         {}", plan.files_already_ok);
    println!("Files missing:            {}", plan.files_missing);
    println!("Files skipped:            {}", plan.files_skipped);
    println!("Helpers overwritten:      {}", plan.helpers_overwritten);
}

pub(crate) fn print_mcp_validation(issues: &[McpValidationIssue]) {
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

pub(crate) fn print_runtime_session_list(sessions: &[RuntimeSessionInfo]) {
    if sessions.is_empty() {
        println!("No runtime sessions found.");
        return;
    }
    println!(
        "{:<14} {:<8} {:<7} {:<18} {:<18} CWD",
        "SESSION", "PID", "STATUS", "PROFILE", "PROVIDER/KEY"
    );
    println!("{}", "─".repeat(96));
    for session in sessions {
        let pid = session
            .state
            .pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".to_string());
        let status = if session.active { "active" } else { "stale" };
        let profile = session
            .state
            .profile_alias
            .as_ref()
            .map(|alias| format!("{} ({alias})", session.state.profile_name))
            .unwrap_or_else(|| session.state.profile_name.clone());
        let provider = format!(
            "{}/{}",
            session.state.provider_name.as_deref().unwrap_or("inline"),
            session.state.key_name.as_deref().unwrap_or("no-key")
        );
        let cwd = session
            .state
            .cwd
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<14} {:<8} {:<7} {:<18} {:<18} {}",
            session.state.session_id, pid, status, profile, provider, cwd
        );
    }
}

pub(crate) fn print_runtime_session(session: &RuntimeSessionInfo) {
    println!("Session:                  {}", session.state.session_id);
    println!(
        "Status:                   {}",
        if session.active { "active" } else { "stale" }
    );
    if let Some(reason) = &session.stale_reason {
        println!("Stale reason:             {}", reason);
    }
    if let Some(pid) = session.state.pid {
        println!("PID:                      {}", pid);
    }
    println!(
        "Profile:                  {}",
        session
            .state
            .profile_alias
            .as_ref()
            .map(|alias| format!("{} ({alias})", session.state.profile_name))
            .unwrap_or_else(|| session.state.profile_name.clone())
    );
    println!(
        "Provider:                 {}",
        session.state.provider_name.as_deref().unwrap_or("inline")
    );
    println!(
        "Key:                      {}",
        session.state.key_name.as_deref().unwrap_or("no-key")
    );
    println!("Base URL:                 {}", session.state.base_url);
    if let Some(model) = &session.state.model {
        println!("Model:                    {}", model);
    }
    if let Some(cwd) = &session.state.cwd {
        println!("CWD:                      {}", cwd.display());
    }
    println!("State path:               {}", session.state_path.display());
    println!(
        "Settings path:            {}",
        session.settings_path.display()
    );
    println!(
        "Created:                  {}",
        session.state.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!(
        "Updated:                  {}",
        session.state.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
}

pub(crate) fn print_runtime_gc_summary(summary: &RuntimeGcSummary) {
    println!("Runtime GC scanned:       {}", summary.scanned);
    println!("Runtime GC removed:       {}", summary.removed);
    println!("Runtime GC kept:          {}", summary.kept);
}

pub(crate) fn render_statusline(info: &StatuslineInfo) -> String {
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
            if *kind == crate::profile::ProfileKind::Lightweight {
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

pub(crate) fn write_or_print(content: &str, output: Option<&PathBuf>, label: &str) -> Result<()> {
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
