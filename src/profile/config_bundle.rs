use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::{HashMap, HashSet};

use super::{
    ConfigBundle, ConfigBundleValidation, ConfigImportPlan, ConfigImportSummary, DiagnosticLevel,
    McpServer, Profile, ProfileKind, ProfileManager, Provider, Registry,
};

impl ProfileManager {
    pub fn export_config_bundle(
        &self,
        profile_queries: &[String],
        include_secrets: bool,
    ) -> Result<String> {
        let registry = self.load_registry()?;
        let mut profiles: Vec<Profile> = if profile_queries.is_empty() {
            registry.profiles.values().cloned().collect()
        } else {
            let mut selected = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for query in profile_queries {
                let (id, profile) = Self::find_profile_in_registry(&registry, query)?;
                if seen.insert(id) {
                    selected.push(profile);
                }
            }
            selected
        };
        let selected_provider_keys: HashMap<String, HashSet<String>> =
            Self::selected_provider_keys_for_profiles(&profiles);
        let selected_provider_ids: HashSet<String> =
            selected_provider_keys.keys().cloned().collect();
        let selected_mcp_ids: std::collections::HashSet<String> = profiles
            .iter()
            .flat_map(|profile| profile.mcp_server_ids.iter().cloned())
            .collect();
        profiles.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        let mut providers: Vec<Provider> = if profile_queries.is_empty() {
            registry.providers.values().cloned().collect()
        } else {
            registry
                .providers
                .values()
                .filter(|provider| selected_provider_ids.contains(&provider.id))
                .cloned()
                .map(|mut provider| {
                    if let Some(key_ids) = selected_provider_keys.get(&provider.id) {
                        provider.keys.retain(|key_id, _| key_ids.contains(key_id));
                    }
                    provider
                })
                .collect()
        };
        providers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        let mut mcp_servers: Vec<McpServer> = if profile_queries.is_empty() {
            registry.mcp_servers.values().cloned().collect()
        } else {
            registry
                .mcp_servers
                .values()
                .filter(|server| selected_mcp_ids.contains(&server.id))
                .cloned()
                .collect()
        };
        mcp_servers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));

        if !include_secrets {
            for profile in &mut profiles {
                if let Some(env) = &mut profile.env {
                    env.auth_token = None;
                }
            }
            for provider in &mut providers {
                for key in provider.keys.values_mut() {
                    key.api_key.clear();
                }
            }
            for server in &mut mcp_servers {
                Self::redact_mcp_server_secrets(server);
            }
        }

        let bundle = ConfigBundle {
            schema: "https://github.com/m2selfA/claude-switch/config-bundle/v1".to_string(),
            exported_at: Utc::now(),
            profiles,
            providers,
            mcp_servers,
            settings: Some(registry.settings),
            secrets_included: include_secrets,
        };
        serde_json::to_string_pretty(&bundle).context("Failed to serialize config bundle")
    }

    pub fn validate_config_bundle(&self, content: &str) -> Result<ConfigBundleValidation> {
        let bundle: ConfigBundle =
            serde_json::from_str(content).context("Failed to parse config bundle JSON")?;
        let mut validation = ConfigBundleValidation {
            schema: bundle.schema.clone(),
            profiles: bundle.profiles.len(),
            providers: bundle.providers.len(),
            mcp_servers: bundle.mcp_servers.len(),
            secrets_included: bundle.secrets_included,
            issues: Vec::new(),
        };

        if bundle.schema != "https://github.com/m2selfA/claude-switch/config-bundle/v1" {
            validation.issues.push(Self::diagnostic(
                DiagnosticLevel::Error,
                "schema",
                format!("unsupported schema '{}'", bundle.schema),
                Some("export a fresh bundle with this cswitch version".to_string()),
            ));
        }

        let mut provider_ids = std::collections::HashSet::new();
        for provider in &bundle.providers {
            if provider.id.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("provider '{}' has an empty id", provider.name),
                    None,
                ));
            } else if !provider_ids.insert(provider.id.clone()) {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("duplicate provider id '{}'", provider.id),
                    None,
                ));
            }
            if provider.name.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("provider '{}' has an empty name", provider.id),
                    None,
                ));
            }
            if provider.base_url.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("provider '{}' has an empty base URL", provider.name),
                    None,
                ));
            }
            if !bundle.secrets_included && provider.keys.values().any(|key| !key.api_key.is_empty())
            {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Warn,
                    "providers",
                    format!(
                        "provider '{}' contains keys despite secrets_included=false",
                        provider.name
                    ),
                    Some(
                        "re-export with the current cswitch version to enforce redaction"
                            .to_string(),
                    ),
                ));
            }
        }

        let mut mcp_ids = std::collections::HashSet::new();
        for server in &bundle.mcp_servers {
            if server.id.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "mcp",
                    format!("MCP '{}' has an empty id", server.name),
                    None,
                ));
            } else if !mcp_ids.insert(server.id.clone()) {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "mcp",
                    format!("duplicate MCP id '{}'", server.id),
                    None,
                ));
            }
            validation
                .issues
                .extend(
                    Self::validate_mcp_server_config(server)
                        .into_iter()
                        .map(|issue| {
                            Self::diagnostic(
                                issue.level,
                                "mcp",
                                format!("{}: {}", issue.server_name, issue.message),
                                issue.hint,
                            )
                        }),
                );
            if !bundle.secrets_included && Self::mcp_server_has_secrets(server) {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Warn,
                    "mcp",
                    format!(
                        "MCP '{}' contains secrets despite secrets_included=false",
                        server.name
                    ),
                    Some(
                        "re-export with the current cswitch version to enforce redaction"
                            .to_string(),
                    ),
                ));
            }
        }

        let mut profile_ids = std::collections::HashSet::new();
        let mcp_id_set: std::collections::HashSet<String> = bundle
            .mcp_servers
            .iter()
            .map(|server| server.id.clone())
            .collect();
        for profile in &bundle.profiles {
            if profile.id.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("profile '{}' has an empty id", profile.name),
                    None,
                ));
            } else if !profile_ids.insert(profile.id.clone()) {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("duplicate profile id '{}'", profile.id),
                    None,
                ));
            }
            if profile.name.trim().is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("profile '{}' has an empty name", profile.id),
                    None,
                ));
            }
            if let Some(alias) = &profile.alias
                && let Err(err) = Self::validate_alias(alias)
            {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("profile '{}' alias is invalid: {err}", profile.name),
                    None,
                ));
            }
            if profile.kind == ProfileKind::Lightweight && profile.env.is_none() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("lightweight profile '{}' has no env block", profile.name),
                    None,
                ));
            }
            if profile.kind != ProfileKind::Lightweight && !profile.mcp_server_ids.is_empty() {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("full profile '{}' has MCP links", profile.name),
                    Some("MCP links are only supported for lightweight profiles".to_string()),
                ));
            }
            if let Some(provider_id) = &profile.provider_id
                && !provider_ids.contains(provider_id)
            {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!(
                        "profile '{}' references missing provider '{}'",
                        profile.name, provider_id
                    ),
                    None,
                ));
            }
            for mcp_id in &profile.mcp_server_ids {
                if !mcp_id_set.contains(mcp_id) {
                    validation.issues.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "profiles",
                        format!(
                            "profile '{}' references missing MCP '{}'",
                            profile.name, mcp_id
                        ),
                        None,
                    ));
                }
            }
            if !bundle.secrets_included
                && profile
                    .env
                    .as_ref()
                    .and_then(|env| env.auth_token.as_ref())
                    .is_some()
            {
                validation.issues.push(Self::diagnostic(
                    DiagnosticLevel::Warn,
                    "profiles",
                    format!(
                        "profile '{}' contains an auth token despite secrets_included=false",
                        profile.name
                    ),
                    Some(
                        "re-export with the current cswitch version to enforce redaction"
                            .to_string(),
                    ),
                ));
            }
        }

        Ok(validation)
    }

    pub fn plan_config_bundle_import(
        &self,
        content: &str,
        replace: bool,
    ) -> Result<ConfigImportPlan> {
        let bundle: ConfigBundle =
            serde_json::from_str(content).context("Failed to parse config bundle JSON")?;
        if bundle.schema != "https://github.com/m2selfA/claude-switch/config-bundle/v1" {
            bail!("Unsupported config bundle schema '{}'.", bundle.schema);
        }
        let registry = self.load_registry()?;
        Self::validate_bundle_references_after_import(&registry, &bundle, replace)?;
        let mut plan = ConfigImportPlan {
            summary: ConfigImportSummary {
                profiles_added: 0,
                profiles_updated: 0,
                profiles_conflicted: 0,
                providers_added: 0,
                providers_updated: 0,
                providers_conflicted: 0,
                mcp_servers_added: 0,
                mcp_servers_updated: 0,
                mcp_servers_conflicted: 0,
            },
            profiles_add: Vec::new(),
            profiles_update: Vec::new(),
            profiles_conflict: Vec::new(),
            providers_add: Vec::new(),
            providers_update: Vec::new(),
            providers_conflict: Vec::new(),
            mcp_servers_add: Vec::new(),
            mcp_servers_update: Vec::new(),
            mcp_servers_conflict: Vec::new(),
            secrets_included: bundle.secrets_included,
        };

        for provider in &bundle.providers {
            if provider.id.trim().is_empty() {
                bail!("Imported provider '{}' has an empty id.", provider.name);
            }
            if registry.providers.contains_key(&provider.id) {
                if !replace {
                    plan.summary.providers_conflicted += 1;
                    plan.providers_conflict
                        .push(format!("{} ({})", provider.name, provider.id));
                    continue;
                }
                plan.summary.providers_updated += 1;
                plan.providers_update
                    .push(format!("{} ({})", provider.name, provider.id));
            } else {
                plan.summary.providers_added += 1;
                plan.providers_add
                    .push(format!("{} ({})", provider.name, provider.id));
            }
        }

        for server in &bundle.mcp_servers {
            if server.id.trim().is_empty() {
                bail!("Imported MCP '{}' has an empty id.", server.name);
            }
            Self::normalize_mcp_server_type(&server.server_type)?;
            if registry.mcp_servers.contains_key(&server.id) {
                if !replace {
                    plan.summary.mcp_servers_conflicted += 1;
                    plan.mcp_servers_conflict
                        .push(format!("{} ({})", server.name, server.id));
                    continue;
                }
                Self::check_mcp_name_unique_in_registry(&registry, &server.id, &server.name)?;
                plan.summary.mcp_servers_updated += 1;
                plan.mcp_servers_update
                    .push(format!("{} ({})", server.name, server.id));
            } else {
                Self::check_mcp_name_unique_in_registry(&registry, "", &server.name)?;
                plan.summary.mcp_servers_added += 1;
                plan.mcp_servers_add
                    .push(format!("{} ({})", server.name, server.id));
            }
        }

        for profile in &bundle.profiles {
            if profile.id.trim().is_empty() {
                bail!("Imported profile '{}' has an empty id.", profile.name);
            }
            if profile.name.trim().is_empty() {
                bail!("Imported profile '{}' has an empty name.", profile.id);
            }
            if registry.profiles.contains_key(&profile.id) {
                if !replace {
                    plan.summary.profiles_conflicted += 1;
                    plan.profiles_conflict
                        .push(format!("{} ({})", profile.name, profile.id));
                    continue;
                }
                Self::check_profile_unique_in_registry(
                    &registry,
                    &profile.id,
                    &profile.name,
                    profile.alias.as_deref(),
                )?;
                plan.summary.profiles_updated += 1;
                plan.profiles_update
                    .push(format!("{} ({})", profile.name, profile.id));
            } else {
                Self::check_profile_unique_in_registry(
                    &registry,
                    "",
                    &profile.name,
                    profile.alias.as_deref(),
                )?;
                plan.summary.profiles_added += 1;
                plan.profiles_add
                    .push(format!("{} ({})", profile.name, profile.id));
            }
        }

        Ok(plan)
    }

    pub fn import_config_bundle(
        &self,
        content: &str,
        replace: bool,
    ) -> Result<ConfigImportSummary> {
        let plan = self.plan_config_bundle_import(content, replace)?;
        if plan.conflict_count() > 0 {
            bail!(
                "Config bundle has {} existing entrie(s). Use --replace to update them.",
                plan.conflict_count()
            );
        }
        let bundle: ConfigBundle =
            serde_json::from_str(content).context("Failed to parse config bundle JSON")?;
        let bundle_settings = bundle.settings.clone();
        let mut registry = self.load_registry()?;
        if let Some(settings) = bundle_settings {
            registry.settings = settings;
        }
        let mut summary = ConfigImportSummary {
            profiles_added: 0,
            profiles_updated: 0,
            profiles_conflicted: 0,
            providers_added: 0,
            providers_updated: 0,
            providers_conflicted: 0,
            mcp_servers_added: 0,
            mcp_servers_updated: 0,
            mcp_servers_conflicted: 0,
        };

        for mut provider in bundle.providers {
            if provider.id.trim().is_empty() {
                bail!("Imported provider '{}' has an empty id.", provider.name);
            }
            if registry.providers.contains_key(&provider.id) {
                if !replace {
                    bail!(
                        "Provider '{}' already exists. Use --replace to update it.",
                        provider.id
                    );
                }
                if !bundle.secrets_included
                    && let Some(existing) = registry.providers.get(&provider.id)
                {
                    for (key_id, key) in &mut provider.keys {
                        if key.api_key.is_empty()
                            && let Some(existing_key) = existing.keys.get(key_id)
                        {
                            key.api_key = existing_key.api_key.clone();
                        }
                    }
                }
                registry.providers.insert(provider.id.clone(), provider);
                summary.providers_updated += 1;
            } else {
                registry.providers.insert(provider.id.clone(), provider);
                summary.providers_added += 1;
            }
        }

        for mut server in bundle.mcp_servers {
            if server.id.trim().is_empty() {
                bail!("Imported MCP '{}' has an empty id.", server.name);
            }
            Self::normalize_mcp_server_type(&server.server_type)?;
            if registry.mcp_servers.contains_key(&server.id) {
                if !replace {
                    bail!(
                        "MCP '{}' already exists. Use --replace to update it.",
                        server.id
                    );
                }
                Self::check_mcp_name_unique_in_registry(&registry, &server.id, &server.name)?;
                if !bundle.secrets_included
                    && let Some(existing) = registry.mcp_servers.get(&server.id)
                {
                    Self::preserve_mcp_server_secrets(&mut server, existing);
                }
                registry.mcp_servers.insert(server.id.clone(), server);
                summary.mcp_servers_updated += 1;
            } else {
                Self::check_mcp_name_unique_in_registry(&registry, "", &server.name)?;
                registry.mcp_servers.insert(server.id.clone(), server);
                summary.mcp_servers_added += 1;
            }
        }

        for mut profile in bundle.profiles {
            if profile.id.trim().is_empty() {
                bail!("Imported profile '{}' has an empty id.", profile.name);
            }
            if profile.name.trim().is_empty() {
                bail!("Imported profile '{}' has an empty name.", profile.id);
            }
            if registry.profiles.contains_key(&profile.id) {
                if !replace {
                    bail!(
                        "Profile '{}' already exists. Use --replace to update it.",
                        profile.id
                    );
                }
                Self::check_profile_unique_in_registry(
                    &registry,
                    &profile.id,
                    &profile.name,
                    profile.alias.as_deref(),
                )?;
                if !bundle.secrets_included
                    && let Some(existing) = registry.profiles.get(&profile.id)
                    && let (Some(incoming_env), Some(existing_env)) =
                        (&mut profile.env, &existing.env)
                    && incoming_env.auth_token.is_none()
                {
                    incoming_env.auth_token = existing_env.auth_token.clone();
                }
                registry.profiles.insert(profile.id.clone(), profile);
                summary.profiles_updated += 1;
            } else {
                Self::check_profile_unique_in_registry(
                    &registry,
                    "",
                    &profile.name,
                    profile.alias.as_deref(),
                )?;
                registry.profiles.insert(profile.id.clone(), profile);
                summary.profiles_added += 1;
            }
        }

        self.migrate_providers(&mut registry)?;
        self.save_registry(&registry)?;
        Ok(summary)
    }

    fn selected_provider_keys_for_profiles(
        profiles: &[Profile],
    ) -> HashMap<String, HashSet<String>> {
        let mut selected = HashMap::new();
        for profile in profiles {
            if let (Some(provider_id), Some(key_id)) = (&profile.provider_id, &profile.key_id) {
                selected
                    .entry(provider_id.clone())
                    .or_insert_with(HashSet::new)
                    .insert(key_id.clone());
            }
        }
        selected
    }

    fn validate_bundle_references_after_import(
        registry: &Registry,
        bundle: &ConfigBundle,
        replace: bool,
    ) -> Result<()> {
        let mut provider_keys: HashMap<String, HashSet<String>> = registry
            .providers
            .iter()
            .map(|(id, provider)| {
                (
                    id.clone(),
                    provider.keys.keys().cloned().collect::<HashSet<_>>(),
                )
            })
            .collect();
        for provider in &bundle.providers {
            let keys = provider.keys.keys().cloned().collect::<HashSet<_>>();
            if replace || !provider_keys.contains_key(&provider.id) {
                provider_keys.insert(provider.id.clone(), keys);
            }
        }

        let mut mcp_ids = registry.mcp_servers.keys().cloned().collect::<HashSet<_>>();
        for server in &bundle.mcp_servers {
            mcp_ids.insert(server.id.clone());
        }

        for profile in &bundle.profiles {
            if profile.kind != ProfileKind::Lightweight {
                if profile.provider_id.is_some() || profile.key_id.is_some() {
                    bail!(
                        "Imported full profile '{}' cannot reference a provider key.",
                        profile.name
                    );
                }
                continue;
            }
            match (&profile.provider_id, &profile.key_id) {
                (Some(provider_id), Some(key_id)) => {
                    let Some(keys) = provider_keys.get(provider_id) else {
                        bail!(
                            "Imported profile '{}' references missing provider '{}'.",
                            profile.name,
                            provider_id
                        );
                    };
                    if !keys.contains(key_id) {
                        bail!(
                            "Imported profile '{}' references missing key '{}' in provider '{}'.",
                            profile.name,
                            key_id,
                            provider_id
                        );
                    }
                }
                (Some(provider_id), None) => bail!(
                    "Imported profile '{}' references provider '{}' without a key_id.",
                    profile.name,
                    provider_id
                ),
                (None, Some(key_id)) => bail!(
                    "Imported profile '{}' references key '{}' without a provider_id.",
                    profile.name,
                    key_id
                ),
                (None, None) => {}
            }

            for mcp_id in &profile.mcp_server_ids {
                if !mcp_ids.contains(mcp_id) {
                    bail!(
                        "Imported profile '{}' references missing MCP '{}'.",
                        profile.name,
                        mcp_id
                    );
                }
            }
        }

        Ok(())
    }
}
