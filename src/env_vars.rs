// Known Claude Code environment variables — canonical list.
//
// HOW TO UPDATE:
//   1. Fetch the latest docs:
//        tinyfish fetch content get --format markdown "https://code.claude.com/docs/en/env-vars"
//   2. Extract every `VARIABLE` from the markdown table and add to the appropriate
//      category below.
//   3. Run `cargo test` to verify.
//
// Last synced: 2026-05-11 from https://code.claude.com/docs/en/env-vars

/// All known Claude Code environment variables, grouped by category.
/// Used for autocomplete hints in the TUI and for env var forwarding at launch.
pub const KNOWN_ENV_VARS: &[(&str, &[&str])] = &[
    // ── API keys / auth ─────────────────────────────────────────────────────
    (
        "API keys / auth",
        &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_FOUNDRY_API_KEY",
            "ANTHROPIC_FOUNDRY_BASE_URL",
            "ANTHROPIC_FOUNDRY_RESOURCE",
            "ANTHROPIC_OAUTH_TOKEN", // alias for CLAUDE_CODE_OAUTH_TOKEN
            "AWS_BEARER_TOKEN_BEDROCK",
            "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
            "CLAUDE_CODE_OAUTH_SCOPES",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
            "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
            "CLAUDE_CODE_SKIP_MANTLE_AUTH",
            "CLAUDE_CODE_SKIP_VERTEX_AUTH",
        ],
    ),
    // ── Model configuration ─────────────────────────────────────────────────
    (
        "Model configuration",
        &[
            "ANTHROPIC_BETAS",
            "ANTHROPIC_CUSTOM_HEADERS",
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL_AWS_REGION",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "FALLBACK_FOR_ALL_PRIMARY_MODELS",
        ],
    ),
    // ── Providers ───────────────────────────────────────────────────────────
    (
        "Providers / endpoints",
        &[
            "ANTHROPIC_BEDROCK_BASE_URL",
            "ANTHROPIC_BEDROCK_MANTLE_BASE_URL",
            "ANTHROPIC_BEDROCK_SERVICE_TIER",
            "ANTHROPIC_VERTEX_BASE_URL",
            "ANTHROPIC_VERTEX_PROJECT_ID",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_MANTLE",
            "CLAUDE_CODE_USE_VERTEX",
            "VERTEX_REGION_CLAUDE_3_5_HAIKU",
            "VERTEX_REGION_CLAUDE_3_5_SONNET",
            "VERTEX_REGION_CLAUDE_3_7_SONNET",
            "VERTEX_REGION_CLAUDE_4_0_OPUS",
            "VERTEX_REGION_CLAUDE_4_0_SONNET",
            "VERTEX_REGION_CLAUDE_4_1_OPUS",
            "VERTEX_REGION_CLAUDE_4_5_OPUS",
            "VERTEX_REGION_CLAUDE_4_5_SONNET",
            "VERTEX_REGION_CLAUDE_4_6_OPUS",
            "VERTEX_REGION_CLAUDE_4_6_SONNET",
            "VERTEX_REGION_CLAUDE_4_7_OPUS",
            "VERTEX_REGION_CLAUDE_HAIKU_4_5",
        ],
    ),
    // ── Timeouts / limits ───────────────────────────────────────────────────
    (
        "Timeouts / limits",
        &[
            "API_TIMEOUT_MS",
            "BASH_DEFAULT_TIMEOUT_MS",
            "BASH_MAX_OUTPUT_LENGTH",
            "BASH_MAX_TIMEOUT_MS",
            "CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS",
            "CLAUDE_CODE_GLOB_TIMEOUT_SECONDS",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            "CLAUDE_CODE_MAX_RETRIES",
            "CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY",
            "CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS",
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
            "MAX_MCP_OUTPUT_TOKENS",
            "MAX_STRUCTURED_OUTPUT_RETRIES",
            "MAX_THINKING_TOKENS",
            "MCP_CONNECT_TIMEOUT_MS",
            "MCP_TIMEOUT",
            "MCP_TOOL_TIMEOUT",
            "TASK_MAX_OUTPUT_LENGTH",
        ],
    ),
    // ── Shell / execution ───────────────────────────────────────────────────
    (
        "Shell / execution",
        &[
            "CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR",
            "CLAUDE_CODE_SHELL",
            "CLAUDE_CODE_SHELL_PREFIX",
            "CLAUDE_CODE_TMPDIR",
            "CLAUDE_ENV_FILE",
            "CLAUDE_CODE_GIT_BASH_PATH",
            "CLAUDE_CODE_USE_POWERSHELL_TOOL",
        ],
    ),
    // ── UI / display ────────────────────────────────────────────────────────
    (
        "UI / display",
        &[
            "CLAUDE_CODE_ACCESSIBILITY",
            "CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN",
            "CLAUDE_CODE_DISABLE_MOUSE",
            "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
            "CLAUDE_CODE_DISABLE_VIRTUAL_SCROLL",
            "CLAUDE_CODE_FORCE_SYNC_OUTPUT",
            "CLAUDE_CODE_HIDE_CWD",
            "CLAUDE_CODE_NATIVE_CURSOR",
            "CLAUDE_CODE_NO_FLICKER",
            "CLAUDE_CODE_SCROLL_SPEED",
            "CLAUDE_CODE_SYNTAX_HIGHLIGHT",
            "CLAUDE_CODE_TMUX_TRUECOLOR",
            "IS_DEMO",
        ],
    ),
    // ── Feature flags / toggles ─────────────────────────────────────────────
    (
        "Feature flags / toggles",
        &[
            "CCR_FORCE_BUNDLE",
            "CLAUDE_AGENT_SDK_DISABLE_BUILTIN_AGENTS",
            "CLAUDE_AGENT_SDK_MCP_NO_PREFIX",
            "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
            "CLAUDE_AUTO_BACKGROUND_TASKS",
            "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD",
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            "CLAUDE_CODE_AUTO_CONNECT_IDE",
            "CLAUDE_CODE_DISABLE_1M_CONTEXT",
            "CLAUDE_CODE_DISABLE_ADAPTIVE_THINKING",
            "CLAUDE_CODE_DISABLE_ATTACHMENTS",
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY",
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
            "CLAUDE_CODE_DISABLE_CLAUDE_MDS",
            "CLAUDE_CODE_DISABLE_CRON",
            "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
            "CLAUDE_CODE_DISABLE_FAST_MODE",
            "CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY",
            "CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING",
            "CLAUDE_CODE_DISABLE_GIT_INSTRUCTIONS",
            "CLAUDE_CODE_DISABLE_LEGACY_MODEL_REMAP",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK",
            "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL",
            "CLAUDE_CODE_DISABLE_POLICY_SKILLS",
            "CLAUDE_CODE_DISABLE_THINKING",
            "CLAUDE_CODE_EFFORT_LEVEL",
            "CLAUDE_CODE_ENABLE_AWAY_SUMMARY",
            "CLAUDE_CODE_ENABLE_BACKGROUND_PLUGIN_REFRESH",
            "CLAUDE_CODE_ENABLE_FINE_GRAINED_TOOL_STREAMING",
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
            "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION",
            "CLAUDE_CODE_ENABLE_TASKS",
            "CLAUDE_CODE_ENABLE_TELEMETRY",
            "CLAUDE_CODE_EXIT_AFTER_STOP_DELAY",
            "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
            "CLAUDE_CODE_EXTRA_BODY",
            "CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS",
            "CLAUDE_CODE_FORK_SUBAGENT",
            "CLAUDE_CODE_GLOB_HIDDEN",
            "CLAUDE_CODE_GLOB_NO_IGNORE",
            "CLAUDE_CODE_IDE_HOST_OVERRIDE",
            "CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL",
            "CLAUDE_CODE_IDE_SKIP_VALID_CHECK",
            "CLAUDE_CODE_MCP_ALLOWLIST_ENV",
            "CLAUDE_CODE_NEW_INIT",
            "CLAUDE_CODE_PACKAGE_MANAGER_AUTO_UPDATE",
            "CLAUDE_CODE_PERFORCE_MODE",
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
            "CLAUDE_CODE_PROXY_RESOLVES_HOSTS",
            "CLAUDE_CODE_RESUME_INTERRUPTED_TURN",
            "CLAUDE_CODE_SCRIPT_CAPS",
            "CLAUDE_CODE_SIMPLE",
            "CLAUDE_CODE_SIMPLE_SYSTEM_PROMPT",
            "CLAUDE_CODE_SKIP_PROMPT_HISTORY",
            "CLAUDE_CODE_SUBPROCESS_ENV_SCRUB",
            "CLAUDE_CODE_SYNC_PLUGIN_INSTALL",
            "CLAUDE_CODE_SYNC_PLUGIN_INSTALL_TIMEOUT_MS",
            "CLAUDE_CODE_TASK_LIST_ID",
            "CLAUDE_CODE_USE_NATIVE_FILE_SEARCH",
            "DISABLE_AUTOUPDATER",
            "DISABLE_AUTO_COMPACT",
            "DISABLE_COMPACT",
            "DISABLE_COST_WARNINGS",
            "DISABLE_DOCTOR_COMMAND",
            "DISABLE_ERROR_REPORTING",
            "DISABLE_EXTRA_USAGE_COMMAND",
            "DISABLE_FEEDBACK_COMMAND",
            "DISABLE_GROWTHBOOK",
            "DISABLE_INSTALLATION_CHECKS",
            "DISABLE_INSTALL_GITHUB_APP_COMMAND",
            "DISABLE_INTERLEAVED_THINKING",
            "DISABLE_LOGIN_COMMAND",
            "DISABLE_LOGOUT_COMMAND",
            "DISABLE_PROMPT_CACHING",
            "DISABLE_PROMPT_CACHING_HAIKU",
            "DISABLE_PROMPT_CACHING_OPUS",
            "DISABLE_PROMPT_CACHING_SONNET",
            "DISABLE_TELEMETRY",
            "DISABLE_UPDATES",
            "DISABLE_UPGRADE_COMMAND",
            "DO_NOT_TRACK",
            "ENABLE_CLAUDEAI_MCP_SERVERS",
            "ENABLE_PROMPT_CACHING_1H",
            "ENABLE_PROMPT_CACHING_1H_BEDROCK",
            "ENABLE_TOOL_SEARCH",
            "FORCE_AUTOUPDATE_PLUGINS",
            "FORCE_PROMPT_CACHING_5M",
            "USE_BUILTIN_RIPGREP",
        ],
    ),
    // ── Plugins ─────────────────────────────────────────────────────────────
    (
        "Plugins",
        &[
            "CLAUDE_CODE_PLUGIN_CACHE_DIR",
            "CLAUDE_CODE_PLUGIN_GIT_TIMEOUT_MS",
            "CLAUDE_CODE_PLUGIN_KEEP_MARKETPLACE_ON_FAILURE",
            "CLAUDE_CODE_PLUGIN_SEED_DIR",
        ],
    ),
    // ── MCP ─────────────────────────────────────────────────────────────────
    (
        "MCP",
        &[
            "MCP_CLIENT_SECRET",
            "MCP_CONNECTION_NONBLOCKING",
            "MCP_OAUTH_CALLBACK_PORT",
            "MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE",
            "MCP_SERVER_CONNECTION_BATCH_SIZE",
        ],
    ),
    // ── Observability (OTel) ────────────────────────────────────────────────
    (
        "Observability (OTel)",
        &[
            "CLAUDE_CODE_DEBUG_LOGS_DIR",
            "CLAUDE_CODE_DEBUG_LOG_LEVEL",
            "CLAUDE_CODE_OTEL_FLUSH_TIMEOUT_MS",
            "CLAUDE_CODE_OTEL_HEADERS_HELPER_DEBOUNCE_MS",
            "CLAUDE_CODE_OTEL_SHUTDOWN_TIMEOUT_MS",
            "OTEL_LOG_RAW_API_BODIES",
            "OTEL_LOG_TOOL_CONTENT",
            "OTEL_LOG_TOOL_DETAILS",
            "OTEL_LOG_USER_PROMPTS",
            "OTEL_METRICS_INCLUDE_ACCOUNT_UUID",
            "OTEL_METRICS_INCLUDE_SESSION_ID",
            "OTEL_METRICS_INCLUDE_VERSION",
        ],
    ),
    // ── Network / TLS ───────────────────────────────────────────────────────
    (
        "Network / TLS",
        &[
            "CLAUDE_CODE_CERT_STORE",
            "CLAUDE_CODE_CLIENT_CERT",
            "CLAUDE_CODE_CLIENT_KEY",
            "CLAUDE_CODE_CLIENT_KEY_PASSPHRASE",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
        ],
    ),
    // ── Session / runtime ───────────────────────────────────────────────────
    (
        "Session / runtime",
        &[
            "CLAUDECODE",
            "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
            "CLAUDE_CODE_REMOTE",
            "CLAUDE_CODE_REMOTE_SESSION_ID",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_TEAM_NAME",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_EFFORT",
            "CLAUDE_ENABLE_BYTE_WATCHDOG",
            "CLAUDE_ENABLE_STREAM_WATCHDOG",
            "CLAUDE_REMOTE_CONTROL_SESSION_NAME_PREFIX",
            "SLASH_COMMAND_TOOL_CHAR_BUDGET",
        ],
    ),
];

/// Return a flat unsorted slice of every known variable name.
/// Useful for iterating over all known keys without navigating categories.
pub fn all_var_names() -> &'static [&'static str] {
    // Lazy static: compute on first call, then return cached slice.
    use std::sync::OnceLock;
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut v: Vec<&str> = Vec::new();
            for (_, vars) in KNOWN_ENV_VARS {
                v.extend_from_slice(vars);
            }
            v.sort_unstable();
            v.dedup();
            v
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_env_vars() {
        let mut seen = HashSet::new();
        for (cat, vars) in KNOWN_ENV_VARS {
            for v in *vars {
                assert!(
                    seen.insert(v),
                    "duplicate env var '{v}' in category '{cat}'"
                );
            }
        }
    }

    #[test]
    fn all_var_names_returns_all() {
        let flat: HashSet<_> = KNOWN_ENV_VARS
            .iter()
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let names: HashSet<_> = all_var_names().iter().copied().collect();
        assert_eq!(flat, names);
    }
}
