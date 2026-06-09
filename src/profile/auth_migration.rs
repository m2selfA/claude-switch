use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{AuthMigrationPlan, AuthMigrationSummary, ProfileKind, ProfileManager, RemoteOs};
use crate::profile::TinyfishToolShell;

const UNIX_HELPER_PREFIX: &str = "sh -lc \"echo \\\"";
const UNIX_HELPER_SUFFIX: &str = "\\\"\"";
const POWERSHELL_HELPER_PREFIX: &str = "powershell -NoProfile -Command \"echo '";
const POWERSHELL_HELPER_SUFFIX: &str = "'\"";

enum AuthMigrationTarget {
    Local {
        path: PathBuf,
        backup_path: PathBuf,
        tool_shell: TinyfishToolShell,
    },
    Remote {
        host: String,
        remote_path: String,
        backup_path: String,
        remote_os: RemoteOs,
        tool_shell: TinyfishToolShell,
    },
}

struct PreparedAuthMigration {
    target: AuthMigrationTarget,
    updated_content: String,
}

struct AuthMigrationState {
    plan: AuthMigrationPlan,
    prepared: Vec<PreparedAuthMigration>,
}

struct SettingsMigrationOutcome {
    updated_content: Option<String>,
    helper_overwritten: bool,
}

impl ProfileManager {
    pub(super) fn inline_api_key_helper_command(
        token: &str,
        tool_shell: TinyfishToolShell,
    ) -> Result<String> {
        if token.contains('\0') || token.contains('\r') || token.contains('\n') {
            bail!("token contains characters that cannot be rendered in apiKeyHelper");
        }

        Ok(match tool_shell {
            TinyfishToolShell::Bash => {
                let escaped = Self::escape_sh_double_quoted(token);
                format!("{UNIX_HELPER_PREFIX}{escaped}{UNIX_HELPER_SUFFIX}")
            }
            TinyfishToolShell::PowerShell => {
                let escaped = token.replace('\'', "''");
                format!("{POWERSHELL_HELPER_PREFIX}{escaped}{POWERSHELL_HELPER_SUFFIX}")
            }
        })
    }

    pub(super) fn extract_inline_api_key_helper_token(command: &str) -> Option<String> {
        if let Some(inner) = command
            .strip_prefix(POWERSHELL_HELPER_PREFIX)
            .and_then(|value| value.strip_suffix(POWERSHELL_HELPER_SUFFIX))
        {
            return Some(inner.replace("''", "'"));
        }

        let inner = command
            .strip_prefix(UNIX_HELPER_PREFIX)
            .and_then(|value| value.strip_suffix(UNIX_HELPER_SUFFIX))?;
        Some(Self::unescape_sh_double_quoted(inner))
    }

    pub(super) fn set_inline_api_key_helper(
        settings: &mut Map<String, Value>,
        token: &str,
        tool_shell: TinyfishToolShell,
    ) -> Result<()> {
        let helper = Self::inline_api_key_helper_command(token, tool_shell)?;
        match settings.get_mut("env") {
            Some(Value::Object(env)) => {
                env.remove("ANTHROPIC_AUTH_TOKEN");
                if env.is_empty() {
                    settings.remove("env");
                }
            }
            Some(_) => bail!("settings env block must be an object"),
            None => {}
        }
        settings.insert("apiKeyHelper".into(), Value::String(helper));
        Ok(())
    }

    pub(super) fn auth_token_from_settings(
        settings: &Map<String, Value>,
    ) -> Result<Option<String>> {
        if let Some(env_value) = settings.get("env") {
            let env = env_value
                .as_object()
                .context("settings env block must be an object")?;
            if let Some(token_value) = env.get("ANTHROPIC_AUTH_TOKEN") {
                let token = token_value
                    .as_str()
                    .context("settings env ANTHROPIC_AUTH_TOKEN must be a string")?;
                return Ok(Some(token.to_string()));
            }
        }

        match settings.get("apiKeyHelper") {
            None => Ok(None),
            Some(Value::String(helper)) => Self::extract_inline_api_key_helper_token(helper)
                .map(Some)
                .context("settings apiKeyHelper is not a cswitch-managed inline helper"),
            Some(_) => bail!("settings apiKeyHelper must be a string"),
        }
    }

    pub fn plan_auth_migration(&self, remote_hosts: &[String]) -> Result<AuthMigrationPlan> {
        Ok(self.build_auth_migration_state(remote_hosts)?.plan)
    }

