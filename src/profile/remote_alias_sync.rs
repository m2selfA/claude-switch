use super::*;

impl ProfileManager {
    fn hosted_plugin_root_name_from_relative_path(relative_path: &str) -> Option<String> {
        let mut parts = relative_path
            .split(['/', '\\'])
            .filter(|part| !part.is_empty());
        let first = parts.next()?;
        let second = parts.next()?;
        Some(format!("{first}/{second}"))
    }

    fn list_remote_hosted_plugin_roots(
        host: &str,
        remote_installed_plugins_dir: &str,
        remote_os: RemoteOs,
    ) -> Result<std::collections::HashSet<String>> {
        let mut roots = std::collections::HashSet::new();
        for marketplace in
            Self::list_remote_files_if_present(host, remote_installed_plugins_dir, remote_os)?
        {
            let marketplace_dir =
                Self::join_remote_path(remote_installed_plugins_dir, remote_os, &marketplace);
            for plugin in Self::list_remote_files_if_present(host, &marketplace_dir, remote_os)? {
                roots.insert(format!("{marketplace}/{plugin}"));
            }
        }
        Ok(roots)
    }

    pub fn sync_remote_aliases_with_progress<F>(
        &self,
        host: &str,
        verbose: bool,
        mut progress: F,
    ) -> Result<String>
    where
        F: FnMut(&str),
    {
        let profiles = self.list_profiles()?;
        let mut skipped_full_profiles = Vec::new();
        if verbose {
            progress(&format!(
                "[remote:{host}] probing remote OS and home via sftp pwd for {} profile(s)...",
                profiles.len()
            ));
        }
        let (remote_os, remote_home) = Self::probe_remote_os_and_home(host)?;
        let remote_bin_dir = match remote_os {
            RemoteOs::Unix => {
                let varusers_on_path = match Self::probe_remote_unix_varusers_on_path(host) {
                    Ok(value) => value,
                    Err(err) => {
                        progress(&format!(
                            "[remote:{host}] warning: failed to probe remote PATH for ~/.varusers/bin ({}); falling back to ~/.local/bin",
                            err
                        ));
                        false
                    }
                };
                if varusers_on_path {
                    format!("{}/.varusers/bin", remote_home.trim_end_matches('/'))
                } else {
                    format!("{}/.local/bin", remote_home.trim_end_matches('/'))
                }
            }
            RemoteOs::Windows => {
                format!("{}\\.local\\bin", remote_home.trim_end_matches(['\\', '/']))
            }
        };
        let remote_generated_root = Self::remote_generated_root_dir(&remote_home, remote_os);
        let remote_prompts_dir =
            Self::join_remote_path(&remote_generated_root, remote_os, "prompts");
        let remote_plugins_dir =
            Self::join_remote_path(&remote_generated_root, remote_os, "plugins");
        let remote_installed_plugins_dir =
            Self::remote_plugin_installed_root_dir(&remote_home, remote_os);
        progress(&format!(
            "[remote:{host}] note: remote auto shims require a compatible remote cswitch on PATH for runtime switching; otherwise they fall back to legacy launch."
        ));
        if verbose {
            progress(&format!(
                "[remote:{host}] detected {:?}, home: {}, shim dir: {}",
                remote_os, remote_home, remote_bin_dir
            ));
        }

        if verbose {
            progress(&format!(
                "[remote:{host}] ensuring remote directory exists..."
            ));
        }
        Self::ensure_remote_dir(host, &remote_bin_dir)?;

        let remote_tool_shell = match remote_os {
            RemoteOs::Unix => TinyfishToolShell::Bash,
            RemoteOs::Windows => TinyfishToolShell::PowerShell,
        };
        let mut desired_shims: Vec<(String, String)> = Vec::new();
        let mut desired_plugin_assets: Vec<(String, String)> = Vec::new();
        let mut desired_plugin_scripts: Vec<(String, String)> = Vec::new();
        let mut desired_mcps: Vec<(String, String)> = Vec::new();
        let mut desired_hosted_plugin_assets: Vec<(String, String)> = Vec::new();
        let mut desired_hosted_plugin_scripts: Vec<(String, String)> = Vec::new();
        let mut desired_plugin_names = std::collections::HashSet::new();
        let mut desired_mcp_names = std::collections::HashSet::new();
        let mut desired_hosted_plugin_roots = std::collections::HashSet::new();
        for profile in &profiles {
            let remote_shim_names = self.remote_shim_file_names(profile, remote_os)?;
            if remote_shim_names.is_empty() {
                let alias_name = profile.alias.as_deref().unwrap_or(&profile.name);
                skipped_full_profiles.push(alias_name.to_string());
                if verbose {
                    progress(&format!(
                        "[remote:{host}] skipping full profile for remote sync: {}",
                        alias_name
                    ));
                }
                continue;
            }
            for (file_name, local_gateway_mode) in remote_shim_names {
                let content = match remote_os {
                    RemoteOs::Windows => self.generate_remote_cmd_content_for_local_gateway_mode(
                        profile,
                        local_gateway_mode,
                    )?,
                    RemoteOs::Unix => self.generate_remote_sh_content_for_local_gateway_mode(
                        profile,
                        local_gateway_mode,
                    )?,
                };
                desired_shims.push((file_name, content));
            }

            if profile.kind == ProfileKind::Lightweight
                && let Some(env) = profile.env.as_ref()
            {
                let (token, url) = self.resolve_credentials(profile)?;
                for local_gateway_mode in self.local_gateway_shim_modes(profile)? {
                    if self.uses_simplified_local_default_shim(profile, local_gateway_mode)? {
                        continue;
                    }
                    let artifacts = build_lightweight_runtime_artifacts_with_local_gateway_mode(
                        env,
                        token.as_deref(),
                        url.as_deref(),
                        remote_tool_shell,
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
                        artifacts.tinyfish_plugin_manifest_json,
                        artifacts.tinyfish_plugin_hooks_json,
                        artifacts.tinyfish_output_style_text,
                        artifacts.tinyfish_hook_script_text,
                        artifacts.tinyfish_statusline_script_text,
                    ) {
                        let plugin_name = Self::tinyfish_plugin_dir_name(plugin_variant);
                        if desired_plugin_names.insert(plugin_name) {
                            desired_plugin_assets.push((
                                Self::tinyfish_plugin_manifest_relative_path(
                                    plugin_variant,
                                    remote_os,
                                ),
                                plugin_manifest_json,
                            ));
                            desired_plugin_assets.push((
                                Self::tinyfish_plugin_hooks_relative_path(
                                    plugin_variant,
                                    remote_os,
                                ),
                                plugin_hooks_json,
                            ));
                            desired_plugin_assets.push((
                                Self::tinyfish_output_style_relative_path(
                                    plugin_variant,
                                    remote_os,
                                ),
                                output_style_text,
                            ));
                            desired_plugin_scripts.push((
                                Self::tinyfish_hook_script_relative_path(
                                    plugin_variant,
                                    remote_os,
                                    remote_tool_shell,
                                ),
                                hook_script_text,
                            ));
                            desired_plugin_scripts.push((
                                Self::tinyfish_statusline_script_relative_path(
                                    plugin_variant,
                                    remote_os,
                                    remote_tool_shell,
                                ),
                                statusline_script_text,
                            ));
                        }
                    }
                }

                let mcp_servers = self.profile_mcp_servers(profile)?;
                if !mcp_servers.is_empty() {
                    let mcp_plugin_name = Self::profile_mcp_plugin_dir_name(profile);
                    if desired_mcp_names.insert(mcp_plugin_name) {
                        desired_mcps.push((
                            Self::profile_mcp_manifest_relative_path(profile, remote_os),
                            Self::profile_mcp_plugin_manifest(profile)?,
                        ));
                        let mcp_config = Self::profile_mcp_config_for_target(
                            &mcp_servers,
                            remote_os,
                            Some(remote_home.as_str()),
                        )?;
                        desired_mcps.push((
                            Self::profile_mcp_config_relative_path(profile, remote_os),
                            mcp_config,
                        ));
                    }
                }
            }

            let (hosted_assets, hosted_scripts, hosted_roots) =
                self.installed_plugin_remote_file_sets(profile, remote_os)?;
            desired_hosted_plugin_assets.extend(hosted_assets);
            desired_hosted_plugin_scripts.extend(hosted_scripts);
            desired_hosted_plugin_roots.extend(hosted_roots);
        }
        if verbose {
            progress(&format!(
                "[remote:{host}] building {} remote shim(s), {} TinyFish plugin file(s), {} MCP plugin file(s), {} hosted plugin file(s); skipping {} full profile(s)...",
                desired_shims.len(),
                desired_plugin_assets.len() + desired_plugin_scripts.len(),
                desired_mcps.len(),
                desired_hosted_plugin_assets.len() + desired_hosted_plugin_scripts.len(),
                skipped_full_profiles.len()
            ));
        }

        if verbose {
            progress(&format!(
                "[remote:{host}] listing existing files in remote shim directory..."
            ));
        }
        let existing_shims = Self::list_remote_files_if_present(host, &remote_bin_dir, remote_os)?;
        let existing_shims_total = existing_shims.len();
        let managed_existing_shims: std::collections::HashSet<String> = existing_shims
            .into_iter()
            .filter(|name| Self::is_managed_remote_name(remote_os, name))
            .collect();
        let existing_shims_managed_count = managed_existing_shims.len();
        let ignored_shims_count = existing_shims_total.saturating_sub(existing_shims_managed_count);
        let existing_prompts =
            Self::list_remote_files_if_present(host, &remote_prompts_dir, remote_os)?;
        let existing_prompts_total = existing_prompts.len();
        let managed_existing_prompts: std::collections::HashSet<String> = existing_prompts
            .into_iter()
            .filter(|name| Self::is_managed_generated_prompt_name(name))
            .collect();
        let ignored_prompts_count =
            existing_prompts_total.saturating_sub(managed_existing_prompts.len());
        let existing_plugins =
            Self::list_remote_files_if_present(host, &remote_plugins_dir, remote_os)?;
        let existing_plugins_total = existing_plugins.len();
        let managed_existing_plugins: std::collections::HashSet<String> = existing_plugins
            .into_iter()
            .filter(|name| Self::is_managed_generated_plugin_dir_name(name))
            .collect();
        let ignored_plugins_count =
            existing_plugins_total.saturating_sub(managed_existing_plugins.len());
        let remote_mcps_dir = Self::join_remote_path(&remote_generated_root, remote_os, "mcps");
        let existing_mcps = Self::list_remote_files_if_present(host, &remote_mcps_dir, remote_os)?;
        let existing_mcps_total = existing_mcps.len();
        let managed_existing_mcps: std::collections::HashSet<String> = existing_mcps
            .into_iter()
            .filter(|name| Self::is_managed_generated_mcp_dir_name(name))
            .collect();
        let ignored_mcps_count = existing_mcps_total.saturating_sub(managed_existing_mcps.len());
        let managed_existing_hosted_plugins =
            Self::list_remote_hosted_plugin_roots(host, &remote_installed_plugins_dir, remote_os)?;
        let ignored_count = ignored_shims_count
            + ignored_prompts_count
            + ignored_plugins_count
            + ignored_mcps_count;

        if verbose {
            progress(&format!(
                "[remote:{host}] found {} managed shim(s), {} managed prompt file(s), {} managed TinyFish plugin dir(s), {} managed MCP plugin dir(s), {} hosted plugin dir(s); ignoring {} unrelated file(s)",
                existing_shims_managed_count,
                managed_existing_prompts.len(),
                managed_existing_plugins.len(),
                managed_existing_mcps.len(),
                managed_existing_hosted_plugins.len(),
                ignored_count
            ));
        }

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;
        let mut details = Vec::new();

        if !managed_existing_prompts.is_empty() {
            Self::ensure_remote_dir(host, &remote_prompts_dir)?;
        }
        if !desired_plugin_assets.is_empty()
            || !desired_plugin_scripts.is_empty()
            || !managed_existing_plugins.is_empty()
        {
            Self::ensure_remote_dir(host, &remote_plugins_dir)?;
        }
        if !desired_mcps.is_empty() || !managed_existing_mcps.is_empty() {
            Self::ensure_remote_dir(host, &remote_mcps_dir)?;
        }
        if !desired_hosted_plugin_assets.is_empty()
            || !desired_hosted_plugin_scripts.is_empty()
            || !managed_existing_hosted_plugins.is_empty()
        {
            Self::ensure_remote_dir(host, &remote_installed_plugins_dir)?;
        }

        if verbose && !desired_shims.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} shim(s) via sftp batch...",
                desired_shims.len()
            ));
        }
        if !desired_shims.is_empty() {
            Self::upload_remote_files(host, &remote_bin_dir, remote_os, &desired_shims, true)?;
        }
        let desired_plugin_file_count = desired_plugin_assets.len() + desired_plugin_scripts.len();
        if verbose && desired_plugin_file_count > 0 {
            progress(&format!(
                "[remote:{host}] uploading {} shared TinyFish plugin file(s)...",
                desired_plugin_file_count
            ));
        }
        if !desired_plugin_assets.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_plugins_dir,
                remote_os,
                &desired_plugin_assets,
                false,
            )?;
        }
        if !desired_plugin_scripts.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_plugins_dir,
                remote_os,
                &desired_plugin_scripts,
                true,
            )?;
        }
        if verbose && !desired_mcps.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} MCP plugin file(s)...",
                desired_mcps.len()
            ));
        }
        if !desired_mcps.is_empty() {
            Self::upload_remote_files(host, &remote_mcps_dir, remote_os, &desired_mcps, false)?;
        }
        for plugin_name in &desired_mcp_names {
            let plugin_root = Self::join_remote_path(&remote_mcps_dir, remote_os, plugin_name);
            let legacy_remote_path = Self::join_remote_path(&plugin_root, remote_os, "mcp.json");
            if Self::remove_remote_file_if_exists(host, &legacy_remote_path, remote_os)? {
                removed += 1;
                if verbose {
                    progress(&format!(
                        "[remote:{host}] removing legacy MCP compatibility file: {}",
                        legacy_remote_path
                    ));
                    details.push(format!("  - {}:{} (legacy)", host, legacy_remote_path));
                }
            }
        }
        let desired_hosted_plugin_file_count =
            desired_hosted_plugin_assets.len() + desired_hosted_plugin_scripts.len();
        if verbose && desired_hosted_plugin_file_count > 0 {
            progress(&format!(
                "[remote:{host}] uploading {} hosted plugin file(s)...",
                desired_hosted_plugin_file_count
            ));
        }
        if !desired_hosted_plugin_assets.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_installed_plugins_dir,
                remote_os,
                &desired_hosted_plugin_assets,
                false,
            )?;
        }
        if !desired_hosted_plugin_scripts.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_installed_plugins_dir,
                remote_os,
                &desired_hosted_plugin_scripts,
                true,
            )?;
        }

        for (file_name, _) in &desired_shims {
            let remote_path = Self::join_remote_path(&remote_bin_dir, remote_os, file_name);
            if managed_existing_shims.contains(file_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in desired_plugin_assets
            .iter()
            .chain(desired_plugin_scripts.iter())
        {
            let remote_path = Self::join_remote_path(&remote_plugins_dir, remote_os, file_name);
            let plugin_dir_name = file_name
                .split(['/', '\\'])
                .next()
                .expect("plugin file path should include root dir");
            if managed_existing_plugins.contains(plugin_dir_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in &desired_mcps {
            let remote_path = Self::join_remote_path(&remote_mcps_dir, remote_os, file_name);
            let plugin_dir_name = file_name
                .split(['/', '\\'])
                .next()
                .expect("MCP plugin file path should include root dir");
            if managed_existing_mcps.contains(plugin_dir_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in desired_hosted_plugin_assets
            .iter()
            .chain(desired_hosted_plugin_scripts.iter())
        {
            let remote_path =
                Self::join_remote_path(&remote_installed_plugins_dir, remote_os, file_name);
            let root_name =
                Self::hosted_plugin_root_name_from_relative_path(file_name).unwrap_or_default();
            if managed_existing_hosted_plugins.contains(&root_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        let desired_shim_names: std::collections::HashSet<&str> = desired_shims
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let stale_shims: Vec<String> = managed_existing_shims
            .iter()
            .filter(|name| !desired_shim_names.contains(name.as_str()))
            .cloned()
            .collect();
        let stale_prompt_count = managed_existing_prompts.len();
        let stale_plugin_count = managed_existing_plugins
            .iter()
            .filter(|name| !desired_plugin_names.contains(*name))
            .count();
        let stale_mcp_count = managed_existing_mcps
            .iter()
            .filter(|name| !desired_mcp_names.contains(*name))
            .count();
        let stale_hosted_plugin_count = managed_existing_hosted_plugins
            .iter()
            .filter(|name| !desired_hosted_plugin_roots.contains(*name))
            .count();
        if verbose {
            progress(&format!(
                "[remote:{host}] checking {} stale shim(s), {} stale prompt file(s), {} stale TinyFish plugin dir(s), {} stale MCP plugin dir(s), {} stale hosted plugin dir(s)...",
                stale_shims.len(),
                stale_prompt_count,
                stale_plugin_count,
                stale_mcp_count,
                stale_hosted_plugin_count
            ));
        }
        for stale in stale_shims {
            let remote_path = Self::join_remote_path(&remote_bin_dir, remote_os, &stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] inspecting stale managed shim: {}",
                    remote_path
                ));
            }
            if Self::remote_file_has_marker(host, &remote_path, remote_os)? {
                if verbose {
                    progress(&format!(
                        "[remote:{host}] removing stale managed shim: {}",
                        remote_path
                    ));
                    details.push(format!("  - {}:{} (stale)", host, remote_path));
                }
                Self::remove_remote_file(host, &remote_path, remote_os)?;
                removed += 1;
            }
        }

        for stale in &managed_existing_prompts {
            let remote_path = Self::join_remote_path(&remote_prompts_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale TinyFish prompt file: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_file(host, &remote_path, remote_os)?;
            removed += 1;
        }

        for stale in managed_existing_plugins
            .iter()
            .filter(|name| !desired_plugin_names.contains(*name))
        {
            let remote_path = Self::join_remote_path(&remote_plugins_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale TinyFish plugin dir: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_plugin_dir(host, &remote_path, remote_os)?;
            removed += 1;
        }

        for stale in managed_existing_mcps
            .iter()
            .filter(|name| !desired_mcp_names.contains(*name))
        {
            let remote_path = Self::join_remote_path(&remote_mcps_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale MCP plugin dir: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_mcp_plugin_dir(host, &remote_path, remote_os)?;
            removed += 1;
        }

        for stale in managed_existing_hosted_plugins
            .iter()
            .filter(|name| !desired_hosted_plugin_roots.contains(*name))
        {
            let remote_path = Self::join_remote_path(
                &remote_installed_plugins_dir,
                remote_os,
                &stale.replace(
                    '/',
                    &match remote_os {
                        RemoteOs::Unix => "/".to_string(),
                        RemoteOs::Windows => "\\".to_string(),
                    },
                ),
            );
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale hosted plugin dir: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_tree(host, &remote_path, remote_os)?;
            removed += 1;
        }

        if verbose {
            progress(&format!("[remote:{host}] remote shim sync complete"));
        }

        let summary = format!(
            "# Remote aliases synced to {} on {} ({:?}): {} added, {} updated, {} removed{}{}",
            remote_bin_dir,
            host,
            remote_os,
            added,
            updated,
            removed,
            if ignored_count > 0 {
                format!(", {} unrelated files ignored", ignored_count)
            } else {
                String::new()
            },
            if skipped_full_profiles.is_empty() {
                String::new()
            } else {
                format!(", {} full profile(s) skipped", skipped_full_profiles.len())
            }
        );
        if verbose {
            let mut output = vec![summary];
            if !skipped_full_profiles.is_empty() {
                output.extend(skipped_full_profiles.iter().map(|profile| {
                    format!("  ! skipped full profile for remote sync: {}", profile)
                }));
            }
            if !details.is_empty() {
                output.extend(details);
            }
            Ok(output.join("\n"))
        } else {
            Ok(summary)
        }
    }
}
