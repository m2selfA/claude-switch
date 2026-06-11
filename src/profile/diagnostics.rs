use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    ConfigInspection, DiagnosticItem, DiagnosticLevel, DoctorReport, Profile, ProfileKind,
    ProfileManager, StatuslineInfo,
};

impl ProfileManager {
    pub fn inspect_config(&self) -> Result<ConfigInspection> {
        let registry = self.load_registry()?;
        let profiles = registry.profiles.values().collect::<Vec<_>>();
        let lightweight_profiles = profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::Lightweight)
            .count();
        let full_profiles = profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::Full)
            .count();
        let provider_keys = registry
            .providers
            .values()
            .map(|provider| provider.keys.len())
            .sum();
        let linked_mcp_refs = profiles
            .iter()
            .map(|profile| profile.mcp_server_ids.len())
            .sum();
        let linked_plugin_refs = profiles
            .iter()
            .map(|profile| profile.plugin_ids.len())
            .sum();

        #[cfg(target_os = "windows")]
        let cmd_shims_dir = Self::cmd_bin_dir().ok();
        #[cfg(not(target_os = "windows"))]
        let cmd_shims_dir = None;

        #[cfg(not(target_os = "windows"))]
        let shell_shims_dir = Self::sh_bin_dir().ok();
        #[cfg(target_os = "windows")]
        let shell_shims_dir = None;

        let runtime_root = self.runtime_root_dir();
        let runtime_sessions = self.list_runtime_sessions().unwrap_or_default();
        let active_runtime_sessions = runtime_sessions
            .iter()
            .filter(|session| session.active)
            .count();
        let stale_runtime_sessions = runtime_sessions
            .len()
            .saturating_sub(active_runtime_sessions);

        Ok(ConfigInspection {
            base_dir: self.base_dir(),
            registry_path: self.registry_path.clone(),
            profiles_dir: self.profiles_dir.clone(),
            generated_root: self.generated_root_dir(),
            plugins_root: self.plugins_root_dir(),
            runtime_root,
            profiles: profiles.len(),
            lightweight_profiles,
            full_profiles,
            providers: registry.providers.len(),
            provider_keys,
            mcp_servers: registry.mcp_servers.len(),
            linked_mcp_refs,
            plugin_marketplaces: registry.plugin_marketplaces.len(),
            installed_plugins: registry.installed_plugins.len(),
            linked_plugin_refs,
            generated_mcp_plugins: Self::count_named_entries(
                &self.generated_mcps_dir(),
                Self::is_managed_generated_mcp_dir_name,
            ),
            generated_tinyfish_plugins: Self::count_named_entries(
                &self.generated_plugins_dir(),
                Self::is_managed_generated_plugin_dir_name,
            ),
            generated_prompts: Self::count_named_entries(
                &self.generated_prompts_dir(),
                Self::is_managed_generated_prompt_name,
            ),
            runtime_sessions: runtime_sessions.len(),
            active_runtime_sessions,
            stale_runtime_sessions,
            allow_local_runtime_hot_switch: registry.settings.allow_local_runtime_hot_switch,
            cmd_shims_dir,
            shell_shims_dir,
        })
    }

    pub fn doctor_report(&self) -> Result<DoctorReport> {
        let mut report = DoctorReport::default();
        let _ = self.garbage_collect_runtime_sessions();
        let base_dir = self.base_dir();
        if base_dir.exists() {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Ok,
                "storage",
                format!("base directory exists: {}", base_dir.display()),
                None,
            ));
        } else {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "storage",
                format!("base directory does not exist yet: {}", base_dir.display()),
                Some("run any cswitch command that writes profiles or providers".to_string()),
            ));
        }

        let registry = match self.load_registry() {
            Ok(registry) => {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Ok,
                    "registry",
                    format!("registry is readable: {}", self.registry_path.display()),
                    None,
                ));
                registry
            }
            Err(err) => {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "registry",
                    format!("registry cannot be read: {err}"),
                    Some("inspect or restore ~/.claude-switch/registry.json".to_string()),
                ));
                return Ok(report);
            }
        };

        if registry.profiles.is_empty() {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "profiles",
                "no profiles are configured".to_string(),
                Some("add one with cswitch add <name>".to_string()),
            ));
        }

        let runtime_root = self.runtime_root_dir();
        if runtime_root.exists() {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Ok,
                "runtime",
                format!("runtime directory exists: {}", runtime_root.display()),
                None,
            ));
        }
        let runtime_sessions = self.list_runtime_sessions().unwrap_or_default();
        let stale_sessions = runtime_sessions
            .iter()
            .filter(|session| !session.active)
            .collect::<Vec<_>>();
        report.items.push(Self::diagnostic(
            DiagnosticLevel::Ok,
            "runtime",
            format!(
                "{} runtime session(s) found ({} active, {} stale)",
                runtime_sessions.len(),
                runtime_sessions.len().saturating_sub(stale_sessions.len()),
                stale_sessions.len()
            ),
            None,
        ));
        for stale in stale_sessions {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "runtime",
                format!("runtime session '{}' is stale", stale.state.session_id),
                stale.stale_reason.clone(),
            ));
        }

        for (profile_id, profile) in &registry.profiles {
            if profile.name.trim().is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("profile '{profile_id}' has an empty name"),
                    None,
                ));
            }
            if profile.kind == ProfileKind::Full {
                let dir = self.profile_dir(profile);
                if !dir.exists() {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "profiles",
                        format!("full profile '{}' directory is missing", profile.name),
                        Some(format!("expected {}", dir.display())),
                    ));
                }
            } else if profile.env.is_none() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("lightweight profile '{}' has no env block", profile.name),
                    Some("edit or recreate this profile".to_string()),
                ));
            }

            if let Some(provider_id) = &profile.provider_id {
                match registry.providers.get(provider_id) {
                    Some(provider) => match &profile.key_id {
                        Some(key_id) if provider.keys.contains_key(key_id) => {}
                        Some(key_id) => report.items.push(Self::diagnostic(
                            DiagnosticLevel::Error,
                            "providers",
                            format!(
                                "profile '{}' references missing key '{}' in provider '{}'",
                                profile.name, key_id, provider.name
                            ),
                            Some("relink the profile with cswitch provider link".to_string()),
                        )),
                        None => report.items.push(Self::diagnostic(
                            DiagnosticLevel::Error,
                            "providers",
                            format!(
                                "profile '{}' references provider '{}' without a key",
                                profile.name, provider.name
                            ),
                            Some("relink the profile with cswitch provider link".to_string()),
                        )),
                    },
                    None => report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "providers",
                        format!(
                            "profile '{}' references missing provider '{}'",
                            profile.name, provider_id
                        ),
                        Some("unlink or relink the profile provider".to_string()),
                    )),
                }
            }

            if !profile.mcp_server_ids.is_empty() && profile.kind != ProfileKind::Lightweight {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "mcp",
                    format!("full profile '{}' has MCP registry links", profile.name),
                    Some("MCP links are only supported for lightweight profiles".to_string()),
                ));
            }
            for mcp_id in &profile.mcp_server_ids {
                if !registry.mcp_servers.contains_key(mcp_id) {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "mcp",
                        format!(
                            "profile '{}' references missing MCP '{}'",
                            profile.name, mcp_id
                        ),
                        Some("unlink stale MCP ids or recreate the MCP entry".to_string()),
                    ));
                }
            }
            for plugin_id in &profile.plugin_ids {
                match registry.installed_plugins.get(plugin_id) {
                    Some(installed) => {
                        let plugin_root = self.plugin_install_root(
                            &installed.marketplace_name,
                            &installed.plugin_name,
                        );
                        if !plugin_root.exists() {
                            report.items.push(Self::diagnostic(
                                DiagnosticLevel::Warn,
                                "plugins",
                                format!(
                                    "profile '{}' hosted plugin '{}' is missing on disk",
                                    profile.name, installed.id
                                ),
                                Some("reinstall or update the hosted plugin".to_string()),
                            ));
                        }
                    }
                    None => report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "plugins",
                        format!(
                            "profile '{}' references missing hosted plugin '{}'",
                            profile.name, plugin_id
                        ),
                        Some("unlink the stale plugin or reinstall it".to_string()),
                    )),
                }
            }
            if profile.kind == ProfileKind::Lightweight && !profile.mcp_server_ids.is_empty() {
                let plugin_root = self.local_profile_mcp_plugin_root(profile);
                if !plugin_root.join(".mcp.json").exists() || !plugin_root.join("mcp.json").exists()
                {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Warn,
                        "mcp",
                        format!(
                            "profile '{}' MCP plugin artifacts have not been generated",
                            profile.name
                        ),
                        Some("run cswitch aliases --local or launch the profile once".to_string()),
                    ));
                }
            }
        }

        for provider in registry.providers.values() {
            if provider.base_url.trim().is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("provider '{}' has an empty base URL", provider.name),
                    Some("edit the provider URL".to_string()),
                ));
            }
            if provider.keys.is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Warn,
                    "providers",
                    format!("provider '{}' has no keys", provider.name),
                    Some("add a key with cswitch provider add-key".to_string()),
                ));
            }
        }

        for issue in registry
            .mcp_servers
            .values()
            .flat_map(Self::validate_mcp_server_config)
        {
            report.items.push(Self::diagnostic(
                issue.level,
                "mcp",
                format!("{}: {}", issue.server_name, issue.message),
                issue.hint,
            ));
        }

        let desired_mcp_dirs: std::collections::HashSet<String> = registry
            .profiles
            .values()
            .filter(|profile| {
                profile.kind == ProfileKind::Lightweight && !profile.mcp_server_ids.is_empty()
            })
            .map(Self::profile_mcp_plugin_dir_name)
            .collect();
        let stale_mcp_dirs = Self::managed_entry_names(
            &self.generated_mcps_dir(),
            Self::is_managed_generated_mcp_dir_name,
        )
        .into_iter()
        .filter(|name| !desired_mcp_dirs.contains(name))
        .count();
        if stale_mcp_dirs > 0 {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "generated",
                format!("{stale_mcp_dirs} stale generated MCP plugin dir(s) found"),
                Some("run cswitch aliases --local to resync generated artifacts".to_string()),
            ));
        }

        if !Self::command_exists("claude") {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "runtime",
                "claude command is not available on PATH".to_string(),
                Some("install Claude Code or adjust PATH before launching profiles".to_string()),
            ));
        }

        if report.error_count() == 0 {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Ok,
                "summary",
                format!(
                    "{} profile(s), {} provider(s), {} MCP server(s)",
                    registry.profiles.len(),
                    registry.providers.len(),
                    registry.mcp_servers.len()
                ),
                None,
            ));
        }

        Ok(report)
    }

    pub fn resolve_project_profile(&self, start: &Path) -> Result<Option<Profile>> {
        let mut current = if start.as_os_str().is_empty() {
            env::current_dir()?
        } else if start.is_file() {
            start.parent().unwrap_or(start).to_path_buf()
        } else {
            start.to_path_buf()
        };
        if current.is_relative() {
            current = env::current_dir()?.join(current);
        }

        loop {
            for marker in [".cswitch-profile", ".claudeprofile"] {
                let marker_path = current.join(marker);
                if let Some(query) = Self::read_profile_marker(&marker_path)? {
                    let (_, profile) = self.find_profile(&query).with_context(|| {
                        format!(
                            "Project marker '{}' references unknown profile '{}'.",
                            marker_path.display(),
                            query
                        )
                    })?;
                    return Ok(Some(profile));
                }
            }

            if !current.pop() {
                break;
            }
        }

        Ok(None)
    }

    pub fn statusline_info(
        &self,
        profile_query: Option<&str>,
        project_dir: Option<&Path>,
    ) -> Result<StatuslineInfo> {
        let registry = self.load_registry()?;
        let (profile, project_marker) = if let Some(query) = profile_query {
            let (_, profile) = Self::find_profile_in_registry(&registry, query)?;
            (Some(profile), false)
        } else if let Some(project_dir) = project_dir {
            (self.resolve_project_profile(project_dir)?, true)
        } else {
            (None, false)
        };

        let Some(profile) = profile else {
            return Ok(StatuslineInfo {
                profile_id: None,
                profile_name: None,
                profile_alias: None,
                kind: None,
                provider_name: None,
                provider_id: None,
                key_name: None,
                key_id: None,
                mcp_servers: 0,
                mcp_names: Vec::new(),
                plugins: 0,
                plugin_names: Vec::new(),
                project_marker: false,
            });
        };

        let (provider_name, key_name) = profile
            .provider_id
            .as_ref()
            .and_then(|provider_id| {
                registry.providers.get(provider_id).map(|provider| {
                    let key_name = profile
                        .key_id
                        .as_ref()
                        .and_then(|key_id| provider.keys.get(key_id))
                        .map(|key| key.name.clone());
                    (Some(provider.name.clone()), key_name)
                })
            })
            .unwrap_or((None, None));
        let mut mcp_names = profile
            .mcp_server_ids
            .iter()
            .filter_map(|id| registry.mcp_servers.get(id))
            .map(|server| server.name.clone())
            .collect::<Vec<_>>();
        mcp_names.sort();
        let mut plugin_names = profile
            .plugin_ids
            .iter()
            .filter_map(|id| registry.installed_plugins.get(id))
            .map(|plugin| plugin.id.clone())
            .collect::<Vec<_>>();
        plugin_names.sort();

        Ok(StatuslineInfo {
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.name.clone()),
            profile_alias: profile.alias.clone(),
            kind: Some(profile.kind.clone()),
            provider_name,
            provider_id: profile.provider_id.clone(),
            key_name,
            key_id: profile.key_id.clone(),
            mcp_servers: mcp_names.len(),
            mcp_names,
            plugins: plugin_names.len(),
            plugin_names,
            project_marker,
        })
    }

    pub fn base_dir(&self) -> PathBuf {
        self.registry_path
            .parent()
            .expect("registry path should always live under a base directory")
            .to_path_buf()
    }

    pub(super) fn diagnostic(
        level: DiagnosticLevel,
        area: impl Into<String>,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> DiagnosticItem {
        DiagnosticItem {
            level,
            area: area.into(),
            message: message.into(),
            hint,
        }
    }

    pub(super) fn looks_like_variable(value: &str) -> bool {
        value.contains("${") || value.contains('%')
    }

    pub(super) fn command_exists(command: &str) -> bool {
        super::local_command_exists(command)
    }

    fn read_profile_marker(path: &Path) -> Result<Option<String>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read project marker '{}'.", path.display()))?;
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                return Ok(Some(line.to_string()));
            }
        }
        bail!(
            "Project marker '{}' does not contain a profile name.",
            path.display()
        )
    }
}