    pub fn migrate_auth(&self, remote_hosts: &[String]) -> Result<AuthMigrationSummary> {
        let state = self.build_auth_migration_state(remote_hosts)?;
        let mut backup_paths = Vec::new();

        for prepared in state.prepared {
            match prepared.target {
                AuthMigrationTarget::Local {
                    path, backup_path, ..
                } => {
                    let existing = fs::read_to_string(&path)
                        .with_context(|| format!("Failed to read {}", path.display()))?;
                    if let Some(parent) = backup_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&backup_path, existing)
                        .with_context(|| format!("Failed to write {}", backup_path.display()))?;
                    fs::write(&path, prepared.updated_content)
                        .with_context(|| format!("Failed to write {}", path.display()))?;
                    backup_paths.push(backup_path.display().to_string());
                }
                AuthMigrationTarget::Remote {
                    host,
                    remote_path,
                    backup_path,
                    remote_os,
                    ..
                } => {
                    let existing = Self::read_remote_text_file(&host, &remote_path, remote_os)?;
                    Self::write_remote_text_file(&host, &backup_path, remote_os, &existing)?;
                    Self::write_remote_text_file(
                        &host,
                        &remote_path,
                        remote_os,
                        &prepared.updated_content,
                    )?;
                    backup_paths.push(format!("{host}:{backup_path}"));
                }
            }
        }

