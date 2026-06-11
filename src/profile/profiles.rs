use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{LightweightEnv, Profile, ProfileKind, ProfileManager, Registry, copy_dir_all};

impl ProfileManager {
    /// Update just the launch_args field for a profile.
    pub fn set_launch_args(&self, query: &str, args: Option<Vec<String>>) -> Result<()> {
        let (id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.launch_args = args;
        }
        self.save_registry(&registry)
    }

    // ── Lookup helpers ───────────────────────────────────────────────────────

    /// Find a profile by id, alias, or name (exact match, in that order).
    /// Returns `(id, profile)`.
    pub fn find_profile(&self, query: &str) -> Result<(String, Profile)> {
        let registry = self.load_registry()?;
        Self::find_profile_in_registry(&registry, query)
    }

    pub(super) fn find_profile_in_registry(
        registry: &Registry,
        query: &str,
    ) -> Result<(String, Profile)> {
        if query.is_empty() {
            bail!("Profile query is empty.");
        }

        // 1. Exact match on id
        if let Some(p) = registry.profiles.get(query) {
            return Ok((query.to_string(), p.clone()));
        }

        // 2. Exact match on alias
        let by_alias: Vec<_> = registry
            .profiles
            .iter()
            .filter(|(_, p)| p.alias.as_deref() == Some(query))
            .collect();
        if by_alias.len() == 1 {
            return Ok((by_alias[0].0.clone(), by_alias[0].1.clone()));
        } else if by_alias.len() > 1 {
            bail!(
                "Multiple profiles match alias '{}'. Use the full id to disambiguate.",
                query
            );
        }

        // 3. Exact match on name
        let by_name: Vec<_> = registry
            .profiles
            .iter()
            .filter(|(_, p)| p.name == query)
            .collect();
        if by_name.len() == 1 {
            return Ok((by_name[0].0.clone(), by_name[0].1.clone()));
        } else if by_name.len() > 1 {
            bail!(
                "Multiple profiles match name '{}'. Use an alias or id to disambiguate.",
                query
            );
        }

        bail!(
            "Profile '{}' not found. Add it with: cswitch add <name>",
            query
        )
    }

    /// Check that `name` and `alias` are not already in use by another profile.
    /// `exclude_id` — the profile being edited (don't check against itself).
    pub fn check_unique(&self, exclude_id: &str, name: &str, alias: Option<&str>) -> Result<()> {
        let registry = self.load_registry()?;
        Self::check_profile_unique_in_registry(&registry, exclude_id, name, alias)
    }

    pub(super) fn check_profile_unique_in_registry(
        registry: &Registry,
        exclude_id: &str,
        name: &str,
        alias: Option<&str>,
    ) -> Result<()> {
        for (id, p) in &registry.profiles {
            if id == exclude_id {
                continue;
            }
            if p.name == name {
                bail!("Profile name '{}' is already in use.", name);
            }
            if let Some(ref a) = p.alias
                && Some(a.as_str()) == alias
            {
                bail!("Alias '{}' is already in use.", a);
            }
        }
        Ok(())
    }

