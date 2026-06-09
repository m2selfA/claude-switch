use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LaunchOptions {
    pub(crate) use_stored_args: bool,
    pub(crate) local_gateway_mode: RequestedLocalGatewayMode,
}

impl ProfileManager {
    fn lightweight_launch_artifacts(
        &self,
        profile: &Profile,
        resolved_token: Option<&str>,
        resolved_url: Option<&str>,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<LightweightRuntimeArtifacts> {
        let env = profile
            .env
            .as_ref()
            .context("Lightweight profile env missing during launch")?;
        let tool_shell = native_tinyfish_tool_shell();
        build_lightweight_runtime_artifacts_with_local_gateway_mode(
            env,
            resolved_token,
            resolved_url,
            tool_shell,
            local_gateway_mode,
        )
    }

    pub(crate) fn resolved_local_gateway_base_url(
        &self,
        profile: &Profile,
    ) -> Result<Option<String>> {
        if profile.kind != ProfileKind::Lightweight || profile.env.is_none() {
            return Ok(None);
        }
        let (_, resolved_url) = self.resolve_credentials(profile)?;
        Ok(resolved_url.filter(|url| is_local_runtime_base_url(url)))
    }

    pub(super) fn prepare_direct_lightweight_launch_with_tinyfish_available(
        &self,
        cmd: &mut std::process::Command,
        profile: &Profile,
        tinyfish_router_available: bool,
        local_gateway_mode: LocalGatewayToolMode,
    ) -> Result<()> {
        let env = profile
            .env
            .as_ref()
            .context("Lightweight profile env missing during launch")?;
        let (resolved_token, resolved_url) = self.resolve_credentials(profile)?;
        let tool_shell = native_tinyfish_tool_shell();
        let artifacts = self.lightweight_launch_artifacts(
            profile,
            resolved_token.as_deref(),
            resolved_url.as_deref(),
            local_gateway_mode,
        )?;
        if artifacts.local_gateway_mode.requires_tinyfish() && !tinyfish_router_available {
            bail!(
                "TinyFish is required for local gateway mode '{}' but the 'tinyfish' command is unavailable.",
                artifacts.local_gateway_mode.as_cli_value()
            );
        }
        let tinyfish_enabled = artifacts.tinyfish_enabled && tinyfish_router_available;
        let tinyfish_plugin_variant = if tinyfish_enabled {
            Some(
                artifacts
                    .tinyfish_plugin_variant
                    .context("TinyFish plugin variant missing for direct runtime launch")?,
            )
        } else {
            None
        };

        if let Some(plugin_variant) = tinyfish_plugin_variant {
            let plugin_hooks_json = artifacts
                .tinyfish_plugin_hooks_json
                .as_deref()
                .context("TinyFish plugin hooks missing for runtime router")?;
            let plugin_manifest_json = artifacts
                .tinyfish_plugin_manifest_json
                .as_deref()
                .context("TinyFish plugin manifest missing for runtime router")?;
            let output_style_text = artifacts
                .tinyfish_output_style_text
                .as_deref()
                .context("TinyFish output style missing for runtime router")?;
            let hook_script_text = artifacts
                .tinyfish_hook_script_text
                .as_deref()
                .context("TinyFish hook script missing for runtime router")?;
            let statusline_script_text = artifacts
                .tinyfish_statusline_script_text
                .as_deref()
                .context("TinyFish statusline script missing for runtime router")?;
            let plugin_root = self.upsert_local_tinyfish_artifacts(
                plugin_variant,
                tool_shell,
                plugin_manifest_json,
                plugin_hooks_json,
                output_style_text,
                hook_script_text,
                statusline_script_text,
            )?;
            cmd.arg("--plugin-dir");
            cmd.arg(plugin_root);
        }

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            let plugin_root = self.upsert_local_profile_mcp_plugin(profile, &mcp_servers)?;
            cmd.arg("--plugin-dir");
            cmd.arg(plugin_root);
        }

        let tinyfish_statusline_script_path = tinyfish_plugin_variant.map(|plugin_variant| {
            self.local_tinyfish_statusline_script_path(plugin_variant, tool_shell)
                .to_string_lossy()
                .to_string()
        });
        let settings = build_lightweight_settings(
            env,
            resolved_token.as_deref(),
            resolved_url.as_deref(),
            tinyfish_enabled,
            tool_shell,
            tinyfish_statusline_script_path.as_deref(),
        )?;
        let settings = if artifacts.local_gateway_mode.is_auto() {
            settings
        } else {
            build_lightweight_settings_with_local_gateway_mode(
                env,
                resolved_token.as_deref(),
                resolved_url.as_deref(),
                tinyfish_enabled,
                tool_shell,
                tinyfish_statusline_script_path.as_deref(),
                artifacts.local_gateway_mode,
            )?
        };
        let settings_json = serde_json::to_string(&settings)
            .context("Failed to serialize direct lightweight settings JSON")?;
        cmd.arg("--settings");
        cmd.arg(settings_json);
        Ok(())
    }

