use anyhow::{Context, Result, bail};
use chrono::Utc;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

use super::{
    HostedPluginCatalogItem, InstalledPlugin, InstalledPluginDetails, PluginMarketplace,
    PluginMarketplaceSourceKind, Profile, ProfileManager, Registry, copy_dir_all,
};

#[derive(Debug, Clone, Deserialize)]
struct MarketplaceFile {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    plugins: Vec<MarketplacePluginEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketplacePluginEntry {
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    strict: Option<bool>,
    #[serde(default, rename = "defaultEnabled")]
    default_enabled: Option<bool>,
    #[serde(default)]
    dependencies: Option<Value>,
    #[serde(default, rename = "allowCrossMarketplaceDependenciesOn")]
    allow_cross_marketplace_dependencies_on: Vec<String>,
    source: MarketplacePluginSourceDef,
    #[serde(default)]
    author: Option<Value>,
    #[serde(default)]
    skills: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum MarketplacePluginSourceDef {
    Relative(String),
    Object(MarketplacePluginSourceObject),
}

#[derive(Debug, Clone, Deserialize)]
struct MarketplacePluginSourceObject {
    source: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default, rename = "ref")]
    ref_name: Option<String>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    commit: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedMarketplaceEntry {
    marketplace: PluginMarketplace,
    entry: MarketplacePluginEntry,
}

#[derive(Debug, Clone)]
struct ResolvedPluginSource {
    plugin_root: PathBuf,
    source_url: Option<String>,
    source_ref: Option<String>,
    source_sha: Option<String>,
}

#[derive(Debug, Clone)]
struct PreparedPluginInstall {
    installed: InstalledPlugin,
    manifest_json: Value,
    plugin_root: PathBuf,
    dependencies: Vec<PluginDependency>,
    allow_cross_marketplace_dependencies_on: Vec<String>,
}

#[derive(Debug, Clone)]
struct PluginDependency {
    name: String,
    marketplace: Option<String>,
    version_req: Option<VersionReq>,
}

#[derive(Debug, Clone)]
struct MarketplaceCacheSource {
    source_kind: PluginMarketplaceSourceKind,
    locator: String,
    canonical_url: Option<String>,
    source_path: Option<PathBuf>,
}

type RemotePluginFile = (String, String);
type InstalledPluginRemoteFileSet = (
    Vec<RemotePluginFile>,
    Vec<RemotePluginFile>,
    HashSet<String>,
);
type PluginSourceDescriptor = (String, Option<String>, Option<String>, Option<String>);

impl ProfileManager {
    pub(crate) fn plugins_root_dir(&self) -> PathBuf {
        self.base_dir().join("plugins")
    }

    pub(crate) fn plugin_marketplaces_root_dir(&self) -> PathBuf {
        self.plugins_root_dir().join("marketplaces")
    }

    pub(crate) fn plugin_sources_root_dir(&self) -> PathBuf {
        self.plugins_root_dir().join("sources")
    }

    pub(crate) fn plugin_installed_root_dir(&self) -> PathBuf {
        self.plugins_root_dir().join("installed")
    }

    pub(super) fn remote_plugin_installed_root_dir(
        remote_home: &str,
        remote_os: super::RemoteOs,
    ) -> String {
        match remote_os {
            super::RemoteOs::Unix => {
                format!(
                    "{}/.claude-switch/plugins/installed",
                    remote_home.trim_end_matches('/')
                )
            }
            super::RemoteOs::Windows => format!(
                "{}\\.claude-switch\\plugins\\installed",
                remote_home.trim_end_matches(['\\', '/'])
            ),
        }
    }

    pub(crate) fn plugin_marketplace_repo_dir(&self, marketplace_name: &str) -> PathBuf {
        self.plugin_marketplaces_root_dir()
            .join(Self::safe_plugin_path_component(marketplace_name))
            .join("repo")
    }

    pub(crate) fn plugin_source_repo_dir(&self, cache_key: &str) -> PathBuf {
        self.plugin_sources_root_dir().join(cache_key).join("repo")
    }

    pub(crate) fn plugin_install_root(&self, marketplace_name: &str, plugin_name: &str) -> PathBuf {
        self.plugin_installed_root_dir()
            .join(Self::safe_plugin_path_component(marketplace_name))
            .join(Self::safe_plugin_path_component(plugin_name))
    }

    pub(crate) fn plugin_home_relative_root(
        marketplace_name: &str,
        plugin_name: &str,
        target_os: super::RemoteOs,
    ) -> String {
        let market = Self::safe_plugin_path_component(marketplace_name);
        let plugin = Self::safe_plugin_path_component(plugin_name);
        match target_os {
            super::RemoteOs::Unix => {
                format!("$HOME/.claude-switch/plugins/installed/{market}/{plugin}")
            }
            super::RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\plugins\\installed\\{market}\\{plugin}")
            }
        }
    }

    pub(crate) fn plugin_id(plugin_name: &str, marketplace_name: &str) -> String {
        format!("{plugin_name}@{marketplace_name}")
    }

