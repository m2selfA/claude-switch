use super::*;

impl ProfileManager {
    pub(super) fn local_cswitch_hint() -> Option<String> {
        std::env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn cmd_bin_dir() -> Result<PathBuf> {
        let home = Self::home_dir()?;
        Ok(Self::cmd_bin_dir_for_home(&home))
    }

    #[cfg(target_os = "windows")]
    pub(super) fn cmd_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".local").join("bin")
    }

    pub(super) fn local_gateway_shim_modes(
        &self,
        profile: &Profile,
    ) -> Result<Vec<LocalGatewayToolMode>> {
        let mut modes = vec![LocalGatewayToolMode::Auto];
        if self.resolved_local_gateway_base_url(profile)?.is_some() {
            modes.extend(LocalGatewayToolMode::EXPLICIT);
        }
        Ok(modes)
    }

    pub(super) fn shim_alias_name(
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> String {
        let alias_name = profile.alias.as_deref().unwrap_or(&profile.name);
        match local_gateway_mode.shim_suffix() {
            Some(suffix) => format!("{alias_name}-{suffix}"),
            None => alias_name.to_string(),
        }
    }

    pub(super) fn uses_simplified_local_default_shim(
        &self,
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<bool> {
        Ok(
            local_gateway_mode.is_auto()
                && self.resolved_local_gateway_base_url(profile)?.is_some(),
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn generate_cmd_content(&self, profile: &Profile) -> Result<String> {
        let hint = Self::local_cswitch_hint();
        self.generate_cmd_content_with_hint(profile, hint.as_deref(), LocalGatewayToolMode::Auto)
    }

    pub(super) fn generate_cmd_content_for_local_gateway_mode(
        &self,
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        let hint = Self::local_cswitch_hint();
        self.generate_cmd_content_with_hint(profile, hint.as_deref(), local_gateway_mode)
    }

    pub(super) fn generate_remote_cmd_content_for_local_gateway_mode(
        &self,
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        self.generate_cmd_content_with_hint(profile, None, local_gateway_mode)
    }

    fn generate_cmd_content_with_hint(
        &self,
        profile: &Profile,
        cswitch_hint: Option<&str>,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        if self.uses_simplified_local_default_shim(profile, local_gateway_mode)? {
            return self.generate_self_contained_cmd_content(profile);
        }
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
        lines.push(format!(
            ":: Local gateway mode: {}",
            local_gateway_mode.as_cli_value()
        ));
        lines.push(format!("set \"_PROFILE_ID={}\"", profile.id));
        if let Some(hint) = cswitch_hint {
            lines.push(format!("set \"_CSWITCH_HINT={}\"", hint));
        }

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
            let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                env,
                token.as_deref(),
                url.as_deref(),
                cmd_tool_shell,
                local_gateway_mode,
            )?;
            cmd_runtime = Some(artifacts);
        }

        let tf_enabled = cmd_runtime
            .as_ref()
            .map(|artifacts| artifacts.tinyfish_enabled)
            .unwrap_or(false);
        let tf_plugin_variant = cmd_runtime
            .as_ref()
            .and_then(|artifacts| artifacts.tinyfish_plugin_variant);
        let tf_required = cmd_runtime
            .as_ref()
            .map(|artifacts| artifacts.local_gateway_mode.requires_tinyfish())
            .unwrap_or(false);
        if let Some(plugin_variant) = tf_plugin_variant {
            lines.push("set \"_TF=\"".into());
            lines.push("where tinyfish >nul 2>&1 && set \"_TF=1\"".into());
            lines.push(
                "set \"_TF_PLUGIN_DIR=".to_string()
                    + &Self::home_relative_tinyfish_plugin_root(plugin_variant, RemoteOs::Windows)
                    + "\"",
            );
        }
        if has_launch {
            let args_str = profile.launch_args.as_ref().unwrap().join(" ");
            lines.push(format!("set \"_LAUNCH_ARGS={args_str}\""));
        }

        lines.push("set \"_E=1\"".into());
        lines.push("set \"_R=\"".into());
        let local_gateway_mode_arg = if local_gateway_mode.is_auto() {
            String::new()
        } else {
            format!(
                " --local-gateway-mode {}",
                local_gateway_mode.as_cli_value()
            )
        };
        lines.push(":loop".into());
        lines.push("if \"%~1\"==\"\" goto dispatch_launch".into());
        lines.push("if /i \"%~1\"==\"--no-extras\" (".into());
        lines.push("    set \"_E=\"".into());
        lines.push("    shift".into());
        lines.push("    goto loop".into());
        lines.push(")".into());
        lines.push("set \"_R=%_R% %1\"".into());
        lines.push("shift".into());
        lines.push("goto loop".into());
        lines.push(":dispatch_launch".into());
        lines.push("set \"_MODE=%CLAUDE_SWITCH_SHIM_MODE%\"".into());
        lines.push("if not defined _MODE set \"_MODE=auto\"".into());
        lines.push("if /i \"%_MODE%\"==\"legacy\" goto build_settings".into());
        lines.push("set \"_CSWITCH=\"".into());
        lines.push("if defined _CSWITCH_HINT if exist \"%_CSWITCH_HINT%\" set \"_CSWITCH=%_CSWITCH_HINT%\"".into());
        lines.push(
            "if not defined _CSWITCH where cswitch >nul 2>&1 && set \"_CSWITCH=cswitch\"".into(),
        );
        lines.push("if not defined _CSWITCH goto dynamic_unavailable".into());
        lines.push("\"%_CSWITCH%\" shim launch --probe >nul 2>&1".into());
        lines.push("if errorlevel 1 goto dynamic_unavailable".into());
        lines.push("if defined _E (".into());
        lines.push(format!(
            "    \"%_CSWITCH%\" shim launch --profile-id \"%_PROFILE_ID%\"{local_gateway_mode_arg} -- %_R%"
        ));
        lines.push(") else (".into());
        lines.push(format!(
            "    \"%_CSWITCH%\" shim launch --profile-id \"%_PROFILE_ID%\"{local_gateway_mode_arg} --no-extras -- %_R%"
        ));
        lines.push(")".into());
        lines.push("exit /b %errorlevel%".into());
        lines.push(":dynamic_unavailable".into());
        lines.push("if /i \"%_MODE%\"==\"dynamic\" (".into());
        lines.push("    >&2 echo claude-switch dynamic launch unavailable for this shim.".into());
        lines.push("    exit /b 1".into());
        lines.push(")".into());
        lines.push(
            ">&2 echo claude-switch dynamic launch unavailable; falling back to legacy shim."
                .into(),
        );
        lines.push(":build_settings".into());

        let hosted_plugin_roots =
            self.profile_plugin_home_relative_roots(profile, RemoteOs::Windows)?;
        if !hosted_plugin_roots.is_empty() {
            let hosted_plugin_args = hosted_plugin_roots
                .iter()
                .map(|root| format!(" --plugin-dir \"{root}\""))
                .collect::<String>();
            lines.push(format!("set \"_HOSTED_PLUGIN_ARGS={hosted_plugin_args}\""));
        } else {
            lines.push("set \"_HOSTED_PLUGIN_ARGS=\"".into());
        }
        let hosted_plugin_part = if hosted_plugin_roots.is_empty() {
            ""
        } else {
            "%_HOSTED_PLUGIN_ARGS%"
        };

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
            if let Some(plugin_variant) = tf_plugin_variant {
                let tf_statusline_script = format!(
                    "$env:USERPROFILE/.claude-switch/generated/plugins/{}/scripts/{}",
                    plugin_variant.dir_name(),
                    tinyfish_statusline_script_file_name(cmd_tool_shell)
                );
                let env = profile
                    .env
                    .as_ref()
                    .context("Lightweight profile env missing while generating CMD shim")?;
                let (token, url) = self.resolve_credentials(profile)?;
                let tinyfish_settings_json =
                    serde_json::to_string(&build_lightweight_settings_with_local_gateway_mode(
                        env,
                        token.as_deref(),
                        url.as_deref(),
                        true,
                        cmd_tool_shell,
                        Some(&tf_statusline_script),
                        runtime.local_gateway_mode,
                    )?)
                    .context("Failed to serialize TinyFish router CMD settings JSON")?;
                assign_cmd_json_var(&mut lines, "_TF_SETTINGS", &tinyfish_settings_json);
                if has_launch {
                    lines.push("if defined _TF if defined _E goto launch_with_hooks_extras".into());
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    if tf_required {
                        lines.push("goto tinyfish_required".into());
                    } else {
                        lines.push("if defined _E goto launch_with_extras".into());
                    }
                    lines.push("goto launch_plain".into());
                } else {
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    if tf_required {
                        lines.push("goto tinyfish_required".into());
                    } else {
                        lines.push("goto launch_plain".into());
                    }
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
            if tf_enabled {
                lines.push(":launch_with_hooks_extras".into());
                lines.push(format!("claude{hosted_plugin_part} --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\"{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"));
                lines.push("exit /b %errorlevel%".into());
            }
            lines.push(":launch_with_extras".into());
            lines.push(format!(
                "{settings_prefix}{hosted_plugin_part}{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"
            ));
            lines.push("exit /b %errorlevel%".into());
        }

        if tf_enabled {
            lines.push(":launch_with_hooks_plain".into());
            lines.push(format!("claude{hosted_plugin_part} --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\"{mcp_plugin_part} %_R%"));
            lines.push("exit /b %errorlevel%".into());
        }

        if tf_required {
            lines.push(":tinyfish_required".into());
            lines.push(">&2 echo TinyFish is required for this shim variant but the 'tinyfish' command is unavailable.".into());
            lines.push("exit /b 1".into());
        }

        lines.push(":launch_plain".into());
        lines.push(format!(
            "{settings_prefix}{hosted_plugin_part}{mcp_plugin_part} %_R%"
        ));
        lines.push("exit /b %errorlevel%".into());

        Ok(lines.join("\r\n") + "\r\n")
    }

    fn generate_self_contained_cmd_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile
            .launch_args
            .as_ref()
            .is_some_and(|args| !args.is_empty());
        let mut lines: Vec<String> = Vec::new();
        lines.push("@echo off".into());
        lines.push("setlocal EnableExtensions DisableDelayedExpansion".into());
        lines.push(CMD_MARKER.into());
        lines.push(format!(":: Profile: {} ({})", profile.name, kind_label));

        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("set \"CLAUDE_CONFIG_DIR={}\"", dir.display()));
        }

        let mut cmd_runtime = None;
        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                env,
                token.as_deref(),
                url.as_deref(),
                TinyfishToolShell::PowerShell,
                LocalGatewayToolMode::Auto,
            )?;
            cmd_runtime = Some(artifacts);
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

        let hosted_plugin_roots =
            self.profile_plugin_home_relative_roots(profile, RemoteOs::Windows)?;
        if !hosted_plugin_roots.is_empty() {
            let hosted_plugin_args = hosted_plugin_roots
                .iter()
                .map(|root| format!(" --plugin-dir \"{root}\""))
                .collect::<String>();
            lines.push(format!("set \"_HOSTED_PLUGIN_ARGS={hosted_plugin_args}\""));
        } else {
            lines.push("set \"_HOSTED_PLUGIN_ARGS=\"".into());
        }
        let hosted_plugin_part = if hosted_plugin_roots.is_empty() {
            ""
        } else {
            "%_HOSTED_PLUGIN_ARGS%"
        };

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
        }
        if has_launch {
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
            lines.push(":launch_with_extras".into());
            lines.push(format!(
                "{settings_prefix}{hosted_plugin_part}{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"
            ));
            lines.push("exit /b %errorlevel%".into());
        }

        lines.push(":launch_plain".into());
        lines.push(format!(
            "{settings_prefix}{hosted_plugin_part}{mcp_plugin_part} %_R%"
        ));
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
            for local_gateway_mode in self.local_gateway_shim_modes(p)? {
                let alias_name = Self::shim_alias_name(p, local_gateway_mode);
                let cmd_name = format!("claude-{}.cmd", alias_name);
                let cmd_path = bin_dir.join(&cmd_name);
                let content =
                    self.generate_cmd_content_for_local_gateway_mode(p, local_gateway_mode)?;
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

    #[cfg(any(test, not(target_os = "windows")))]
    pub(super) fn sh_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".varusers").join("bin")
    }

    #[cfg(any(test, not(target_os = "windows")))]
    pub(super) fn local_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".local").join("bin")
    }

    #[cfg(any(test, not(target_os = "windows")))]
    pub(super) fn unix_path_env_contains_dir(
        path_env: Option<&std::ffi::OsStr>,
        dir: &Path,
    ) -> bool {
        let Some(path_env) = path_env else {
            return false;
        };
        std::env::split_paths(path_env).any(|entry| {
            entry == dir
                || match (fs::canonicalize(&entry), fs::canonicalize(dir)) {
                    (Ok(left), Ok(right)) => left == right,
                    _ => false,
                }
        })
    }

    #[cfg(any(test, not(target_os = "windows")))]
    pub(super) fn preferred_local_shim_bin_dir_for_home(
        home: &Path,
        path_env: Option<&std::ffi::OsStr>,
    ) -> Option<PathBuf> {
        let varusers_bin = Self::sh_bin_dir_for_home(home);
        if varusers_bin.exists()
            && varusers_bin.is_dir()
            && Self::unix_path_env_contains_dir(path_env, &varusers_bin)
        {
            return Some(varusers_bin);
        }

        let local_bin = Self::local_bin_dir_for_home(home);
        if local_bin.exists() && local_bin.is_dir() {
            return Some(local_bin);
        }

        None
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn preferred_local_shim_bin_dir() -> Result<Option<PathBuf>> {
        let home = Self::home_dir()?;
        Ok(Self::preferred_local_shim_bin_dir_for_home(
            &home,
            std::env::var_os("PATH").as_deref(),
        ))
    }

    pub(super) fn escape_sh_value(s: &str) -> String {
        s.replace('\'', "'\\''")
    }

    #[cfg(any(test, not(target_os = "windows")))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn generate_sh_content(&self, profile: &Profile) -> Result<String> {
        let hint = Self::local_cswitch_hint();
        self.generate_sh_content_with_hint(profile, hint.as_deref(), LocalGatewayToolMode::Auto)
    }

    #[cfg(any(test, not(target_os = "windows")))]
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(super) fn generate_sh_content_for_local_gateway_mode(
        &self,
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        let hint = Self::local_cswitch_hint();
        self.generate_sh_content_with_hint(profile, hint.as_deref(), local_gateway_mode)
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(super) fn generate_remote_sh_content_for_local_gateway_mode(
        &self,
        profile: &Profile,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        self.generate_sh_content_with_hint(profile, None, local_gateway_mode)
    }

    fn generate_sh_content_with_hint(
        &self,
        profile: &Profile,
        cswitch_hint: Option<&str>,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<String> {
        if self.uses_simplified_local_default_shim(profile, local_gateway_mode)? {
            return self.generate_self_contained_sh_content(profile);
        }
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
        lines.push(format!(
            "# Local gateway mode: {}",
            local_gateway_mode.as_cli_value()
        ));
        lines.push("set -euo pipefail".into());
        lines.push(format!(
            "PROFILE_ID='{}'",
            Self::escape_sh_value(&profile.id)
        ));
        lines.push(format!(
            "CSWITCH_HINT='{}'",
            Self::escape_sh_value(cswitch_hint.unwrap_or_default())
        ));
        lines.push(String::new());

        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("export CLAUDE_CONFIG_DIR=\"{}\"", dir.display()));
        }

        let mut settings_enabled = false;
        let mut tinyfish_enabled_for_profile = false;
        let mut tinyfish_required = false;
        let sh_tool_shell = TinyfishToolShell::Bash;

        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                env,
                token.as_deref(),
                url.as_deref(),
                sh_tool_shell,
                local_gateway_mode,
            )?;
            let base_settings_json = artifacts.base_settings_json.clone();
            tinyfish_enabled_for_profile = artifacts.tinyfish_enabled;
            let tinyfish_plugin_variant = artifacts.tinyfish_plugin_variant;
            tinyfish_required = artifacts.local_gateway_mode.requires_tinyfish();
            lines.push(format!(
                "BASE_SETTINGS='{}'",
                Self::escape_sh_value(&base_settings_json)
            ));
            settings_enabled = true;
            if let Some(plugin_variant) = tinyfish_plugin_variant {
                let tf_statusline_script = format!(
                    "$HOME/.claude-switch/generated/plugins/{}/scripts/{}",
                    plugin_variant.dir_name(),
                    tinyfish_statusline_script_file_name(sh_tool_shell)
                );
                let tf_settings_json =
                    serde_json::to_string(&build_lightweight_settings_with_local_gateway_mode(
                        env,
                        token.as_deref(),
                        url.as_deref(),
                        true,
                        sh_tool_shell,
                        Some(&tf_statusline_script),
                        artifacts.local_gateway_mode,
                    )?)
                    .context("Failed to serialize TinyFish router shell settings JSON")?;
                lines.push(format!(
                    "TF_SETTINGS='{}'",
                    Self::escape_sh_value(&tf_settings_json)
                ));
                lines.push(format!(
                    "TF_PLUGIN_DIR=\"{}\"",
                    Self::home_relative_tinyfish_plugin_root(plugin_variant, RemoteOs::Unix)
                ));
            }
        }

        if settings_enabled {
            lines.push("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")".into());
        }

        if tinyfish_enabled_for_profile {
            lines.push(String::new());
            lines.push("# Check if tinyfish is available for web search/fetch".into());
            lines.push("if command -v tinyfish >/dev/null 2>&1; then".into());
            lines.push("    TF_PLUGIN_ARGS=(--plugin-dir \"$TF_PLUGIN_DIR\")".into());
            lines.push("    SETTINGS_ARG=(--settings \"$TF_SETTINGS\")".into());
            lines.push("else".into());
            if tinyfish_required {
                lines.push(
                    "    echo \"TinyFish is required for this shim variant but the 'tinyfish' command is unavailable.\" >&2"
                        .into(),
                );
                lines.push("    exit 1".into());
            } else {
                lines.push("    TF_PLUGIN_ARGS=()".into());
            }
            lines.push("fi".into());
        } else {
            lines.push("TF_PLUGIN_ARGS=()".into());
        }

        let hosted_plugin_roots =
            self.profile_plugin_home_relative_roots(profile, RemoteOs::Unix)?;
        if !hosted_plugin_roots.is_empty() {
            let mut hosted_plugin_line = "HOSTED_PLUGIN_ARGS=(".to_string();
            for root in &hosted_plugin_roots {
                hosted_plugin_line.push_str(&format!("--plugin-dir \"{root}\" "));
            }
            hosted_plugin_line.push(')');
            lines.push(hosted_plugin_line);
        } else {
            lines.push("HOSTED_PLUGIN_ARGS=()".into());
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
        lines.push("MODE=\"${CLAUDE_SWITCH_SHIM_MODE:-auto}\"".into());
        let local_gateway_mode_arg = if local_gateway_mode.is_auto() {
            String::new()
        } else {
            format!(
                " --local-gateway-mode {}",
                local_gateway_mode.as_cli_value()
            )
        };
        lines.push("if [[ \"$MODE\" != \"legacy\" ]]; then".into());
        lines.push("    CSWITCH_CMD=\"\"".into());
        lines.push("    if [[ -n \"$CSWITCH_HINT\" && -x \"$CSWITCH_HINT\" ]]; then".into());
        lines.push("        CSWITCH_CMD=\"$CSWITCH_HINT\"".into());
        lines.push("    elif command -v cswitch >/dev/null 2>&1; then".into());
        lines.push("        CSWITCH_CMD=\"$(command -v cswitch)\"".into());
        lines.push("    fi".into());
        lines.push("    if [[ -n \"$CSWITCH_CMD\" ]] && \"$CSWITCH_CMD\" shim launch --probe >/dev/null 2>&1; then".into());
        if has_launch {
            lines.push("        if $EXTRA; then".into());
            lines.push(format!(
                "            exec \"$CSWITCH_CMD\" shim launch --profile-id \"$PROFILE_ID\"{local_gateway_mode_arg} -- \"${{ARGS[@]}}\""
            ));
            lines.push("        else".into());
            lines.push(format!(
                "            exec \"$CSWITCH_CMD\" shim launch --profile-id \"$PROFILE_ID\"{local_gateway_mode_arg} --no-extras -- \"${{ARGS[@]}}\""
            ));
            lines.push("        fi".into());
        } else {
            lines.push(format!(
                "        exec \"$CSWITCH_CMD\" shim launch --profile-id \"$PROFILE_ID\"{local_gateway_mode_arg} -- \"${{ARGS[@]}}\""
            ));
        }
        lines.push("    fi".into());
        lines.push("    if [[ \"$MODE\" == \"dynamic\" ]]; then".into());
        lines.push(
            "        echo \"claude-switch dynamic launch unavailable for this shim.\" >&2".into(),
        );
        lines.push("        exit 1".into());
        lines.push("    fi".into());
        lines.push("    echo \"claude-switch dynamic launch unavailable; falling back to legacy shim.\" >&2".into());
        lines.push("fi".into());

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
                "if $EXTRA; then exec claude{0}{1} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{TF_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; else exec claude{0} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{TF_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; fi",
                settings_part, launch_part
            ));
        } else {
            lines.push(format!(
                "exec claude{0} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{TF_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"",
                settings_part
            ));
        }

        Ok(lines.join("\n") + "\n")
    }

    fn generate_self_contained_sh_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile
            .launch_args
            .as_ref()
            .is_some_and(|args| !args.is_empty());
        let launch_str = profile
            .launch_args
            .as_ref()
            .map(|args| args.join(" "))
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
        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                env,
                token.as_deref(),
                url.as_deref(),
                TinyfishToolShell::Bash,
                LocalGatewayToolMode::Auto,
            )?;
            lines.push(format!(
                "BASE_SETTINGS='{}'",
                Self::escape_sh_value(&artifacts.base_settings_json)
            ));
            settings_enabled = true;
        }

        if settings_enabled {
            lines.push("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")".into());
        }

        let hosted_plugin_roots =
            self.profile_plugin_home_relative_roots(profile, RemoteOs::Unix)?;
        if !hosted_plugin_roots.is_empty() {
            let mut hosted_plugin_line = "HOSTED_PLUGIN_ARGS=(".to_string();
            for root in &hosted_plugin_roots {
                hosted_plugin_line.push_str(&format!("--plugin-dir \"{root}\" "));
            }
            hosted_plugin_line.push(')');
            lines.push(hosted_plugin_line);
        } else {
            lines.push("HOSTED_PLUGIN_ARGS=()".into());
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
            format!(" {}", launch_str)
        } else {
            String::new()
        };

        if has_launch {
            lines.push(format!(
                "if $EXTRA; then exec claude{settings_part}{launch_part} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; else exec claude{settings_part} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; fi"
            ));
        } else {
            lines.push(format!(
                "exec claude{settings_part} \"${{HOSTED_PLUGIN_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\""
            ));
        }

        Ok(lines.join("\n") + "\n")
    }

    #[cfg(not(target_os = "windows"))]
    pub fn sync_sh_scripts(&self) -> Result<String> {
        let Some(bin_dir) = Self::preferred_local_shim_bin_dir()? else {
            return Ok(String::new());
        };
        self.sync_sh_scripts_to_dir(&bin_dir)
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn sync_sh_scripts_to_dir(&self, bin_dir: &Path) -> Result<String> {
        let profiles = self.list_profiles()?;
        self.sync_local_tinyfish_artifacts(&profiles)?;
        self.sync_local_mcp_artifacts(&profiles)?;

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut report = Vec::new();

        for p in &profiles {
            for local_gateway_mode in self.local_gateway_shim_modes(p)? {
                let alias_name = Self::shim_alias_name(p, local_gateway_mode);
                let sh_name = format!("claude-{}", alias_name);
                let sh_path = bin_dir.join(&sh_name);
                let content =
                    self.generate_sh_content_for_local_gateway_mode(p, local_gateway_mode)?;
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
