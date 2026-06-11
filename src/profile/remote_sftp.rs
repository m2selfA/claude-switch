use super::*;

impl ProfileManager {
    const REMOTE_UNIX_VARUSERS_PATH_PROBE: &str = "sh -lc 'if [ -d \"$HOME/.varusers/bin\" ]; then case \":$PATH:\" in *\":$HOME/.varusers/bin:\"*) printf \"1\\n\" ;; *) printf \"0\\n\" ;; esac; else printf \"0\\n\"; fi'";

    pub(super) fn run_local_command(program: &str, args: &[&str]) -> Result<String> {
        let output = super::with_local_command_candidates(program, |resolved| {
            Command::new(resolved).args(args).output()
        })
        .with_context(|| format!("Failed to run {}", program))?;
        if !output.status.success() {
            bail!(
                "{} failed: {}",
                program,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub(super) fn run_remote_sftp_commands(host: &str, stdin: &str) -> Result<String> {
        let mut child = super::with_local_command_candidates("sftp", |resolved| {
            Command::new(resolved)
                .args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=10",
                    "-o",
                    "StrictHostKeyChecking=accept-new",
                    "-o",
                    "ForwardX11=no",
                    "-b",
                    "-",
                    host,
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
        })
        .context("Failed to spawn sftp")?;

        use std::io::Write;
        {
            let mut stdin_handle = child.stdin.take().unwrap();
            let _ = stdin_handle.write_all(stdin.as_bytes());
        }

        let output = child.wait_with_output().context("Failed to wait on sftp")?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            if !stderr.is_empty() {
                bail!("sftp error: {}", stderr);
            }
            if stdout.is_empty() {
                bail!("sftp failed silently");
            }
            let lower = stdout.to_lowercase();
            if lower.contains("no such file")
                || lower.contains("not found")
                || lower.contains("permission denied")
                || lower.contains("failure")
                || lower.contains("couldn't")
                || lower.contains("cannot")
            {
                bail!("sftp error (stdout): {}", stdout);
            }
        }
        Ok(stdout)
    }

    pub(super) fn run_remote_sftp_batch(host: &str, batch_path: &str) -> Result<String> {
        Self::run_local_command(
            "sftp",
            &[
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ForwardX11=no",
                "-b",
                batch_path,
                host,
            ],
        )
    }

    pub(super) fn run_remote_ssh_command(host: &str, command: &str) -> Result<String> {
        Self::run_local_command(
            "ssh",
            &[
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ForwardX11=no",
                "-T",
                host,
                command,
            ],
        )
    }

    pub(super) fn probe_remote_os_and_home(host: &str) -> Result<(RemoteOs, String)> {
        let output = Self::run_remote_sftp_commands(host, "pwd\n")?;
        let home = output
            .lines()
            .find(|line| line.contains("Remote working directory:"))
            .and_then(|line| line.split(": ").nth(1))
            .map(str::trim)
            .map(str::to_string)
            .context("sftp pwd did not produce a usable directory")?;

        let bytes = home.as_bytes();
        let is_windows = bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        let remote_os = if is_windows {
            RemoteOs::Windows
        } else if home.starts_with('/') {
            RemoteOs::Unix
        } else {
            bail!(
                "Could not determine remote OS for '{}' from sftp pwd output: {}",
                host,
                home
            );
        };
        Ok((remote_os, home))
    }

    pub(super) fn probe_remote_unix_varusers_on_path(host: &str) -> Result<bool> {
        Self::probe_remote_unix_varusers_on_path_with_runner(host, |host, command| {
            Self::run_remote_ssh_command(host, command)
        })
    }

    pub(super) fn probe_remote_unix_varusers_on_path_with_runner<F>(
        host: &str,
        runner: F,
    ) -> Result<bool>
    where
        F: FnOnce(&str, &str) -> Result<String>,
    {
        let output = runner(host, Self::REMOTE_UNIX_VARUSERS_PATH_PROBE)?;
        let marker = output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .context("remote Unix PATH probe returned no usable output")?;
        match marker {
            "1" => Ok(true),
            "0" => Ok(false),
            _ => bail!(
                "remote Unix PATH probe returned unexpected output: {}",
                output
            ),
        }
    }

    pub(super) fn sftp_quote(path: &str) -> String {
        format!("\"{}\"", path.replace('"', "\\\""))
    }

    pub(super) fn ensure_remote_dir(host: &str, remote_bin_dir: &str) -> Result<()> {
        let dir = remote_bin_dir.replace('\\', "/");
        let mut cmds = String::new();
        let mut accumulated = String::new();
        for component in dir.split('/') {
            if component.is_empty() {
                if dir.starts_with('/') {
                    accumulated.push('/');
                }
                continue;
            }
            if accumulated == "/" {
                accumulated.push_str(component);
            } else if accumulated.is_empty() {
                accumulated = component.to_string();
            } else {
                accumulated.push('/');
                accumulated.push_str(component);
            }
            cmds.push_str(&format!("-mkdir {}\n", Self::sftp_quote(&accumulated)));
        }
        if !cmds.is_empty() {
            let _ = Self::run_remote_sftp_commands(host, &cmds);
        }
        Ok(())
    }

    pub(super) fn list_remote_files(
        host: &str,
        remote_bin_dir: &str,
        remote_os: RemoteOs,
    ) -> Result<Vec<String>> {
        let sftp_dir = if matches!(remote_os, RemoteOs::Windows) {
            remote_bin_dir.replace('\\', "/")
        } else {
            remote_bin_dir.to_string()
        };
        let output = Self::run_remote_sftp_commands(
            host,
            &format!("ls -1 {}\n", Self::sftp_quote(&format!("{}/", sftp_dir))),
        )?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let name = line.trim();
                if name.is_empty() || name.starts_with("sftp>") {
                    None
                } else {
                    name.rsplit('/').next().map(str::to_string)
                }
            })
            .collect())
    }

    pub(super) fn list_remote_files_if_present(
        host: &str,
        remote_dir: &str,
        remote_os: RemoteOs,
    ) -> Result<Vec<String>> {
        match Self::list_remote_files(host, remote_dir, remote_os) {
            Ok(files) => Ok(files),
            Err(err) => {
                let msg = err.to_string().to_lowercase();
                if msg.contains("no such file") || msg.contains("not found") {
                    Ok(Vec::new())
                } else {
                    Err(err)
                }
            }
        }
    }

    pub(super) fn remote_file_has_marker(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<bool> {
        let sftp_path = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let local_tmp = std::env::temp_dir().join(format!("cswitch-marker-{}", Uuid::new_v4()));
        let sftp_cmd = format!(
            "get {} {}\n",
            Self::sftp_quote(&sftp_path),
            Self::sftp_quote(&local_tmp.display().to_string()),
        );
        let get_result = Self::run_remote_sftp_commands(host, &sftp_cmd);
        let content = match &get_result {
            Ok(_) => fs::read_to_string(&local_tmp).context("Failed to read temp marker file")?,
            Err(_) => String::new(),
        };
        let _ = fs::remove_file(&local_tmp);
        if get_result.is_err() {
            return Ok(false);
        }
        get_result?;
        Ok(content.contains(CMD_MARKER) || content.contains(SH_MARKER))
    }

    pub(super) fn remove_remote_file(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<()> {
        let sftp_path = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        Self::run_remote_sftp_commands(host, &format!("rm {}\n", Self::sftp_quote(&sftp_path)))?;
        Ok(())
    }

    pub(super) fn is_benign_sftp_missing_error(error: &anyhow::Error) -> bool {
        let message = error.to_string().to_ascii_lowercase();
        message.contains("no such file")
            || message.contains("not found")
            || message.contains("couldn't stat remote file")
    }

    pub(super) fn remove_remote_plugin_dir(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<()> {
        Self::remove_remote_plugin_dir_with_runner(host, remote_path, remote_os, |stdin| {
            Self::run_remote_sftp_commands(host, stdin)
        })
    }

    pub(super) fn remove_remote_mcp_plugin_dir(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<()> {
        Self::remove_remote_mcp_plugin_dir_with_runner(host, remote_path, remote_os, |stdin| {
            Self::run_remote_sftp_commands(host, stdin)
        })
    }

    pub(super) fn remove_remote_mcp_plugin_dir_with_runner<F>(
        _host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
        mut run_sftp: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let manifest_dir = Self::join_remote_path(remote_path, remote_os, ".claude-plugin");
        let manifest_json = Self::join_remote_path(&manifest_dir, remote_os, "plugin.json");
        let dot_mcp_json = Self::join_remote_path(remote_path, remote_os, ".mcp.json");
        let mcp_json = Self::join_remote_path(remote_path, remote_os, "mcp.json");
        let manifest_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_json.replace('\\', "/")
        } else {
            manifest_json
        };
        let manifest_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_dir.replace('\\', "/")
        } else {
            manifest_dir
        };
        let dot_mcp_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            dot_mcp_json.replace('\\', "/")
        } else {
            dot_mcp_json
        };
        let mcp_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            mcp_json.replace('\\', "/")
        } else {
            mcp_json
        };
        let plugin_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let cmds = format!(
            "rm {}\nrm {}\nrm {}\nrmdir {}\nrmdir {}\n",
            Self::sftp_quote(&manifest_json_sftp),
            Self::sftp_quote(&dot_mcp_json_sftp),
            Self::sftp_quote(&mcp_json_sftp),
            Self::sftp_quote(&manifest_dir_sftp),
            Self::sftp_quote(&plugin_dir_sftp),
        );
        match run_sftp(&cmds) {
            Ok(_) => Ok(()),
            Err(error) if Self::is_benign_sftp_missing_error(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn remove_remote_plugin_dir_with_runner<F>(
        _host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
        mut run_sftp: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let tool_shell = match remote_os {
            RemoteOs::Unix => TinyfishToolShell::Bash,
            RemoteOs::Windows => TinyfishToolShell::PowerShell,
        };
        let manifest_dir = Self::join_remote_path(remote_path, remote_os, ".claude-plugin");
        let manifest_json = Self::join_remote_path(&manifest_dir, remote_os, "plugin.json");
        let hooks_dir = Self::join_remote_path(remote_path, remote_os, "hooks");
        let hooks_json = Self::join_remote_path(&hooks_dir, remote_os, "hooks.json");
        let scripts_dir = Self::join_remote_path(remote_path, remote_os, "scripts");
        let hook_script = Self::join_remote_path(
            &scripts_dir,
            remote_os,
            tinyfish_plugin_script_file_name(tool_shell),
        );
        let statusline_script = Self::join_remote_path(
            &scripts_dir,
            remote_os,
            tinyfish_statusline_script_file_name(tool_shell),
        );
        let output_styles_dir = Self::join_remote_path(remote_path, remote_os, "output-styles");
        let output_style =
            Self::join_remote_path(&output_styles_dir, remote_os, "route-default.md");
        let manifest_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_json.replace('\\', "/")
        } else {
            manifest_json
        };
        let manifest_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_dir.replace('\\', "/")
        } else {
            manifest_dir
        };
        let hooks_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            hooks_json.replace('\\', "/")
        } else {
            hooks_json
        };
        let scripts_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            scripts_dir.replace('\\', "/")
        } else {
            scripts_dir
        };
        let hook_script_sftp = if matches!(remote_os, RemoteOs::Windows) {
            hook_script.replace('\\', "/")
        } else {
            hook_script
        };
        let statusline_script_sftp = if matches!(remote_os, RemoteOs::Windows) {
            statusline_script.replace('\\', "/")
        } else {
            statusline_script
        };
        let output_styles_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            output_styles_dir.replace('\\', "/")
        } else {
            output_styles_dir
        };
        let output_style_sftp = if matches!(remote_os, RemoteOs::Windows) {
            output_style.replace('\\', "/")
        } else {
            output_style
        };
        let hooks_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            hooks_dir.replace('\\', "/")
        } else {
            hooks_dir
        };
        let plugin_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let cmds = format!(
            "rm {}\nrm {}\nrm {}\nrm {}\nrm {}\nrmdir {}\nrmdir {}\nrmdir {}\nrmdir {}\nrmdir {}\n",
            Self::sftp_quote(&manifest_json_sftp),
            Self::sftp_quote(&hooks_json_sftp),
            Self::sftp_quote(&hook_script_sftp),
            Self::sftp_quote(&statusline_script_sftp),
            Self::sftp_quote(&output_style_sftp),
            Self::sftp_quote(&manifest_dir_sftp),
            Self::sftp_quote(&hooks_dir_sftp),
            Self::sftp_quote(&scripts_dir_sftp),
            Self::sftp_quote(&output_styles_dir_sftp),
            Self::sftp_quote(&plugin_dir_sftp),
        );
        match run_sftp(&cmds) {
            Ok(_) => Ok(()),
            Err(error) if Self::is_benign_sftp_missing_error(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub(super) fn join_remote_path(
        remote_dir: &str,
        remote_os: RemoteOs,
        file_name: &str,
    ) -> String {
        match remote_os {
            RemoteOs::Unix => format!("{}/{}", remote_dir.trim_end_matches('/'), file_name),
            RemoteOs::Windows => format!(
                "{}\\{}",
                remote_dir.trim_end_matches(['\\', '/']),
                file_name
            ),
        }
    }

    pub(super) fn remote_shim_file_names(
        &self,
        profile: &Profile,
        remote_os: RemoteOs,
    ) -> Result<Vec<(String, LocalGatewayToolMode)>> {
        if profile.kind == ProfileKind::Full {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for local_gateway_mode in self.local_gateway_shim_modes(profile)? {
            let alias_name = Self::shim_alias_name(profile, local_gateway_mode);
            let file_name = match remote_os {
                RemoteOs::Unix => format!("claude-{}", alias_name),
                RemoteOs::Windows => format!("claude-{}.cmd", alias_name),
            };
            names.push((file_name, local_gateway_mode));
        }
        Ok(names)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn remote_shim_file_name(profile: &Profile, remote_os: RemoteOs) -> Option<String> {
        if profile.kind == ProfileKind::Full {
            return None;
        }
        let alias_name = Self::shim_alias_name(profile, LocalGatewayToolMode::Auto);
        Some(match remote_os {
            RemoteOs::Unix => format!("claude-{}", alias_name),
            RemoteOs::Windows => format!("claude-{}.cmd", alias_name),
        })
    }

    pub(super) fn is_managed_remote_name(remote_os: RemoteOs, file_name: &str) -> bool {
        match remote_os {
            RemoteOs::Unix => file_name.starts_with("claude-"),
            RemoteOs::Windows => file_name.starts_with("claude-") && file_name.ends_with(".cmd"),
        }
    }

    pub(super) fn build_remote_upload_batch(
        temp_root: &Path,
        remote_dir: &str,
        remote_os: RemoteOs,
        desired: &[(String, String)],
        chmod_unix: bool,
    ) -> String {
        let mut batch = String::new();
        for (file_name, _) in desired {
            let local_path = temp_root.join(file_name);
            let remote_path = Self::join_remote_path(remote_dir, remote_os, file_name);
            batch.push_str(&format!(
                "put {} {}\n",
                Self::sftp_quote(&local_path.display().to_string()),
                Self::sftp_quote(&remote_path),
            ));
            if matches!(remote_os, RemoteOs::Unix) && chmod_unix {
                batch.push_str(&format!("chmod 755 {}\n", Self::sftp_quote(&remote_path),));
            }
        }
        batch
    }

    pub(super) fn remote_parent_dirs(
        remote_dir: &str,
        remote_os: RemoteOs,
        relative_path: &str,
    ) -> Vec<String> {
        let normalized = relative_path.replace('\\', "/");
        let mut components: Vec<&str> = normalized.split('/').collect();
        if components.len() <= 1 {
            return Vec::new();
        }
        components.pop();
        let mut dirs = Vec::new();
        let mut current = remote_dir.trim_end_matches(['\\', '/']).to_string();
        for component in components {
            current = match remote_os {
                RemoteOs::Unix => format!("{}/{}", current.trim_end_matches('/'), component),
                RemoteOs::Windows => {
                    format!("{}\\{}", current.trim_end_matches(['\\', '/']), component)
                }
            };
            dirs.push(current.clone());
        }
        dirs
    }

    pub(super) fn upload_remote_files(
        host: &str,
        remote_dir: &str,
        remote_os: RemoteOs,
        desired: &[(String, String)],
        chmod_unix: bool,
    ) -> Result<()> {
        let temp_root = std::env::temp_dir().join(format!("cswitch-remote-{}", Uuid::new_v4()));
        let batch_path =
            std::env::temp_dir().join(format!("cswitch-remote-{}.sftp", Uuid::new_v4()));
        fs::create_dir_all(&temp_root).context("Failed to create temp shim directory")?;

        let mut remote_parent_dirs = std::collections::BTreeSet::new();

        for (file_name, content) in desired {
            let local_path = temp_root.join(file_name);
            if let Some(parent) = local_path.parent() {
                fs::create_dir_all(parent).context("Failed to create temp shim subdirectory")?;
            }
            fs::write(&local_path, content).context("Failed to write temp shim file")?;
            for parent_dir in Self::remote_parent_dirs(remote_dir, remote_os, file_name) {
                remote_parent_dirs.insert(parent_dir);
            }
        }
        for parent_dir in remote_parent_dirs {
            Self::ensure_remote_dir(host, &parent_dir)?;
        }
        let batch =
            Self::build_remote_upload_batch(&temp_root, remote_dir, remote_os, desired, chmod_unix);

        fs::write(&batch_path, batch).context("Failed to write temp sftp batch")?;
        let batch_path_str = batch_path.to_string_lossy().to_string();
        let result = Self::run_remote_sftp_batch(host, &batch_path_str);
        let _ = fs::remove_dir_all(&temp_root);
        let _ = fs::remove_file(&batch_path);
        result?;
        Ok(())
    }

    pub(super) fn remove_remote_tree(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<()> {
        match Self::list_remote_files(host, remote_path, remote_os) {
            Ok(children) => {
                for child in children {
                    let child_path = Self::join_remote_path(remote_path, remote_os, &child);
                    Self::remove_remote_tree(host, &child_path, remote_os)?;
                }
                let sftp_path = if matches!(remote_os, RemoteOs::Windows) {
                    remote_path.replace('\\', "/")
                } else {
                    remote_path.to_string()
                };
                match Self::run_remote_sftp_commands(
                    host,
                    &format!("rmdir {}\n", Self::sftp_quote(&sftp_path)),
                ) {
                    Ok(_) => Ok(()),
                    Err(error) if Self::is_benign_sftp_missing_error(&error) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Err(error) if Self::is_benign_sftp_missing_error(&error) => {
                match Self::remove_remote_file(host, remote_path, remote_os) {
                    Ok(_) => Ok(()),
                    Err(remove_error) if Self::is_benign_sftp_missing_error(&remove_error) => {
                        Ok(())
                    }
                    Err(remove_error) => Err(remove_error),
                }
            }
            Err(error) => Err(error),
        }
    }
}
