use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use sysinfo::{Pid, System};
use uuid::Uuid;

use super::{
    LightweightEnv, Profile, ProfileManager, Provider, ProviderKey, RuntimeGcSummary,
    RuntimeSessionInfo, RuntimeSessionState, RuntimeSessionStatus, TinyfishToolShell,
    build_lightweight_runtime_artifacts, build_lightweight_settings, discover_models_with_timeout,
    is_local_runtime_base_url, tinyfish_available,
};

const RUNTIME_SCHEMA: &str = "claude-switch-runtime-v1";
const RUNTIME_DIR_PREFIX: &str = "proc_";
const RUNTIME_HELPER_TTL_MS: &str = "5000";
const RUNTIME_GATEWAY_MODELS_FILE_NAME: &str = "gateway-models.json";
const RUNTIME_GATEWAY_DISCOVERY_TIMEOUT_SECS: u64 = 2;
const RUNTIME_PROVIDER_ROUTING_EXTRA_KEYS: &[&str] = &[
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
    "CLAUDE_CODE_SKIP_MANTLE_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_VERTEX",
];
const RUNTIME_PROVIDER_ROUTING_EXTRA_PREFIXES: &[&str] = &[
    "ANTHROPIC_BEDROCK_",
    "ANTHROPIC_FOUNDRY_",
    "ANTHROPIC_VERTEX_",
    "VERTEX_REGION_",
];

impl ProfileManager {
    fn ensure_runtime_hot_switch_allowed_for_base_url(
        &self,
        base_url: &str,
        context: &str,
    ) -> Result<()> {
        if !is_local_runtime_base_url(base_url) {
            return Ok(());
        }
        bail!(
            "{context} uses local/self-hosted API base URL '{}'; local/self-hosted lite profiles bypass runtime sessions and do not support dynamic hot switch.",
            base_url.trim()
        );
    }

    pub(super) fn runtime_root_dir(&self) -> PathBuf {
        self.base_dir().join("runtime")
    }

    pub(super) fn runtime_session_dir(&self, session_id: &str) -> PathBuf {
        self.runtime_root_dir().join(session_id)
    }

    pub(super) fn runtime_settings_path(&self, session_id: &str) -> PathBuf {
        self.runtime_session_dir(session_id).join("settings.json")
    }

    pub(super) fn runtime_state_path(&self, session_id: &str) -> PathBuf {
        self.runtime_session_dir(session_id).join("state.json")
    }

    pub(super) fn runtime_gateway_models_path(&self, session_id: &str) -> PathBuf {
        self.runtime_session_dir(session_id)
            .join(RUNTIME_GATEWAY_MODELS_FILE_NAME)
    }

