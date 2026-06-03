use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{
    CMD_MARKER, LightweightEnv, Profile, ProfileKind, ProfileManager, Provider, ProviderKey,
    Registry, SH_MARKER, ShimRecoveryPlan, ShimRecoverySummary,
};

pub(super) struct RecoveredShimProfile {
    pub(super) file_name: String,
    pub(super) name: String,
    pub(super) alias: String,
    pub(super) token: String,
    pub(super) base_url: String,
    pub(super) env: LightweightEnv,
    pub(super) launch_args: Option<Vec<String>>,
}

struct ShimRecoveryState {
    plan: ShimRecoveryPlan,
    registry: Registry,
}

impl ProfileManager {
    pub fn plan_shim_recovery(&self, shim_dir: &Path, replace: bool) -> Result<ShimRecoveryPlan> {
        Ok(self.build_shim_recovery_state(shim_dir, replace)?.plan)
    }

    pub fn recover_shims(&self, shim_dir: &Path, replace: bool) -> Result<ShimRecoverySummary> {
        let state = self.build_shim_recovery_state(shim_dir, replace)?;
        if state.plan.conflict_count() > 0 {
            bail!(
                "Shim recovery has {} conflicted profile(s). Use --replace to update them.",
                state.plan.conflict_count()
            );
        }

        let backup_path = if self.registry_path.exists() {
            let backup_path = self.registry_backup_path();
            fs::copy(&self.registry_path, &backup_path).with_context(|| {
                format!(
                    "Failed to create registry backup '{}'.",
                    backup_path.display()
                )
            })?;
            Some(backup_path)
        } else {
            None
        };

        let mut registry = state.registry;
        self.migrate_providers(&mut registry)?;
        self.save_registry(&registry)?;
        Ok(ShimRecoverySummary {
            plan: state.plan,
            backup_path,
        })
    }