    pub(crate) fn safe_plugin_path_component(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                out.push('_');
            } else {
                out.push(ch);
            }
        }
        let trimmed = out.trim().trim_matches('.').trim();
        if trimmed.is_empty() {
            "unnamed".to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub(crate) fn plugin_github_mirror_base_url(&self) -> Result<Option<String>> {
        Ok(self.global_settings()?.plugin_github_mirror_base_url)
    }

    pub fn list_plugin_marketplaces(&self) -> Result<Vec<PluginMarketplace>> {
        let registry = self.load_registry()?;
        let mut marketplaces: Vec<_> = registry.plugin_marketplaces.into_values().collect();
        marketplaces.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(marketplaces)
    }

    pub fn add_plugin_marketplace(
        &self,
        locator: &str,
        replace: bool,
    ) -> Result<(PluginMarketplace, Vec<HostedPluginCatalogItem>)> {
        let source = self.resolve_marketplace_source(locator)?;
        let temp_dir = self.base_dir().join(format!(
            "plugin-marketplace-tmp-{}",
            &Uuid::new_v4().to_string()[..8]
        ));
        if temp_dir.exists() {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        self.populate_marketplace_cache(&source, &temp_dir)?;
        let manifest = self.load_marketplace_manifest(&temp_dir)?;
        let marketplace = PluginMarketplace {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            source_kind: source.source_kind,
            locator: source.locator,
            canonical_url: source.canonical_url,
            added_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };

        let mut registry = self.load_registry()?;
        if registry.plugin_marketplaces.contains_key(&marketplace.name) && !replace {
            bail!(
                "Plugin marketplace '{}' already exists. Use --replace to update it.",
                marketplace.name
            );
        }

        let final_repo_dir = self.plugin_marketplace_repo_dir(&marketplace.name);
        self.replace_dir_with_temp(&temp_dir, &final_repo_dir)?;
        registry
            .plugin_marketplaces
            .insert(marketplace.name.clone(), marketplace.clone());
        self.save_registry(&registry)?;

        Ok((
            marketplace.clone(),
            self.read_marketplace_catalog(&marketplace, &manifest),
        ))
    }

    pub fn update_plugin_marketplace(
        &self,
        query: &str,
    ) -> Result<(PluginMarketplace, Vec<HostedPluginCatalogItem>)> {
        let mut registry = self.load_registry()?;
        let (key, existing) = Self::find_plugin_marketplace_in_registry(&registry, query)?;
        let source = self.resolve_marketplace_source(&existing.locator)?;
        let temp_dir = self.base_dir().join(format!(
            "plugin-marketplace-tmp-{}",
            &Uuid::new_v4().to_string()[..8]
        ));
        if temp_dir.exists() {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        self.populate_marketplace_cache(&source, &temp_dir)?;
        let manifest = self.load_marketplace_manifest(&temp_dir)?;
        if manifest.name != existing.name {
            bail!(
                "Updated marketplace manifest changed name from '{}' to '{}'.",
                existing.name,
                manifest.name
            );
        }
        let updated = PluginMarketplace {
            name: existing.name.clone(),
            description: manifest.description.clone(),
            source_kind: source.source_kind,
            locator: source.locator,
            canonical_url: source.canonical_url,
            added_at: existing.added_at,
            updated_at: Some(Utc::now()),
        };
        let final_repo_dir = self.plugin_marketplace_repo_dir(&existing.name);
        self.replace_dir_with_temp(&temp_dir, &final_repo_dir)?;
        registry.plugin_marketplaces.insert(key, updated.clone());
        self.save_registry(&registry)?;
        Ok((
            updated.clone(),
            self.read_marketplace_catalog(&updated, &manifest),
        ))
    }

    pub fn remove_plugin_marketplace(&self, query: &str) -> Result<()> {
        let mut registry = self.load_registry()?;
        let (key, marketplace) = Self::find_plugin_marketplace_in_registry(&registry, query)?;
        let in_use = registry
            .installed_plugins
            .values()
            .filter(|installed| installed.marketplace_name == marketplace.name)
            .map(|installed| installed.id.clone())
            .collect::<Vec<_>>();
        if !in_use.is_empty() {
            bail!(
                "Plugin marketplace '{}' still has installed plugins: {}. Uninstall them first.",
                marketplace.name,
                in_use.join(", ")
            );
        }
        registry.plugin_marketplaces.remove(&key);
        self.save_registry(&registry)?;
        let repo_dir = self.plugin_marketplace_repo_dir(&marketplace.name);
        if repo_dir.exists() {
            let _ = fs::remove_dir_all(repo_dir.parent().unwrap_or(&repo_dir));
        }
        Ok(())
    }

    pub fn list_hosted_plugin_catalog(
        &self,
        marketplace_query: Option<&str>,
    ) -> Result<Vec<HostedPluginCatalogItem>> {
        let registry = self.load_registry()?;
        let mut items = Vec::new();
        let marketplaces: Vec<PluginMarketplace> = if let Some(query) = marketplace_query {
            vec![Self::find_plugin_marketplace_in_registry(&registry, query)?.1]
        } else {
            registry.plugin_marketplaces.values().cloned().collect()
        };
        for marketplace in marketplaces {
            let manifest = self
                .load_marketplace_manifest(&self.plugin_marketplace_repo_dir(&marketplace.name))?;
            items.extend(self.read_marketplace_catalog(&marketplace, &manifest));
        }
        items.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(items)
    }

    pub fn list_installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        let registry = self.load_registry()?;
        let mut plugins: Vec<_> = registry.installed_plugins.into_values().collect();
        plugins.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(plugins)
    }

    pub fn get_installed_plugin(&self, query: &str) -> Result<InstalledPlugin> {
        let registry = self.load_registry()?;
        Self::find_installed_plugin_in_registry(&registry, query).map(|(_, value)| value)
    }

    pub fn installed_plugin_details(&self, query: &str) -> Result<InstalledPluginDetails> {
        let registry = self.load_registry()?;
        let (_, installed) = Self::find_installed_plugin_in_registry(&registry, query)?;
        let linked_profiles = registry
            .profiles
            .values()
            .filter(|profile| profile.plugin_ids.iter().any(|id| id == &installed.id))
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        let install_root =
            self.plugin_install_root(&installed.marketplace_name, &installed.plugin_name);
        Ok(InstalledPluginDetails {
            installed,
            linked_profiles,
            exists: install_root.exists(),
            install_root,
        })
    }

    pub fn resolve_hosted_plugin_candidates(
        &self,
        query: Option<&str>,
        marketplace_query: Option<&str>,
    ) -> Result<Vec<HostedPluginCatalogItem>> {
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let mut candidates = self.list_hosted_plugin_catalog(marketplace_query)?;
        if let Some(query) = query {
            if query.contains('@') {
                candidates.retain(|item| item.id == query);
            } else {
                candidates.retain(|item| item.plugin_name == query || item.id == query);
            }
        }
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(candidates)
    }

    pub fn install_hosted_plugin(&self, query: &str, explicit: bool) -> Result<InstalledPlugin> {
        let resolved = self.resolve_catalog_item_exact(query)?;
        let mut registry = self.load_registry()?;
        let mut visiting = HashSet::new();
        let installed =
            self.install_hosted_plugin_inner(&resolved, explicit, &mut registry, &mut visiting)?;
        self.save_registry(&registry)?;
        Ok(installed)
    }

    pub fn update_installed_plugin(&self, query: &str) -> Result<InstalledPlugin> {
        let installed = self.get_installed_plugin(query)?;
        self.install_hosted_plugin(&installed.id, installed.explicit)
    }

    pub fn update_all_installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        let plugins = self.list_installed_plugins()?;
        let mut updated = Vec::new();
        for plugin in plugins {
            updated.push(self.update_installed_plugin(&plugin.id)?);
        }
        Ok(updated)
    }

    pub fn uninstall_installed_plugin(&self, query: &str, prune: bool) -> Result<()> {
        let mut registry = self.load_registry()?;
        let (id, installed) = Self::find_installed_plugin_in_registry(&registry, query)?;
        let required_by = registry
            .installed_plugins
            .values()
            .filter(|other| {
                other.id != installed.id
                    && other.dependencies.iter().any(|dep| dep == &installed.id)
            })
            .map(|other| other.id.clone())
            .collect::<Vec<_>>();
        if !required_by.is_empty() {
            bail!(
                "Plugin '{}' is still required by: {}",
                installed.id,
                required_by.join(", ")
            );
        }
        let linked_profiles = registry
            .profiles
            .values()
            .filter(|profile| profile.plugin_ids.iter().any(|plugin_id| plugin_id == &id))
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        if !linked_profiles.is_empty() {
            bail!(
                "Plugin '{}' is still linked to profiles: {}",
                installed.id,
                linked_profiles.join(", ")
            );
        }
        registry.installed_plugins.remove(&id);
        self.save_registry(&registry)?;
        let install_root =
            self.plugin_install_root(&installed.marketplace_name, &installed.plugin_name);
        if install_root.exists() {
            let _ = fs::remove_dir_all(&install_root);
        }
        if prune {
            let _ = self.prune_installed_plugins()?;
        }
        Ok(())
    }

    pub fn prune_installed_plugins(&self) -> Result<Vec<String>> {
        let mut registry = self.load_registry()?;
        let mut removed = Vec::new();
        loop {
            let reverse_refs = Self::installed_plugin_reverse_refs(&registry);
            let next = registry
                .installed_plugins
                .values()
                .find(|plugin| {
                    !plugin.explicit
                        && reverse_refs
                            .get(&plugin.id)
                            .is_none_or(|value| value.is_empty())
                })
                .cloned();
            let Some(plugin) = next else {
                break;
            };
            registry.installed_plugins.remove(&plugin.id);
            let install_root =
                self.plugin_install_root(&plugin.marketplace_name, &plugin.plugin_name);
            if install_root.exists() {
                let _ = fs::remove_dir_all(&install_root);
            }
            removed.push(plugin.id);
        }
        self.save_registry(&registry)?;
        Ok(removed)
    }

    pub fn list_profiles_using_plugin(&self, plugin_id: &str) -> Result<Vec<Profile>> {
        let registry = self.load_registry()?;
        if !registry.installed_plugins.contains_key(plugin_id) {
            bail!("Plugin '{}' is not installed.", plugin_id);
        }
        let mut profiles = registry
            .profiles
            .values()
            .filter(|profile| profile.plugin_ids.iter().any(|id| id == plugin_id))
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(profiles)
    }

    pub fn set_profile_plugins(&self, query: &str, plugin_queries: &[String]) -> Result<Profile> {
        let (profile_id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        let mut plugin_ids = Vec::new();
        for plugin_query in plugin_queries {
            let (id, _) = Self::find_installed_plugin_in_registry(&registry, plugin_query)?;
            if !plugin_ids.contains(&id) {
                plugin_ids.push(id);
            }
        }
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        profile.plugin_ids = plugin_ids;
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn add_profile_plugins(&self, query: &str, plugin_queries: &[String]) -> Result<Profile> {
        let (profile_id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        let mut additions = Vec::new();
        for plugin_query in plugin_queries {
            let (id, _) = Self::find_installed_plugin_in_registry(&registry, plugin_query)?;
            additions.push(id);
        }
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        for id in additions {
            if !profile.plugin_ids.contains(&id) {
                profile.plugin_ids.push(id);
            }
        }
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn remove_profile_plugins(
        &self,
        query: &str,
        plugin_queries: &[String],
        remove_all: bool,
    ) -> Result<Profile> {
        let (profile_id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        let remove_ids = if remove_all {
            Vec::new()
        } else {
            let mut ids = Vec::new();
            for plugin_query in plugin_queries {
                let (id, _) = Self::find_installed_plugin_in_registry(&registry, plugin_query)?;
                ids.push(id);
            }
            ids
        };
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        if remove_all {
            profile.plugin_ids.clear();
        } else {
            profile
                .plugin_ids
                .retain(|id| !remove_ids.iter().any(|remove_id| remove_id == id));
        }
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub(super) fn profile_plugin_dirs(&self, profile: &Profile) -> Result<Vec<PathBuf>> {
        let registry = self.load_registry()?;
        let mut dirs = Vec::new();
        for plugin_id in &profile.plugin_ids {
            let installed = registry.installed_plugins.get(plugin_id).with_context(|| {
                format!(
                    "Profile '{}' references missing installed plugin '{}'",
                    profile.name, plugin_id
                )
            })?;
            let root =
                self.plugin_install_root(&installed.marketplace_name, &installed.plugin_name);
            if !root.exists() {
                bail!(
                    "Installed plugin '{}' for profile '{}' is missing at {}",
                    installed.id,
                    profile.name,
                    root.display()
                );
            }
            dirs.push(root);
        }
        Ok(dirs)
    }

    pub(super) fn profile_plugin_home_relative_roots(
        &self,
        profile: &Profile,
        target_os: super::RemoteOs,
    ) -> Result<Vec<String>> {
        let registry = self.load_registry()?;
        let mut roots = Vec::new();
        for plugin_id in &profile.plugin_ids {
            let installed = registry.installed_plugins.get(plugin_id).with_context(|| {
                format!(
                    "Profile '{}' references missing installed plugin '{}'",
                    profile.name, plugin_id
                )
            })?;
            roots.push(Self::plugin_home_relative_root(
                &installed.marketplace_name,
                &installed.plugin_name,
                target_os,
            ));
        }
        Ok(roots)
    }

    pub(super) fn append_profile_hosted_plugin_args(
        &self,
        cmd: &mut std::process::Command,
        profile: &Profile,
    ) -> Result<()> {
        for root in self.profile_plugin_dirs(profile)? {
            cmd.arg("--plugin-dir");
            cmd.arg(root);
        }
        Ok(())
    }

    pub(super) fn installed_plugin_remote_file_sets(
        &self,
        profile: &Profile,
        remote_os: super::RemoteOs,
    ) -> Result<InstalledPluginRemoteFileSet> {
        let registry = self.load_registry()?;
        let mut assets = Vec::new();
        let mut scripts = Vec::new();
        let mut roots = HashSet::new();
        for plugin_id in &profile.plugin_ids {
            let installed = registry.installed_plugins.get(plugin_id).with_context(|| {
                format!(
                    "Profile '{}' references missing installed plugin '{}'",
                    profile.name, plugin_id
                )
            })?;
            let root =
                self.plugin_install_root(&installed.marketplace_name, &installed.plugin_name);
            let remote_root = format!(
                "{}{}{}",
                Self::safe_plugin_path_component(&installed.marketplace_name),
                match remote_os {
                    super::RemoteOs::Unix => "/",
                    super::RemoteOs::Windows => "\\",
                },
                Self::safe_plugin_path_component(&installed.plugin_name)
            );
            roots.insert(remote_root.clone());
            Self::collect_plugin_remote_files(
                &root,
                &root,
                &remote_root,
                remote_os,
                &mut assets,
                &mut scripts,
            )?;
        }
        Ok((assets, scripts, roots))
    }

    fn collect_plugin_remote_files(
        root: &Path,
        current: &Path,
        remote_root: &str,
        remote_os: super::RemoteOs,
        assets: &mut Vec<(String, String)>,
        scripts: &mut Vec<(String, String)>,
    ) -> Result<()> {
        for entry in fs::read_dir(current)
            .with_context(|| format!("Failed to read plugin dir {}", current.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                Self::collect_plugin_remote_files(
                    root,
                    &path,
                    remote_root,
                    remote_os,
                    assets,
                    scripts,
                )?;
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("plugin file should live under root")
                .to_string_lossy()
                .replace('\\', "/");
            let remote_rel = match remote_os {
                super::RemoteOs::Unix => format!("{remote_root}/{rel}"),
                super::RemoteOs::Windows => format!(
                    "{}\\{}",
                    remote_root.replace('/', "\\"),
                    rel.replace('/', "\\")
                ),
            };
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read plugin file {}", path.display()))?;
            let is_script = rel.starts_with("scripts/");
            if is_script {
                scripts.push((remote_rel, content));
            } else {
                assets.push((remote_rel, content));
            }
        }
        Ok(())
    }

    fn install_hosted_plugin_inner(
        &self,
        resolved: &HostedPluginCatalogItem,
        explicit: bool,
        registry: &mut Registry,
        visiting: &mut HashSet<String>,
    ) -> Result<InstalledPlugin> {
        if !visiting.insert(resolved.id.clone()) {
            bail!("Circular plugin dependency detected at '{}'.", resolved.id);
        }
        let prepared = self.prepare_plugin_install(resolved)?;
        for dependency in &prepared.dependencies {
            let dependency_item = self.resolve_dependency_catalog_item(
                registry,
                dependency,
                &prepared.installed.marketplace_name,
                &prepared.allow_cross_marketplace_dependencies_on,
            )?;
            self.install_hosted_plugin_inner(&dependency_item, false, registry, visiting)?;
        }
        visiting.remove(&resolved.id);

        let install_root = self.plugin_install_root(
            &prepared.installed.marketplace_name,
            &prepared.installed.plugin_name,
        );
        let temp_root = self.base_dir().join(format!(
            "plugin-install-tmp-{}",
            &Uuid::new_v4().to_string()[..8]
        ));
        if temp_root.exists() {
            let _ = fs::remove_dir_all(&temp_root);
        }
        copy_dir_all(&prepared.plugin_root, &temp_root)?;
        let manifest_path = temp_root.join(".claude-plugin").join("plugin.json");
        if let Some(parent) = manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&prepared.manifest_json)
                .context("Failed to serialize effective plugin manifest")?,
        )?;
        self.replace_dir_with_temp(&temp_root, &install_root)?;

        let mut installed = prepared.installed.clone();
        installed.explicit = explicit
            || registry
                .installed_plugins
                .get(&installed.id)
                .is_some_and(|existing| existing.explicit);
        registry
            .installed_plugins
            .insert(installed.id.clone(), installed.clone());
        Ok(installed)
    }

    fn prepare_plugin_install(
        &self,
        item: &HostedPluginCatalogItem,
    ) -> Result<PreparedPluginInstall> {
        let resolved = self.resolve_marketplace_entry(&item.id)?;
        let source = self.resolve_plugin_source(&resolved, None)?;
        let source_manifest_path = source
            .plugin_root
            .join(".claude-plugin")
            .join("plugin.json");
        let source_manifest = if source_manifest_path.exists() {
            let raw = fs::read_to_string(&source_manifest_path).with_context(|| {
                format!(
                    "Failed to read source plugin manifest {}",
                    source_manifest_path.display()
                )
            })?;
            serde_json::from_str::<Value>(&raw)
                .context("Source plugin manifest is not valid JSON")?
        } else {
            Value::Object(Map::new())
        };
        let manifest_json = Self::build_effective_plugin_manifest(
            &resolved.entry,
            &source_manifest,
            item,
            source.source_sha.clone(),
        )?;
        Self::validate_plugin_manifest_paths(&source.plugin_root, &manifest_json)?;
        let dependencies = Self::parse_plugin_dependencies(
            manifest_json
                .get("dependencies")
                .or_else(|| resolved.entry.extra.get("dependencies"))
                .or(resolved.entry.dependencies.as_ref()),
        )?;
        let installed = InstalledPlugin {
            id: item.id.clone(),
            plugin_name: item.plugin_name.clone(),
            marketplace_name: item.marketplace_name.clone(),
            version: manifest_json
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| item.version.clone()),
            description: item.description.clone(),
            source_url: source.source_url.clone(),
            source_ref: source.source_ref.clone(),
            source_sha: source.source_sha.clone(),
            dependencies: dependencies
                .iter()
                .map(|dep| {
                    Self::plugin_id(
                        &dep.name,
                        dep.marketplace.as_deref().unwrap_or(&item.marketplace_name),
                    )
                })
                .collect(),
            explicit: false,
            default_enabled: item.default_enabled,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
        };
        Ok(PreparedPluginInstall {
            installed,
            manifest_json,
            plugin_root: source.plugin_root,
            dependencies,
            allow_cross_marketplace_dependencies_on: resolved
                .entry
                .allow_cross_marketplace_dependencies_on
                .clone(),
        })
    }

    fn resolve_dependency_catalog_item(
        &self,
        registry: &Registry,
        dependency: &PluginDependency,
        current_marketplace: &str,
        allow_cross_marketplace_dependencies_on: &[String],
    ) -> Result<HostedPluginCatalogItem> {
        let target_marketplace = dependency
            .marketplace
            .clone()
            .unwrap_or_else(|| current_marketplace.to_string());
        if target_marketplace != current_marketplace
            && !allow_cross_marketplace_dependencies_on
                .iter()
                .any(|entry| entry == &target_marketplace)
        {
            bail!(
                "Dependency '{}@{}' is cross-marketplace but '{}' is not allowed by allowCrossMarketplaceDependenciesOn.",
                dependency.name,
                target_marketplace,
                target_marketplace
            );
        }
        let marketplace =
            Self::find_plugin_marketplace_in_registry(registry, &target_marketplace)?.1;
        let manifest =
            self.load_marketplace_manifest(&self.plugin_marketplace_repo_dir(&marketplace.name))?;
        let mut candidates = self.read_marketplace_catalog(&marketplace, &manifest);
        candidates.retain(|item| item.plugin_name == dependency.name);
        if candidates.is_empty() {
            bail!(
                "Dependency '{}' could not be found in marketplace '{}'.",
                dependency.name,
                target_marketplace
            );
        }
        if let Some(version_req) = &dependency.version_req {
            candidates.retain(|item| {
                item.version
                    .as_deref()
                    .and_then(|value| Version::parse(value.trim_start_matches('v')).ok())
                    .is_some_and(|version| version_req.matches(&version))
            });
        }
        candidates.sort_by(|left, right| left.version.cmp(&right.version));
        candidates.pop().context("No dependency candidate matched")
    }

    fn resolve_catalog_item_exact(&self, query: &str) -> Result<HostedPluginCatalogItem> {
        let candidates = self.resolve_hosted_plugin_candidates(Some(query), None)?;
        match candidates.as_slice() {
            [] => bail!(
                "Plugin '{}' was not found in configured marketplaces.",
                query
            ),
            [single] => Ok(single.clone()),
            _ => bail!(
                "Plugin '{}' is ambiguous across marketplaces. Use plugin@marketplace.",
                query
            ),
        }
    }

    fn resolve_marketplace_entry(&self, query: &str) -> Result<ResolvedMarketplaceEntry> {
        let item = self.resolve_catalog_item_exact(query)?;
        let registry = self.load_registry()?;
        let marketplace =
            Self::find_plugin_marketplace_in_registry(&registry, &item.marketplace_name)?.1;
        let manifest =
            self.load_marketplace_manifest(&self.plugin_marketplace_repo_dir(&marketplace.name))?;
        let entry = manifest
            .plugins
            .into_iter()
            .find(|entry| entry.name == item.plugin_name)
            .with_context(|| {
                format!(
                    "Plugin '{}' disappeared from marketplace '{}'.",
                    item.plugin_name, marketplace.name
                )
            })?;
        Ok(ResolvedMarketplaceEntry { marketplace, entry })
    }

    fn resolve_plugin_source(
        &self,
        resolved: &ResolvedMarketplaceEntry,
        override_ref: Option<&str>,
    ) -> Result<ResolvedPluginSource> {
        match &resolved.entry.source {
            MarketplacePluginSourceDef::Relative(relative) => {
                let mut repo_root = self.plugin_marketplace_repo_dir(&resolved.marketplace.name);
                if let Some(reference) = override_ref {
                    let temp_dir = self.base_dir().join(format!(
                        "plugin-source-tmp-{}",
                        &Uuid::new_v4().to_string()[..8]
                    ));
                    if temp_dir.exists() {
                        let _ = fs::remove_dir_all(&temp_dir);
                    }
                    self.clone_repo_snapshot(&repo_root, &temp_dir, Some(reference))?;
                    repo_root = temp_dir;
                }
                let plugin_root = Self::validated_join(&repo_root, relative)?;
                if !plugin_root.exists() {
                    bail!(
                        "Marketplace plugin source '{}' does not exist under '{}'.",
                        relative,
                        repo_root.display()
                    );
                }
                Ok(ResolvedPluginSource {
                    source_url: resolved.marketplace.canonical_url.clone(),
                    source_ref: override_ref.map(str::to_string),
                    source_sha: self.git_head_sha_if_any(&repo_root).ok(),
                    plugin_root,
                })
            }
            MarketplacePluginSourceDef::Object(source) => {
                let (canonical_url, subdir, default_ref, default_sha) =
                    self.resolve_object_source_descriptor(source)?;
                let chosen_ref = override_ref
                    .map(str::to_string)
                    .or(default_ref)
                    .or(default_sha.clone());
                let cache_key = self.plugin_source_cache_key(
                    &canonical_url,
                    subdir.as_deref(),
                    chosen_ref.as_deref(),
                );
                let repo_root = self.plugin_source_repo_dir(&cache_key);
                self.sync_remote_source_repo(&canonical_url, &repo_root, chosen_ref.as_deref())?;
                let plugin_root = if let Some(subdir) = &subdir {
                    Self::validated_join(&repo_root, subdir)?
                } else {
                    repo_root.clone()
                };
                if !plugin_root.exists() {
                    bail!(
                        "Plugin source path '{}' does not exist in {}.",
                        subdir.unwrap_or_else(|| ".".to_string()),
                        canonical_url
                    );
                }
                Ok(ResolvedPluginSource {
                    source_url: Some(canonical_url),
                    source_ref: chosen_ref,
                    source_sha: self.git_head_sha_if_any(&repo_root).ok(),
                    plugin_root,
                })
            }
        }
    }

    fn resolve_object_source_descriptor(
        &self,
        source: &MarketplacePluginSourceObject,
    ) -> Result<PluginSourceDescriptor> {
        match source.source.trim() {
            "github" => {
                let repo = source
                    .repo
                    .as_deref()
                    .context("github plugin source is missing repo")?;
                Ok((
                    Self::canonical_github_repo_url(repo),
                    source.path.clone(),
                    source.commit.clone(),
                    source.sha.clone(),
                ))
            }
            "url" => Ok((
                source
                    .url
                    .clone()
                    .context("url plugin source is missing url")?,
                source.path.clone(),
                source.commit.clone().or(source.ref_name.clone()),
                source.sha.clone(),
            )),
            "git-subdir" => Ok((
                source
                    .url
                    .clone()
                    .context("git-subdir plugin source is missing url")?,
                source.path.clone(),
                source.ref_name.clone(),
                source.sha.clone(),
            )),
            unsupported => bail!("Unsupported plugin source type '{}'.", unsupported),
        }
    }

    fn resolve_marketplace_source(&self, locator: &str) -> Result<MarketplaceCacheSource> {
        let locator = locator.trim();
        if locator.is_empty() {
            bail!("Marketplace locator cannot be empty.");
        }
        let path = PathBuf::from(locator);
        if path.exists() {
            return Ok(MarketplaceCacheSource {
                source_kind: PluginMarketplaceSourceKind::Local,
                locator: locator.to_string(),
                canonical_url: None,
                source_path: Some(path),
            });
        }
        if Self::is_github_shorthand(locator) {
            return Ok(MarketplaceCacheSource {
                source_kind: PluginMarketplaceSourceKind::GitHub,
                locator: locator.to_string(),
                canonical_url: Some(Self::canonical_github_repo_url(locator)),
                source_path: None,
            });
        }
        if locator.starts_with("http://")
            || locator.starts_with("https://")
            || locator.ends_with(".git")
        {
            return Ok(MarketplaceCacheSource {
                source_kind: PluginMarketplaceSourceKind::Git,
                locator: locator.to_string(),
                canonical_url: Some(locator.to_string()),
                source_path: None,
            });
        }
        bail!(
            "Marketplace locator '{}' is not a local path, GitHub shorthand, or git URL.",
            locator
        )
    }

    fn populate_marketplace_cache(
        &self,
        source: &MarketplaceCacheSource,
        dest: &Path,
    ) -> Result<()> {
        if let Some(source_path) = &source.source_path {
            copy_dir_all(source_path, dest)?;
            return Ok(());
        }
        let url = source
            .canonical_url
            .as_deref()
            .context("remote marketplace is missing canonical URL")?;
        self.sync_remote_source_repo(url, dest, None)
    }

    fn load_marketplace_manifest(&self, repo_root: &Path) -> Result<MarketplaceFile> {
        let manifest_path = repo_root.join(".claude-plugin").join("marketplace.json");
        let raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "Marketplace manifest '{}' is not valid JSON",
                manifest_path.display()
            )
        })
    }

    fn read_marketplace_catalog(
        &self,
        marketplace: &PluginMarketplace,
        manifest: &MarketplaceFile,
    ) -> Vec<HostedPluginCatalogItem> {
        let mut items = manifest
            .plugins
            .iter()
            .map(|entry| HostedPluginCatalogItem {
                id: Self::plugin_id(&entry.name, &marketplace.name),
                marketplace_name: marketplace.name.clone(),
                plugin_name: entry.name.clone(),
                display_name: entry.display_name.clone(),
                description: entry.description.clone(),
                version: entry.version.clone(),
                category: entry.category.clone(),
                homepage: entry.homepage.clone(),
                default_enabled: entry.default_enabled,
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        items
    }

    fn find_plugin_marketplace_in_registry(
        registry: &Registry,
        query: &str,
    ) -> Result<(String, PluginMarketplace)> {
        let query = query.trim();
        if query.is_empty() {
            bail!("Plugin marketplace query is empty.");
        }
        if let Some(value) = registry.plugin_marketplaces.get(query) {
            return Ok((query.to_string(), value.clone()));
        }
        let by_name = registry
            .plugin_marketplaces
            .iter()
            .filter(|(_, value)| value.name == query)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        match by_name.as_slice() {
            [(key, value)] => Ok((key.clone(), value.clone())),
            [] => bail!("Plugin marketplace '{}' is not configured.", query),
            _ => bail!("Plugin marketplace '{}' is ambiguous.", query),
        }
    }

    fn find_installed_plugin_in_registry(
        registry: &Registry,
        query: &str,
    ) -> Result<(String, InstalledPlugin)> {
        let query = query.trim();
        if query.is_empty() {
            bail!("Installed plugin query is empty.");
        }
        if let Some(value) = registry.installed_plugins.get(query) {
            return Ok((query.to_string(), value.clone()));
        }
        let by_name = registry
            .installed_plugins
            .iter()
            .filter(|(_, value)| value.plugin_name == query || value.id == query)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        match by_name.as_slice() {
            [(key, value)] => Ok((key.clone(), value.clone())),
            [] => bail!("Installed plugin '{}' was not found.", query),
            _ => bail!(
                "Installed plugin '{}' is ambiguous. Use plugin@marketplace.",
                query
            ),
        }
    }

    fn build_effective_plugin_manifest(
        entry: &MarketplacePluginEntry,
        source_manifest: &Value,
        item: &HostedPluginCatalogItem,
        source_sha: Option<String>,
    ) -> Result<Value> {
        let mut manifest = source_manifest
            .as_object()
            .cloned()
            .unwrap_or_else(Map::new);
        manifest.insert("name".to_string(), Value::String(item.plugin_name.clone()));
        if let Some(display_name) = &item.display_name {
            manifest
                .entry("displayName".to_string())
                .or_insert_with(|| Value::String(display_name.clone()));
        }
        if let Some(description) = &item.description {
            manifest
                .entry("description".to_string())
                .or_insert_with(|| Value::String(description.clone()));
        }
        if let Some(author) = &entry.author {
            manifest
                .entry("author".to_string())
                .or_insert_with(|| author.clone());
        }
        if let Some(homepage) = &item.homepage {
            manifest
                .entry("homepage".to_string())
                .or_insert_with(|| Value::String(homepage.clone()));
        }
        if let Some(default_enabled) = item.default_enabled {
            manifest
                .entry("defaultEnabled".to_string())
                .or_insert_with(|| Value::Bool(default_enabled));
        }
        if let Some(strict) = entry.strict {
            manifest
                .entry("strict".to_string())
                .or_insert_with(|| Value::Bool(strict));
        }
        if let Some(skills) = &entry.skills {
            manifest
                .entry("skills".to_string())
                .or_insert_with(|| skills.clone());
        }
        for (key, value) in &entry.extra {
            if !manifest.contains_key(key) {
                manifest.insert(key.clone(), value.clone());
            }
        }
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| item.version.clone())
            .or(source_sha);
        if let Some(version) = version {
            manifest.insert("version".to_string(), Value::String(version));
        }

        let source_component_keys = ["skills", "commands", "agents", "hooks", "outputStyles"];
        let source_has_components = source_manifest.as_object().is_some_and(|object| {
            source_component_keys
                .iter()
                .any(|key| object.contains_key(*key))
        });
        let entry_has_components = entry.skills.is_some()
            || source_component_keys
                .iter()
                .any(|key| entry.extra.contains_key(*key));
        let strict = manifest
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if !strict && source_has_components && entry_has_components {
            bail!(
                "Plugin '{}' uses strict=false but both the source manifest and marketplace entry define components.",
                item.id
            );
        }
        Ok(Value::Object(manifest))
    }

    fn validate_plugin_manifest_paths(plugin_root: &Path, manifest_json: &Value) -> Result<()> {
        let Some(object) = manifest_json.as_object() else {
            return Ok(());
        };
        for key in [
            "skills",
            "commands",
            "agents",
            "hooks",
            "outputStyles",
            "output-styles",
        ] {
            if let Some(value) = object.get(key) {
                Self::validate_component_path_value(plugin_root, value)
                    .with_context(|| format!("Invalid plugin component path in '{}'", key))?;
            }
        }
        Ok(())
    }

    fn validate_component_path_value(plugin_root: &Path, value: &Value) -> Result<()> {
        match value {
            Value::String(path) => {
                let _ = Self::validated_join(plugin_root, path)?;
            }
            Value::Array(items) => {
                for item in items {
                    Self::validate_component_path_value(plugin_root, item)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_plugin_dependencies(value: Option<&Value>) -> Result<Vec<PluginDependency>> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        let Some(items) = value.as_array() else {
            return Ok(Vec::new());
        };
        let mut dependencies = Vec::new();
        for item in items {
            match item {
                Value::String(text) => dependencies.push(Self::parse_string_dependency(text)?),
                Value::Object(object) => {
                    let name = object
                        .get("name")
                        .or_else(|| object.get("plugin"))
                        .and_then(Value::as_str)
                        .context("plugin dependency object is missing name")?;
                    let marketplace = object
                        .get("marketplace")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let version_req = object
                        .get("version")
                        .or_else(|| object.get("constraint"))
                        .and_then(Value::as_str)
                        .map(VersionReq::parse)
                        .transpose()
                        .context("plugin dependency version requirement is invalid")?;
                    dependencies.push(PluginDependency {
                        name: name.to_string(),
                        marketplace,
                        version_req,
                    });
                }
                _ => {}
            }
        }
        Ok(dependencies)
    }

    fn parse_string_dependency(text: &str) -> Result<PluginDependency> {
        let text = text.trim();
        if text.is_empty() {
            bail!("plugin dependency string is empty");
        }
        if let Some((name, suffix)) = text.split_once('@') {
            if suffix.starts_with('^')
                || suffix.starts_with('~')
                || suffix.starts_with('>')
                || suffix.starts_with('<')
                || suffix.chars().next().is_some_and(|ch| ch.is_ascii_digit())
            {
                return Ok(PluginDependency {
                    name: name.to_string(),
                    marketplace: None,
                    version_req: Some(VersionReq::parse(suffix).with_context(|| {
                        format!("Invalid dependency version requirement '{suffix}'")
                    })?),
                });
            }
            return Ok(PluginDependency {
                name: name.to_string(),
                marketplace: Some(suffix.to_string()),
                version_req: None,
            });
        }
        Ok(PluginDependency {
            name: text.to_string(),
            marketplace: None,
            version_req: None,
        })
    }

    fn installed_plugin_reverse_refs(registry: &Registry) -> HashMap<String, Vec<String>> {
        let mut reverse = HashMap::new();
        for plugin in registry.installed_plugins.values() {
            for dependency in &plugin.dependencies {
                reverse
                    .entry(dependency.clone())
                    .or_insert_with(Vec::new)
                    .push(plugin.id.clone());
            }
        }
        reverse
    }

    fn validated_join(root: &Path, relative: &str) -> Result<PathBuf> {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute() {
            bail!("Absolute plugin paths are not allowed: {}", relative);
        }
        let mut joined = root.to_path_buf();
        for component in relative_path.components() {
            match component {
                Component::Normal(part) => joined.push(part),
                Component::CurDir => {}
                Component::ParentDir => bail!("Path escapes plugin root: {}", relative),
                Component::Prefix(_) | Component::RootDir => {
                    bail!("Path escapes plugin root: {}", relative)
                }
            }
        }
        Ok(joined)
    }

    fn replace_dir_with_temp(&self, temp_dir: &Path, final_dir: &Path) -> Result<()> {
        if final_dir.exists() {
            fs::remove_dir_all(final_dir).with_context(|| {
                format!(
                    "Failed to remove existing directory {}",
                    final_dir.display()
                )
            })?;
        }
        if let Some(parent) = final_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(temp_dir, final_dir).or_else(|_| {
            copy_dir_all(temp_dir, final_dir)?;
            fs::remove_dir_all(temp_dir)?;
            Ok(())
        })
    }

    fn sync_remote_source_repo(
        &self,
        canonical_url: &str,
        dest: &Path,
        reference: Option<&str>,
    ) -> Result<()> {
        let temp_dir = self.base_dir().join(format!(
            "plugin-source-cache-{}",
            &Uuid::new_v4().to_string()[..8]
        ));
        if temp_dir.exists() {
            let _ = fs::remove_dir_all(&temp_dir);
        }
        let urls = self.preferred_clone_urls(canonical_url)?;
        let mut last_error = None;
        for url in urls {
            match self.clone_remote_repo_to(&url, &temp_dir, reference) {
                Ok(_) => {
                    self.replace_dir_with_temp(&temp_dir, dest)?;
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                    if temp_dir.exists() {
                        let _ = fs::remove_dir_all(&temp_dir);
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Failed to clone {}", canonical_url)))
    }

    fn clone_remote_repo_to(&self, url: &str, dest: &Path, reference: Option<&str>) -> Result<()> {
        let mut cmd = super::build_local_command("git");
        cmd.arg("clone").arg(url).arg(dest);
        let output = cmd.output().context("Failed to spawn git clone")?;
        if !output.status.success() {
            bail!(
                "git clone failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if let Some(reference) = reference {
            let mut checkout = super::build_local_command("git");
            checkout.arg("-C").arg(dest).arg("checkout").arg(reference);
            let output = checkout.output().context("Failed to spawn git checkout")?;
            if !output.status.success() {
                bail!(
                    "git checkout '{}' failed: {}",
                    reference,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
        }
        Ok(())
    }

    fn clone_repo_snapshot(
        &self,
        source_repo: &Path,
        dest: &Path,
        reference: Option<&str>,
    ) -> Result<()> {
        let mut clone = super::build_local_command("git");
        clone.arg("clone").arg(source_repo).arg(dest);
        let output = clone.output().context("Failed to spawn git clone")?;
        if !output.status.success() {
            bail!(
                "git clone '{}' failed: {}",
                source_repo.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if let Some(reference) = reference {
            let mut checkout = super::build_local_command("git");
            checkout.arg("-C").arg(dest).arg("checkout").arg(reference);
            let output = checkout.output().context("Failed to spawn git checkout")?;
            if !output.status.success() {
                bail!(
                    "git checkout '{}' failed: {}",
                    reference,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
        }
        Ok(())
    }

    fn git_head_sha_if_any(&self, repo_root: &Path) -> Result<String> {
        let mut cmd = super::build_local_command("git");
        cmd.arg("-C").arg(repo_root).arg("rev-parse").arg("HEAD");
        let output = cmd.output().context("Failed to spawn git rev-parse")?;
        if !output.status.success() {
            bail!(
                "git rev-parse failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn preferred_clone_urls(&self, canonical_url: &str) -> Result<Vec<String>> {
        let mut urls = Vec::new();
        if let Some(mirror_base) = self.plugin_github_mirror_base_url()?
            && let Some(mirrored) = Self::mirror_github_url(&mirror_base, canonical_url)
        {
            urls.push(mirrored);
        }
        urls.push(canonical_url.to_string());
        urls.dedup();
        Ok(urls)
    }

    fn mirror_github_url(mirror_base: &str, canonical_url: &str) -> Option<String> {
        if !canonical_url.starts_with("https://github.com/") {
            return None;
        }
        let base = mirror_base.trim_end_matches('/');
        Some(format!("{base}/{canonical_url}"))
    }

    fn canonical_github_repo_url(repo: &str) -> String {
        let repo = repo.trim().trim_start_matches("https://github.com/");
        if repo.ends_with(".git") {
            format!("https://github.com/{repo}")
        } else {
            format!("https://github.com/{repo}.git")
        }
    }

    fn is_github_shorthand(locator: &str) -> bool {
        let parts = locator.split('/').collect::<Vec<_>>();
        parts.len() == 2
            && parts
                .iter()
                .all(|part| !part.is_empty() && !part.contains(' ') && !part.contains(':'))
    }

    fn plugin_source_cache_key(
        &self,
        canonical_url: &str,
        subdir: Option<&str>,
        reference: Option<&str>,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(canonical_url.as_bytes());
        hasher.update(b"\n");
        if let Some(subdir) = subdir {
            hasher.update(subdir.as_bytes());
        }
        hasher.update(b"\n");
        if let Some(reference) = reference {
            hasher.update(reference.as_bytes());
        }
        let digest = hasher.finalize();
        digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