        Ok(AuthMigrationSummary {
            plan: state.plan,
            backup_paths,
        })
    }

    fn build_auth_migration_state(&self, remote_hosts: &[String]) -> Result<AuthMigrationState> {
        let mut plan = AuthMigrationPlan::default();
        let mut prepared = Vec::new();
        let mut seen_labels = std::collections::HashSet::new();

        for target in self.local_auth_migration_targets()? {
            let label = Self::auth_migration_target_label(&target);
            if !seen_labels.insert(label.clone()) {
                continue;
            }
            plan.local_files_scanned += 1;
            self.inspect_auth_migration_target(target, label, &mut plan, &mut prepared)?;
        }

        for host in remote_hosts {
            let (remote_os, remote_home) = Self::probe_remote_os_and_home(host)?;
            let remote_path = Self::remote_user_settings_path(&remote_home, remote_os);
            let backup_path = format!("{}.bak-{}", remote_path, Utc::now().format("%Y%m%d%H%M%S"));
            let tool_shell = match remote_os {
                RemoteOs::Unix => TinyfishToolShell::Bash,
                RemoteOs::Windows => TinyfishToolShell::PowerShell,
            };
            let target = AuthMigrationTarget::Remote {
                host: host.clone(),
                remote_path,
                backup_path,
                remote_os,
                tool_shell,
            };
            plan.remote_files_scanned += 1;
            let label = Self::auth_migration_target_label(&target);
            self.inspect_auth_migration_target(target, label, &mut plan, &mut prepared)?;
        }

        plan.files_to_update.sort();
        plan.helper_overwrite.sort();
        plan.warnings.sort();

        Ok(AuthMigrationState { plan, prepared })
    }

    fn local_auth_migration_targets(&self) -> Result<Vec<AuthMigrationTarget>> {
        let home = Self::home_dir()?;
        let tool_shell = self.native_runtime_tool_shell();
        let mut targets = Vec::new();
        targets.push(AuthMigrationTarget::Local {
            path: Self::local_user_settings_path(&home),
            backup_path: Self::local_user_settings_path(&home).with_file_name(format!(
                "settings.json.bak-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            )),
            tool_shell,
        });

        for profile in self.list_profiles()? {
            if profile.kind != ProfileKind::Full {
                continue;
            }
            let path = self.profile_dir(&profile).join("settings.json");
            let backup_path = path.with_file_name(format!(
                "settings.json.bak-{}",
                Utc::now().format("%Y%m%d%H%M%S")
            ));
            targets.push(AuthMigrationTarget::Local {
                path,
                backup_path,
                tool_shell,
            });
        }

        Ok(targets)
    }

    fn inspect_auth_migration_target(
        &self,
        target: AuthMigrationTarget,
        label: String,
        plan: &mut AuthMigrationPlan,
        prepared: &mut Vec<PreparedAuthMigration>,
    ) -> Result<()> {
        let content = match &target {
            AuthMigrationTarget::Local { path, .. } => {
                if !path.exists() {
                    plan.files_missing += 1;
                    plan.warnings
                        .push(format!("{label}: settings file does not exist"));
                    return Ok(());
                }
                fs::read_to_string(path)
                    .with_context(|| format!("Failed to read {}", path.display()))?
            }
            AuthMigrationTarget::Remote {
                host,
                remote_path,
                remote_os,
                ..
            } => match Self::read_remote_text_file(host, remote_path, *remote_os) {
                Ok(content) => content,
                Err(err) if Self::is_benign_sftp_missing_error(&err) => {
                    plan.files_missing += 1;
                    plan.warnings
                        .push(format!("{label}: settings file does not exist"));
                    return Ok(());
                }
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings.push(format!("{label}: {err}"));
                    return Ok(());
                }
            },
        };

        let tool_shell = match &target {
            AuthMigrationTarget::Local { tool_shell, .. } => *tool_shell,
            AuthMigrationTarget::Remote { tool_shell, .. } => *tool_shell,
        };

        match Self::migrate_settings_json_content(&content, tool_shell) {
            Ok(SettingsMigrationOutcome {
                updated_content: Some(updated_content),
                helper_overwritten,
            }) => {
                plan.files_to_update_count += 1;
                if helper_overwritten {
                    plan.helpers_overwritten += 1;
                    plan.helper_overwrite.push(label.clone());
                    plan.warnings.push(format!(
                        "{label}: existing apiKeyHelper will be replaced by the token-derived helper"
                    ));
                }
                plan.files_to_update.push(label.clone());
                prepared.push(PreparedAuthMigration {
                    target,
                    updated_content,
                });
            }
            Ok(SettingsMigrationOutcome {
                updated_content: None,
                helper_overwritten: _,
            }) => {
                plan.files_already_ok += 1;
            }
            Err(err) => {
                plan.files_skipped += 1;
                plan.warnings.push(format!("{label}: {err}"));
            }
        }

        Ok(())
    }

    fn migrate_settings_json_content(
        content: &str,
        tool_shell: TinyfishToolShell,
    ) -> Result<SettingsMigrationOutcome> {
        let mut value: Value =
            serde_json::from_str(content).context("settings file is not valid JSON")?;
        let object = value
            .as_object_mut()
            .context("settings root must be a JSON object")?;

        let existing_helper = match object.get("apiKeyHelper") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => bail!("settings apiKeyHelper must be a string"),
        };

        let token = match object.get("env") {
            None | Some(Value::Null) => None,
            Some(Value::Object(env)) => match env.get("ANTHROPIC_AUTH_TOKEN") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => bail!("settings env ANTHROPIC_AUTH_TOKEN must be a string"),
            },
            Some(_) => bail!("settings env block must be an object"),
        };
        let Some(token) = token else {
            return Ok(SettingsMigrationOutcome {
                updated_content: None,
                helper_overwritten: false,
            });
        };

        let helper_overwritten = existing_helper.is_some();
        Self::set_inline_api_key_helper(object, &token, tool_shell)?;
        Ok(SettingsMigrationOutcome {
            updated_content: Some(
                serde_json::to_string_pretty(&value)
                    .context("Failed to serialize migrated settings JSON")?
                    + "\n",
            ),
            helper_overwritten,
        })
    }

    fn auth_migration_target_label(target: &AuthMigrationTarget) -> String {
        match target {
            AuthMigrationTarget::Local { path, .. } => path.display().to_string(),
            AuthMigrationTarget::Remote {
                host, remote_path, ..
            } => format!("{host}:{remote_path}"),
        }
    }

    fn local_user_settings_path(home: &Path) -> PathBuf {
        home.join(".claude").join("settings.json")
    }

    fn remote_user_settings_path(remote_home: &str, remote_os: RemoteOs) -> String {
        let relative = match remote_os {
            RemoteOs::Unix => ".claude/settings.json",
            RemoteOs::Windows => ".claude\\settings.json",
        };
        Self::join_remote_path(remote_home, remote_os, relative)
    }

    fn escape_sh_double_quoted(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '$' => out.push_str("\\$"),
                '`' => out.push_str("\\`"),
                _ => out.push(ch),
            }
        }
        out
    }

    fn unescape_sh_double_quoted(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some(next @ ('\\' | '"' | '$' | '`')) => out.push(next),
                    Some(next) => {
                        out.push('\\');
                        out.push(next);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn remote_path_for_sftp(remote_path: &str, remote_os: RemoteOs) -> String {
        if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        }
    }

    fn read_remote_text_file(host: &str, remote_path: &str, remote_os: RemoteOs) -> Result<String> {
        let sftp_path = Self::remote_path_for_sftp(remote_path, remote_os);
        let local_tmp = std::env::temp_dir().join(format!("cswitch-auth-{}", Uuid::new_v4()));
        let cmd = format!(
            "get {} {}\n",
            Self::sftp_quote(&sftp_path),
            Self::sftp_quote(&local_tmp.display().to_string())
        );
        let result = Self::run_remote_sftp_commands(host, &cmd);
        let content = match &result {
            Ok(_) => fs::read_to_string(&local_tmp)
                .with_context(|| format!("Failed to read temp file '{}'", local_tmp.display()))?,
            Err(_) => String::new(),
        };
        let _ = fs::remove_file(&local_tmp);
        result?;
        Ok(content)
    }

    fn write_remote_text_file(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
        content: &str,
    ) -> Result<()> {
        let local_tmp = std::env::temp_dir().join(format!("cswitch-auth-{}", Uuid::new_v4()));
        fs::write(&local_tmp, content)
            .with_context(|| format!("Failed to write temp file '{}'", local_tmp.display()))?;
        let sftp_path = Self::remote_path_for_sftp(remote_path, remote_os);
        let cmd = format!(
            "put {} {}\n",
            Self::sftp_quote(&local_tmp.display().to_string()),
            Self::sftp_quote(&sftp_path)
        );
        let result = Self::run_remote_sftp_commands(host, &cmd);
        let _ = fs::remove_file(&local_tmp);
        result?;
        Ok(())
    }
}