    fn build_shim_recovery_state(
        &self,
        shim_dir: &Path,
        replace: bool,
    ) -> Result<ShimRecoveryState> {
        if !shim_dir.exists() {
            bail!("Shim directory '{}' does not exist.", shim_dir.display());
        }
        if !shim_dir.is_dir() {
            bail!("Shim path '{}' is not a directory.", shim_dir.display());
        }

        let mut registry = self.load_registry()?;
        let mut plan = ShimRecoveryPlan {
            shim_dir: shim_dir.to_path_buf(),
            ..Default::default()
        };
        let mut provider_names = registry
            .providers
            .values()
            .map(|provider| provider.name.clone())
            .collect::<HashSet<_>>();
        let mut entries = fs::read_dir(shim_dir)
            .with_context(|| format!("Failed to read shim directory '{}'.", shim_dir.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("Failed to list shim directory '{}'.", shim_dir.display()))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !Self::is_recoverable_shim_file_name(&file_name) {
                continue;
            }

            plan.files_scanned += 1;
            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings
                        .push(format!("{}: failed to read shim: {}", file_name, err));
                    continue;
                }
            };
            let recovered = match Self::parse_recoverable_shim(&file_name, &content) {
                Ok(profile) => profile,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings.push(format!("{}: {}", file_name, err));
                    continue;
                }
            };
            plan.files_recoverable += 1;

            let existing_profile_id = match Self::find_profile_conflict_id(
                &registry,
                &recovered.name,
                Some(&recovered.alias),
            ) {
                Ok(id) => id,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings.push(format!("{}: {}", file_name, err));
                    continue;
                }
            };
            if existing_profile_id.is_some() && !replace {
                plan.profiles_conflicted += 1;
                plan.profiles_conflict.push(format!(
                    "{} ({}) from {}",
                    recovered.name, recovered.alias, recovered.file_name
                ));
                continue;
            }

            let existing_profile = existing_profile_id
                .as_ref()
                .and_then(|id| registry.profiles.get(id))
                .cloned();
            let (provider_id, key_id) = Self::ensure_recovered_provider_key(
                &mut registry,
                &mut plan,
                &mut provider_names,
                &recovered,
            );
            let profile = Self::build_recovered_profile(
                existing_profile_id.clone(),
                &recovered,
                provider_id,
                key_id,
                existing_profile.as_ref(),
            );

            if let Some(id) = existing_profile_id {
                registry.profiles.insert(id, profile.clone());
                plan.profiles_updated += 1;
                plan.profiles_update.push(format!(
                    "{} ({})",
                    profile.name,
                    profile.alias.as_deref().unwrap_or("")
                ));
            } else {
                registry
                    .profiles
                    .insert(profile.id.clone(), profile.clone());
                plan.profiles_added += 1;
                plan.profiles_add.push(format!(
                    "{} ({})",
                    profile.name,
                    profile.alias.as_deref().unwrap_or("")
                ));
            }
        }

        plan.profiles_add.sort();
        plan.profiles_update.sort();
        plan.profiles_conflict.sort();
        plan.providers_add.sort();
        plan.provider_keys_add.sort();
        plan.warnings.sort();

        Ok(ShimRecoveryState { plan, registry })
    }

    fn registry_backup_path(&self) -> PathBuf {
        let timestamp = Utc::now().format("%Y%m%d%H%M%S");
        self.registry_path
            .with_file_name(format!("registry.json.bak-{timestamp}"))
    }

    fn is_recoverable_shim_file_name(file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        (lower.starts_with("claude-") && lower.ends_with(".cmd"))
            || (lower.starts_with("claude-") && !lower.contains('.'))
    }

    pub(super) fn parse_recoverable_shim(
        file_name: &str,
        content: &str,
    ) -> Result<RecoveredShimProfile> {
        if !content.contains(CMD_MARKER) && !content.contains(SH_MARKER) {
            bail!("not a cswitch generated shim");
        }
        let alias = Self::alias_from_shim_file_name(file_name)?;
        let (name, kind) = Self::parse_shim_profile_header(content)
            .with_context(|| "missing generated profile header".to_string())?;
        if kind != ProfileKind::Lightweight {
            bail!("only lightweight shims can be recovered");
        }
        let settings = Self::extract_shim_settings(content)
            .with_context(|| "missing recoverable --settings JSON".to_string())?;
        let env_object = settings
            .get("env")
            .and_then(serde_json::Value::as_object)
            .with_context(|| "settings JSON does not contain an env object".to_string())?;
        let token = Self::json_env_string(env_object, "ANTHROPIC_AUTH_TOKEN")?
            .with_context(|| "settings env is missing ANTHROPIC_AUTH_TOKEN".to_string())?;
        let base_url = Self::json_env_string(env_object, "ANTHROPIC_BASE_URL")?
            .with_context(|| "settings env is missing ANTHROPIC_BASE_URL".to_string())?;
        let mut extras = Vec::new();
        for (key, value) in env_object {
            if Self::known_lightweight_env_key(key) {
                continue;
            }
            if let Some(value) = value.as_str() {
                extras.push(format!("{key}={value}"));
            }
        }
        extras.sort();
        let env = LightweightEnv {
            auth_token: None,
            base_url: None,
            default_opus_model: Self::json_env_string(env_object, "ANTHROPIC_DEFAULT_OPUS_MODEL")?,
            default_sonnet_model: Self::json_env_string(
                env_object,
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
            )?,
            default_haiku_model: Self::json_env_string(
                env_object,
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            )?,
            model: Self::json_env_string(env_object, "ANTHROPIC_MODEL")?,
            subagent_model: Self::json_env_string(env_object, "CLAUDE_CODE_SUBAGENT_MODEL")?,
            extras,
        };
        Ok(RecoveredShimProfile {
            file_name: file_name.to_string(),
            name,
            alias,
            token,
            base_url,
            env,
            launch_args: Self::extract_shim_launch_args(content),
        })
    }

    fn alias_from_shim_file_name(file_name: &str) -> Result<String> {
        let stem = file_name
            .strip_suffix(".cmd")
            .or_else(|| file_name.strip_suffix(".CMD"))
            .unwrap_or(file_name);
        let alias = stem
            .strip_prefix("claude-")
            .with_context(|| format!("shim '{}' does not use the claude- prefix", file_name))?;
        if alias.trim().is_empty() {
            bail!("shim '{}' has an empty alias", file_name);
        }
        Self::validate_alias(alias)?;
        Ok(alias.to_string())
    }

    fn parse_shim_profile_header(content: &str) -> Option<(String, ProfileKind)> {
        for line in content.lines() {
            let trimmed = line.trim();
            let header = trimmed
                .strip_prefix(":: Profile: ")
                .or_else(|| trimmed.strip_prefix("# Profile: "));
            let Some(header) = header else {
                continue;
            };
            let (name, kind) = header.rsplit_once(" (")?;
            let kind = kind.strip_suffix(')')?;
            let kind = match kind {
                "lightweight" => ProfileKind::Lightweight,
                "full" => ProfileKind::Full,
                _ => return None,
            };
            return Some((name.to_string(), kind));
        }
        None
    }

    fn extract_shim_settings(content: &str) -> Result<serde_json::Value> {
        let candidates = [
            Self::extract_cmd_var(content, "_SETTINGS"),
            Self::extract_cmd_var(content, "_TF_SETTINGS"),
            Self::extract_legacy_inline_cmd_settings(content),
            Self::extract_shell_settings_env(content),
        ];
        for candidate in candidates.into_iter().flatten() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&candidate)
                && value.get("env").is_some()
            {
                return Ok(value);
            }
        }
        bail!("no parseable settings JSON found")
    }

    fn extract_cmd_var(content: &str, var_name: &str) -> Option<String> {
        let prefix = format!("set \"{var_name}=");
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix('"'))
            {
                return Some(Self::unescape_cmd_json_fragment(value));
            }
        }
        None
    }

    fn extract_legacy_inline_cmd_settings(content: &str) -> Option<String> {
        let marker = "claude --settings \"";
        let start = content.find(marker)? + marker.len();
        let rest = &content[start..];
        let end = Self::find_cmd_quoted_value_end(rest)?;
        Some(Self::unescape_cmd_json_fragment(&rest[..end]))
    }

    fn find_cmd_quoted_value_end(value: &str) -> Option<usize> {
        let mut backslashes = 0usize;
        for (idx, ch) in value.char_indices() {
            if ch == '\\' {
                backslashes += 1;
                continue;
            }
            if ch == '"' && backslashes.is_multiple_of(2) {
                return Some(idx);
            }
            backslashes = 0;
        }
        None
    }

    fn extract_shell_settings_env(content: &str) -> Option<String> {
        for line in content.lines() {
            let line = line.trim();
            let Some(value) = line.strip_prefix("SETTINGS_ENV=") else {
                continue;
            };
            let value = value.strip_prefix('\'')?.strip_suffix('\'')?;
            let mut settings = Self::unescape_shell_single_quoted_value(value);
            settings.push('}');
            return Some(settings);
        }
        None
    }

    fn unescape_cmd_json_fragment(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' if chars.peek() == Some(&'"') => {
                    chars.next();
                    out.push('"');
                }
                '^' => {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                '%' if chars.peek() == Some(&'%') => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push(ch),
            }
        }
        out
    }

    fn unescape_shell_single_quoted_value(value: &str) -> String {
        value.replace("'\\''", "'")
    }

    fn json_env_string(
        object: &serde_json::Map<String, serde_json::Value>,
        key: &str,
    ) -> Result<Option<String>> {
        match object.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => bail!("settings env field '{}' must be a string", key),
        }
    }

    fn known_lightweight_env_key(key: &str) -> bool {
        matches!(
            key,
            "ANTHROPIC_AUTH_TOKEN"
                | "ANTHROPIC_BASE_URL"
                | "ANTHROPIC_DEFAULT_OPUS_MODEL"
                | "ANTHROPIC_DEFAULT_SONNET_MODEL"
                | "ANTHROPIC_DEFAULT_HAIKU_MODEL"
                | "ANTHROPIC_MODEL"
                | "CLAUDE_CODE_SUBAGENT_MODEL"
        )
    }

    fn extract_shim_launch_args(content: &str) -> Option<Vec<String>> {
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix("set \"_LAUNCH_ARGS=")
                .and_then(|value| value.strip_suffix('"'))
            {
                return Some(Self::split_recovered_launch_args(value));
            }
        }
        None
    }

    fn split_recovered_launch_args(value: &str) -> Vec<String> {
        value
            .split_whitespace()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect()
    }

    fn find_profile_conflict_id(
        registry: &Registry,
        name: &str,
        alias: Option<&str>,
    ) -> Result<Option<String>> {
        let by_name = registry
            .profiles
            .iter()
            .find(|(_, profile)| profile.name == name)
            .map(|(id, _)| id.clone());
        let by_alias = alias.and_then(|alias| {
            registry
                .profiles
                .iter()
                .find(|(_, profile)| profile.alias.as_deref() == Some(alias))
                .map(|(id, _)| id.clone())
        });
        match (by_name, by_alias) {
            (Some(left), Some(right)) if left != right => bail!(
                "name '{}' and alias '{}' match different existing profiles",
                name,
                alias.unwrap_or_default()
            ),
            (Some(id), _) | (_, Some(id)) => Ok(Some(id)),
            (None, None) => Ok(None),
        }
    }

    fn ensure_recovered_provider_key(
        registry: &mut Registry,
        plan: &mut ShimRecoveryPlan,
        provider_names: &mut HashSet<String>,
        recovered: &RecoveredShimProfile,
    ) -> (String, String) {
        if let Some((provider_id, key_id)) = Self::find_provider_key_by_url_and_token(
            registry,
            &recovered.base_url,
            &recovered.token,
        ) {
            plan.provider_keys_reused += 1;
            return (provider_id, key_id);
        }

        let provider_id = if let Some((id, _)) = registry
            .providers
            .iter()
            .find(|(_, provider)| provider.base_url == recovered.base_url)
        {
            id.clone()
        } else {
            let id = Self::new_unique_provider_id(registry);
            let name = Self::unique_recovered_provider_name(provider_names, &recovered.base_url);
            registry.providers.insert(
                id.clone(),
                Provider {
                    id: id.clone(),
                    name: name.clone(),
                    base_url: recovered.base_url.clone(),
                    keys: HashMap::new(),
                    api_key: String::new(),
                },
            );
            plan.providers_added += 1;
            plan.providers_add
                .push(format!("{} ({})", name, recovered.base_url));
            id
        };

        let key_id = Self::new_unique_key_id(
            registry
                .providers
                .get(&provider_id)
                .expect("provider was just created or found"),
        );
        let key_name = Self::unique_recovered_key_name(
            registry
                .providers
                .get(&provider_id)
                .expect("provider was just created or found"),
            &recovered.alias,
        );
        let provider = registry
            .providers
            .get_mut(&provider_id)
            .expect("provider was just created or found");
        provider.keys.insert(
            key_id.clone(),
            ProviderKey {
                id: key_id.clone(),
                name: key_name.clone(),
                api_key: recovered.token.clone(),
            },
        );
        plan.provider_keys_added += 1;
        plan.provider_keys_add.push(format!(
            "{} / {} from {}",
            provider.name, key_name, recovered.file_name
        ));
        (provider_id, key_id)
    }

    fn find_provider_key_by_url_and_token(
        registry: &Registry,
        base_url: &str,
        token: &str,
    ) -> Option<(String, String)> {
        for (provider_id, provider) in &registry.providers {
            if provider.base_url != base_url {
                continue;
            }
            for (key_id, key) in &provider.keys {
                if key.api_key == token {
                    return Some((provider_id.clone(), key_id.clone()));
                }
            }
        }
        None
    }

    fn new_unique_provider_id(registry: &Registry) -> String {
        loop {
            let id = format!("prov_{}", &Uuid::new_v4().to_string()[..8]);
            if !registry.providers.contains_key(&id) {
                return id;
            }
        }
    }

    fn new_unique_key_id(provider: &Provider) -> String {
        loop {
            let id = format!("key_{}", &Uuid::new_v4().to_string()[..8]);
            if !provider.keys.contains_key(&id) {
                return id;
            }
        }
    }

    fn unique_recovered_provider_name(names: &mut HashSet<String>, base_url: &str) -> String {
        let mut base = "Recovered provider".to_string();
        if let Some(host) = Self::host_from_url(base_url)
            && !host.is_empty()
        {
            base = format!("Recovered {host}");
        }
        let mut candidate = base.clone();
        let mut index = 2usize;
        while names.contains(&candidate) {
            candidate = format!("{base} {index}");
            index += 1;
        }
        names.insert(candidate.clone());
        candidate
    }

    fn host_from_url(url: &str) -> Option<String> {
        let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
        let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
        let host = authority.split(':').next()?.trim();
        if host.is_empty() {
            None
        } else {
            Some(host.to_string())
        }
    }

    fn unique_recovered_key_name(provider: &Provider, alias: &str) -> String {
        let base = format!("Recovered {alias}");
        let names = provider
            .keys
            .values()
            .map(|key| key.name.as_str())
            .collect::<HashSet<_>>();
        if !names.contains(base.as_str()) {
            return base;
        }
        let mut index = 2usize;
        loop {
            let candidate = format!("{base} {index}");
            if !names.contains(candidate.as_str()) {
                return candidate;
            }
            index += 1;
        }
    }

    fn build_recovered_profile(
        existing_id: Option<String>,
        recovered: &RecoveredShimProfile,
        provider_id: String,
        key_id: String,
        existing: Option<&Profile>,
    ) -> Profile {
        Profile {
            id: existing_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: recovered.name.clone(),
            alias: Some(recovered.alias.clone()),
            added: existing
                .map(|profile| profile.added)
                .unwrap_or_else(Utc::now),
            last_used: existing.and_then(|profile| profile.last_used),
            kind: ProfileKind::Lightweight,
            env: Some(recovered.env.clone()),
            launch_args: recovered.launch_args.clone(),
            provider_id: Some(provider_id),
            key_id: Some(key_id),
            mcp_server_ids: existing
                .map(|profile| profile.mcp_server_ids.clone())
                .unwrap_or_default(),
        }
    }
}
