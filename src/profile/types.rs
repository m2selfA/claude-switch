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
    pub profiles: usize,
    pub lightweight_profiles: usize,
    pub full_profiles: usize,
    pub providers: usize,
    pub provider_keys: usize,
    pub mcp_servers: usize,
    pub linked_mcp_refs: usize,
    pub generated_mcp_plugins: usize,
    pub generated_tinyfish_plugins: usize,
    pub generated_prompts: usize,
    pub cmd_shims_dir: Option<PathBuf>,
    pub shell_shims_dir: Option<PathBuf>,
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
    pub project_marker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBundle {
    pub schema: String,
    pub exported_at: DateTime<Utc>,
    pub profiles: Vec<Profile>,
    pub providers: Vec<Provider>,
    pub mcp_servers: Vec<McpServer>,
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
    pub secrets_included: bool,
}

impl ConfigImportPlan {
    pub fn conflict_count(&self) -> usize {
        self.summary.profiles_conflicted
            + self.summary.providers_conflicted
            + self.summary.mcp_servers_conflicted
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigBundleValidation {
    pub schema: String,
    pub profiles: usize,
    pub providers: usize,
    pub mcp_servers: usize,
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
    /// Keyed by provider `id`.
    #[serde(default)]
    pub providers: HashMap<String, Provider>,
    /// Keyed by profile `id` (UUID).
    pub profiles: HashMap<String, Profile>,
}