    pub(super) fn global_gateway_models_path(&self) -> PathBuf {
        let home_root = self
            .base_dir()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.base_dir());
        home_root
            .join(".claude")
            .join("cache")
            .join(RUNTIME_GATEWAY_MODELS_FILE_NAME)
    }

    pub(super) fn is_managed_runtime_dir_name(file_name: &str) -> bool {
        file_name.starts_with(RUNTIME_DIR_PREFIX)
    }

    pub(super) fn next_runtime_session_id(&self) -> String {
        loop {
            let candidate = format!("{RUNTIME_DIR_PREFIX}{}", &Uuid::new_v4().to_string()[..8]);
            if !self.runtime_session_dir(&candidate).exists() {
                return candidate;
            }
        }
    }

    pub(super) fn runtime_auth_command(&self, session_id: &str) -> Result<String> {
        let exe = std::env::current_exe().context("Failed to resolve current cswitch path")?;
        let path = exe.to_string_lossy().replace('\\', "/");
        #[cfg(target_os = "windows")]
        {
            let escaped = path.replace('\'', "''");
            Ok(format!(
                "powershell -NoProfile -Command \"& '{escaped}' runtime auth {session_id}\""
            ))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let escaped = path.replace('"', "\\\"");
            Ok(format!("\"{escaped}\" runtime auth {session_id}"))
        }
    }

    pub(super) fn runtime_session_state_from_profile(
        &self,
        session_id: String,
        profile: &Profile,
        provider: Option<&Provider>,
        key: Option<&ProviderKey>,
        auth: (&str, &str),
        cwd: Option<PathBuf>,
    ) -> RuntimeSessionState {
        let (token, base_url) = auth;
        let env = profile.env.clone().unwrap_or_default();
        let now = Utc::now();
        RuntimeSessionState {
            schema: RUNTIME_SCHEMA.to_string(),
            session_id,
            status: RuntimeSessionStatus::Active,
            pid: None,
            process_started_at: None,
            created_at: now,
            updated_at: now,
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            profile_alias: profile.alias.clone(),
            cwd,
            provider_id: provider.map(|prov| prov.id.clone()),
            provider_name: provider.map(|prov| prov.name.clone()),
            key_id: key.map(|selected| selected.id.clone()),
            key_name: key.map(|selected| selected.name.clone()),
            auth_token: token.to_string(),
            base_url: base_url.to_string(),
            default_opus_model: env.default_opus_model,
            default_sonnet_model: env.default_sonnet_model,
            default_haiku_model: env.default_haiku_model,
            model: env.model,
            subagent_model: env.subagent_model,
            extras: env.extras,
        }
    }

    pub(super) fn runtime_env_from_session(session: &RuntimeSessionState) -> LightweightEnv {
        LightweightEnv {
            auth_token: None,
            base_url: None,
            default_opus_model: session.default_opus_model.clone(),
            default_sonnet_model: session.default_sonnet_model.clone(),
            default_haiku_model: session.default_haiku_model.clone(),
            model: session.model.clone(),
            subagent_model: session.subagent_model.clone(),
            extras: session.extras.clone(),
        }
    }

    fn runtime_switch_model_value(model: &str) -> Option<String> {
        Some(model.trim().to_string()).filter(|value| !value.is_empty())
    }

    fn runtime_nonempty_model(model: Option<&str>) -> Option<&str> {
        model.and_then(|value| (!value.trim().is_empty()).then_some(value))
    }

    fn runtime_primary_model(session: &RuntimeSessionState) -> Option<&str> {
        Self::runtime_nonempty_model(session.model.as_deref())
            .or_else(|| Self::runtime_nonempty_model(session.default_sonnet_model.as_deref()))
            .or_else(|| Self::runtime_nonempty_model(session.default_opus_model.as_deref()))
            .or_else(|| Self::runtime_nonempty_model(session.default_haiku_model.as_deref()))
            .or_else(|| Self::runtime_nonempty_model(session.subagent_model.as_deref()))
    }

    fn runtime_gateway_cache_model_id(model: &str) -> Option<String> {
        let trimmed = model.trim();
        let normalized = trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim_end();
        (!normalized.is_empty()).then(|| normalized.to_string())
    }

    fn push_runtime_gateway_cache_model(models: &mut Vec<String>, model: Option<&str>) {
        let Some(model) = model.and_then(Self::runtime_gateway_cache_model_id) else {
            return;
        };
        if !models.iter().any(|existing| existing == &model) {
            models.push(model);
        }
    }

    fn runtime_gateway_cache_fallback_models(session: &RuntimeSessionState) -> Vec<String> {
        let mut models = Vec::new();
        Self::push_runtime_gateway_cache_model(&mut models, Self::runtime_primary_model(session));
        Self::push_runtime_gateway_cache_model(&mut models, session.default_opus_model.as_deref());
        Self::push_runtime_gateway_cache_model(
            &mut models,
            session.default_sonnet_model.as_deref(),
        );
        Self::push_runtime_gateway_cache_model(&mut models, session.default_haiku_model.as_deref());
        Self::push_runtime_gateway_cache_model(&mut models, session.subagent_model.as_deref());
        models
    }

    fn runtime_gateway_cache_discovered_models(
        session: &RuntimeSessionState,
        discovered: &[String],
    ) -> Vec<String> {
        let mut models = Vec::new();
        Self::push_runtime_gateway_cache_model(&mut models, Self::runtime_primary_model(session));
        for model in discovered {
            Self::push_runtime_gateway_cache_model(&mut models, Some(model.as_str()));
        }
        models
    }

    fn runtime_custom_model_option_name(session: &RuntimeSessionState, model: &str) -> String {
        let provider = session
            .provider_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Gateway");
        format!("{provider}: {model}")
    }

    fn runtime_custom_model_option_description(session: &RuntimeSessionState) -> String {
        let provider = session
            .provider_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(session.base_url.as_str());
        format!("Runtime gateway model via {provider}")
    }

    fn build_gateway_models_cache_json(base_url: &str, models: &[String]) -> Result<String> {
        let model_entries: Vec<Value> = models
            .iter()
            .map(|model| serde_json::json!({ "id": model }))
            .collect();
        serde_json::to_string(&serde_json::json!({
            "baseUrl": base_url.trim(),
            "fetchedAt": Utc::now().timestamp_millis(),
            "models": model_entries,
        }))
        .context("Failed to serialize gateway models cache JSON")
    }

    fn normalized_runtime_gateway_base_url(base_url: &str) -> Option<String> {
        let trimmed = base_url.trim();
        (!trimmed.is_empty()).then(|| trimmed.trim_end_matches('/').to_string())
    }

    fn read_gateway_models_cache_models(content: &str) -> Vec<String> {
        let mut models = Vec::new();
        let Ok(json) = serde_json::from_str::<Value>(content) else {
            return models;
        };
        for entry in json
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            Self::push_runtime_gateway_cache_model(
                &mut models,
                entry.get("id").and_then(Value::as_str),
            );
        }
        models
    }

    fn active_gateway_runtime_sessions(
        &self,
        exclude_session_id: Option<&str>,
    ) -> Result<Vec<RuntimeSessionState>> {
        Ok(self
            .list_runtime_sessions()?
            .into_iter()
            .filter(|session| session.active)
            .filter(|session| {
                exclude_session_id.is_none_or(|excluded| session.state.session_id != excluded)
            })
            .filter(|session| {
                Self::normalized_runtime_gateway_base_url(&session.state.base_url).is_some()
            })
            .map(|session| session.state)
            .collect())
    }

    fn runtime_gateway_cache_models_for_session(
        &self,
        session: &RuntimeSessionState,
    ) -> Vec<String> {
        let shadow_path = self.runtime_gateway_models_path(&session.session_id);
        let shadow_models = fs::read_to_string(shadow_path)
            .ok()
            .map(|content| Self::read_gateway_models_cache_models(&content))
            .unwrap_or_default();
        if shadow_models.is_empty() {
            Self::runtime_gateway_cache_fallback_models(session)
        } else {
            shadow_models
        }
    }

    fn merged_runtime_gateway_cache_models(
        &self,
        session: &RuntimeSessionState,
        current_models: &[String],
    ) -> Result<Vec<String>> {
        let Some(target_base_url) = Self::normalized_runtime_gateway_base_url(&session.base_url)
        else {
            return Ok(Vec::new());
        };
        let mut merged = Vec::new();
        for model in current_models {
            Self::push_runtime_gateway_cache_model(&mut merged, Some(model.as_str()));
        }
        for other in self.active_gateway_runtime_sessions(Some(&session.session_id))? {
            if Self::normalized_runtime_gateway_base_url(&other.base_url).as_deref()
                != Some(target_base_url.as_str())
            {
                continue;
            }
            for model in self.runtime_gateway_cache_models_for_session(&other) {
                Self::push_runtime_gateway_cache_model(&mut merged, Some(model.as_str()));
            }
        }
        Ok(merged)
    }

    pub(super) fn ensure_runtime_gateway_cache_compatible(
        &self,
        exclude_session_id: Option<&str>,
        target_base_url: &str,
    ) -> Result<()> {
        let Some(target_base_url) = Self::normalized_runtime_gateway_base_url(target_base_url)
        else {
            return Ok(());
        };
        let conflicts: Vec<_> = self
            .active_gateway_runtime_sessions(exclude_session_id)?
            .into_iter()
            .filter(|session| {
                Self::normalized_runtime_gateway_base_url(&session.base_url).as_deref()
                    != Some(target_base_url.as_str())
            })
            .collect();
        if conflicts.is_empty() {
            return Ok(());
        }
        let details = conflicts
            .iter()
            .map(|session| {
                let provider = session.provider_name.as_deref().unwrap_or("inline");
                let base_url = Self::normalized_runtime_gateway_base_url(&session.base_url)
                    .unwrap_or_else(|| session.base_url.trim().to_string());
                format!("{} ({provider}) -> {base_url}", session.session_id)
            })
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "Cannot activate gateway '{}' while active runtime session(s) use different gateway URL(s): {}. Claude Code has a single global gateway cache; close or switch the conflicting session(s) first.",
            target_base_url,
            details
        );
    }

    fn runtime_extra_is_provider_routing(extra: &str) -> bool {
        let Some((key, _)) = extra.split_once('=') else {
            return false;
        };
        let normalized = key.trim().to_ascii_uppercase();
        RUNTIME_PROVIDER_ROUTING_EXTRA_KEYS
            .iter()
            .any(|candidate| normalized == *candidate)
            || RUNTIME_PROVIDER_ROUTING_EXTRA_PREFIXES
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
    }

    fn scrub_runtime_switch_extras(extras: &[String]) -> Vec<String> {
        extras
            .iter()
            .filter(|extra| !Self::runtime_extra_is_provider_routing(extra))
            .cloned()
            .collect()
    }

    fn apply_runtime_switch_selection(
        state: &mut RuntimeSessionState,
        provider: &Provider,
        key: &ProviderKey,
        model: &str,
    ) {
        let selected_model = Self::runtime_switch_model_value(model);
        state.provider_id = Some(provider.id.clone());
        state.provider_name = Some(provider.name.clone());
        state.key_id = Some(key.id.clone());
        state.key_name = Some(key.name.clone());
        state.auth_token = key.api_key.clone();
        state.base_url = provider.base_url.clone();
        state.default_opus_model = selected_model.clone();
        state.default_sonnet_model = selected_model.clone();
        state.default_haiku_model = selected_model.clone();
        state.model = selected_model.clone();
        state.subagent_model = selected_model;
        state.extras = Self::scrub_runtime_switch_extras(&state.extras);
    }

    pub(super) fn build_runtime_settings_json(
        &self,
        session: &RuntimeSessionState,
    ) -> Result<String> {
        let env = Self::runtime_env_from_session(session);
        let tool_shell = self.native_runtime_tool_shell();
        let artifacts =
            build_lightweight_runtime_artifacts(&env, None, Some(&session.base_url), tool_shell)?;
        let tinyfish_enabled = artifacts.tinyfish_enabled && tinyfish_available();
        let tinyfish_plugin_variant = if tinyfish_enabled {
            Some(
                artifacts
                    .tinyfish_plugin_variant
                    .context("TinyFish plugin variant missing for runtime settings")?,
            )
        } else {
            None
        };
        let tinyfish_statusline_script_path = tinyfish_plugin_variant.map(|plugin_variant| {
            self.local_tinyfish_statusline_script_path(plugin_variant, tool_shell)
                .to_string_lossy()
                .to_string()
        });
        let mut settings = build_lightweight_settings(
            &env,
            None,
            Some(&session.base_url),
            tinyfish_enabled,
            tool_shell,
            tinyfish_statusline_script_path.as_deref(),
        )?;
        if let Some(model) = Self::runtime_primary_model(session) {
            settings.insert("model".into(), Value::String(model.to_string()));
        }
        settings.insert(
            "apiKeyHelper".into(),
            Value::String(self.runtime_auth_command(&session.session_id)?),
        );
        let env_map = settings
            .get_mut("env")
            .and_then(Value::as_object_mut)
            .context("Runtime settings env block missing")?;
        env_map.insert(
            "CLAUDE_CODE_API_KEY_HELPER_TTL_MS".into(),
            Value::String(RUNTIME_HELPER_TTL_MS.to_string()),
        );
        if let Some(model) = Self::runtime_primary_model(session) {
            env_map.insert(
                "ANTHROPIC_CUSTOM_MODEL_OPTION".into(),
                Value::String(model.to_string()),
            );
            env_map.insert(
                "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME".into(),
                Value::String(Self::runtime_custom_model_option_name(session, model)),
            );
            env_map.insert(
                "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION".into(),
                Value::String(Self::runtime_custom_model_option_description(session)),
            );
        }
        serde_json::to_string(&settings).context("Failed to serialize runtime settings JSON")
    }

    pub(super) fn ensure_runtime_session_dir(&self, session_id: &str) -> Result<PathBuf> {
        let dir = self.runtime_session_dir(session_id);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub(super) fn write_runtime_state_atomic(
        &self,
        state_path: &Path,
        state: &RuntimeSessionState,
    ) -> Result<()> {
        let content =
            serde_json::to_string_pretty(state).context("Failed to serialize runtime state")?;
        let tmp_path = state_path.with_extension("json.tmp");
        fs::write(&tmp_path, content)?;
        fs::rename(&tmp_path, state_path)?;
        Ok(())
    }

    pub(super) fn write_runtime_settings_in_place(
        &self,
        settings_path: &Path,
        content: &str,
    ) -> Result<()> {
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(settings_path)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    pub(super) fn remove_runtime_session_dir(&self, session_id: &str) -> Result<()> {
        let dir = self.runtime_session_dir(session_id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    pub(super) fn refresh_runtime_gateway_models_cache_for_session(
        &self,
        session: &RuntimeSessionState,
    ) -> Result<()> {
        let Some(base_url) = Self::normalized_runtime_gateway_base_url(&session.base_url) else {
            return Ok(());
        };
        self.ensure_runtime_gateway_cache_compatible(Some(&session.session_id), &base_url)?;
        let models = match discover_models_with_timeout(
            &base_url,
            &session.auth_token,
            Duration::from_secs(RUNTIME_GATEWAY_DISCOVERY_TIMEOUT_SECS),
        ) {
            Ok(discovery) => {
                Self::runtime_gateway_cache_discovered_models(session, &discovery.models)
            }
            Err(_) => Self::runtime_gateway_cache_fallback_models(session),
        };
        let content = Self::build_gateway_models_cache_json(&base_url, &models)?;
        self.write_runtime_settings_in_place(
            &self.runtime_gateway_models_path(&session.session_id),
            &content,
        )?;
        let merged_models = self.merged_runtime_gateway_cache_models(session, &models)?;
        let global_content = Self::build_gateway_models_cache_json(&base_url, &merged_models)?;
        self.write_runtime_settings_in_place(&self.global_gateway_models_path(), &global_content)?;
        Ok(())
    }

    pub(super) fn refresh_runtime_gateway_models_cache_best_effort(
        &self,
        session: &RuntimeSessionState,
    ) {
        if let Err(error) = self.refresh_runtime_gateway_models_cache_for_session(session) {
            eprintln!(
                "Warning: failed to refresh gateway model cache for runtime session '{}': {}",
                session.session_id, error
            );
        }
    }

    pub fn load_runtime_session(&self, session_id: &str) -> Result<RuntimeSessionState> {
        let state_path = self.runtime_state_path(session_id);
        let content = fs::read_to_string(&state_path)
            .with_context(|| format!("Failed to read runtime state '{}'.", state_path.display()))?;
        let state: RuntimeSessionState = serde_json::from_str(&content).with_context(|| {
            format!("Failed to parse runtime state '{}'.", state_path.display())
        })?;
        Ok(state)
    }

    pub fn list_runtime_sessions(&self) -> Result<Vec<RuntimeSessionInfo>> {
        let runtime_root = self.runtime_root_dir();
        if !runtime_root.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&runtime_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !Self::is_managed_runtime_dir_name(&file_name) {
                continue;
            }
            let state_path = path.join("state.json");
            let settings_path = path.join("settings.json");
            let state = match fs::read_to_string(&state_path)
                .ok()
                .and_then(|content| serde_json::from_str::<RuntimeSessionState>(&content).ok())
            {
                Some(state) => state,
                None => continue,
            };
            let stale_reason = self.runtime_session_stale_reason(&state);
            sessions.push(RuntimeSessionInfo {
                state,
                state_path,
                settings_path,
                active: stale_reason.is_none(),
                stale_reason,
            });
        }
        sessions.sort_by(|left, right| {
            right
                .active
                .cmp(&left.active)
                .then_with(|| right.state.updated_at.cmp(&left.state.updated_at))
        });
        Ok(sessions)
    }

    pub fn switch_runtime_session(
        &self,
        session_id: &str,
        provider_id: &str,
        key_id: &str,
        model: &str,
    ) -> Result<RuntimeSessionState> {
        let mut state = self.load_runtime_session(session_id)?;
        if self.runtime_session_stale_reason(&state).is_some() {
            bail!("Runtime session '{}' is no longer active.", session_id);
        }
        self.ensure_runtime_hot_switch_allowed_for_base_url(
            &state.base_url,
            &format!("Runtime session '{}'", session_id),
        )?;
        let provider = self.get_provider(provider_id)?;
        let key = provider.keys.get(key_id).cloned().with_context(|| {
            format!("Key '{}' not found in provider '{}'.", key_id, provider_id)
        })?;
        self.ensure_runtime_hot_switch_allowed_for_base_url(
            &provider.base_url,
            &format!("Provider '{}'", provider.name),
        )?;
        self.ensure_runtime_gateway_cache_compatible(Some(session_id), &provider.base_url)?;
        Self::apply_runtime_switch_selection(&mut state, &provider, &key, model);
        state.updated_at = Utc::now();

        let settings_content = self.build_runtime_settings_json(&state)?;
        self.write_runtime_state_atomic(&self.runtime_state_path(session_id), &state)?;
        self.write_runtime_settings_in_place(
            &self.runtime_settings_path(session_id),
            &settings_content,
        )?;
        self.refresh_runtime_gateway_models_cache_best_effort(&state);
        Ok(state)
    }

    pub fn garbage_collect_runtime_sessions(&self) -> Result<RuntimeGcSummary> {
        let runtime_root = self.runtime_root_dir();
        if !runtime_root.exists() {
            return Ok(RuntimeGcSummary {
                scanned: 0,
                removed: 0,
                kept: 0,
            });
        }

        let now = Utc::now();
        let mut summary = RuntimeGcSummary {
            scanned: 0,
            removed: 0,
            kept: 0,
        };

        for entry in fs::read_dir(&runtime_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !Self::is_managed_runtime_dir_name(&file_name) {
                continue;
            }
            summary.scanned += 1;
            let state_path = path.join("state.json");
            let state = fs::read_to_string(&state_path)
                .ok()
                .and_then(|content| serde_json::from_str::<RuntimeSessionState>(&content).ok());

            let should_remove = match state {
                Some(state) => {
                    self.runtime_session_stale_reason(&state).is_some()
                        && now.signed_duration_since(state.updated_at) >= ChronoDuration::hours(24)
                }
                None => {
                    let modified = fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .map(DateTime::<Utc>::from);
                    modified
                        .map(|modified_at| {
                            now.signed_duration_since(modified_at) >= ChronoDuration::minutes(5)
                        })
                        .unwrap_or(false)
                }
            };

            if should_remove {
                fs::remove_dir_all(&path)?;
                summary.removed += 1;
            } else {
                summary.kept += 1;
            }
        }

        Ok(summary)
    }

    pub(super) fn runtime_session_stale_reason(
        &self,
        state: &RuntimeSessionState,
    ) -> Option<String> {
        let pid = match state.pid {
            Some(pid) => pid,
            None => return Some("missing pid".to_string()),
        };
        let current = Self::runtime_process_started_at(pid);
        match (state.process_started_at, current) {
            (_, None) => Some(format!("pid {} is not running", pid)),
            (Some(expected), Some(actual)) if expected != actual => Some(format!(
                "pid {} start time changed (expected {}, got {})",
                pid, expected, actual
            )),
            _ => None,
        }
    }

    pub(super) fn runtime_process_started_at(pid: u32) -> Option<DateTime<Utc>> {
        let mut system = System::new_all();
        system.refresh_all();
        let process = system.process(Pid::from_u32(pid))?;
        DateTime::<Utc>::from_timestamp(process.start_time() as i64, 0)
    }

    pub(super) fn native_runtime_tool_shell(&self) -> TinyfishToolShell {
        #[cfg(target_os = "windows")]
        {
            TinyfishToolShell::PowerShell
        }
        #[cfg(not(target_os = "windows"))]
        {
            TinyfishToolShell::Bash
        }
    }
}
