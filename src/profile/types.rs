use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    Lightweight,
    #[default]
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalGatewayToolMode {
    #[default]
    Auto,
    SearchFetch,
    FetchOnly,
    GatewayOnly,
}

impl LocalGatewayToolMode {
    pub const EXPLICIT: [Self; 3] = [Self::SearchFetch, Self::FetchOnly, Self::GatewayOnly];

    pub fn parse_cli(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "search-fetch" => Some(Self::SearchFetch),
            "fetch-only" => Some(Self::FetchOnly),
            "gateway-only" => Some(Self::GatewayOnly),
            _ => None,
        }
    }

    pub fn as_cli_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::SearchFetch => "search-fetch",
            Self::FetchOnly => "fetch-only",
            Self::GatewayOnly => "gateway-only",
        }
    }

    pub fn shim_suffix(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::SearchFetch => Some("search-fetch"),
            Self::FetchOnly => Some("fetch-only"),
            Self::GatewayOnly => Some("gateway"),
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn requires_tinyfish(self) -> bool {
        matches!(self, Self::SearchFetch | Self::FetchOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestedLocalGatewayMode {
    #[default]
    Omitted,
    Explicit(LocalGatewayToolMode),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    /// Auto-generated short ID (e.g. "mcp_a1b2").
    pub id: String,
    /// Claude Code MCP server name. This becomes the key under `mcpServers`.
    pub name: String,
    /// "stdio" (default), "http", "streamable-http", or "sse".
    #[serde(default = "default_mcp_server_type", rename = "type")]
    pub server_type: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub oauth: Option<serde_json::Value>,
    #[serde(default, rename = "headersHelper")]
    pub headers_helper: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default, rename = "alwaysLoad")]
    pub always_load: Option<bool>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

pub(super) fn default_mcp_server_type() -> String {
    "stdio".to_string()
}

#[derive(Debug, Clone, Default)]
pub struct McpServerInput {
    pub name: String,
    pub server_type: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    pub oauth: Option<serde_json::Value>,
    pub headers_helper: Option<String>,
    pub timeout: Option<u64>,
    pub always_load: Option<bool>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct McpServerUpdate {
    pub name: Option<String>,
    pub server_type: Option<String>,
    pub command: Option<Option<String>>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub cwd: Option<Option<String>>,
    pub url: Option<Option<String>>,
    pub headers: Option<HashMap<String, String>>,
    pub oauth: Option<Option<serde_json::Value>>,
    pub headers_helper: Option<Option<String>>,
    pub timeout: Option<Option<u64>>,
    pub always_load: Option<Option<bool>>,
    pub disabled: Option<Option<bool>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct McpSmartPasteImportResult {
    pub imported: Vec<McpServer>,
    pub skipped_existing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginMarketplaceSourceKind {
    GitHub,
    Git,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginMarketplace {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub source_kind: PluginMarketplaceSourceKind,
    pub locator: String,
    #[serde(default)]
    pub canonical_url: Option<String>,
    #[serde(default)]
    pub added_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub id: String,
    pub plugin_name: String,
    pub marketplace_name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub source_sha: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub default_enabled: Option<bool>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostedPluginCatalogItem {
    pub id: String,
    pub marketplace_name: String,
    pub plugin_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub default_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledPluginDetails {
    pub installed: InstalledPlugin,
    pub linked_profiles: Vec<String>,
    pub install_root: PathBuf,
    pub exists: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    Ok,
    Warn,
    Error,
}

impl DiagnosticLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticLevel::Ok => "ok",
            DiagnosticLevel::Warn => "warn",
            DiagnosticLevel::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticItem {
    pub level: DiagnosticLevel,
    pub area: String,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DoctorReport {
    pub items: Vec<DiagnosticItem>,
}

impl DoctorReport {
    pub fn error_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.level == DiagnosticLevel::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.level == DiagnosticLevel::Warn)
            .count()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigInspection {
    pub base_dir: PathBuf,
    pub registry_path: PathBuf,
    pub profiles_dir: PathBuf,
    pub generated_root: PathBuf,
    pub plugins_root: PathBuf,
    pub runtime_root: PathBuf,
    pub profiles: usize,
    pub lightweight_profiles: usize,
    pub full_profiles: usize,
    pub providers: usize,
    pub provider_keys: usize,
    pub mcp_servers: usize,
    pub linked_mcp_refs: usize,
    pub plugin_marketplaces: usize,
    pub installed_plugins: usize,
    pub linked_plugin_refs: usize,
    pub generated_mcp_plugins: usize,
    pub generated_tinyfish_plugins: usize,
    pub generated_prompts: usize,
    pub runtime_sessions: usize,
    pub active_runtime_sessions: usize,
    pub stale_runtime_sessions: usize,
    pub allow_local_runtime_hot_switch: bool,
    pub cmd_shims_dir: Option<PathBuf>,
    pub shell_shims_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalSettings {
    #[serde(default)]
    pub allow_local_runtime_hot_switch: bool,
    #[serde(default = "default_plugin_github_mirror_base_url")]
    pub plugin_github_mirror_base_url: Option<String>,
}

pub fn default_plugin_github_mirror_base_url() -> Option<String> {
    Some("https://wget.la".to_string())
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            allow_local_runtime_hot_switch: false,
            plugin_github_mirror_base_url: default_plugin_github_mirror_base_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpValidationIssue {
    pub level: DiagnosticLevel,
    pub server_id: String,
    pub server_name: String,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatuslineInfo {
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub profile_alias: Option<String>,
    pub kind: Option<ProfileKind>,
    pub provider_name: Option<String>,
    pub provider_id: Option<String>,
    pub key_name: Option<String>,
    pub key_id: Option<String>,
    pub mcp_servers: usize,
    pub mcp_names: Vec<String>,
    pub plugins: usize,
    pub plugin_names: Vec<String>,
    pub project_marker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBundle {
    pub schema: String,
    pub exported_at: DateTime<Utc>,
    pub profiles: Vec<Profile>,
    pub providers: Vec<Provider>,
    pub mcp_servers: Vec<McpServer>,
    #[serde(default)]
    pub plugin_marketplaces: Vec<PluginMarketplace>,
    #[serde(default)]
    pub installed_plugins: Vec<InstalledPlugin>,
    #[serde(default)]
    pub settings: Option<GlobalSettings>,
    pub secrets_included: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigImportSummary {
    pub profiles_added: usize,
    pub profiles_updated: usize,
    pub profiles_conflicted: usize,
    pub providers_added: usize,
    pub providers_updated: usize,
    pub providers_conflicted: usize,
    pub mcp_servers_added: usize,
    pub mcp_servers_updated: usize,
    pub mcp_servers_conflicted: usize,
    pub plugin_marketplaces_added: usize,
    pub plugin_marketplaces_updated: usize,
    pub plugin_marketplaces_conflicted: usize,
    pub installed_plugins_added: usize,
    pub installed_plugins_updated: usize,
    pub installed_plugins_conflicted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigImportPlan {
    pub summary: ConfigImportSummary,
    pub profiles_add: Vec<String>,
    pub profiles_update: Vec<String>,
    pub profiles_conflict: Vec<String>,
    pub providers_add: Vec<String>,
    pub providers_update: Vec<String>,
    pub providers_conflict: Vec<String>,
    pub mcp_servers_add: Vec<String>,
    pub mcp_servers_update: Vec<String>,
    pub mcp_servers_conflict: Vec<String>,
    pub plugin_marketplaces_add: Vec<String>,
    pub plugin_marketplaces_update: Vec<String>,
    pub plugin_marketplaces_conflict: Vec<String>,
    pub installed_plugins_add: Vec<String>,
    pub installed_plugins_update: Vec<String>,
    pub installed_plugins_conflict: Vec<String>,
    pub secrets_included: bool,
}

impl ConfigImportPlan {
    pub fn conflict_count(&self) -> usize {
        self.summary.profiles_conflicted
            + self.summary.providers_conflicted
            + self.summary.mcp_servers_conflicted
            + self.summary.plugin_marketplaces_conflicted
            + self.summary.installed_plugins_conflicted
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigBundleValidation {
    pub schema: String,
    pub profiles: usize,
    pub providers: usize,
    pub mcp_servers: usize,
    pub plugin_marketplaces: usize,
    pub installed_plugins: usize,
    pub secrets_included: bool,
    pub issues: Vec<DiagnosticItem>,
}

impl ConfigBundleValidation {
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|item| item.level == DiagnosticLevel::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|item| item.level == DiagnosticLevel::Warn)
            .count()
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ShimRecoveryPlan {
    pub shim_dir: PathBuf,
    pub files_scanned: usize,
    pub files_recoverable: usize,
    pub files_skipped: usize,
    pub profiles_added: usize,
    pub profiles_updated: usize,
    pub profiles_conflicted: usize,
    pub providers_added: usize,
    pub provider_keys_added: usize,
    pub provider_keys_reused: usize,
    pub profiles_add: Vec<String>,
    pub profiles_update: Vec<String>,
    pub profiles_conflict: Vec<String>,
    pub providers_add: Vec<String>,
    pub provider_keys_add: Vec<String>,
    pub warnings: Vec<String>,
}

impl ShimRecoveryPlan {
    pub fn conflict_count(&self) -> usize {
        self.profiles_conflicted
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShimRecoverySummary {
    #[serde(flatten)]
    pub plan: ShimRecoveryPlan,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AuthMigrationPlan {
    pub local_files_scanned: usize,
    pub remote_files_scanned: usize,
    pub files_to_update_count: usize,
    pub files_already_ok: usize,
    pub files_missing: usize,
    pub files_skipped: usize,
    pub helpers_overwritten: usize,
    pub files_to_update: Vec<String>,
    pub helper_overwrite: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthMigrationSummary {
    #[serde(flatten)]
    pub plan: AuthMigrationPlan,
    pub backup_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderKey {
    /// Auto-generated short ID (e.g. "key_a1b2")
    pub id: String,
    /// Human-readable name (e.g. "My Personal Key", "Team Shared Key")
    #[serde(default)]
    pub name: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provider {
    /// Auto-generated short ID (e.g. "prov_a1b2")
    pub id: String,
    /// Human-readable name (editable, e.g. "Anthropic Official", "My Relay")
    #[serde(default)]
    pub name: String,
    pub base_url: String,
    /// Named API keys. Keyed by key `id`.
    #[serde(default)]
    pub keys: HashMap<String, ProviderKey>,
    /// DEPRECATED: use `keys` map instead. Present only during migration.
    #[serde(default, skip_serializing)]
    pub(super) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LightweightEnv {
    pub auth_token: Option<String>,
    pub base_url: Option<String>,
    pub default_opus_model: Option<String>,
    pub default_sonnet_model: Option<String>,
    pub default_haiku_model: Option<String>,
    pub model: Option<String>,
    pub subagent_model: Option<String>,
    #[serde(default)]
    pub extras: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Stable unique identifier (UUID v4).
    #[serde(default)]
    pub id: String,
    /// Display name — any characters (Chinese, spaces, etc.).
    pub name: String,
    /// Short CLI-friendly name (alphanumeric, hyphens, underscores). Optional.
    #[serde(default)]
    pub alias: Option<String>,
    pub added: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    #[serde(default)]
    pub kind: ProfileKind,
    #[serde(default)]
    pub env: Option<LightweightEnv>,
    /// Extra CLI args to pass to `claude` on launch (e.g. `--dangerously-skip-permissions`).
    #[serde(default)]
    pub launch_args: Option<Vec<String>>,
    /// Reference to a shared provider (replaces inline base_url + auth_token).
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Reference to a specific key within the provider.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Selected MCP servers for lightweight profiles. Stored as MCP server IDs.
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    /// Linked hosted plugins for this profile. Stored as installed plugin ids.
    #[serde(default)]
    pub plugin_ids: Vec<String>,
}

impl Profile {
    /// Filesystem-safe directory name for this profile's data.
    pub fn dir_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    /// Keyed by MCP server `id`.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServer>,
    /// Keyed by marketplace `name`.
    #[serde(default)]
    pub plugin_marketplaces: HashMap<String, PluginMarketplace>,
    /// Keyed by installed plugin id (`plugin@marketplace`).
    #[serde(default)]
    pub installed_plugins: HashMap<String, InstalledPlugin>,
    /// Keyed by provider `id`.
    #[serde(default)]
    pub providers: HashMap<String, Provider>,
    /// Keyed by profile `id` (UUID).
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
    #[serde(default)]
    pub settings: GlobalSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeSessionStatus {
    #[default]
    Active,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSessionState {
    pub schema: String,
    pub session_id: String,
    #[serde(default)]
    pub status: RuntimeSessionStatus,
    pub pid: Option<u32>,
    pub process_started_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub profile_id: String,
    pub profile_name: String,
    #[serde(default)]
    pub profile_alias: Option<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
    pub auth_token: String,
    pub base_url: String,
    #[serde(default)]
    pub default_opus_model: Option<String>,
    #[serde(default)]
    pub default_sonnet_model: Option<String>,
    #[serde(default)]
    pub default_haiku_model: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub subagent_model: Option<String>,
    #[serde(default)]
    pub extras: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSessionInfo {
    pub state: RuntimeSessionState,
    pub state_path: PathBuf,
    pub settings_path: PathBuf,
    pub active: bool,
    pub stale_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeGcSummary {
    pub scanned: usize,
    pub removed: usize,
    pub kept: usize,
}