    pub(super) fn validate_alias(alias: &str) -> Result<()> {
        if !alias
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!(
                "Alias '{}' is invalid. Use only a-z, 0-9, hyphens, and underscores.",
                alias
            );
        }
        Ok(())
    }

    fn profile_name_in_use(registry: &Registry, name: &str) -> bool {
        registry
            .profiles
            .values()
            .any(|profile| profile.name == name)
    }

    fn profile_alias_in_use(registry: &Registry, alias: &str) -> bool {
        registry
            .profiles
            .values()
            .any(|profile| profile.alias.as_deref() == Some(alias))
    }

    fn derive_alias_from_name(name: &str) -> Option<String> {
        let mut alias = String::new();
        let mut last_was_sep = false;
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                alias.push(ch.to_ascii_lowercase());
                last_was_sep = false;
            } else if (ch == '-' || ch == '_' || ch.is_ascii_whitespace())
                && !alias.is_empty()
                && !last_was_sep
            {
                alias.push('-');
                last_was_sep = true;
            }
        }
        let alias = alias.trim_matches(['-', '_']).to_string();
        (!alias.is_empty()).then_some(alias)
    }

    fn suggest_duplicate_name_in_registry(registry: &Registry, source_name: &str) -> String {
        let base = format!("{source_name} (copy)");
        if !Self::profile_name_in_use(registry, &base) {
            return base;
        }

        let mut suffix = 2usize;
        loop {
            let candidate = format!("{source_name} (copy {suffix})");
            if !Self::profile_name_in_use(registry, &candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn suggest_duplicate_alias_in_registry(
        registry: &Registry,
        source_alias: Option<&str>,
        new_name: &str,
    ) -> Option<String> {
        let base = source_alias
            .map(str::to_string)
            .or_else(|| Self::derive_alias_from_name(new_name))?;
        if !Self::profile_alias_in_use(registry, &base) {
            return Some(base);
        }

        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !Self::profile_alias_in_use(registry, &candidate) {
                return Some(candidate);
            }
            suffix += 1;
        }
    }

    pub fn suggest_duplicate_name(&self, query: &str) -> Result<String> {
        let registry = self.load_registry()?;
        let (_, source) = Self::find_profile_in_registry(&registry, query)?;
        Ok(Self::suggest_duplicate_name_in_registry(
            &registry,
            &source.name,
        ))
    }

    pub fn suggest_duplicate_alias(&self, query: &str, new_name: &str) -> Result<Option<String>> {
        let registry = self.load_registry()?;
        let (_, source) = Self::find_profile_in_registry(&registry, query)?;
        Ok(Self::suggest_duplicate_alias_in_registry(
            &registry,
            source.alias.as_deref(),
            new_name,
        ))
    }

    // ── Public CRUD ──────────────────────────────────────────────────────────

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let registry = self.load_registry()?;
        let mut profiles: Vec<Profile> = registry.profiles.into_values().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    pub fn get_profile(&self, query: &str) -> Result<Profile> {
        self.find_profile(query).map(|(_, p)| p)
    }

    /// Full profile: copy `src` into `profiles/<dir_name>`.
    pub fn add_profile_from(&self, name: &str, alias: Option<&str>, src: &Path) -> Result<Profile> {
        if !src.exists() {
            bail!("Source directory '{}' does not exist.", src.display());
        }
        if name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }
        self.check_unique("", name, alias)?;
        if let Some(a) = alias {
            Self::validate_alias(a)?;
        }
        let id = Uuid::new_v4().to_string();
        let profile = self.copy_and_build_profile(&id, name, alias, src)?;
        self.upsert_profile(&profile)?;
        Ok(profile)
    }

    /// Force-add: overwrite any conflicting profile (same name or alias).
    pub fn add_profile_from_force(
        &self,
        name: &str,
        alias: Option<&str>,
        src: &Path,
    ) -> Result<Profile> {
        if !src.exists() {
            bail!("Source directory '{}' does not exist.", src.display());
        }
        if name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }
        if let Some(a) = alias {
            Self::validate_alias(a)?;
        }

        // Remove conflicting profiles
        let registry = self.load_registry()?;
        let conflicts: Vec<_> = registry
            .profiles
            .iter()
            .filter(|(_, p)| p.name == name || p.alias.as_deref() == alias)
            .map(|(id, p)| (id.clone(), p.dir_name()))
            .collect();
        if !conflicts.is_empty() {
            let mut reg = self.load_registry()?;
            for (id, dir_name) in conflicts {
                let dir = self.profiles_dir.join(dir_name);
                if dir.exists() {
                    let _ = fs::remove_dir_all(&dir);
                }
                reg.profiles.remove(&id);
            }
            self.save_registry(&reg)?;
        }

        let id = Uuid::new_v4().to_string();
        let profile = self.copy_and_build_profile(&id, name, alias, src)?;
        self.upsert_profile(&profile)?;
        Ok(profile)
    }

    /// Add current `~/.claude` as a full profile.
    pub fn add_profile(&self, name: &str, alias: Option<&str>) -> Result<Profile> {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let src = home.join(".claude");
        if !src.exists() {
            bail!("~/.claude does not exist. Is Claude Code installed and logged in?");
        }
        self.add_profile_from(name, alias, &src)
    }

    /// Force-add current `~/.claude` as a full profile.
    pub fn add_profile_force(&self, name: &str, alias: Option<&str>) -> Result<Profile> {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let src = home.join(".claude");
        self.add_profile_from_force(name, alias, &src)
    }

    /// Refresh a full profile's data from `~/.claude` (preserves id, name, alias).
    pub fn refresh_profile(&self, query: &str) -> Result<Profile> {
        let (id, profile) = self.find_profile(query)?;
        if profile.kind != ProfileKind::Full {
            bail!("Refresh applies only to full profiles.");
        }
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let src = home.join(".claude");
        if !src.exists() {
            bail!("~/.claude does not exist.");
        }
        let dir = self.profiles_dir.join(profile.dir_name());
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        copy_dir_all(&src, &dir)?;

        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.added = Utc::now();
        }
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn remove_profile(&self, query: &str) -> Result<()> {
        let (id, profile) = self.find_profile(query)?;
        let dir = self.profiles_dir.join(profile.dir_name());
        if profile.kind == ProfileKind::Full && dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        let mut registry = self.load_registry()?;
        registry.profiles.remove(&id);
        self.save_registry(&registry)
    }

    pub(crate) fn duplicate_profile_with_alias_override(
        &self,
        query: &str,
        new_name: &str,
        new_alias: Option<&str>,
        auto_suggest_alias: bool,
    ) -> Result<Profile> {
        let registry = self.load_registry()?;
        let (_, source) = Self::find_profile_in_registry(&registry, query)?;
        if new_name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }

        let alias = if auto_suggest_alias && new_alias.is_none() {
            Self::suggest_duplicate_alias_in_registry(&registry, source.alias.as_deref(), new_name)
        } else {
            match new_alias {
                Some(alias) => {
                    Self::validate_alias(alias)?;
                    Some(alias.to_string())
                }
                None => None,
            }
        };
        Self::check_profile_unique_in_registry(&registry, "", new_name, alias.as_deref())?;

        let mut duplicated = source.clone();
        duplicated.id = Uuid::new_v4().to_string();
        duplicated.name = new_name.to_string();
        duplicated.alias = alias;
        duplicated.added = Utc::now();
        duplicated.last_used = None;

        if duplicated.kind == ProfileKind::Full {
            let src_dir = self.profile_dir(&source);
            if !src_dir.exists() {
                bail!(
                    "Source profile directory '{}' does not exist.",
                    src_dir.display()
                );
            }
            let dest_dir = self.profile_dir(&duplicated);
            copy_dir_all(&src_dir, &dest_dir)?;
        }

        self.upsert_profile(&duplicated)?;
        Ok(duplicated)
    }

    pub fn duplicate_profile(
        &self,
        query: &str,
        new_name: &str,
        new_alias: Option<&str>,
    ) -> Result<Profile> {
        self.duplicate_profile_with_alias_override(query, new_name, new_alias, new_alias.is_none())
    }

    /// Rename a profile (change name and/or alias). Checks uniqueness.
    pub fn rename_profile(
        &self,
        query: &str,
        new_name: &str,
        new_alias: Option<&str>,
    ) -> Result<Profile> {
        let (id, mut profile) = self.find_profile(query)?;
        if new_name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }
        self.check_unique(&id, new_name, new_alias)?;
        if let Some(a) = new_alias {
            Self::validate_alias(a)?;
        }

        let old_dir_name = profile.dir_name().to_string();
        profile.name = new_name.to_string();
        profile.alias = new_alias.map(String::from);
        let new_dir_name = profile.dir_name().to_string();

        // Rename directory if it changed (full profiles)
        if old_dir_name != new_dir_name {
            let old_dir = self.profiles_dir.join(&old_dir_name);
            let new_dir = self.profiles_dir.join(&new_dir_name);
            if old_dir.exists() {
                if new_dir.exists() {
                    fs::remove_dir_all(&new_dir)?;
                }
                fs::rename(&old_dir, &new_dir)?;
            }
        }

        let mut registry = self.load_registry()?;
        registry.profiles.insert(id.clone(), profile.clone());
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn profile_dir(&self, profile: &Profile) -> PathBuf {
        self.profiles_dir.join(profile.dir_name())
    }

    // ── Lightweight profiles ─────────────────────────────────────────────────

    pub fn create_lightweight_profile(
        &self,
        name: &str,
        alias: Option<&str>,
        env: LightweightEnv,
    ) -> Result<Profile> {
        if name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }
        self.check_unique("", name, alias)?;
        if let Some(a) = alias {
            Self::validate_alias(a)?;
        }
        let id = Uuid::new_v4().to_string();
        let profile = Profile {
            id,
            name: name.to_string(),
            alias: alias.map(String::from),
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Lightweight,
            env: Some(env),
            launch_args: None,
            provider_id: None,
            key_id: None,
            mcp_server_ids: Vec::new(),
            plugin_ids: Vec::new(),
        };
        let mut registry = self.load_registry()?;
        registry
            .profiles
            .insert(profile.id.clone(), profile.clone());
        self.save_registry(&registry)?;
        Ok(profile)
    }

    /// Update name, alias, and env vars for an existing lightweight profile.
    pub fn update_lightweight(
        &self,
        query: &str,
        new_name: &str,
        new_alias: Option<&str>,
        env: LightweightEnv,
    ) -> Result<Profile> {
        let (id, existing) = self.find_profile(query)?;
        if new_name.trim().is_empty() {
            bail!("Profile name cannot be empty.");
        }
        self.check_unique(&id, new_name, new_alias)?;
        if let Some(a) = new_alias {
            Self::validate_alias(a)?;
        }

        let profile = Profile {
            id,
            name: new_name.to_string(),
            alias: new_alias.map(String::from),
            added: existing.added,
            last_used: existing.last_used,
            kind: ProfileKind::Lightweight,
            env: Some(env),
            launch_args: existing.launch_args.clone(),
            provider_id: existing.provider_id.clone(),
            key_id: existing.key_id.clone(),
            mcp_server_ids: existing.mcp_server_ids.clone(),
            plugin_ids: existing.plugin_ids.clone(),
        };

        let mut registry = self.load_registry()?;
        registry
            .profiles
            .insert(profile.id.clone(), profile.clone());
        self.save_registry(&registry)?;
        Ok(profile)
    }

    fn copy_and_build_profile(
        &self,
        id: &str,
        name: &str,
        alias: Option<&str>,
        src: &Path,
    ) -> Result<Profile> {
        let dir_name = alias.unwrap_or(name);
        let dest = self.profiles_dir.join(dir_name);
        copy_dir_all(src, &dest)?;
        Ok(Profile {
            id: id.to_string(),
            name: name.to_string(),
            alias: alias.map(String::from),
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Full,
            env: None,
            launch_args: None,
            provider_id: None,
            key_id: None,
            mcp_server_ids: Vec::new(),
            plugin_ids: Vec::new(),
        })
    }

    fn upsert_profile(&self, profile: &Profile) -> Result<()> {
        let mut registry = self.load_registry()?;
        registry
            .profiles
            .insert(profile.id.clone(), profile.clone());
        self.save_registry(&registry)
    }
}
