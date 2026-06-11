use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{CLAUDE_SWITCH_HOME_ENV, ProfileKind, ProfileManager, Provider, ProviderKey, Registry};

impl ProfileManager {
    pub(crate) fn global_settings(&self) -> Result<super::GlobalSettings> {
        Ok(self.load_registry()?.settings)
    }

    pub(crate) fn allow_local_runtime_hot_switch(&self) -> Result<bool> {
        Ok(self.global_settings()?.allow_local_runtime_hot_switch)
    }

    pub(crate) fn set_allow_local_runtime_hot_switch(&self, allowed: bool) -> Result<()> {
        let mut registry = self.load_registry()?;
        registry.settings.allow_local_runtime_hot_switch = allowed;
        self.save_registry(&registry)
    }

    pub(crate) fn set_plugin_github_mirror_base_url(&self, value: Option<String>) -> Result<()> {
        let mut registry = self.load_registry()?;
        registry.settings.plugin_github_mirror_base_url = value;
        self.save_registry(&registry)
    }

    pub fn new() -> Result<Self> {
        let home = Self::home_dir()?;
        Self::new_in_home_dir(&home)
    }

    pub(super) fn new_in_home_dir(home: &Path) -> Result<Self> {
        let base_dir = home.join(".claude-switch");
        Self::new_in_base_dir(&base_dir)
    }

    pub(super) fn new_in_base_dir(base_dir: &Path) -> Result<Self> {
        let profiles_dir = base_dir.join("profiles");
        let registry_path = base_dir.join("registry.json");
        fs::create_dir_all(&profiles_dir)?;
        Ok(Self {
            profiles_dir,
            registry_path,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(base_dir: &Path) -> Result<Self> {
        Self::new_in_base_dir(base_dir)
    }

    pub(super) fn home_dir() -> Result<PathBuf> {
        if let Some(value) = env::var_os(CLAUDE_SWITCH_HOME_ENV) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return Ok(path);
            }
        }
        dirs::home_dir().context("Cannot determine home directory")
    }

    pub fn load_registry(&self) -> Result<Registry> {
        if !self.registry_path.exists() {
            return Ok(Registry::default());
        }
        let content = fs::read_to_string(&self.registry_path)?;
        let mut registry: Registry = serde_json::from_str(&content)?;
        self.migrate_providers(&mut registry)?;
        Ok(registry)
    }

    fn new_key_id(provider: &Provider) -> String {
        loop {
            let id = format!("key_{}", &Uuid::new_v4().to_string()[..8]);
            if !provider.keys.contains_key(&id) {
                return id;
            }
        }
    }

    fn provider_key_exists(registry: &Registry, provider_id: &str, key_id: &str) -> bool {
        registry
            .providers
            .get(provider_id)
            .and_then(|p| p.keys.get(key_id))
            .is_some()
    }

    pub(super) fn migrate_providers(&self, registry: &mut Registry) -> Result<()> {
        let mut changed = false;
        for provider in registry.providers.values_mut() {
            if provider.api_key.is_empty() {
                continue;
            }
            let api_key = provider.api_key.clone();
            if !provider.keys.values().any(|key| key.api_key == api_key) {
                let key_id = Self::new_key_id(provider);
                let key_name = if provider.keys.is_empty() {
                    "Default".to_string()
                } else {
                    format!("Key {}", provider.keys.len() + 1)
                };
                provider.keys.insert(
                    key_id.clone(),
                    ProviderKey {
                        id: key_id,
                        name: key_name,
                        api_key,
                    },
                );
            }
            provider.api_key.clear();
            changed = true;
        }

        let profile_ids: Vec<String> = registry.profiles.keys().cloned().collect();
        for profile_id in profile_ids {
            let (kind, provider_id, key_id) = {
                let profile = registry
                    .profiles
                    .get(&profile_id)
                    .expect("profile id came from registry");
                (
                    profile.kind.clone(),
                    profile.provider_id.clone(),
                    profile.key_id.clone(),
                )
            };

            if kind != ProfileKind::Lightweight {
                if provider_id.is_some() || key_id.is_some() {
                    let profile = registry
                        .profiles
                        .get_mut(&profile_id)
                        .expect("profile id came from registry");
                    profile.provider_id = None;
                    profile.key_id = None;
                    profile.mcp_server_ids.clear();
                    changed = true;
                }
                if registry
                    .profiles
                    .get(&profile_id)
                    .is_some_and(|profile| !profile.mcp_server_ids.is_empty())
                {
                    let profile = registry
                        .profiles
                        .get_mut(&profile_id)
                        .expect("profile id came from registry");
                    profile.mcp_server_ids.clear();
                    changed = true;
                }
                continue;
            }

            if let (Some(provider_id), Some(key_id)) = (&provider_id, &key_id) {
                if Self::provider_key_exists(registry, provider_id, key_id) {
                    continue;
                }

                let profile = registry
                    .profiles
                    .get_mut(&profile_id)
                    .expect("profile id came from registry");
                profile.provider_id = None;
                profile.key_id = None;
                changed = true;
                continue;
            }

            if let Some(provider_id) = provider_id.as_deref()
                && key_id.is_none()
            {
                let only_key_id = registry.providers.get(provider_id).and_then(|provider| {
                    if provider.keys.len() == 1 {
                        provider.keys.keys().next().cloned()
                    } else {
                        None
                    }
                });
                if let Some(only_key_id) = only_key_id {
                    let profile = registry
                        .profiles
                        .get_mut(&profile_id)
                        .expect("profile id came from registry");
                    profile.key_id = Some(only_key_id);
                    changed = true;
                }
            }

            let known_mcp_ids = &registry.mcp_servers;
            if let Some(profile) = registry.profiles.get_mut(&profile_id) {
                let before = profile.mcp_server_ids.len();
                profile
                    .mcp_server_ids
                    .retain(|mcp_id| known_mcp_ids.contains_key(mcp_id));
                profile.mcp_server_ids.sort();
                profile.mcp_server_ids.dedup();
                changed |= before != profile.mcp_server_ids.len();
            }
        }

        if changed {
            self.save_registry(registry)?;
        }
        Ok(())
    }

    pub(super) fn save_registry(&self, registry: &Registry) -> Result<()> {
        let content = serde_json::to_string_pretty(registry)?;
        fs::write(&self.registry_path, content)?;
        Ok(())
    }
}
