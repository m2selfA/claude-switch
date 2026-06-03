use super::*;

impl ProfileManager {
    pub fn launch_claude(&self, query: &str, args: &[String], use_stored_args: bool) -> Result<()> {
        let (id, profile) = self.find_profile(query)?;

        // Update last_used
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.last_used = Some(Utc::now());
        }
        self.save_registry(&registry)?;

        let mut cmd = std::process::Command::new("claude");
        if use_stored_args && let Some(ref stored) = profile.launch_args {
            cmd.args(stored);
        }
        cmd.args(args);

        if profile.kind == ProfileKind::Lightweight {
            if let Some(ref env) = profile.env {
                let (resolved_token, resolved_url) = self.resolve_credentials(&profile)?;
                let tool_shell = native_tinyfish_tool_shell();
                let artifacts = build_lightweight_runtime_artifacts(
                    env,
                    resolved_token.as_deref(),
                    resolved_url.as_deref(),
                    tool_shell,
                )?;

                cmd.arg("--settings");
                if artifacts.tinyfish_mode != TinyfishMode::None && tinyfish_available() {
                    let settings_json = artifacts
                        .tinyfish_settings_json
                        .as_deref()
                        .context("TinyFish settings missing for non-native mode")?;
                    let prompt_text = artifacts
                        .tinyfish_prompt_text
                        .as_deref()
                        .context("TinyFish prompt missing for non-native mode")?;
                    let plugin_hooks_json = artifacts
                        .tinyfish_plugin_hooks_json
                        .as_deref()
                        .context("TinyFish plugin hooks missing for non-native mode")?;
                    let plugin_manifest_json =
                        artifacts
                            .tinyfish_plugin_manifest_json
                            .as_deref()
                            .context("TinyFish plugin manifest missing for non-native mode")?;
                    let (plugin_root, prompt_path) = self.upsert_local_tinyfish_artifacts(
                        artifacts.tinyfish_mode,
                        tool_shell,
                        plugin_manifest_json,
                        plugin_hooks_json,
                        prompt_text,
                    )?;
                    cmd.arg(settings_json);
                    cmd.arg("--plugin-dir");
                    cmd.arg(plugin_root);
                    cmd.arg("--append-system-prompt-file");
                    cmd.arg(prompt_path);
                } else {
                    cmd.arg(&artifacts.base_settings_json);
                }

                let mcp_servers = self.profile_mcp_servers(&profile)?;
                if !mcp_servers.is_empty() {
                    let plugin_root =
                        self.upsert_local_profile_mcp_plugin(&profile, &mcp_servers)?;
                    cmd.arg("--plugin-dir");
                    cmd.arg(plugin_root);
                }
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
