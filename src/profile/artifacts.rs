use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::tinyfish::{
    TinyfishMode, TinyfishToolShell, build_lightweight_runtime_artifacts,
    native_tinyfish_tool_shell,
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

    pub(super) fn profile_mcp_config_relative_paths(
        profile: &Profile,
        remote_os: RemoteOs,
    ) -> [String; 2] {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match remote_os {
            RemoteOs::Unix => [
                format!("{dir_name}/.mcp.json"),
                format!("{dir_name}/mcp.json"),
            ],
            RemoteOs::Windows => [
                format!("{dir_name}\\.mcp.json"),
                format!("{dir_name}\\mcp.json"),
            ],
        }
    }

    pub(super) fn is_managed_generated_mcp_dir_name(file_name: &str) -> bool {
        file_name.starts_with("cswitch-mcp-profile-")
    }

    pub(super) fn tinyfish_plugin_dir_name(mode: TinyfishMode) -> Option<String> {
        match mode {
            TinyfishMode::None => None,
            TinyfishMode::SearchOnly => Some("tinyfish-search-only".to_string()),
            TinyfishMode::FetchOnly => Some("tinyfish-fetch-only".to_string()),
            TinyfishMode::Full => Some("tinyfish-full".to_string()),
        }
    }

    pub(super) fn tinyfish_prompt_file_name(
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
    ) -> Option<String> {
        let mode_name = match mode {
            TinyfishMode::None => return None,
            TinyfishMode::SearchOnly => "search-only",
            TinyfishMode::FetchOnly => "fetch-only",
            TinyfishMode::Full => "full",
        };
        let shell_name = match tool_shell {
            TinyfishToolShell::Bash => "bash",
            TinyfishToolShell::PowerShell => "powershell",
        };
        Some(format!("tinyfish-{mode_name}.{shell_name}.txt"))
    }

    pub(super) fn local_tinyfish_plugin_root(&self, mode: TinyfishMode) -> PathBuf {
        self.generated_plugins_dir().join(
            Self::tinyfish_plugin_dir_name(mode)
                .expect("plugin path is only valid for TinyFish modes"),
        )
    }

    pub(super) fn local_tinyfish_plugin_hooks_path(&self, mode: TinyfishMode) -> PathBuf {
        self.local_tinyfish_plugin_root(mode)
            .join("hooks")
            .join("hooks.json")
    }

    pub(super) fn local_tinyfish_plugin_manifest_path(&self, mode: TinyfishMode) -> PathBuf {
        self.local_tinyfish_plugin_root(mode)
            .join(".claude-plugin")
            .join("plugin.json")
    }

    pub(super) fn local_tinyfish_prompt_path(
        &self,
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
    ) -> PathBuf {
        self.generated_prompts_dir().join(
            Self::tinyfish_prompt_file_name(mode, tool_shell)
                .expect("prompt path is only valid for TinyFish modes"),
        )
    }

    pub(super) fn home_relative_tinyfish_prompt_path(
        mode: TinyfishMode,
        target_os: RemoteOs,
    ) -> String {
        let file_name = Self::tinyfish_prompt_file_name(
            mode,
            match target_os {
                RemoteOs::Unix => TinyfishToolShell::Bash,
                RemoteOs::Windows => TinyfishToolShell::PowerShell,
            },
        )
        .expect("prompt path is only valid for TinyFish modes");
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/prompts/{file_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\prompts\\{file_name}")
            }
        }
    }

    pub(super) fn home_relative_tinyfish_plugin_root(
        mode: TinyfishMode,
        target_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/plugins/{dir_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\plugins\\{dir_name}")
            }
        }
    }

    pub(super) fn tinyfish_plugin_hooks_relative_path(
        mode: TinyfishMode,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/hooks/hooks.json"),
            RemoteOs::Windows => format!("{dir_name}\\hooks\\hooks.json"),
        }
    }

    pub(super) fn tinyfish_plugin_manifest_relative_path(
        mode: TinyfishMode,
        remote_os: RemoteOs,
    ) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.claude-plugin/plugin.json"),
            RemoteOs::Windows => format!("{dir_name}\\.claude-plugin\\plugin.json"),
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
        matches!(
            file_name,
            "tinyfish-full" | "tinyfish-search-only" | "tinyfish-fetch-only"
        )
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
        let mcp_config = Self::profile_mcp_config(servers)?;
        Self::write_if_changed(&manifest_path, &Self::profile_mcp_plugin_manifest(profile)?)?;
        Self::write_if_changed(&plugin_root.join(".mcp.json"), &mcp_config)?;
        Self::write_if_changed(&plugin_root.join("mcp.json"), &mcp_config)?;
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

    pub(super) fn upsert_local_tinyfish_artifacts(
        &self,
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
        plugin_manifest_json: &str,
        plugin_hooks_json: &str,
        prompt_text: &str,
    ) -> Result<(PathBuf, PathBuf)> {
        let plugin_root = self.local_tinyfish_plugin_root(mode);
        let manifest_path = self.local_tinyfish_plugin_manifest_path(mode);
        let hooks_path = self.local_tinyfish_plugin_hooks_path(mode);
        let prompt_path = self.local_tinyfish_prompt_path(mode, tool_shell);
        Self::write_if_changed(&manifest_path, plugin_manifest_json)?;
        Self::write_if_changed(&hooks_path, plugin_hooks_json)?;
        Self::write_if_changed(&prompt_path, prompt_text)?;
        Ok((plugin_root, prompt_path))
    }

    pub(super) fn sync_local_tinyfish_artifacts(&self, profiles: &[Profile]) -> Result<()> {
        let tool_shell = native_tinyfish_tool_shell();
        let mut desired_prompts = HashSet::new();
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
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                tool_shell,
            )?;
            match (
                artifacts.tinyfish_plugin_manifest_json.as_deref(),
                artifacts.tinyfish_plugin_hooks_json.as_deref(),
                artifacts.tinyfish_prompt_text.as_deref(),
            ) {
                (Some(plugin_manifest_json), Some(plugin_hooks_json), Some(prompt_text)) => {
                    self.upsert_local_tinyfish_artifacts(
                        artifacts.tinyfish_mode,
                        tool_shell,
                        plugin_manifest_json,
                        plugin_hooks_json,
                        prompt_text,
                    )?;
                    desired_prompts.insert(
                        Self::tinyfish_prompt_file_name(artifacts.tinyfish_mode, tool_shell)
                            .expect("prompt file name should exist for TinyFish modes"),
                    );
                    desired_plugins.insert(
                        Self::tinyfish_plugin_dir_name(artifacts.tinyfish_mode)
                            .expect("plugin dir name should exist for TinyFish modes"),
                    );
                }
                _ => self.remove_local_tinyfish_artifacts(&profile.id)?,
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
                if Self::is_managed_generated_prompt_name(&file_name)
                    && !desired_prompts.contains(&file_name)
                {
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