    pub(super) fn prepare_default_local_direct_lightweight_launch(
        &self,
        cmd: &mut std::process::Command,
        profile: &Profile,
    ) -> Result<()> {
        let env = profile
            .env
            .as_ref()
            .context("Lightweight profile env missing during launch")?;
        let (resolved_token, resolved_url) = self.resolve_credentials(profile)?;
        let tool_shell = native_tinyfish_tool_shell();

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            let plugin_root = self.upsert_local_profile_mcp_plugin(profile, &mcp_servers)?;
            cmd.arg("--plugin-dir");
            cmd.arg(plugin_root);
        }

        let settings = build_lightweight_settings(
            env,
            resolved_token.as_deref(),
            resolved_url.as_deref(),
            false,
            tool_shell,
            None,
        )?;
        let settings_json = serde_json::to_string(&settings)
            .context("Failed to serialize direct lightweight settings JSON")?;
        cmd.arg("--settings");
        cmd.arg(settings_json);
        Ok(())
    }

    pub(super) fn prepare_lightweight_launch(
        &self,
        cmd: &mut std::process::Command,
        profile: &Profile,
    ) -> Result<(String, PathBuf, RuntimeSessionState)> {
        self.prepare_lightweight_launch_with_tinyfish_available(cmd, profile, tinyfish_available())
    }

    pub(super) fn prepare_lightweight_launch_with_tinyfish_available(
        &self,
        cmd: &mut std::process::Command,
        profile: &Profile,
        tinyfish_router_available: bool,
    ) -> Result<(String, PathBuf, RuntimeSessionState)> {
        profile
            .env
            .as_ref()
            .context("Lightweight profile env missing during launch")?;
        let (resolved_token, resolved_url) = self.resolve_credentials(profile)?;
        let linked_provider = profile
            .provider_id
            .as_deref()
            .and_then(|provider_id| self.get_provider(provider_id).ok());
        let linked_key = linked_provider.as_ref().and_then(|provider| {
            profile
                .key_id
                .as_deref()
                .and_then(|key_id| provider.keys.get(key_id))
                .cloned()
        });
        let session_id = self.next_runtime_session_id();
        let session = self.runtime_session_state_from_profile(
            session_id.clone(),
            profile,
            linked_provider.as_ref(),
            linked_key.as_ref(),
            (
                resolved_token.as_deref().unwrap_or_default(),
                resolved_url.as_deref().unwrap_or_default(),
            ),
            std::env::current_dir().ok(),
        );
        let tool_shell = native_tinyfish_tool_shell();
        let artifacts = self.lightweight_launch_artifacts(
            profile,
            resolved_token.as_deref(),
            resolved_url.as_deref(),
            LocalGatewayToolMode::Auto,
        )?;
        let tinyfish_enabled = artifacts.tinyfish_enabled && tinyfish_router_available;
        let tinyfish_plugin_variant = if tinyfish_enabled {
            Some(
                artifacts
                    .tinyfish_plugin_variant
                    .context("TinyFish plugin variant missing for runtime router")?,
            )
        } else {
            None
        };
        if let Some(plugin_variant) = tinyfish_plugin_variant {
            let plugin_hooks_json = artifacts
                .tinyfish_plugin_hooks_json
                .as_deref()
                .context("TinyFish plugin hooks missing for runtime router")?;
            let plugin_manifest_json = artifacts
                .tinyfish_plugin_manifest_json
                .as_deref()
                .context("TinyFish plugin manifest missing for runtime router")?;
            let output_style_text = artifacts
                .tinyfish_output_style_text
                .as_deref()
                .context("TinyFish output style missing for runtime router")?;
            let hook_script_text = artifacts
                .tinyfish_hook_script_text
                .as_deref()
                .context("TinyFish hook script missing for runtime router")?;
            let statusline_script_text = artifacts
                .tinyfish_statusline_script_text
                .as_deref()
                .context("TinyFish statusline script missing for runtime router")?;
            let plugin_root = self.upsert_local_tinyfish_artifacts(
                plugin_variant,
                tool_shell,
                plugin_manifest_json,
                plugin_hooks_json,
                output_style_text,
                hook_script_text,
                statusline_script_text,
            )?;
            cmd.arg("--plugin-dir");
            cmd.arg(plugin_root);
        }

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            let plugin_root = self.upsert_local_profile_mcp_plugin(profile, &mcp_servers)?;
            cmd.arg("--plugin-dir");
            cmd.arg(plugin_root);
        }

        self.ensure_runtime_gateway_cache_compatible(None, &session.base_url)?;
        self.ensure_runtime_session_dir(&session_id)?;
        let settings_json = self.build_runtime_settings_json(&session)?;
        let settings_path = self.runtime_settings_path(&session_id);
        let state_path = self.runtime_state_path(&session_id);
        if let Err(error) = (|| -> Result<()> {
            self.write_runtime_state_atomic(&state_path, &session)?;
            self.write_runtime_settings_in_place(&settings_path, &settings_json)?;
            self.refresh_runtime_gateway_models_cache_best_effort(&session);
            Ok(())
        })() {
            let _ = self.remove_runtime_session_dir(&session_id);
            return Err(error);
        }

        cmd.arg("--settings");
        cmd.arg(&settings_path);

        Ok((session_id, state_path, session))
    }

    fn spawn_prepared_runtime_session(
        &self,
        cmd: &mut std::process::Command,
        session_id: &str,
        state_path: &Path,
        mut session: RuntimeSessionState,
    ) -> Result<(std::process::Child, RuntimeSessionState)> {
        let mut child = match cmd
            .spawn()
            .context("Failed to launch claude. Is it installed and in your PATH?")
        {
            Ok(child) => child,
            Err(error) => {
                let _ = self.remove_runtime_session_dir(session_id);
                return Err(error);
            }
        };
        session.pid = Some(child.id());
        session.process_started_at = Self::runtime_process_started_at(child.id());
        session.updated_at = Utc::now();
        if let Err(error) = self.write_runtime_state_atomic(state_path, &session) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = self.remove_runtime_session_dir(session_id);
            return Err(error);
        }
        Ok((child, session))
    }

    pub fn launch_claude(
        &self,
        query: &str,
        args: &[String],
        options: LaunchOptions,
    ) -> Result<()> {
        let (id, profile) = self.find_profile(query)?;
        let _ = self.garbage_collect_runtime_sessions();

        // Update last_used
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.last_used = Some(Utc::now());
        }
        self.save_registry(&registry)?;

        let mut cmd = super::build_local_command("claude");
        if options.use_stored_args
            && let Some(ref stored) = profile.launch_args
        {
            cmd.args(stored);
        }
        cmd.args(args);

        if profile.kind == ProfileKind::Lightweight {
            if profile.env.is_some() {
                let (_, resolved_url) = self.resolve_credentials(&profile)?;
                if matches!(
                    options.local_gateway_mode,
                    RequestedLocalGatewayMode::Explicit(mode) if !mode.is_auto()
                ) && !resolved_url
                    .as_deref()
                    .is_some_and(is_local_runtime_base_url)
                {
                    let RequestedLocalGatewayMode::Explicit(local_gateway_mode) =
                        options.local_gateway_mode
                    else {
                        unreachable!("matched explicit local gateway mode");
                    };
                    bail!(
                        "Local gateway mode '{}' only applies to localhost/LAN self-hosted APIs.",
                        local_gateway_mode.as_cli_value()
                    );
                }
                if resolved_url
                    .as_deref()
                    .is_some_and(is_local_runtime_base_url)
                {
                    match options.local_gateway_mode {
                        RequestedLocalGatewayMode::Omitted => {
                            self.prepare_default_local_direct_lightweight_launch(
                                &mut cmd, &profile,
                            )?;
                        }
                        RequestedLocalGatewayMode::Explicit(local_gateway_mode) => {
                            self.prepare_direct_lightweight_launch_with_tinyfish_available(
                                &mut cmd,
                                &profile,
                                tinyfish_available(),
                                local_gateway_mode,
                            )?;
                        }
                    }
                    let status = cmd
                        .status()
                        .context("Failed to launch claude. Is it installed and in your PATH?")?;
                    std::process::exit(status.code().unwrap_or(0));
                }
                let (session_id, state_path, session) =
                    self.prepare_lightweight_launch(&mut cmd, &profile)?;
                let (mut child, _) = self.spawn_prepared_runtime_session(
                    &mut cmd,
                    &session_id,
                    &state_path,
                    session,
                )?;
                let status_result = child.wait();
                let cleanup_result = self.remove_runtime_session_dir(&session_id);
                let status = status_result?;
                cleanup_result?;
                std::process::exit(status.code().unwrap_or(0));
            }
        } else {
            let profile_dir = self.profile_dir(&profile);
            if !profile_dir.exists() {
                bail!(
                    "Profile directory for '{}' not found. Re-add it with: cswitch add --full {}",
                    profile.name,
                    profile.name
                );
            }
            cmd.env("CLAUDE_CONFIG_DIR", &profile_dir);
        }

        let status = cmd
            .status()
            .context("Failed to launch claude. Is it installed and in your PATH?")?;
        std::process::exit(status.code().unwrap_or(0));
    }
}
