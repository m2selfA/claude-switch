use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use uuid::Uuid;

use super::{Profile, ProfileKind, ProfileManager, Provider, ProviderKey};

impl ProfileManager {
    /// Set the provider_id for a profile.
    pub fn set_provider(&self, query: &str, provider_id: &str, key_id: &str) -> Result<()> {
        let (id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        let profile = registry
            .profiles
            .get(&id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        if profile.kind != ProfileKind::Lightweight {
            bail!("Providers can only be linked to lightweight profiles.");
        }
        let provider = registry
            .providers
            .get(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        if !provider.keys.contains_key(key_id) {
            bail!("Key '{}' not found in provider '{}'.", key_id, provider_id);
        }
        let profile = registry
            .profiles
            .get_mut(&id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        profile.provider_id = Some(provider_id.to_string());
        profile.key_id = Some(key_id.to_string());
        self.save_registry(&registry)
    }

    /// Remove the provider/key association from a profile.
    pub fn unset_provider(&self, query: &str) -> Result<()> {
        let (id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.provider_id = None;
            p.key_id = None;
        }
        self.save_registry(&registry)
    }

    /// Resolve credentials for a profile (provider lookup with inline fallback).
    pub fn resolve_credentials(
        &self,
        profile: &Profile,
    ) -> Result<(Option<String>, Option<String>)> {
        if let Some(ref provider_id) = profile.provider_id {
            let registry = self.load_registry()?;
            let provider = registry.providers.get(provider_id).with_context(|| {
                format!(
                    "Profile '{}' references missing provider '{}'.",
                    profile.name, provider_id
                )
            })?;
            let key_id = profile.key_id.as_ref().with_context(|| {
                format!(
                    "Profile '{}' is linked to provider '{}' but has no key_id.",
                    profile.name, provider_id
                )
            })?;
            let key = provider.keys.get(key_id).with_context(|| {
                format!(
                    "Profile '{}' references missing key '{}' in provider '{}'.",
                    profile.name, key_id, provider_id
                )
            })?;
            return Ok((Some(key.api_key.clone()), Some(provider.base_url.clone())));
        }
        if let Some(ref env) = profile.env {
            return Ok((env.auth_token.clone(), env.base_url.clone()));
        }
        Ok((None, None))
    }

    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let registry = self.load_registry()?;
        let mut providers: Vec<Provider> = registry.providers.into_values().collect();
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(providers)
    }

    pub fn get_provider(&self, id: &str) -> Result<Provider> {
        let registry = self.load_registry()?;
        registry
            .providers
            .get(id)
            .cloned()
            .with_context(|| format!("Provider '{}' not found.", id))
    }

    /// Add a provider with an initial key.
    pub fn add_provider(&self, name: &str, base_url: &str, api_key: &str) -> Result<Provider> {
        self.add_provider_with_key_name(name, base_url, "Default", api_key)
    }

    /// Add a provider with a named initial key.
    pub fn add_provider_with_key_name(
        &self,
        name: &str,
        base_url: &str,
        key_name: &str,
        api_key: &str,
    ) -> Result<Provider> {
        let name = name.trim();
        let base_url = base_url.trim();
        let key_name = key_name.trim();
        let api_key = api_key.trim();
        if name.is_empty() {
            bail!("Provider name cannot be empty.");
        }
        if base_url.is_empty() {
            bail!("Provider base URL cannot be empty.");
        }
        if key_name.is_empty() {
            bail!("Key name cannot be empty.");
        }
        if api_key.is_empty() {
            bail!("Provider API key cannot be empty.");
        }
        let pid = format!("prov_{}", &Uuid::new_v4().to_string()[..8]);
        let kid = format!("key_{}", &Uuid::new_v4().to_string()[..8]);
        let mut keys = HashMap::new();
        keys.insert(
            kid.clone(),
            ProviderKey {
                id: kid,
                name: key_name.to_string(),
                api_key: api_key.to_string(),
            },
        );
        let provider = Provider {
            id: pid.clone(),
            name: name.to_string(),
            base_url: base_url.to_string(),
            keys,
            api_key: String::new(),
        };
        let mut registry = self.load_registry()?;
        registry.providers.insert(pid, provider.clone());
        self.save_registry(&registry)?;
        Ok(provider)
    }

    pub fn update_provider(&self, id: &str, name: &str, base_url: &str) -> Result<Provider> {
        let name = name.trim();
        let base_url = base_url.trim();
        if name.is_empty() {
            bail!("Provider name cannot be empty.");
        }
        if base_url.is_empty() {
            bail!("Provider base URL cannot be empty.");
        }
        let mut registry = self.load_registry()?;
        let provider = registry
            .providers
            .get_mut(id)
            .with_context(|| format!("Provider '{}' not found.", id))?;
        provider.name = name.to_string();
        provider.base_url = base_url.to_string();
        let p = provider.clone();
        self.save_registry(&registry)?;
        Ok(p)
    }

    pub fn remove_provider(&self, id: &str) -> Result<()> {
        let registry = self.load_registry()?;
        if !registry.providers.contains_key(id) {
            bail!("Provider '{}' not found.", id);
        }
        let refs: Vec<_> = registry
            .profiles
            .values()
            .filter(|p| p.provider_id.as_deref() == Some(id))
            .map(|p| p.name.clone())
            .collect();
        if !refs.is_empty() {
            bail!(
                "Provider '{}' is used by profiles: {}. Remove those profiles first.",
                id,
                refs.join(", ")
            );
        }
        let mut registry = self.load_registry()?;
        registry.providers.remove(id);
        self.save_registry(&registry)
    }

    pub fn list_keys(&self, provider_id: &str) -> Result<Vec<ProviderKey>> {
        let registry = self.load_registry()?;
        let prov = registry
            .providers
            .get(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        let mut keys: Vec<ProviderKey> = prov.keys.values().cloned().collect();
        keys.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(keys)
    }

    pub fn add_key(&self, provider_id: &str, name: &str, api_key: &str) -> Result<ProviderKey> {
        let name = name.trim();
        let api_key = api_key.trim();
        if name.is_empty() {
            bail!("Key name cannot be empty.");
        }
        if api_key.is_empty() {
            bail!("API key cannot be empty.");
        }
        let kid = format!("key_{}", &Uuid::new_v4().to_string()[..8]);
        let key = ProviderKey {
            id: kid.clone(),
            name: name.to_string(),
            api_key: api_key.to_string(),
        };
        let mut registry = self.load_registry()?;
        let prov = registry
            .providers
            .get_mut(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        prov.keys.insert(kid, key.clone());
        self.save_registry(&registry)?;
        Ok(key)
    }

    pub fn remove_key(&self, provider_id: &str, key_id: &str) -> Result<()> {
        let registry = self.load_registry()?;
        let prov = registry
            .providers
            .get(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        if !prov.keys.contains_key(key_id) {
            bail!("Key '{}' not found.", key_id);
        }
        let refs: Vec<_> = self
            .list_profiles_using_key(provider_id, key_id)?
            .into_iter()
            .map(|p| p.name)
            .collect();
        if !refs.is_empty() {
            bail!(
                "Key '{}' is used by profiles: {}. Remove those profiles first.",
                key_id,
                refs.join(", ")
            );
        }
        let mut registry = self.load_registry()?;
        let prov = registry
            .providers
            .get_mut(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        prov.keys.remove(key_id);
        self.save_registry(&registry)
    }

    pub fn list_profiles_using_key(&self, provider_id: &str, key_id: &str) -> Result<Vec<Profile>> {
        let registry = self.load_registry()?;
        let provider = registry
            .providers
            .get(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        if !provider.keys.contains_key(key_id) {
            bail!("Key '{}' not found.", key_id);
        }
        let mut profiles: Vec<Profile> = registry
            .profiles
            .values()
            .filter(|p| {
                p.provider_id.as_deref() == Some(provider_id) && p.key_id.as_deref() == Some(key_id)
            })
            .cloned()
            .collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub fn update_key(
        &self,
        provider_id: &str,
        key_id: &str,
        name: &str,
        api_key: &str,
    ) -> Result<ProviderKey> {
        let name = name.trim();
        let api_key = api_key.trim();
        if name.is_empty() {
            bail!("Key name cannot be empty.");
        }
        if api_key.is_empty() {
            bail!("API key cannot be empty.");
        }
        let mut registry = self.load_registry()?;
        let prov = registry
            .providers
            .get_mut(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        let key = prov
            .keys
            .get_mut(key_id)
            .with_context(|| format!("Key '{}' not found.", key_id))?;
        key.name = name.to_string();
        key.api_key = api_key.to_string();
        let k = key.clone();
        self.save_registry(&registry)?;
        Ok(k)
    }

    pub fn rename_key(&self, provider_id: &str, key_id: &str, name: &str) -> Result<ProviderKey> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Key name cannot be empty.");
        }
        let mut registry = self.load_registry()?;
        let prov = registry
            .providers
            .get_mut(provider_id)
            .with_context(|| format!("Provider '{}' not found.", provider_id))?;
        let key = prov
            .keys
            .get_mut(key_id)
            .with_context(|| format!("Key '{}' not found.", key_id))?;
        key.name = name.to_string();
        let k = key.clone();
        self.save_registry(&registry)?;
        Ok(k)
    }

    pub fn find_provider_by_url_and_key(
        &self,
        base_url: &str,
        api_key: &str,
    ) -> Option<(Provider, ProviderKey)> {
        let registry = self.load_registry().ok()?;
        for prov in registry.providers.values() {
            if prov.base_url == base_url {
                for key in prov.keys.values() {
                    if key.api_key == api_key {
                        return Some((prov.clone(), key.clone()));
                    }
                }
            }
        }
        None
    }
}
