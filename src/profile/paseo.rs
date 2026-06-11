use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;

use super::url_match::{is_local_runtime_base_url, url_matches};
use super::{
    LightweightEnv, Profile, ProfileKind, ProfileManager, build_lightweight_runtime_artifacts,
    build_lightweight_settings, discover_models, tinyfish_available,
    tinyfish_statusline_script_file_name,
};

const PASEO_SCHEMA_URL: &str = "https://paseo.sh/schemas/paseo.config.v1.json";
const PASEO_PROVIDER_PREFIX: &str = "csw-";
const PASEO_PROVIDER_FALLBACK_PREFIX: &str = "csw-profile-";
const PASEO_ADDITIONAL_MODEL_URLS: &[&str] =
    &["https://api.anthropic.com", "https://anyrouter.top"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaseoOutputShape {
    ProvidersOnly,
    #[default]
    AgentsFragment,
    FullConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaseoSecretMode {
    #[default]
    Wrapper,
    SelfContained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaseoModelPolicy {
    None,
    #[default]
    DiscoverThenFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaseoExportWarningKind {
    ModelDiscovery,
    SecretFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaseoExportWarning {
    pub kind: PaseoExportWarningKind,
    pub profile_id: String,
    pub provider_id: String,
    pub message: String,
}

impl PaseoExportWarning {
    fn model_discovery(profile: &Profile, provider_id: &str, message: impl Into<String>) -> Self {
        Self {
            kind: PaseoExportWarningKind::ModelDiscovery,
            profile_id: profile.id.clone(),
            provider_id: provider_id.to_string(),
            message: message.into(),
        }
    }

    fn secret_fallback(profile: &Profile, provider_id: &str, message: impl Into<String>) -> Self {
        Self {
            kind: PaseoExportWarningKind::SecretFallback,
            profile_id: profile.id.clone(),
            provider_id: provider_id.to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaseoExportOptions {
    pub output_shape: PaseoOutputShape,
    pub secret_mode: PaseoSecretMode,
    pub model_policy: PaseoModelPolicy,
    pub include_stored_launch_args: bool,
    pub strict_model_discovery: bool,
}

impl Default for PaseoExportOptions {
    fn default() -> Self {
        Self {
            output_shape: PaseoOutputShape::AgentsFragment,
            secret_mode: PaseoSecretMode::Wrapper,
            model_policy: PaseoModelPolicy::DiscoverThenFallback,
            include_stored_launch_args: false,
            strict_model_discovery: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaseoExportResult {
    pub content: String,
    pub warnings: Vec<PaseoExportWarning>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PaseoProviderModel {
    id: String,
    label: String,
    #[serde(rename = "isDefault", skip_serializing_if = "Option::is_none")]
    is_default: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PaseoProviderConfig {
    extends: String,
    label: String,
    description: String,
    command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<Vec<PaseoProviderModel>>,
    #[serde(rename = "additionalModels", skip_serializing_if = "Option::is_none")]
    additional_models: Option<Vec<PaseoProviderModel>>,
    enabled: bool,
    order: usize,
}

#[derive(Debug, Clone, Default)]
struct ExportCredentialContext {
    auth_token: Option<String>,
    base_url: Option<String>,
    env: Option<LightweightEnv>,
}

type PaseoCommandExport = (Vec<String>, Option<HashMap<String, String>>, &'static str);

#[derive(Debug, Clone)]
struct ModelSelection {
    models: Option<Vec<PaseoProviderModel>>,
    additional_models: Option<Vec<PaseoProviderModel>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelFamily {
    Claude,
    NonClaude,
    Unknown,
}

impl ProfileManager {
    pub fn export_paseo_config(
        &self,
        profile_queries: &[String],
        options: &PaseoExportOptions,
    ) -> Result<PaseoExportResult> {
        let registry = self.load_registry()?;
        let mut profiles = if profile_queries.is_empty() {
            registry.profiles.values().cloned().collect::<Vec<_>>()
        } else {
            let mut selected = Vec::new();
            let mut seen = HashSet::new();
            for query in profile_queries {
                let (id, profile) = Self::find_profile_in_registry(&registry, query)?;
                if seen.insert(id) {
                    selected.push(profile);
                }
            }
            selected
        };
        profiles.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));

        let mut used_provider_ids = HashSet::new();
        let mut providers = Vec::with_capacity(profiles.len());
        let mut warnings = Vec::new();
        for (order, profile) in profiles.iter().enumerate() {
            let provider_id = Self::paseo_provider_id_for_profile(profile, &mut used_provider_ids);
            let provider =
                self.build_paseo_provider(profile, &provider_id, order, options, &mut warnings)?;
            providers.push((provider_id, provider));
        }

        if options.strict_model_discovery
            && warnings
                .iter()
                .any(|warning| warning.kind == PaseoExportWarningKind::ModelDiscovery)
        {
            bail!("Paseo export aborted because strict model discovery is enabled.");
        }

        let mut provider_map = Map::with_capacity(providers.len());
        for (provider_id, provider) in providers {
            provider_map.insert(provider_id, serde_json::to_value(provider)?);
        }

        let value = match options.output_shape {
            PaseoOutputShape::ProvidersOnly => Value::Object(provider_map),
            PaseoOutputShape::AgentsFragment => {
                let mut agents = Map::new();
                agents.insert("providers".into(), Value::Object(provider_map));
                let mut root = Map::new();
                root.insert("agents".into(), Value::Object(agents));
                Value::Object(root)
            }
            PaseoOutputShape::FullConfig => {
                let mut agents = Map::new();
                agents.insert("providers".into(), Value::Object(provider_map));
                let mut root = Map::new();
                root.insert(
                    "$schema".into(),
                    Value::String(PASEO_SCHEMA_URL.to_string()),
                );
                root.insert("version".into(), Value::Number(1.into()));
                root.insert("agents".into(), Value::Object(agents));
                Value::Object(root)
            }
        };

        Ok(PaseoExportResult {
            content: serde_json::to_string_pretty(&value)
                .context("Failed to serialize Paseo export JSON")?,
            warnings,
        })
    }

    fn build_paseo_provider(
        &self,
        profile: &Profile,
        provider_id: &str,
        order: usize,
        options: &PaseoExportOptions,
        warnings: &mut Vec<PaseoExportWarning>,
    ) -> Result<PaseoProviderConfig> {
        let credential_context = self.export_credential_context(profile, provider_id, warnings)?;
        let (command, env, mode_description) = self.paseo_command_and_env(
            profile,
            provider_id,
            &credential_context,
            options,
            warnings,
        )?;
        let models = self.paseo_models_for_profile(
            profile,
            provider_id,
            &credential_context,
            options,
            warnings,
        )?;

        Ok(PaseoProviderConfig {
            extends: "claude".to_string(),
            label: profile.name.clone(),
            description: self.paseo_provider_description(profile, mode_description),
            command,
            env,
            models: models.models,
            additional_models: models.additional_models,
            enabled: true,
            order,
        })
    }

    fn paseo_provider_description(&self, profile: &Profile, mode_description: &str) -> String {
        let kind = match profile.kind {
            ProfileKind::Lightweight => "lightweight",
            ProfileKind::Full => "full",
        };
        if let Some(alias) = &profile.alias {
            format!(
                "claude-switch {kind} profile '{}' ({alias}); {mode_description}",
                profile.name
            )
        } else {
            format!(
                "claude-switch {kind} profile '{}'; {mode_description}",
                profile.name
            )
        }
    }

    fn export_credential_context(
        &self,
        profile: &Profile,
        provider_id: &str,
        warnings: &mut Vec<PaseoExportWarning>,
    ) -> Result<ExportCredentialContext> {
        match profile.kind {
            ProfileKind::Lightweight => {
                let (auth_token, base_url) = self.resolve_credentials(profile)?;
                Ok(ExportCredentialContext {
                    auth_token,
                    base_url,
                    env: profile.env.clone(),
                })
            }
            ProfileKind::Full => {
                let settings_path = self.profile_dir(profile).join("settings.json");
                if !settings_path.is_file() {
                    warnings.push(PaseoExportWarning::model_discovery(
                        profile,
                        provider_id,
                        format!(
                            "Full profile '{}' has no settings.json; models will be omitted unless Claude discovers them at runtime.",
                            profile.name
                        ),
                    ));
                    return Ok(ExportCredentialContext::default());
                }
                let content = fs::read_to_string(&settings_path).with_context(|| {
                    format!(
                        "Failed to read full profile settings from {}",
                        settings_path.display()
                    )
                })?;
                let value: Value = serde_json::from_str(&content).with_context(|| {
                    format!(
                        "Failed to parse full profile settings JSON from {}",
                        settings_path.display()
                    )
                })?;
                let object = value
                    .as_object()
                    .context("Full profile settings root must be a JSON object")?;
                let env = object.get("env").and_then(Value::as_object);
                Ok(ExportCredentialContext {
                    auth_token: Self::auth_token_from_settings(object)?,
                    base_url: env
                        .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    env: Some(LightweightEnv {
                        auth_token: None,
                        base_url: env
                            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        default_opus_model: env
                            .and_then(|env| env.get("ANTHROPIC_DEFAULT_OPUS_MODEL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        default_sonnet_model: env
                            .and_then(|env| env.get("ANTHROPIC_DEFAULT_SONNET_MODEL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        default_haiku_model: env
                            .and_then(|env| env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        model: env
                            .and_then(|env| env.get("ANTHROPIC_MODEL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        subagent_model: env
                            .and_then(|env| env.get("CLAUDE_CODE_SUBAGENT_MODEL"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        extras: Vec::new(),
                    }),
                })
            }
        }
    }

    fn paseo_command_and_env(
        &self,
        profile: &Profile,
        provider_id: &str,
        credentials: &ExportCredentialContext,
        options: &PaseoExportOptions,
        warnings: &mut Vec<PaseoExportWarning>,
    ) -> Result<PaseoCommandExport> {
        if options.secret_mode == PaseoSecretMode::SelfContained {
            match profile.kind {
                ProfileKind::Lightweight => {
                    if let Some((command, mode_description)) = self
                        .try_self_contained_lightweight_paseo_command(
                            profile,
                            provider_id,
                            credentials,
                            options,
                            warnings,
                        )?
                    {
                        return Ok((command, None, mode_description));
                    }
                }
                ProfileKind::Full => {
                    let mut env = HashMap::new();
                    env.insert(
                        "CLAUDE_CONFIG_DIR".to_string(),
                        self.profile_dir(profile).display().to_string(),
                    );
                    let mut command = vec!["claude".to_string()];
                    self.append_profile_launch_args(
                        &mut command,
                        profile,
                        options.include_stored_launch_args,
                    );
                    return Ok((command, Some(env), "self-contained"));
                }
            }
        }

        let mut command = vec!["cswitch".to_string(), "use".to_string()];
        if !options.include_stored_launch_args {
            command.push("--no-extras".to_string());
        }
        command.push(profile.id.clone());
        command.push("--".to_string());
        Ok((command, None, "wrapper"))
    }

    fn try_self_contained_lightweight_paseo_command(
        &self,
        profile: &Profile,
        provider_id: &str,
        credentials: &ExportCredentialContext,
        options: &PaseoExportOptions,
        warnings: &mut Vec<PaseoExportWarning>,
    ) -> Result<Option<(Vec<String>, &'static str)>> {
        let Some(env) = profile.env.as_ref() else {
            warnings.push(PaseoExportWarning::secret_fallback(
                profile,
                provider_id,
                format!(
                    "Lightweight profile '{}' has no env block; falling back to cswitch wrapper.",
                    profile.name
                ),
            ));
            return Ok(None);
        };

        let tool_shell = self.native_runtime_tool_shell();
        let mut command = vec!["claude".to_string()];
        self.append_profile_launch_args(&mut command, profile, options.include_stored_launch_args);
        for root in self.profile_plugin_dirs(profile)? {
            command.push("--plugin-dir".to_string());
            command.push(root.display().to_string());
        }

        let token = credentials.auth_token.as_deref();
        let url = credentials.base_url.as_deref();
        if credentials
            .base_url
            .as_deref()
            .is_some_and(is_local_runtime_base_url)
        {
            let settings = build_lightweight_settings(env, token, url, false, tool_shell, None)?;
            command.push("--settings".to_string());
            command.push(serde_json::to_string(&settings)?);
            let mcp_servers = self.profile_mcp_servers(profile)?;
            if !mcp_servers.is_empty() {
                let plugin_root = self.upsert_local_profile_mcp_plugin(profile, &mcp_servers)?;
                command.push("--plugin-dir".to_string());
                command.push(plugin_root.display().to_string());
            }
            return Ok(Some((command, "self-contained")));
        }

        let artifacts = build_lightweight_runtime_artifacts(env, token, url, tool_shell)?;
        let mut settings = build_lightweight_settings(env, token, url, false, tool_shell, None)?;
        if artifacts.tinyfish_enabled {
            if !tinyfish_available() {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' needs TinyFish for self-contained export, but tinyfish is unavailable; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            }
            let Some(plugin_variant) = artifacts.tinyfish_plugin_variant else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish plugin metadata; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let Some(plugin_manifest_json) = artifacts.tinyfish_plugin_manifest_json.as_deref()
            else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish plugin manifest; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let Some(plugin_hooks_json) = artifacts.tinyfish_plugin_hooks_json.as_deref() else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish hook metadata; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let Some(output_style_text) = artifacts.tinyfish_output_style_text.as_deref() else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish output-style metadata; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let Some(hook_script_text) = artifacts.tinyfish_hook_script_text.as_deref() else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish hook script metadata; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let Some(statusline_script_text) = artifacts.tinyfish_statusline_script_text.as_deref()
            else {
                warnings.push(PaseoExportWarning::secret_fallback(
                    profile,
                    provider_id,
                    format!(
                        "Profile '{}' is missing TinyFish statusline metadata; falling back to cswitch wrapper.",
                        profile.name
                    ),
                ));
                return Ok(None);
            };
            let plugin_root = self.upsert_local_tinyfish_artifacts(
                plugin_variant,
                tool_shell,
                plugin_manifest_json,
                plugin_hooks_json,
                output_style_text,
                hook_script_text,
                statusline_script_text,
            )?;
            let statusline_path = plugin_root
                .join("scripts")
                .join(tinyfish_statusline_script_file_name(tool_shell));
            settings = build_lightweight_settings(
                env,
                token,
                url,
                true,
                tool_shell,
                Some(&statusline_path.display().to_string()),
            )?;
            command.push("--plugin-dir".to_string());
            command.push(plugin_root.display().to_string());
        }

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            let plugin_root = self.upsert_local_profile_mcp_plugin(profile, &mcp_servers)?;
            command.push("--plugin-dir".to_string());
            command.push(plugin_root.display().to_string());
        }

        command.push("--settings".to_string());
        command.push(serde_json::to_string(&settings)?);
        Ok(Some((command, "self-contained")))
    }

    fn paseo_models_for_profile(
        &self,
        profile: &Profile,
        provider_id: &str,
        credentials: &ExportCredentialContext,
        options: &PaseoExportOptions,
        warnings: &mut Vec<PaseoExportWarning>,
    ) -> Result<ModelSelection> {
        if options.model_policy == PaseoModelPolicy::None {
            return Ok(ModelSelection {
                models: None,
                additional_models: None,
            });
        }

        let env = credentials.env.as_ref();
        let static_models = env.map_or_else(Vec::new, Self::fallback_model_ids);
        let default_model = env.and_then(Self::primary_model_id);

        let discovered_models = match (
            credentials.base_url.as_deref(),
            credentials.auth_token.as_deref(),
        ) {
            (Some(base_url), Some(auth_token))
                if !base_url.trim().is_empty() && !auth_token.trim().is_empty() =>
            {
                match discover_models(base_url, auth_token) {
                    Ok(discovery) => Some(discovery.models),
                    Err(failure) => {
                        let tried = if failure.tried_endpoints.is_empty() {
                            String::new()
                        } else {
                            format!(" Tried: {}.", failure.tried_endpoints.join(" -> "))
                        };
                        warnings.push(PaseoExportWarning::model_discovery(
                            profile,
                            provider_id,
                            format!(
                                "Model discovery failed for profile '{}': {}. Falling back to static model fields if present.{}",
                                profile.name, failure.message, tried
                            ),
                        ));
                        None
                    }
                }
            }
            _ => {
                if static_models.is_empty() {
                    warnings.push(PaseoExportWarning::model_discovery(
                        profile,
                        provider_id,
                        format!(
                            "Profile '{}' has no discoverable token/base URL and no static model fields; omitting Paseo models.",
                            profile.name
                        ),
                    ));
                } else {
                    warnings.push(PaseoExportWarning::model_discovery(
                        profile,
                        provider_id,
                        format!(
                            "Profile '{}' has no discoverable token/base URL; using static model fields only.",
                            profile.name
                        ),
                    ));
                }
                None
            }
        };

        let model_ids = discovered_models.unwrap_or(static_models);
        if model_ids.is_empty() {
            return Ok(ModelSelection {
                models: None,
                additional_models: None,
            });
        }

        let use_additional_models = credentials
            .base_url
            .as_deref()
            .is_some_and(|base_url| self.paseo_should_use_additional_models(base_url, env));
        let model_entries = Self::paseo_model_entries(&model_ids, default_model.as_deref());
        Ok(if use_additional_models {
            ModelSelection {
                models: None,
                additional_models: Some(model_entries),
            }
        } else {
            ModelSelection {
                models: Some(model_entries),
                additional_models: None,
            }
        })
    }

    fn paseo_should_use_additional_models(
        &self,
        base_url: &str,
        env: Option<&LightweightEnv>,
    ) -> bool {
        if !url_matches(base_url, PASEO_ADDITIONAL_MODEL_URLS) {
            return false;
        }
        match env.and_then(Self::main_model_family) {
            Some(ModelFamily::NonClaude) => false,
            Some(ModelFamily::Claude) | Some(ModelFamily::Unknown) | None => true,
        }
    }

    fn paseo_model_entries(
        model_ids: &[String],
        default_model: Option<&str>,
    ) -> Vec<PaseoProviderModel> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for model_id in model_ids {
            let Some(normalized) = Self::normalize_model_id(Some(model_id.as_str())) else {
                continue;
            };
            if !seen.insert(normalized.clone()) {
                continue;
            }
            entries.push(PaseoProviderModel {
                label: normalized.clone(),
                is_default: default_model
                    .is_some_and(|default_model| default_model == normalized)
                    .then_some(true),
                id: normalized,
            });
        }
        entries
    }

    fn fallback_model_ids(env: &LightweightEnv) -> Vec<String> {
        let mut models = Vec::new();
        Self::push_model_id(&mut models, env.model.as_deref());
        Self::push_model_id(&mut models, env.default_sonnet_model.as_deref());
        Self::push_model_id(&mut models, env.default_opus_model.as_deref());
        Self::push_model_id(&mut models, env.default_haiku_model.as_deref());
        Self::push_model_id(&mut models, env.subagent_model.as_deref());
        models
    }

    fn primary_model_id(env: &LightweightEnv) -> Option<String> {
        [
            env.model.as_deref(),
            env.default_sonnet_model.as_deref(),
            env.default_opus_model.as_deref(),
            env.default_haiku_model.as_deref(),
            env.subagent_model.as_deref(),
        ]
        .into_iter()
        .find_map(Self::normalize_model_id)
    }

    fn main_model_family(env: &LightweightEnv) -> Option<ModelFamily> {
        [
            env.model.as_deref(),
            env.default_sonnet_model.as_deref(),
            env.default_opus_model.as_deref(),
            env.default_haiku_model.as_deref(),
            env.subagent_model.as_deref(),
        ]
        .into_iter()
        .find_map(|model| Self::normalize_model_id(model).map(|model| Self::model_family(&model)))
    }

    fn model_family(model: &str) -> ModelFamily {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.starts_with("claude") {
            ModelFamily::Claude
        } else if normalized.is_empty() {
            ModelFamily::Unknown
        } else {
            ModelFamily::NonClaude
        }
    }

    fn push_model_id(models: &mut Vec<String>, model: Option<&str>) {
        let Some(model_id) = Self::normalize_model_id(model) else {
            return;
        };
        if !models.iter().any(|existing| existing == &model_id) {
            models.push(model_id);
        }
    }

    fn normalize_model_id(model: Option<&str>) -> Option<String> {
        let model = model?;
        let trimmed = model.trim();
        let normalized = trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim_end();
        (!normalized.is_empty()).then(|| normalized.to_string())
    }

    fn append_profile_launch_args(
        &self,
        command: &mut Vec<String>,
        profile: &Profile,
        include_stored_launch_args: bool,
    ) {
        if !include_stored_launch_args {
            return;
        }
        if let Some(args) = &profile.launch_args {
            command.extend(args.iter().cloned());
        }
    }

    fn paseo_provider_id_for_profile(profile: &Profile, used: &mut HashSet<String>) -> String {
        let fallback_suffix = profile
            .id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>();
        let mut base = profile
            .alias
            .as_deref()
            .or(Some(profile.name.as_str()))
            .map(Self::slugify_paseo_provider_component)
            .filter(|value| !value.is_empty())
            .map(|value| format!("{PASEO_PROVIDER_PREFIX}{value}"))
            .unwrap_or_else(|| format!("{PASEO_PROVIDER_FALLBACK_PREFIX}{fallback_suffix}"));
        if used.insert(base.clone()) {
            return base;
        }
        let root = base.clone();
        for index in 2.. {
            base = format!("{root}-{index}");
            if used.insert(base.clone()) {
                return base;
            }
        }
        unreachable!("provider id generation should always find a free suffix")
    }

    fn slugify_paseo_provider_component(raw: &str) -> String {
        let mut slug = String::new();
        let mut last_was_dash = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                last_was_dash = false;
            } else if !last_was_dash && !slug.is_empty() {
                slug.push('-');
                last_was_dash = true;
            }
        }
        slug.trim_matches('-').to_string()
    }
}
