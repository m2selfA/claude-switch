use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::tinyfish::{
    TinyfishPluginVariant, TinyfishToolShell,
    build_lightweight_runtime_artifacts_with_local_gateway_mode, native_tinyfish_tool_shell,
    tinyfish_plugin_script_file_name, tinyfish_statusline_script_file_name,
};
use super::{McpServer, Profile, ProfileKind, ProfileManager, RemoteOs};

impl ProfileManager {
    pub(super) fn managed_entry_names(dir: &Path, predicate: fn(&str) -> bool) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(ToString::to_string))
            .filter(|name| predicate(name))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub(super) fn count_named_entries(dir: &Path, predicate: fn(&str) -> bool) -> usize {
        Self::managed_entry_names(dir, predicate).len()
    }

    pub(super) fn generated_root_dir(&self) -> PathBuf {
        self.base_dir().join("generated")
    }

    pub(super) fn generated_prompts_dir(&self) -> PathBuf {
        self.generated_root_dir().join("prompts")
    }

    pub(super) fn generated_plugins_dir(&self) -> PathBuf {
        self.generated_root_dir().join("plugins")
    }

    pub(super) fn generated_mcps_dir(&self) -> PathBuf {
        self.generated_root_dir().join("mcps")
    }

    pub(super) fn profile_mcp_plugin_dir_name(profile: &Profile) -> String {
        let suffix: String = profile
            .id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(12)
            .collect();
        let suffix = if suffix.is_empty() {
            "unknown".to_string()
        } else {
            suffix
        };
        format!("cswitch-mcp-profile-{suffix}")
    }

    pub(super) fn local_profile_mcp_plugin_root(&self, profile: &Profile) -> PathBuf {
        self.generated_mcps_dir()
            .join(Self::profile_mcp_plugin_dir_name(profile))
    }

    pub(super) fn home_relative_profile_mcp_plugin_root(
        profile: &Profile,
        target_os: RemoteOs,
    ) -> String {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/mcps/{dir_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\mcps\\{dir_name}")
            }
        }
    }

    pub(super) fn profile_mcp_manifest_relative_path(
        profile: &Profile,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.claude-plugin/plugin.json"),
            RemoteOs::Windows => format!("{dir_name}\\.claude-plugin\\plugin.json"),
        }
    }

    pub(super) fn profile_mcp_config_relative_path(
        profile: &Profile,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.mcp.json"),
            RemoteOs::Windows => format!("{dir_name}\\.mcp.json"),
        }
    }

    pub(super) fn is_managed_generated_mcp_dir_name(file_name: &str) -> bool {
        file_name.starts_with("cswitch-mcp-profile-")
    }

    pub(super) fn tinyfish_plugin_dir_name(variant: TinyfishPluginVariant) -> String {
        variant.dir_name().to_string()
    }

    pub(super) fn local_tinyfish_plugin_root(&self, variant: TinyfishPluginVariant) -> PathBuf {
        self.generated_plugins_dir()
            .join(Self::tinyfish_plugin_dir_name(variant))
    }

    pub(super) fn local_tinyfish_plugin_hooks_path(
        &self,
        variant: TinyfishPluginVariant,
    ) -> PathBuf {
        self.local_tinyfish_plugin_root(variant)
            .join("hooks")
            .join("hooks.json")
    }

    pub(super) fn local_tinyfish_plugin_manifest_path(
        &self,
        variant: TinyfishPluginVariant,
    ) -> PathBuf {
        self.local_tinyfish_plugin_root(variant)
            .join(".claude-plugin")
            .join("plugin.json")
    }

    pub(super) fn local_tinyfish_output_style_path(
        &self,
        variant: TinyfishPluginVariant,
    ) -> PathBuf {
        self.local_tinyfish_plugin_root(variant)
            .join("output-styles")
            .join("route-default.md")
    }

    pub(super) fn local_tinyfish_hook_script_path(
        &self,
        variant: TinyfishPluginVariant,
        tool_shell: TinyfishToolShell,
    ) -> PathBuf {
        self.local_tinyfish_plugin_root(variant)
            .join("scripts")
            .join(tinyfish_plugin_script_file_name(tool_shell))
    }

    pub(super) fn local_tinyfish_statusline_script_path(
        &self,
        variant: TinyfishPluginVariant,
        tool_shell: TinyfishToolShell,
    ) -> PathBuf {
        self.local_tinyfish_plugin_root(variant)
            .join("scripts")
            .join(tinyfish_statusline_script_file_name(tool_shell))
    }

    pub(super) fn home_relative_tinyfish_plugin_root(
        variant: TinyfishPluginVariant,
        target_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/plugins/{dir_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\plugins\\{dir_name}")
            }
        }
    }

    pub(super) fn tinyfish_plugin_hooks_relative_path(
        variant: TinyfishPluginVariant,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/hooks/hooks.json"),
            RemoteOs::Windows => format!("{dir_name}\\hooks\\hooks.json"),
        }
    }

    pub(super) fn tinyfish_plugin_manifest_relative_path(
        variant: TinyfishPluginVariant,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.claude-plugin/plugin.json"),
            RemoteOs::Windows => format!("{dir_name}\\.claude-plugin\\plugin.json"),
        }
    }

    pub(super) fn tinyfish_output_style_relative_path(
        variant: TinyfishPluginVariant,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/output-styles/route-default.md"),
            RemoteOs::Windows => format!("{dir_name}\\output-styles\\route-default.md"),
        }
    }

    pub(super) fn tinyfish_hook_script_relative_path(
        variant: TinyfishPluginVariant,
        remote_os: RemoteOs,
        tool_shell: TinyfishToolShell,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        let file_name = tinyfish_plugin_script_file_name(tool_shell);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/scripts/{file_name}"),
            RemoteOs::Windows => format!("{dir_name}\\scripts\\{file_name}"),
        }
    }

    pub(super) fn tinyfish_statusline_script_relative_path(
        variant: TinyfishPluginVariant,
        remote_os: RemoteOs,
        tool_shell: TinyfishToolShell,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(variant);
        let file_name = tinyfish_statusline_script_file_name(tool_shell);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/scripts/{file_name}"),
            RemoteOs::Windows => format!("{dir_name}\\scripts\\{file_name}"),
        }
    }

    pub(super) fn remote_generated_root_dir(remote_home: &str, remote_os: RemoteOs) -> String {
        match remote_os {
            RemoteOs::Unix => {
                format!(
                    "{}/.claude-switch/generated",
                    remote_home.trim_end_matches('/')
                )
            }
            RemoteOs::Windows => format!(
                "{}\\.claude-switch\\generated",
                remote_home.trim_end_matches(['\\', '/'])
            ),
        }
    }

    pub(super) fn is_managed_generated_prompt_name(file_name: &str) -> bool {
        file_name.starts_with("tinyfish-") && file_name.ends_with(".txt")
    }

    pub(super) fn is_managed_generated_plugin_dir_name(file_name: &str) -> bool {
        [
            TinyfishPluginVariant::Router,
            TinyfishPluginVariant::Full,
            TinyfishPluginVariant::FetchOnly,
        ]
        .iter()
        .any(|variant| file_name == variant.dir_name())
    }

    pub(super) fn write_if_changed(path: &Path, content: &str) -> Result<()> {
        let needs_write = match fs::read_to_string(path) {
            Ok(existing) => existing != content,
            Err(_) => true,
        };
        if needs_write {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)?;
        }
        Ok(())
    }

    pub(super) fn remove_file_if_exists(path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(unix)]
    pub(super) fn set_executable_if_possible(path: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub(super) fn set_executable_if_possible(_path: &Path) -> Result<()> {
        Ok(())
    }

    pub(super) fn profile_mcp_plugin_manifest(profile: &Profile) -> Result<String> {
        serde_json::to_string_pretty(&serde_json::json!({
            "name": Self::profile_mcp_plugin_dir_name(profile),
            "displayName": format!("claude-switch MCPs for {}", profile.name),
            "description": "Generated by claude-switch to attach selected MCP servers to this profile.",
        }))
        .context("Failed to serialize MCP plugin manifest JSON")
    }

    pub(super) fn upsert_local_profile_mcp_plugin(
        &self,
        profile: &Profile,
        servers: &[McpServer],
    ) -> Result<PathBuf> {
        let plugin_root = self.local_profile_mcp_plugin_root(profile);
        let manifest_path = plugin_root.join(".claude-plugin").join("plugin.json");
        let legacy_config_path = plugin_root.join("mcp.json");
        #[cfg(windows)]
        let mcp_config = {
            let home = Self::home_dir()?;
            Self::profile_mcp_config_for_target(
                servers,
                RemoteOs::Windows,
                Some(home.to_string_lossy().as_ref()),
            )?
        };
        #[cfg(not(windows))]
        let mcp_config = Self::profile_mcp_config_for_target(servers, RemoteOs::Unix, None)?;
        Self::write_if_changed(&manifest_path, &Self::profile_mcp_plugin_manifest(profile)?)?;
        Self::write_if_changed(&plugin_root.join(".mcp.json"), &mcp_config)?;
        Self::remove_file_if_exists(&legacy_config_path)?;
        Ok(plugin_root)
    }

    pub(super) fn sync_local_mcp_artifacts(&self, profiles: &[Profile]) -> Result<()> {
        let mut desired = HashSet::new();
        for profile in profiles {
            if profile.kind != ProfileKind::Lightweight || profile.mcp_server_ids.is_empty() {
                continue;
            }
            let servers = self.profile_mcp_servers(profile)?;
            self.upsert_local_profile_mcp_plugin(profile, &servers)?;
            desired.insert(Self::profile_mcp_plugin_dir_name(profile));
        }

        let mcps_dir = self.generated_mcps_dir();
        if let Ok(entries) = fs::read_dir(&mcps_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_mcp_dir_name(&file_name)
                    && !desired.contains(&file_name)
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
        Ok(())
    }

    pub(super) fn remove_local_tinyfish_artifacts(&self, _profile_id: &str) -> Result<()> {
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn upsert_local_tinyfish_artifacts(
        &self,
        variant: TinyfishPluginVariant,
        tool_shell: TinyfishToolShell,
        plugin_manifest_json: &str,
        plugin_hooks_json: &str,
        output_style_text: &str,
        hook_script_text: &str,
        statusline_script_text: &str,
    ) -> Result<PathBuf> {
        let plugin_root = self.local_tinyfish_plugin_root(variant);
        let manifest_path = self.local_tinyfish_plugin_manifest_path(variant);
        let hooks_path = self.local_tinyfish_plugin_hooks_path(variant);
        let output_style_path = self.local_tinyfish_output_style_path(variant);
        let hook_script_path = self.local_tinyfish_hook_script_path(variant, tool_shell);
        let statusline_script_path =
            self.local_tinyfish_statusline_script_path(variant, tool_shell);
        Self::write_if_changed(&manifest_path, plugin_manifest_json)?;
        Self::write_if_changed(&hooks_path, plugin_hooks_json)?;
        Self::write_if_changed(&output_style_path, output_style_text)?;
        Self::write_if_changed(&hook_script_path, hook_script_text)?;
        Self::write_if_changed(&statusline_script_path, statusline_script_text)?;
        Self::set_executable_if_possible(&hook_script_path)?;
        Self::set_executable_if_possible(&statusline_script_path)?;
        Ok(plugin_root)
    }

    pub(super) fn sync_local_tinyfish_artifacts(&self, profiles: &[Profile]) -> Result<()> {
        let tool_shell = native_tinyfish_tool_shell();
        let mut desired_plugins = HashSet::new();

        for profile in profiles {
            if profile.kind != ProfileKind::Lightweight {
                continue;
            }
            let Some(env) = profile.env.as_ref() else {
                self.remove_local_tinyfish_artifacts(&profile.id)?;
                continue;
            };
            let (token, url) = self.resolve_credentials(profile)?;
            for local_gateway_mode in self.local_gateway_shim_modes(profile)? {
                if self.uses_simplified_local_default_shim(profile, local_gateway_mode)? {
                    continue;
                }
                let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                    env,
                    token.as_deref(),
                    url.as_deref(),
                    tool_shell,
                    local_gateway_mode,
                )?;
                if let (
                    Some(plugin_variant),
                    Some(plugin_manifest_json),
                    Some(plugin_hooks_json),
                    Some(output_style_text),
                    Some(hook_script_text),
                    Some(statusline_script_text),
                ) = (
                    artifacts.tinyfish_plugin_variant,
                    artifacts.tinyfish_plugin_manifest_json.as_deref(),
                    artifacts.tinyfish_plugin_hooks_json.as_deref(),
                    artifacts.tinyfish_output_style_text.as_deref(),
                    artifacts.tinyfish_hook_script_text.as_deref(),
                    artifacts.tinyfish_statusline_script_text.as_deref(),
                ) && desired_plugins.insert(Self::tinyfish_plugin_dir_name(plugin_variant))
                {
                    self.upsert_local_tinyfish_artifacts(
                        plugin_variant,
                        tool_shell,
                        plugin_manifest_json,
                        plugin_hooks_json,
                        output_style_text,
                        hook_script_text,
                        statusline_script_text,
                    )?;
                }
            }
        }

        let prompts_dir = self.generated_prompts_dir();
        if let Ok(entries) = fs::read_dir(&prompts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_prompt_name(&file_name) {
                    let _ = fs::remove_file(path);
                }
            }
        }

        let plugins_dir = self.generated_plugins_dir();
        if let Ok(entries) = fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_plugin_dir_name(&file_name)
                    && !desired_plugins.contains(&file_name)
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }

        Ok(())
    }
}
