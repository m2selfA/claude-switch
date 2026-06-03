use super::*;

impl ProfileManager {
    #[cfg(target_os = "windows")]
    pub(super) fn cmd_bin_dir() -> Result<PathBuf> {
        let home = Self::home_dir()?;
        Ok(Self::cmd_bin_dir_for_home(&home))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn cmd_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".local").join("bin")
    }

    pub(super) fn build_sh_settings_env_prefix(
        env: &LightweightEnv,
        token: Option<&str>,
        url: Option<&str>,
    ) -> String {
        // The tail added by `build_sh_settings_tail` closes the root settings object.
        let prefix = build_lightweight_settings_env_prefix(env, token, url);
        format!("'{}'", Self::escape_sh_value(&prefix))
    }

    pub(super) fn build_sh_settings_tail(
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
    ) -> String {
        if let Some(allowlist) = tinyfish_permissions_allowlist(mode, tool_shell) {
            let permissions_json = serde_json::json!({
                "allow": allowlist,
            })
            .to_string();
            return format!(
                "'{}'",
                Self::escape_sh_value(&format!(",\"permissions\":{permissions_json}}}"))
            );
        }
        "'}'".to_string()
    }

    pub(super) fn generate_cmd_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile.launch_args.as_ref().is_some_and(|a| !a.is_empty());

        let mut lines: Vec<String> = Vec::new();
        lines.push("@echo off".into());
        lines.push("setlocal EnableExtensions DisableDelayedExpansion".into());
        lines.push(CMD_MARKER.into());
        lines.push(format!(":: Profile: {} ({})", profile.name, kind_label));

        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("set \"CLAUDE_CONFIG_DIR={}\"", dir.display()));
        }

        let cmd_tool_shell = TinyfishToolShell::PowerShell;
        let mut cmd_runtime = None;

        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                cmd_tool_shell,
            )?;
            cmd_runtime = Some(artifacts);
        }

        let tf_mode = cmd_runtime
            .as_ref()
            .map(|artifacts| artifacts.tinyfish_mode)
            .unwrap_or(TinyfishMode::None);
        if tf_mode != TinyfishMode::None {
            lines.push("set \"_TF=\"".into());
            lines.push("where tinyfish >nul 2>&1 && set \"_TF=1\"".into());
            lines.push(
                "set \"_TF_PLUGIN_DIR=".to_string()
                    + &Self::home_relative_tinyfish_plugin_root(tf_mode, RemoteOs::Windows)
                    + "\"",
            );
            lines.push(format!(
                "set \"_TF_PROMPT_FILE={}\"",
                Self::home_relative_tinyfish_prompt_path(tf_mode, RemoteOs::Windows)
            ));
        }
        if has_launch {
            let args_str = profile.launch_args.as_ref().unwrap().join(" ");
            lines.push(format!("set \"_LAUNCH_ARGS={args_str}\""));
        }

        lines.push("set \"_E=1\"".into());
        lines.push("set \"_R=\"".into());
        lines.push(":loop".into());
        lines.push("if \"%~1\"==\"\" goto build_settings".into());
        lines.push("if /i \"%~1\"==\"--no-extras\" (".into());
        lines.push("    set \"_E=\"".into());
        lines.push("    shift".into());
        lines.push("    goto loop".into());
        lines.push(")".into());
        lines.push("set \"_R=%_R% %1\"".into());
        lines.push("shift".into());
        lines.push("goto loop".into());
        lines.push(":build_settings".into());

        let mcp_servers = self.profile_mcp_servers(profile)?;
        let mcp_plugin_enabled = !mcp_servers.is_empty();
        if mcp_plugin_enabled {
            lines.push(
                "set \"_MCP_PLUGIN_DIR=".to_string()
                    + &Self::home_relative_profile_mcp_plugin_root(profile, RemoteOs::Windows)
                    + "\"",
            );
        }

        if let Some(runtime) = cmd_runtime.as_ref() {
            assign_cmd_json_var(&mut lines, "_SETTINGS", &runtime.base_settings_json);
            if tf_mode != TinyfishMode::None {
                let tinyfish_settings_json = runtime
                    .tinyfish_settings_json
                    .as_deref()
                    .context("TinyFish settings missing for non-native mode")?;
                assign_cmd_json_var(&mut lines, "_TF_SETTINGS", tinyfish_settings_json);
                if has_launch {
                    lines.push("if defined _TF if defined _E goto launch_with_hooks_extras".into());
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    lines.push("if defined _E goto launch_with_extras".into());
                    lines.push("goto launch_plain".into());
                } else {
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    lines.push("goto launch_plain".into());
                }
            } else if has_launch {
                lines.push("if defined _E goto launch_with_extras".into());
                lines.push("goto launch_plain".into());
            } else {
                lines.push("goto launch_plain".into());
            }
        } else if has_launch {
            lines.push("if defined _E goto launch_with_extras".into());
            lines.push("goto launch_plain".into());
        } else {
            lines.push("goto launch_plain".into());
        }

        let settings_prefix = if cmd_runtime.is_some() {
            "claude --settings \"%_SETTINGS%\""
        } else {
            "claude"
        };
        let mcp_plugin_part = if mcp_plugin_enabled {
            " --plugin-dir \"%_MCP_PLUGIN_DIR%\""
        } else {
            ""
        };

        if has_launch {
            if tf_mode != TinyfishMode::None {
                lines.push(":launch_with_hooks_extras".into());
                lines.push(format!("claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\"{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"));
                lines.push("exit /b %errorlevel%".into());
            }
            lines.push(":launch_with_extras".into());
            lines.push(format!(
                "{settings_prefix}{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"
            ));
            lines.push("exit /b %errorlevel%".into());
        }

        if tf_mode != TinyfishMode::None {
            lines.push(":launch_with_hooks_plain".into());
            lines.push(format!("claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\"{mcp_plugin_part} %_R%"));
            lines.push("exit /b %errorlevel%".into());
        }

        lines.push(":launch_plain".into());
        lines.push(format!("{settings_prefix}{mcp_plugin_part} %_R%"));
        lines.push("exit /b %errorlevel%".into());

        Ok(lines.join("\r\n") + "\r\n")
    }

    #[cfg(target_os = "windows")]
    pub fn sync_cmd_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        self.sync_local_tinyfish_artifacts(&profiles)?;
        self.sync_local_mcp_artifacts(&profiles)?;
        let bin_dir = Self::cmd_bin_dir()?;
        fs::create_dir_all(&bin_dir)?;

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut report = Vec::new();

        for p in &profiles {
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            let cmd_name = format!("claude-{}.cmd", alias_name);
            let cmd_path = bin_dir.join(&cmd_name);
            let content = self.generate_cmd_content(p)?;
            let needs_write = match fs::read_to_string(&cmd_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                fs::write(&cmd_path, &content)?;
                report.push(format!("  + {}", cmd_path.display()));
            } else {
                report.push(format!("  = {}", cmd_path.display()));
            }
            written.insert(cmd_name.to_lowercase());
        }

        // Remove stale cmd files (have marker but no matching profile)
        if let Ok(entries) = fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "cmd") {
                    let fname = path.file_name().unwrap().to_string_lossy().to_lowercase();
                    if !written.contains(&fname)
                        && let Ok(content) = fs::read_to_string(&path)
                        && content.contains(CMD_MARKER)
                    {
                        let _ = fs::remove_file(&path);
                        report.push(format!("  - {} (stale)", path.display()));
                    }
                }
            }
        }

        let bin_str = bin_dir.display().to_string();
        Ok(format!(
            "# CMD aliases synced to {} ({} profiles)\n{}",
            bin_str,
            profiles.len(),
            report.join("\n")
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn sh_bin_dir() -> Result<PathBuf> {
        let home = Self::home_dir()?;
        Ok(Self::sh_bin_dir_for_home(&home))
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn sh_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".varusers").join("bin")
    }

    pub(super) fn escape_sh_value(s: &str) -> String {
        s.replace('\'', "'\\''")
    }

    pub(super) fn generate_sh_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile.launch_args.as_ref().is_some_and(|a| !a.is_empty());
        let launch_str = profile
            .launch_args
            .as_ref()
            .map(|a| a.join(" "))
            .unwrap_or_default();

        let mut lines: Vec<String> = Vec::new();

        lines.push("#!/usr/bin/env bash".into());
        lines.push(SH_MARKER.into());
        lines.push(format!("# Profile: {} ({})", profile.name, kind_label));
        lines.push("set -euo pipefail".into());
        lines.push(String::new());

        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("export CLAUDE_CONFIG_DIR=\"{}\"", dir.display()));
        }

        let mut settings_enabled = false;
        let mut tinyfish_mode_for_profile = TinyfishMode::None;
        let sh_tool_shell = TinyfishToolShell::Bash;

        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                sh_tool_shell,
            )?;
            tinyfish_mode_for_profile = artifacts.tinyfish_mode;
            lines.push(format!(
                "SETTINGS_ENV={}",
                Self::build_sh_settings_env_prefix(env, token.as_deref(), url.as_deref())
            ));
            lines.push(format!(
                "BASE_SETTINGS=\"${{SETTINGS_ENV}}\"{}",
                Self::build_sh_settings_tail(TinyfishMode::None, sh_tool_shell)
            ));
            settings_enabled = true;
            if tinyfish_mode_for_profile != TinyfishMode::None {
                lines.push(format!(
                    "TF_SETTINGS=\"${{SETTINGS_ENV}}\"{}",
                    Self::build_sh_settings_tail(tinyfish_mode_for_profile, sh_tool_shell)
                ));
                lines.push(format!(
                    "TF_PROMPT_FILE=\"{}\"",
                    Self::home_relative_tinyfish_prompt_path(
                        tinyfish_mode_for_profile,
                        RemoteOs::Unix
                    )
                ));
                lines.push(format!(
                    "TF_PLUGIN_DIR=\"{}\"",
                    Self::home_relative_tinyfish_plugin_root(
                        tinyfish_mode_for_profile,
                        RemoteOs::Unix
                    )
                ));
            }
        }

        if settings_enabled {
            lines.push("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")".into());
        }

        if tinyfish_mode_for_profile != TinyfishMode::None {
            lines.push(String::new());
            lines.push("# Check if tinyfish is available for web search/fetch".into());
            lines.push("if command -v tinyfish >/dev/null 2>&1; then".into());
            lines.push("    TF_PLUGIN_ARGS=(--plugin-dir \"$TF_PLUGIN_DIR\")".into());
            lines.push("    TF_SP_ARGS=(--append-system-prompt-file \"$TF_PROMPT_FILE\")".into());
            lines.push("    SETTINGS_ARG=(--settings \"$TF_SETTINGS\")".into());
            lines.push("else".into());
            lines.push("    TF_PLUGIN_ARGS=()".into());
            lines.push("    TF_SP_ARGS=()".into());
            lines.push("fi".into());
        } else {
            lines.push("TF_PLUGIN_ARGS=()".into());
            lines.push("TF_SP_ARGS=()".into());
        }

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            lines.push(format!(
                "MCP_PLUGIN_ARGS=(--plugin-dir \"{}\")",
                Self::home_relative_profile_mcp_plugin_root(profile, RemoteOs::Unix)
            ));
        } else {
            lines.push("MCP_PLUGIN_ARGS=()".into());
        }

        lines.push(String::new());
        lines.push("EXTRA=true".into());
        lines.push("ARGS=()".into());
        lines.push("while [[ $# -gt 0 ]]; do".into());
        lines.push("    case \"$1\" in".into());
        lines.push("        --no-extras) EXTRA=false; shift ;;".into());
        lines.push("        *) ARGS+=(\"$1\"); shift ;;".into());
        lines.push("    esac".into());
        lines.push("done".into());

        let settings_part = if settings_enabled {
            " \"${SETTINGS_ARG[@]}\""
        } else {
            ""
        };
        let launch_part = if has_launch {
            &format!(" {}", launch_str)
        } else {
            ""
        };

        if has_launch {
            lines.push(format!(
                "if $EXTRA; then exec claude{0}{1} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; else exec claude{0} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; fi",
                settings_part, launch_part
            ));
        } else {
            lines.push(format!(
                "exec claude{0} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"",
                settings_part
            ));
        }

        Ok(lines.join("\n") + "\n")
    }

    #[cfg(not(target_os = "windows"))]
    pub fn sync_sh_scripts(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        self.sync_local_tinyfish_artifacts(&profiles)?;
        self.sync_local_mcp_artifacts(&profiles)?;
        let bin_dir = Self::sh_bin_dir()?;
        if !bin_dir.exists() || !bin_dir.is_dir() {
            return Ok(String::new());
        }

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut report = Vec::new();

        for p in &profiles {
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            let sh_name = format!("claude-{}", alias_name);
            let sh_path = bin_dir.join(&sh_name);
            let content = self.generate_sh_content(p)?;
            let needs_write = match fs::read_to_string(&sh_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                fs::write(&sh_path, &content)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&sh_path)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&sh_path, perms)?;
                }
                report.push(format!("  + {}", sh_path.display()));
            } else {
                report.push(format!("  = {}", sh_path.display()));
            }
            written.insert(sh_name);
        }

        if let Ok(entries) = fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let fname = path.file_name().unwrap().to_string_lossy();
                if !written.contains(fname.as_ref())
                    && let Ok(content) = fs::read_to_string(&path)
                    && content.contains(SH_MARKER)
                {
                    let _ = fs::remove_file(&path);
                    report.push(format!("  - {} (stale)", path.display()));
                }
            }
        }

        Ok(format!(
            "{} profiles\n{}",
            profiles.len(),
            report.join("\n")
        ))
    }
}
