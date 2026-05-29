use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use ureq::RequestExt;
use uuid::Uuid;

const KNOWN_COMPAT_SUFFIXES: &[&str] = &[
    "/compatible-mode/v1", // DashScope style
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/v1/messages", // full message endpoint
    "/api/coding",
    "/claudecode",
    "/step_plan",
    "/anthropic",
    "/messages", // bare message endpoint
    "/api/v1",   // new-api/one-api style
    "/coding",
    "/claude",
    "/api", // OpenRouter style
    "/v1",  // LM Studio bare /v1
];

const API_TEST_TIMEOUT_SECS: u64 = 8;
const CLAUDE_SWITCH_HOME_ENV: &str = "CLAUDE_SWITCH_HOME";

const NATIVE_SEARCH_URLS: &[&str] = &[
    "https://api.deepseek.com/anthropic", // DeepSeek: has search, lacks fetch
    "https://a-ocnfniawgw.cn-shanghai.fcapp.run", // AnyRouter: has both
    "https://anyrouter.top",              // AnyRouter: has both
    "https://api.anthropic.com",          // Claude official: has both
];

const NATIVE_FETCH_URLS: &[&str] = &[
    "https://a-ocnfniawgw.cn-shanghai.fcapp.run",
    "https://anyrouter.top",
    "https://api.anthropic.com",
];

fn url_matches(url: &str, known: &[&str]) -> bool {
    let Some(u) = canonical_url_for_match(url) else {
        return false;
    };
    known.iter().any(|known_url| {
        canonical_url_for_match(known_url)
            .is_some_and(|k| u == k || u.starts_with(&format!("{k}/")))
    })
}

fn canonical_url_for_match(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let (scheme, rest) = trimmed.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let path_and_more = &rest[authority_end..];
    let host_port = authority.rsplit('@').next()?;
    let (host, port) = split_host_port(host_port);
    if host.is_empty() {
        return None;
    }

    let host = host.to_ascii_lowercase();
    let authority = match port {
        Some("") => host,
        Some(port) if !port.chars().all(|ch| ch.is_ascii_digit()) => return None,
        Some(port) if is_default_port(&scheme, port) => host,
        Some(port) => format!("{host}:{port}"),
        None => host,
    };

    Some(format!("{scheme}://{authority}{path_and_more}"))
}

fn split_host_port(authority: &str) -> (&str, Option<&str>) {
    if let Some(close_bracket) = authority.find(']')
        && authority.starts_with('[')
    {
        let host = &authority[..=close_bracket];
        let rest = &authority[close_bracket + 1..];
        return rest
            .strip_prefix(':')
            .map_or((host, None), |port| (host, Some(port)));
    }

    if let Some((host, port)) = authority.rsplit_once(':')
        && !host.contains(':')
    {
        return (host, Some(port));
    }

    (authority, None)
}

fn is_default_port(scheme: &str, port: &str) -> bool {
    matches!((scheme, port), ("http", "80") | ("https", "443"))
}

const TINYFISH_FULL_PROMPT_BASH: &str = "\
## WebSearch or Search\n\
When looking to use Search or WebSearch, always use the TinyFish Web Search tool, \
run via the Bash tool: `tinyfish search query \"<query>\"`\n\
Only fall back to native Search when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\n\
\n\
## WebFetch or Fetch\n\
When looking to use Fetch or WebFetch, always use the TinyFish Fetch tool, \
run via the Bash tool: `tinyfish fetch content get \"<url>\"`\n\
Only fall back to native Fetch when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

const TINYFISH_FULL_PROMPT_POWERSHELL: &str = "\
## WebSearch or Search\n\
When looking to use Search or WebSearch, always use the TinyFish Web Search tool, \
run via the PowerShell tool: `tinyfish search query \"<query>\"`\n\
Only fall back to native Search when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\n\
\n\
## WebFetch or Fetch\n\
When looking to use Fetch or WebFetch, always use the TinyFish Fetch tool, \
run via the PowerShell tool: `tinyfish fetch content get \"<url>\"`\n\
Only fall back to native Fetch when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

const TINYFISH_FETCH_ONLY_PROMPT_BASH: &str = "\
## WebFetch or Fetch\n\
When looking to use Fetch or WebFetch, always use the TinyFish Fetch tool, \
run via the Bash tool: `tinyfish fetch content get \"<url>\"`\n\
Only fall back to native Fetch when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

const TINYFISH_FETCH_ONLY_PROMPT_POWERSHELL: &str = "\
## WebFetch or Fetch\n\
When looking to use Fetch or WebFetch, always use the TinyFish Fetch tool, \
run via the PowerShell tool: `tinyfish fetch content get \"<url>\"`\n\
Only fall back to native Fetch when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

const TINYFISH_SEARCH_ONLY_PROMPT_BASH: &str = "\
## WebSearch or Search\n\
When looking to use Search or WebSearch, always use the TinyFish Web Search tool, \
run via the Bash tool: `tinyfish search query \"<query>\"`\n\
Only fall back to native Search when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

const TINYFISH_SEARCH_ONLY_PROMPT_POWERSHELL: &str = "\
## WebSearch or Search\n\
When looking to use Search or WebSearch, always use the TinyFish Web Search tool, \
run via the PowerShell tool: `tinyfish search query \"<query>\"`\n\
Only fall back to native Search when you get rate limited by tinyfish \
(for a minute, then you can try using tinyfish again)\
";

fn tinyfish_full_hooks(tool_shell: TinyfishToolShell) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "WebSearch",
                    "hooks": [tinyfish_command_hook(
                        tool_shell,
                        "PreToolUse",
                        Some("allow"),
                        TINYFISH_WEBSEARCH_PRETOOL_CONTEXT,
                    )]
                },
                {
                    "matcher": "WebFetch",
                    "hooks": [tinyfish_command_hook(
                        tool_shell,
                        "PreToolUse",
                        Some("allow"),
                        TINYFISH_WEBFETCH_PRETOOL_CONTEXT,
                    )]
                }
            ],
            "SubagentStart": [{
                "hooks": [tinyfish_command_hook(
                    tool_shell,
                    "SubagentStart",
                    None,
                    tinyfish_full_subagent_context(tool_shell),
                )]
            }]
        }
    })
}

fn tinyfish_fetch_only_hooks(tool_shell: TinyfishToolShell) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "WebFetch",
                "hooks": [tinyfish_command_hook(
                    tool_shell,
                    "PreToolUse",
                    Some("allow"),
                    TINYFISH_WEBFETCH_PRETOOL_CONTEXT,
                )]
            }],
            "SubagentStart": [{
                "hooks": [tinyfish_command_hook(
                    tool_shell,
                    "SubagentStart",
                    None,
                    tinyfish_fetch_only_subagent_context(tool_shell),
                )]
            }]
        }
    })
}

fn tinyfish_search_only_hooks(tool_shell: TinyfishToolShell) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "WebSearch",
                "hooks": [tinyfish_command_hook(
                    tool_shell,
                    "PreToolUse",
                    Some("allow"),
                    TINYFISH_WEBSEARCH_PRETOOL_CONTEXT,
                )]
            }],
            "SubagentStart": [{
                "hooks": [tinyfish_command_hook(
                    tool_shell,
                    "SubagentStart",
                    None,
                    tinyfish_search_only_subagent_context(tool_shell),
                )]
            }]
        }
    })
}

const TINYFISH_WEBSEARCH_PRETOOL_CONTEXT: &str =
    "Follow the instructions about which search provider to use, listed in your Claude.md file";
const TINYFISH_WEBFETCH_PRETOOL_CONTEXT: &str =
    "Follow the instructions about which fetch provider to use, listed in your Claude.md file";
const TINYFISH_CONTROL_EXTRA_KEY: &str = "CLAUDE_SWITCH_TINYFISH";
const TINYFISH_BASH_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)"];
const TINYFISH_WINDOWS_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)", "PowerShell(tinyfish:*)"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TinyfishToolShell {
    Bash,
    PowerShell,
}

fn tinyfish_full_subagent_context(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => {
            "\
The user prefers TinyFish for web operations. Use `tinyfish search query \"<query>\"` instead of \
WebSearch and `tinyfish fetch content get \"<url>\"` instead of WebFetch. Run them via the Bash \
tool. Native WebSearch and WebFetch are fallbacks only - use them when TinyFish is rate-limited or \
unavailable. A PreToolUse hook will remind you if you reach for native tools. Refer to your \
Claude.md for details"
        }
        TinyfishToolShell::PowerShell => {
            "\
The user prefers TinyFish for web operations. Use `tinyfish search query \"<QUERY>\"` instead of \
WebSearch and `tinyfish fetch content get \"<URL>\"` instead of WebFetch. Run them via the \
PowerShell tool. Native WebSearch and WebFetch are fallbacks only - use them when TinyFish is \
rate-limited or unavailable. A PreToolUse hook will remind you if you reach for native tools. \
Refer to your Claude.md for details"
        }
    }
}

fn tinyfish_fetch_only_subagent_context(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => {
            "\
The user prefers TinyFish for web fetch operations. Use `tinyfish fetch content get \"<url>\"` \
instead of WebFetch. Run it via the Bash tool. Native WebFetch is fallback only - use it when \
TinyFish is rate-limited or unavailable. A PreToolUse hook will remind you if you reach for native \
fetch. Refer to your Claude.md for details"
        }
        TinyfishToolShell::PowerShell => {
            "\
The user prefers TinyFish for web fetch operations. Use `tinyfish fetch content get \"<URL>\"` \
instead of WebFetch. Run it via the PowerShell tool. Native WebFetch is fallback only - use it when \
TinyFish is rate-limited or unavailable. A PreToolUse hook will remind you if you reach for native \
fetch. Refer to your Claude.md for details"
        }
    }
}

fn tinyfish_search_only_subagent_context(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => {
            "\
The user prefers TinyFish for web search operations. Use `tinyfish search query \"<query>\"` \
instead of WebSearch. Run it via the Bash tool. Native WebSearch is fallback only - use it when \
TinyFish is rate-limited or unavailable. A PreToolUse hook will remind you if you reach for native \
search. Refer to your Claude.md for details"
        }
        TinyfishToolShell::PowerShell => {
            "\
The user prefers TinyFish for web search operations. Use `tinyfish search query \"<QUERY>\"` \
instead of WebSearch. Run it via the PowerShell tool. Native WebSearch is fallback only - use it \
when TinyFish is rate-limited or unavailable. A PreToolUse hook will remind you if you reach for \
native search. Refer to your Claude.md for details"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TinyfishMode {
    None,
    SearchOnly,
    FetchOnly,
    Full,
}

fn tinyfish_mode(base_url: &str) -> TinyfishMode {
    if base_url.trim().is_empty() {
        return TinyfishMode::None;
    }
    let has_search = url_matches(base_url, NATIVE_SEARCH_URLS);
    let has_fetch = url_matches(base_url, NATIVE_FETCH_URLS);
    tinyfish_mode_for_capabilities(has_search, has_fetch)
}

fn tinyfish_mode_for_capabilities(has_search: bool, has_fetch: bool) -> TinyfishMode {
    match (has_search, has_fetch) {
        (true, true) => TinyfishMode::None,
        (true, false) => TinyfishMode::FetchOnly,
        (false, true) => TinyfishMode::SearchOnly,
        (false, false) => TinyfishMode::Full,
    }
}

fn native_tinyfish_tool_shell() -> TinyfishToolShell {
    if cfg!(windows) {
        TinyfishToolShell::PowerShell
    } else {
        TinyfishToolShell::Bash
    }
}

fn tinyfish_prompt(mode: TinyfishMode, tool_shell: TinyfishToolShell) -> Option<&'static str> {
    match (mode, tool_shell) {
        (TinyfishMode::None, _) => None,
        (TinyfishMode::SearchOnly, TinyfishToolShell::Bash) => {
            Some(TINYFISH_SEARCH_ONLY_PROMPT_BASH)
        }
        (TinyfishMode::SearchOnly, TinyfishToolShell::PowerShell) => {
            Some(TINYFISH_SEARCH_ONLY_PROMPT_POWERSHELL)
        }
        (TinyfishMode::FetchOnly, TinyfishToolShell::Bash) => Some(TINYFISH_FETCH_ONLY_PROMPT_BASH),
        (TinyfishMode::FetchOnly, TinyfishToolShell::PowerShell) => {
            Some(TINYFISH_FETCH_ONLY_PROMPT_POWERSHELL)
        }
        (TinyfishMode::Full, TinyfishToolShell::Bash) => Some(TINYFISH_FULL_PROMPT_BASH),
        (TinyfishMode::Full, TinyfishToolShell::PowerShell) => {
            Some(TINYFISH_FULL_PROMPT_POWERSHELL)
        }
    }
}

fn tinyfish_permissions_allowlist(
    mode: TinyfishMode,
    tool_shell: TinyfishToolShell,
) -> Option<&'static [&'static str]> {
    if mode == TinyfishMode::None {
        return None;
    }
    Some(match tool_shell {
        TinyfishToolShell::Bash => TINYFISH_BASH_ALLOWLIST,
        TinyfishToolShell::PowerShell => TINYFISH_WINDOWS_ALLOWLIST,
    })
}

fn tinyfish_command_hook(
    tool_shell: TinyfishToolShell,
    hook_event_name: &str,
    permission_decision: Option<&str>,
    additional_context: &str,
) -> serde_json::Value {
    let mut hook = serde_json::json!({
        "type": "command",
        "command": tinyfish_hook_command(
            tool_shell,
            hook_event_name,
            permission_decision,
            additional_context,
        ),
    });
    if matches!(tool_shell, TinyfishToolShell::PowerShell) {
        hook["shell"] = serde_json::Value::String("powershell".to_string());
    }
    hook
}

fn escape_bash_single_quoted(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn tinyfish_hook_command(
    tool_shell: TinyfishToolShell,
    hook_event_name: &str,
    permission_decision: Option<&str>,
    additional_context: &str,
) -> String {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "hookEventName".into(),
        serde_json::Value::String(hook_event_name.to_string()),
    );
    if let Some(decision) = permission_decision {
        payload.insert(
            "permissionDecision".into(),
            serde_json::Value::String(decision.to_string()),
        );
    }
    payload.insert(
        "additionalContext".into(),
        serde_json::Value::String(additional_context.to_string()),
    );
    let json = serde_json::Value::Object(
        [(
            "hookSpecificOutput".to_string(),
            serde_json::Value::Object(payload),
        )]
        .into_iter()
        .collect(),
    )
    .to_string();
    match tool_shell {
        TinyfishToolShell::Bash => {
            format!("printf '%s\\n' '{}'", escape_bash_single_quoted(&json))
        }
        TinyfishToolShell::PowerShell => {
            format!("Write-Output '{}'", json.replace('\'', "''"))
        }
    }
}

fn tinyfish_command_succeeds_with_timeout(program: &str, args: &[&str], timeout: Duration) -> bool {
    let mut child = match std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn tinyfish_available() -> bool {
    tinyfish_command_succeeds_with_timeout("tinyfish", &["--version"], Duration::from_secs(2))
}

fn tinyfish_disabled_via_extra(extras: &[String]) -> bool {
    extras.iter().any(|extra| {
        let Some((key, value)) = extra.split_once('=') else {
            return false;
        };
        key.trim().eq_ignore_ascii_case(TINYFISH_CONTROL_EXTRA_KEY)
            && matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
    })
}

fn build_lightweight_env_entries(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
) -> Vec<(String, String)> {
    let mut entries = Vec::new();

    if let Some(t) = token {
        entries.push(("ANTHROPIC_AUTH_TOKEN".into(), t.to_string()));
    }
    if let Some(u) = url {
        entries.push(("ANTHROPIC_BASE_URL".into(), u.to_string()));
    }
    if let Some(ref m) = env.default_opus_model {
        entries.push(("ANTHROPIC_DEFAULT_OPUS_MODEL".into(), m.clone()));
    }
    if let Some(ref m) = env.default_sonnet_model {
        entries.push(("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), m.clone()));
    }
    if let Some(ref m) = env.default_haiku_model {
        entries.push(("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), m.clone()));
    }
    if let Some(ref m) = env.model {
        entries.push(("ANTHROPIC_MODEL".into(), m.clone()));
    }
    if let Some(ref m) = env.subagent_model {
        entries.push(("CLAUDE_CODE_SUBAGENT_MODEL".into(), m.clone()));
    }
    for extra in &env.extras {
        if let Some((k, v)) = extra.split_once('=') {
            if k.trim().eq_ignore_ascii_case(TINYFISH_CONTROL_EXTRA_KEY) {
                continue;
            }
            entries.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    entries
}

fn build_lightweight_env_map(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    build_lightweight_env_entries(env, token, url)
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect()
}

fn build_lightweight_settings(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    mode: TinyfishMode,
    tool_shell: TinyfishToolShell,
) -> serde_json::Map<String, serde_json::Value> {
    let mut settings = serde_json::Map::new();
    settings.insert(
        "env".into(),
        serde_json::Value::Object(build_lightweight_env_map(env, token, url)),
    );
    if let Some(allowlist) = tinyfish_permissions_allowlist(mode, tool_shell) {
        let allow: Vec<serde_json::Value> = allowlist
            .iter()
            .map(|tool| serde_json::Value::String((*tool).to_string()))
            .collect();
        settings.insert(
            "permissions".into(),
            serde_json::json!({
                "allow": allow,
            }),
        );
    }
    settings
}

fn build_lightweight_settings_env_prefix(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
) -> String {
    let entries = build_lightweight_env_entries(env, token, url);
    let mut prefix = String::from("{\"env\":{");
    for (idx, (key, value)) in entries.into_iter().enumerate() {
        if idx > 0 {
            prefix.push(',');
        }
        prefix.push_str(&serde_json::to_string(&key).unwrap_or_default());
        prefix.push(':');
        prefix.push_str(&serde_json::to_string(&value).unwrap_or_default());
    }
    prefix.push('}');
    prefix
}

fn tinyfish_plugin_hooks(mode: TinyfishMode, tool_shell: TinyfishToolShell) -> Option<String> {
    match mode {
        TinyfishMode::None => None,
        TinyfishMode::SearchOnly => Some(tinyfish_search_only_hooks(tool_shell).to_string()),
        TinyfishMode::FetchOnly => Some(tinyfish_fetch_only_hooks(tool_shell).to_string()),
        TinyfishMode::Full => Some(tinyfish_full_hooks(tool_shell).to_string()),
    }
}

fn tinyfish_plugin_manifest(mode: TinyfishMode) -> Option<String> {
    let (name, display_name, description) = match mode {
        TinyfishMode::None => return None,
        TinyfishMode::SearchOnly => (
            "tinyfish-search-only",
            "TinyFish Search Only",
            "Generated by claude-switch to inject TinyFish search hooks.",
        ),
        TinyfishMode::FetchOnly => (
            "tinyfish-fetch-only",
            "TinyFish Fetch Only",
            "Generated by claude-switch to inject TinyFish fetch hooks.",
        ),
        TinyfishMode::Full => (
            "tinyfish-full",
            "TinyFish Full",
            "Generated by claude-switch to inject TinyFish search and fetch hooks.",
        ),
    };
    Some(
        serde_json::json!({
            "name": name,
            "displayName": display_name,
            "description": description,
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LightweightRuntimeArtifacts {
    base_settings_json: String,
    tinyfish_mode: TinyfishMode,
    tinyfish_settings_json: Option<String>,
    tinyfish_prompt_text: Option<String>,
    tinyfish_plugin_hooks_json: Option<String>,
    tinyfish_plugin_manifest_json: Option<String>,
}

fn build_lightweight_runtime_artifacts(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    tool_shell: TinyfishToolShell,
) -> Result<LightweightRuntimeArtifacts> {
    let base_settings_json = serde_json::to_string(&build_lightweight_settings(
        env,
        token,
        url,
        TinyfishMode::None,
        tool_shell,
    ))
    .context("Failed to serialize base lightweight settings JSON")?;
    let tinyfish_mode = if tinyfish_disabled_via_extra(&env.extras) {
        TinyfishMode::None
    } else {
        tinyfish_mode(url.unwrap_or_default())
    };

    let (
        tinyfish_settings_json,
        tinyfish_prompt_text,
        tinyfish_plugin_hooks_json,
        tinyfish_plugin_manifest_json,
    ) = if tinyfish_mode == TinyfishMode::None {
        (None, None, None, None)
    } else {
        let settings_json = serde_json::to_string(&build_lightweight_settings(
            env,
            token,
            url,
            tinyfish_mode,
            tool_shell,
        ))
        .context("Failed to serialize TinyFish lightweight settings JSON")?;
        let prompt = tinyfish_prompt(tinyfish_mode, tool_shell)
            .map(std::string::ToString::to_string)
            .context("TinyFish prompt missing for non-native mode")?;
        let plugin_hooks = tinyfish_plugin_hooks(tinyfish_mode, tool_shell)
            .context("TinyFish plugin hooks missing for non-native mode")?;
        let plugin_manifest = tinyfish_plugin_manifest(tinyfish_mode)
            .context("TinyFish plugin manifest missing for non-native mode")?;
        (
            Some(settings_json),
            Some(prompt),
            Some(plugin_hooks),
            Some(plugin_manifest),
        )
    };

    Ok(LightweightRuntimeArtifacts {
        base_settings_json,
        tinyfish_mode,
        tinyfish_settings_json,
        tinyfish_prompt_text,
        tinyfish_plugin_hooks_json,
        tinyfish_plugin_manifest_json,
    })
}

fn escape_cmd_json_fragment(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len() * 2);
    for ch in fragment.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '^' => out.push_str("^^"),
            _ => out.push(ch),
        }
    }
    out
}

fn assign_cmd_json_var(lines: &mut Vec<String>, var_name: &str, json: &str) {
    let escaped = escape_cmd_json_fragment(json);
    lines.push(format!("set \"{var_name}={escaped}\""));
}

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

fn default_mcp_server_type() -> String {
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
    api_key: String,
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

struct RecoveredShimProfile {
    file_name: String,
    name: String,
    alias: String,
    token: String,
    base_url: String,
    env: LightweightEnv,
    launch_args: Option<Vec<String>>,
}

struct ShimRecoveryState {
    plan: ShimRecoveryPlan,
    registry: Registry,
}

// ── ProfileManager ────────────────────────────────────────────────────────────

const CMD_MARKER: &str = ":: Generated by cswitch (claude-switch) — do not edit manually";
const SH_MARKER: &str = "# Generated by cswitch (claude-switch) — do not edit manually";

pub struct ProfileManager {
    pub profiles_dir: PathBuf,
    registry_path: PathBuf,
}

impl ProfileManager {
    pub fn new() -> Result<Self> {
        let home = Self::home_dir()?;
        Self::new_in_home_dir(&home)
    }

    fn new_in_home_dir(home: &Path) -> Result<Self> {
        let base_dir = home.join(".claude-switch");
        Self::new_in_base_dir(&base_dir)
    }

    fn new_in_base_dir(base_dir: &Path) -> Result<Self> {
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

    fn home_dir() -> Result<PathBuf> {
        if let Some(value) = env::var_os(CLAUDE_SWITCH_HOME_ENV) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return Ok(path);
            }
        }
        dirs::home_dir().context("Cannot determine home directory")
    }

    // ── Registry I/O ─────────────────────────────────────────────────────────

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

    /// Auto-migrate legacy inline credentials and deprecated provider `api_key`
    /// fields into explicit provider/key references.
    fn migrate_providers(&self, registry: &mut Registry) -> Result<()> {
        // Phase 1: move deprecated per-provider `api_key` values into `keys`.
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

        // Phase 2: keep lightweight provider links valid when the provider still
        // exists but older data lacks an explicit key id.
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

    fn save_registry(&self, registry: &Registry) -> Result<()> {
        let content = serde_json::to_string_pretty(registry)?;
        fs::write(&self.registry_path, content)?;
        Ok(())
    }

    /// Update just the launch_args field for a profile.
    pub fn set_launch_args(&self, query: &str, args: Option<Vec<String>>) -> Result<()> {
        let (id, _) = self.find_profile(query)?;
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.launch_args = args;
        }
        self.save_registry(&registry)
    }

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

    // ── Lookup helpers ───────────────────────────────────────────────────────

    /// Find a profile by id, alias, or name (exact match, in that order).
    /// Returns `(id, profile)`.
    pub fn find_profile(&self, query: &str) -> Result<(String, Profile)> {
        let registry = self.load_registry()?;
        Self::find_profile_in_registry(&registry, query)
    }

    fn find_profile_in_registry(registry: &Registry, query: &str) -> Result<(String, Profile)> {
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

    fn check_profile_unique_in_registry(
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

    fn validate_alias(alias: &str) -> Result<()> {
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

    fn normalize_mcp_server_type(server_type: &str) -> Result<String> {
        let value = server_type.trim();
        let normalized = if value.is_empty() { "stdio" } else { value }.to_ascii_lowercase();
        match normalized.as_str() {
            "stdio" | "http" | "streamable-http" | "sse" => Ok(normalized),
            _ => bail!(
                "MCP type '{}' is invalid. Use stdio, http, streamable-http, or sse.",
                server_type
            ),
        }
    }

    fn normalize_optional_string(value: Option<String>) -> Option<String> {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn json_object_field<'a>(
        object: &'a serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<Option<&'a serde_json::Map<String, serde_json::Value>>> {
        match object.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Object(value)) => Ok(Some(value)),
            Some(_) => bail!("MCP '{}' field '{}' must be an object.", mcp_name, field),
        }
    }

    fn json_string_field(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<Option<String>> {
        match object.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(value)) => {
                Ok(Self::normalize_optional_string(Some(value.clone())))
            }
            Some(_) => bail!("MCP '{}' field '{}' must be a string.", mcp_name, field),
        }
    }

    fn json_string_vec_field(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<Vec<String>> {
        match object.get(field) {
            None | Some(serde_json::Value::Null) => Ok(Vec::new()),
            Some(serde_json::Value::Array(values)) => values
                .iter()
                .map(|value| match value {
                    serde_json::Value::String(value) => Ok(value.clone()),
                    _ => bail!("MCP '{}' field '{}' must contain strings.", mcp_name, field),
                })
                .collect(),
            Some(_) => bail!("MCP '{}' field '{}' must be an array.", mcp_name, field),
        }
    }

    fn json_string_map_field(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        let Some(values) = Self::json_object_field(object, field, mcp_name)? else {
            return Ok(map);
        };
        for (key, value) in values {
            if key.trim().is_empty() {
                bail!("MCP '{}' field '{}' has an empty key.", mcp_name, field);
            }
            let serde_json::Value::String(value) = value else {
                bail!(
                    "MCP '{}' field '{}' values must be strings.",
                    mcp_name,
                    field
                );
            };
            map.insert(key.clone(), value.clone());
        }
        Ok(map)
    }

    fn json_bool_field(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<Option<bool>> {
        match object.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => bail!("MCP '{}' field '{}' must be a boolean.", mcp_name, field),
        }
    }

    fn json_u64_field(
        object: &serde_json::Map<String, serde_json::Value>,
        field: &str,
        mcp_name: &str,
    ) -> Result<Option<u64>> {
        match object.get(field) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::Number(value)) => value
                .as_u64()
                .with_context(|| {
                    format!(
                        "MCP '{}' field '{}' must be a non-negative integer.",
                        mcp_name, field
                    )
                })
                .map(Some),
            Some(_) => bail!("MCP '{}' field '{}' must be a number.", mcp_name, field),
        }
    }

    fn mcp_server_input_from_config(
        name: &str,
        value: &serde_json::Value,
    ) -> Result<McpServerInput> {
        let Some(object) = value.as_object() else {
            bail!("MCP '{}' must be an object.", name);
        };
        let server_type =
            Self::json_string_field(object, "type", name)?.unwrap_or_else(default_mcp_server_type);
        let oauth = match object.get("oauth") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(value.clone()),
        };
        Ok(McpServerInput {
            name: name.to_string(),
            server_type,
            command: Self::json_string_field(object, "command", name)?,
            args: Self::json_string_vec_field(object, "args", name)?,
            env: Self::json_string_map_field(object, "env", name)?,
            cwd: Self::json_string_field(object, "cwd", name)?,
            url: Self::json_string_field(object, "url", name)?,
            headers: Self::json_string_map_field(object, "headers", name)?,
            oauth,
            headers_helper: Self::json_string_field(object, "headersHelper", name)?,
            timeout: Self::json_u64_field(object, "timeout", name)?,
            always_load: Self::json_bool_field(object, "alwaysLoad", name)?,
            disabled: Self::json_bool_field(object, "disabled", name)?,
        })
    }

    fn build_mcp_server(id: String, input: McpServerInput) -> Result<McpServer> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            bail!("MCP name cannot be empty.");
        }
        let server_type = Self::normalize_mcp_server_type(&input.server_type)?;
        let command = Self::normalize_optional_string(input.command);
        let cwd = Self::normalize_optional_string(input.cwd);
        let url = Self::normalize_optional_string(input.url);
        let headers_helper = Self::normalize_optional_string(input.headers_helper);
        match server_type.as_str() {
            "stdio" if command.is_none() => {
                bail!("MCP '{}' uses stdio and requires a command.", name);
            }
            "http" | "streamable-http" | "sse" if url.is_none() => {
                bail!("MCP '{}' uses {} and requires a URL.", name, server_type);
            }
            _ => {}
        }

        Ok(McpServer {
            id,
            name,
            server_type,
            command,
            args: input.args,
            env: input.env,
            cwd,
            url,
            headers: input.headers,
            oauth: input.oauth,
            headers_helper,
            timeout: input.timeout,
            always_load: input.always_load,
            disabled: input.disabled,
        })
    }

    fn mcp_secret_key_likely(key: &str) -> bool {
        let normalized: String = key
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        normalized.contains("api_key")
            || normalized.contains("apikey")
            || normalized.contains("access_token")
            || normalized.contains("refresh_token")
            || normalized.contains("auth_token")
            || normalized.contains("personal_access_token")
            || normalized == "token"
            || normalized.starts_with("token_")
            || normalized.ends_with("_token")
            || normalized.contains("_token_")
            || normalized.contains("secret")
            || normalized.contains("password")
            || normalized.contains("passwd")
            || normalized.contains("private_key")
            || normalized.contains("authorization")
            || normalized.contains("credential")
            || normalized.contains("bearer")
    }

    fn mcp_value_likely_secret(value: &str) -> bool {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return false;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("bearer ")
            || lower.starts_with("basic ")
            || lower.starts_with("token ")
            || lower.starts_with("sk-")
        {
            return true;
        }
        (lower.contains("${") || lower.contains('%'))
            && [
                "api_key",
                "apikey",
                "token",
                "secret",
                "password",
                "passwd",
                "private_key",
                "authorization",
                "credential",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
    }

    fn redact_mcp_string_map_secrets(map: &mut HashMap<String, String>) {
        for (key, value) in map {
            if Self::mcp_secret_key_likely(key) || Self::mcp_value_likely_secret(value) {
                value.clear();
            }
        }
    }

    fn mcp_string_map_has_secrets(map: &HashMap<String, String>) -> bool {
        map.iter().any(|(key, value)| {
            !value.is_empty()
                && (Self::mcp_secret_key_likely(key) || Self::mcp_value_likely_secret(value))
        })
    }

    fn redact_mcp_json_secrets(value: &mut serde_json::Value, inherited_secret: bool) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, child) in object {
                    Self::redact_mcp_json_secrets(
                        child,
                        inherited_secret || Self::mcp_secret_key_likely(key),
                    );
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    Self::redact_mcp_json_secrets(child, inherited_secret);
                }
            }
            serde_json::Value::String(text)
                if inherited_secret || Self::mcp_value_likely_secret(text) =>
            {
                text.clear();
            }
            _ => {}
        }
    }

    fn mcp_json_has_secrets(value: &serde_json::Value, inherited_secret: bool) -> bool {
        match value {
            serde_json::Value::Object(object) => object.iter().any(|(key, child)| {
                Self::mcp_json_has_secrets(
                    child,
                    inherited_secret || Self::mcp_secret_key_likely(key),
                )
            }),
            serde_json::Value::Array(items) => items
                .iter()
                .any(|child| Self::mcp_json_has_secrets(child, inherited_secret)),
            serde_json::Value::String(text) => {
                !text.is_empty() && (inherited_secret || Self::mcp_value_likely_secret(text))
            }
            _ => false,
        }
    }

    fn mcp_json_value_is_redacted(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Null => true,
            serde_json::Value::String(text) => text.is_empty(),
            serde_json::Value::Array(items) => items.iter().all(Self::mcp_json_value_is_redacted),
            serde_json::Value::Object(object) => {
                object.values().all(Self::mcp_json_value_is_redacted)
            }
            _ => false,
        }
    }

    fn preserve_mcp_json_secrets(
        incoming: &mut serde_json::Value,
        existing: &serde_json::Value,
        inherited_secret: bool,
    ) {
        match (incoming, existing) {
            (serde_json::Value::Object(incoming_map), serde_json::Value::Object(existing_map)) => {
                for (key, existing_value) in existing_map {
                    let key_is_secret = inherited_secret || Self::mcp_secret_key_likely(key);
                    if let Some(incoming_value) = incoming_map.get_mut(key) {
                        if key_is_secret && Self::mcp_json_value_is_redacted(incoming_value) {
                            *incoming_value = existing_value.clone();
                        } else {
                            Self::preserve_mcp_json_secrets(
                                incoming_value,
                                existing_value,
                                key_is_secret,
                            );
                        }
                    } else if key_is_secret {
                        incoming_map.insert(key.clone(), existing_value.clone());
                    }
                }
            }
            (
                serde_json::Value::Array(incoming_items),
                serde_json::Value::Array(existing_items),
            ) => {
                for (incoming_value, existing_value) in
                    incoming_items.iter_mut().zip(existing_items.iter())
                {
                    Self::preserve_mcp_json_secrets(
                        incoming_value,
                        existing_value,
                        inherited_secret,
                    );
                }
            }
            _ => {}
        }
    }

    fn redact_mcp_server_secrets(server: &mut McpServer) {
        Self::redact_mcp_string_map_secrets(&mut server.env);
        Self::redact_mcp_string_map_secrets(&mut server.headers);
        if let Some(oauth) = &mut server.oauth {
            Self::redact_mcp_json_secrets(oauth, false);
        }
    }

    fn mcp_server_has_secrets(server: &McpServer) -> bool {
        Self::mcp_string_map_has_secrets(&server.env)
            || Self::mcp_string_map_has_secrets(&server.headers)
            || server
                .oauth
                .as_ref()
                .is_some_and(|oauth| Self::mcp_json_has_secrets(oauth, false))
    }

    fn preserve_mcp_server_secrets(incoming: &mut McpServer, existing: &McpServer) {
        for (key, existing_value) in &existing.env {
            if let Some(incoming_value) = incoming.env.get_mut(key)
                && incoming_value.is_empty()
                && (Self::mcp_secret_key_likely(key)
                    || Self::mcp_value_likely_secret(existing_value))
            {
                *incoming_value = existing_value.clone();
            }
        }
        for (key, existing_value) in &existing.headers {
            if let Some(incoming_value) = incoming.headers.get_mut(key)
                && incoming_value.is_empty()
                && (Self::mcp_secret_key_likely(key)
                    || Self::mcp_value_likely_secret(existing_value))
            {
                *incoming_value = existing_value.clone();
            }
        }
        if let (Some(incoming_oauth), Some(existing_oauth)) = (&mut incoming.oauth, &existing.oauth)
        {
            Self::preserve_mcp_json_secrets(incoming_oauth, existing_oauth, false);
        }
    }

    fn check_mcp_name_unique_in_registry(
        registry: &Registry,
        exclude_id: &str,
        name: &str,
    ) -> Result<()> {
        if registry
            .mcp_servers
            .values()
            .any(|server| server.id != exclude_id && server.name == name)
        {
            bail!("MCP name '{}' is already in use.", name);
        }
        Ok(())
    }

    fn find_mcp_server_in_registry(
        registry: &Registry,
        query: &str,
    ) -> Result<(String, McpServer)> {
        let query = query.trim();
        if query.is_empty() {
            bail!("MCP query is empty.");
        }
        if let Some(server) = registry.mcp_servers.get(query) {
            return Ok((query.to_string(), server.clone()));
        }
        let by_name: Vec<_> = registry
            .mcp_servers
            .iter()
            .filter(|(_, server)| server.name == query)
            .collect();
        if by_name.len() == 1 {
            return Ok((by_name[0].0.clone(), by_name[0].1.clone()));
        }
        if by_name.len() > 1 {
            bail!(
                "Multiple MCP servers match name '{}'. Use the full id to disambiguate.",
                query
            );
        }
        bail!("MCP '{}' not found. Add it with: cswitch mcp add", query)
    }

    pub fn find_mcp_server(&self, query: &str) -> Result<(String, McpServer)> {
        let registry = self.load_registry()?;
        Self::find_mcp_server_in_registry(&registry, query)
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

    // ── MCP CRUD ────────────────────────────────────────────────────────────

    pub fn list_mcp_servers(&self) -> Result<Vec<McpServer>> {
        let registry = self.load_registry()?;
        let mut servers: Vec<McpServer> = registry.mcp_servers.into_values().collect();
        servers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(servers)
    }

    pub fn get_mcp_server(&self, query: &str) -> Result<McpServer> {
        let (_, server) = self.find_mcp_server(query)?;
        Ok(server)
    }

    pub fn add_mcp_server(&self, input: McpServerInput) -> Result<McpServer> {
        let mut registry = self.load_registry()?;
        let server =
            Self::build_mcp_server(format!("mcp_{}", &Uuid::new_v4().to_string()[..8]), input)?;
        Self::check_mcp_name_unique_in_registry(&registry, "", &server.name)?;
        registry
            .mcp_servers
            .insert(server.id.clone(), server.clone());
        self.save_registry(&registry)?;
        Ok(server)
    }

    pub fn update_mcp_server(&self, query: &str, update: McpServerUpdate) -> Result<McpServer> {
        let (id, existing) = self.find_mcp_server(query)?;
        let mut registry = self.load_registry()?;
        let input = McpServerInput {
            name: update.name.unwrap_or(existing.name),
            server_type: update.server_type.unwrap_or(existing.server_type),
            command: update.command.unwrap_or(existing.command),
            args: update.args.unwrap_or(existing.args),
            env: update.env.unwrap_or(existing.env),
            cwd: update.cwd.unwrap_or(existing.cwd),
            url: update.url.unwrap_or(existing.url),
            headers: update.headers.unwrap_or(existing.headers),
            oauth: update.oauth.unwrap_or(existing.oauth),
            headers_helper: update.headers_helper.unwrap_or(existing.headers_helper),
            timeout: update.timeout.unwrap_or(existing.timeout),
            always_load: update.always_load.unwrap_or(existing.always_load),
            disabled: update.disabled.unwrap_or(existing.disabled),
        };
        let server = Self::build_mcp_server(id.clone(), input)?;
        Self::check_mcp_name_unique_in_registry(&registry, &id, &server.name)?;
        registry.mcp_servers.insert(id, server.clone());
        self.save_registry(&registry)?;
        Ok(server)
    }

    pub fn remove_mcp_server(&self, query: &str) -> Result<()> {
        let (id, server) = self.find_mcp_server(query)?;
        let registry = self.load_registry()?;
        let refs: Vec<_> = registry
            .profiles
            .values()
            .filter(|profile| profile.mcp_server_ids.iter().any(|mcp_id| mcp_id == &id))
            .map(|profile| profile.name.clone())
            .collect();
        if !refs.is_empty() {
            bail!(
                "MCP '{}' is used by profiles: {}. Unlink it first.",
                server.name,
                refs.join(", ")
            );
        }
        let mut registry = self.load_registry()?;
        registry.mcp_servers.remove(&id);
        self.save_registry(&registry)
    }

    pub fn list_profiles_using_mcp(&self, mcp_id: &str) -> Result<Vec<Profile>> {
        let registry = self.load_registry()?;
        if !registry.mcp_servers.contains_key(mcp_id) {
            bail!("MCP '{}' not found.", mcp_id);
        }
        let mut profiles: Vec<Profile> = registry
            .profiles
            .values()
            .filter(|profile| profile.mcp_server_ids.iter().any(|id| id == mcp_id))
            .cloned()
            .collect();
        profiles.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(profiles)
    }

    pub fn set_profile_mcps(&self, query: &str, mcp_queries: &[String]) -> Result<Profile> {
        let (profile_id, profile) = self.find_profile(query)?;
        if profile.kind != ProfileKind::Lightweight {
            bail!("MCP servers can only be linked to lightweight profiles.");
        }
        let mut registry = self.load_registry()?;
        let mut mcp_ids = Vec::new();
        for query in mcp_queries {
            let (id, _) = Self::find_mcp_server_in_registry(&registry, query)?;
            if !mcp_ids.contains(&id) {
                mcp_ids.push(id);
            }
        }
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        profile.mcp_server_ids = mcp_ids;
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn add_profile_mcps(&self, query: &str, mcp_queries: &[String]) -> Result<Profile> {
        let (profile_id, profile) = self.find_profile(query)?;
        if profile.kind != ProfileKind::Lightweight {
            bail!("MCP servers can only be linked to lightweight profiles.");
        }
        let mut registry = self.load_registry()?;
        let mut additions = Vec::new();
        for query in mcp_queries {
            let (id, _) = Self::find_mcp_server_in_registry(&registry, query)?;
            additions.push(id);
        }
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        for id in additions {
            if !profile.mcp_server_ids.contains(&id) {
                profile.mcp_server_ids.push(id);
            }
        }
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn remove_profile_mcps(
        &self,
        query: &str,
        mcp_queries: &[String],
        remove_all: bool,
    ) -> Result<Profile> {
        let (profile_id, profile) = self.find_profile(query)?;
        if profile.kind != ProfileKind::Lightweight {
            bail!("MCP servers can only be unlinked from lightweight profiles.");
        }
        let mut registry = self.load_registry()?;
        let remove_ids = if remove_all {
            Vec::new()
        } else {
            let mut ids = Vec::new();
            for query in mcp_queries {
                let (id, _) = Self::find_mcp_server_in_registry(&registry, query)?;
                ids.push(id);
            }
            ids
        };
        let profile = registry
            .profiles
            .get_mut(&profile_id)
            .with_context(|| format!("Profile '{}' not found.", query))?;
        if remove_all {
            profile.mcp_server_ids.clear();
        } else {
            profile
                .mcp_server_ids
                .retain(|id| !remove_ids.iter().any(|remove_id| remove_id == id));
        }
        let profile = profile.clone();
        self.save_registry(&registry)?;
        Ok(profile)
    }

    pub fn export_mcp_config(&self, queries: &[String], all: bool) -> Result<String> {
        let registry = self.load_registry()?;
        let servers = Self::selected_mcp_servers_in_registry(&registry, queries, all)?;
        Self::profile_mcp_config(&servers)
    }

    pub fn import_mcp_config(&self, content: &str, replace: bool) -> Result<Vec<McpServer>> {
        let root: serde_json::Value =
            serde_json::from_str(content).context("Failed to parse MCP JSON")?;
        let mcp_servers = root
            .get("mcpServers")
            .and_then(|value| value.as_object())
            .context("MCP JSON must contain an object field named 'mcpServers'.")?;
        let mut registry = self.load_registry()?;
        let mut imported = Vec::new();

        for (name, value) in mcp_servers {
            let input = Self::mcp_server_input_from_config(name, value)?;
            let existing_id = registry
                .mcp_servers
                .iter()
                .find(|(_, server)| server.name == input.name)
                .map(|(id, _)| id.clone());
            let id = if let Some(id) = existing_id {
                if !replace {
                    bail!(
                        "MCP name '{}' already exists. Use --replace to update it.",
                        input.name
                    );
                }
                id
            } else {
                loop {
                    let id = format!("mcp_{}", &Uuid::new_v4().to_string()[..8]);
                    if !registry.mcp_servers.contains_key(&id) {
                        break id;
                    }
                }
            };

            let server = Self::build_mcp_server(id.clone(), input)?;
            Self::check_mcp_name_unique_in_registry(&registry, &id, &server.name)?;
            registry.mcp_servers.insert(id, server.clone());
            imported.push(server);
        }

        self.save_registry(&registry)?;
        imported.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(imported)
    }

    pub fn validate_mcp_servers(
        &self,
        queries: &[String],
        all: bool,
    ) -> Result<Vec<McpValidationIssue>> {
        let registry = self.load_registry()?;
        let servers = Self::selected_mcp_servers_in_registry(&registry, queries, all)?;
        let mut issues = Vec::new();
        for server in &servers {
            issues.extend(Self::validate_mcp_server_config(server));
        }
        Ok(issues)
    }

    pub fn inspect_config(&self) -> Result<ConfigInspection> {
        let registry = self.load_registry()?;
        let profiles = registry.profiles.values().collect::<Vec<_>>();
        let lightweight_profiles = profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::Lightweight)
            .count();
        let full_profiles = profiles
            .iter()
            .filter(|profile| profile.kind == ProfileKind::Full)
            .count();
        let provider_keys = registry
            .providers
            .values()
            .map(|provider| provider.keys.len())
            .sum();
        let linked_mcp_refs = profiles
            .iter()
            .map(|profile| profile.mcp_server_ids.len())
            .sum();

        #[cfg(target_os = "windows")]
        let cmd_shims_dir = Self::cmd_bin_dir().ok();
        #[cfg(not(target_os = "windows"))]
        let cmd_shims_dir = None;

        #[cfg(not(target_os = "windows"))]
        let shell_shims_dir = Self::sh_bin_dir().ok();
        #[cfg(target_os = "windows")]
        let shell_shims_dir = None;

        Ok(ConfigInspection {
            base_dir: self.base_dir(),
            registry_path: self.registry_path.clone(),
            profiles_dir: self.profiles_dir.clone(),
            generated_root: self.generated_root_dir(),
            profiles: profiles.len(),
            lightweight_profiles,
            full_profiles,
            providers: registry.providers.len(),
            provider_keys,
            mcp_servers: registry.mcp_servers.len(),
            linked_mcp_refs,
            generated_mcp_plugins: Self::count_named_entries(
                &self.generated_mcps_dir(),
                Self::is_managed_generated_mcp_dir_name,
            ),
            generated_tinyfish_plugins: Self::count_named_entries(
                &self.generated_plugins_dir(),
                Self::is_managed_generated_plugin_dir_name,
            ),
            generated_prompts: Self::count_named_entries(
                &self.generated_prompts_dir(),
                Self::is_managed_generated_prompt_name,
            ),
            cmd_shims_dir,
            shell_shims_dir,
        })
    }

    pub fn doctor_report(&self) -> Result<DoctorReport> {
        let mut report = DoctorReport::default();
        let base_dir = self.base_dir();
        if base_dir.exists() {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Ok,
                "storage",
                format!("base directory exists: {}", base_dir.display()),
                None,
            ));
        } else {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "storage",
                format!("base directory does not exist yet: {}", base_dir.display()),
                Some("run any cswitch command that writes profiles or providers".to_string()),
            ));
        }

        let registry = match self.load_registry() {
            Ok(registry) => {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Ok,
                    "registry",
                    format!("registry is readable: {}", self.registry_path.display()),
                    None,
                ));
                registry
            }
            Err(err) => {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "registry",
                    format!("registry cannot be read: {err}"),
                    Some("inspect or restore ~/.claude-switch/registry.json".to_string()),
                ));
                return Ok(report);
            }
        };

        if registry.profiles.is_empty() {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "profiles",
                "no profiles are configured".to_string(),
                Some("add one with cswitch add <name>".to_string()),
            ));
        }

        for (profile_id, profile) in &registry.profiles {
            if profile.name.trim().is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("profile '{profile_id}' has an empty name"),
                    None,
                ));
            }
            if profile.kind == ProfileKind::Full {
                let dir = self.profile_dir(profile);
                if !dir.exists() {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "profiles",
                        format!("full profile '{}' directory is missing", profile.name),
                        Some(format!("expected {}", dir.display())),
                    ));
                }
            } else if profile.env.is_none() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "profiles",
                    format!("lightweight profile '{}' has no env block", profile.name),
                    Some("edit or recreate this profile".to_string()),
                ));
            }

            if let Some(provider_id) = &profile.provider_id {
                match registry.providers.get(provider_id) {
                    Some(provider) => match &profile.key_id {
                        Some(key_id) if provider.keys.contains_key(key_id) => {}
                        Some(key_id) => report.items.push(Self::diagnostic(
                            DiagnosticLevel::Error,
                            "providers",
                            format!(
                                "profile '{}' references missing key '{}' in provider '{}'",
                                profile.name, key_id, provider.name
                            ),
                            Some("relink the profile with cswitch provider link".to_string()),
                        )),
                        None => report.items.push(Self::diagnostic(
                            DiagnosticLevel::Error,
                            "providers",
                            format!(
                                "profile '{}' references provider '{}' without a key",
                                profile.name, provider.name
                            ),
                            Some("relink the profile with cswitch provider link".to_string()),
                        )),
                    },
                    None => report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "providers",
                        format!(
                            "profile '{}' references missing provider '{}'",
                            profile.name, provider_id
                        ),
                        Some("unlink or relink the profile provider".to_string()),
                    )),
                }
            }

            if !profile.mcp_server_ids.is_empty() && profile.kind != ProfileKind::Lightweight {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "mcp",
                    format!("full profile '{}' has MCP registry links", profile.name),
                    Some("MCP links are only supported for lightweight profiles".to_string()),
                ));
            }
            for mcp_id in &profile.mcp_server_ids {
                if !registry.mcp_servers.contains_key(mcp_id) {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Error,
                        "mcp",
                        format!(
                            "profile '{}' references missing MCP '{}'",
                            profile.name, mcp_id
                        ),
                        Some("unlink stale MCP ids or recreate the MCP entry".to_string()),
                    ));
                }
            }
            if profile.kind == ProfileKind::Lightweight && !profile.mcp_server_ids.is_empty() {
                let plugin_root = self.local_profile_mcp_plugin_root(profile);
                if !plugin_root.join(".mcp.json").exists() || !plugin_root.join("mcp.json").exists()
                {
                    report.items.push(Self::diagnostic(
                        DiagnosticLevel::Warn,
                        "mcp",
                        format!(
                            "profile '{}' MCP plugin artifacts have not been generated",
                            profile.name
                        ),
                        Some("run cswitch aliases --local or launch the profile once".to_string()),
                    ));
                }
            }
        }

        for provider in registry.providers.values() {
            if provider.base_url.trim().is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Error,
                    "providers",
                    format!("provider '{}' has an empty base URL", provider.name),
                    Some("edit the provider URL".to_string()),
                ));
            }
            if provider.keys.is_empty() {
                report.items.push(Self::diagnostic(
                    DiagnosticLevel::Warn,
                    "providers",
                    format!("provider '{}' has no keys", provider.name),
                    Some("add a key with cswitch provider add-key".to_string()),
                ));
            }
        }

        for issue in registry
            .mcp_servers
            .values()
            .flat_map(Self::validate_mcp_server_config)
        {
            report.items.push(Self::diagnostic(
                issue.level,
                "mcp",
                format!("{}: {}", issue.server_name, issue.message),
                issue.hint,
            ));
        }

        let desired_mcp_dirs: std::collections::HashSet<String> = registry
            .profiles
            .values()
            .filter(|profile| {
                profile.kind == ProfileKind::Lightweight && !profile.mcp_server_ids.is_empty()
            })
            .map(Self::profile_mcp_plugin_dir_name)
            .collect();
        let stale_mcp_dirs = Self::managed_entry_names(
            &self.generated_mcps_dir(),
            Self::is_managed_generated_mcp_dir_name,
        )
        .into_iter()
        .filter(|name| !desired_mcp_dirs.contains(name))
        .count();
        if stale_mcp_dirs > 0 {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "generated",
                format!("{stale_mcp_dirs} stale generated MCP plugin dir(s) found"),
                Some("run cswitch aliases --local to resync generated artifacts".to_string()),
            ));
        }

        if !Self::command_exists("claude") {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Warn,
                "runtime",
                "claude command is not available on PATH".to_string(),
                Some("install Claude Code or adjust PATH before launching profiles".to_string()),
            ));
        }

        if report.error_count() == 0 {
            report.items.push(Self::diagnostic(
                DiagnosticLevel::Ok,
                "summary",
                format!(
                    "{} profile(s), {} provider(s), {} MCP server(s)",
                    registry.profiles.len(),
                    registry.providers.len(),
                    registry.mcp_servers.len()
                ),
                None,
            ));
        }

        Ok(report)
    }

    pub fn resolve_project_profile(&self, start: &Path) -> Result<Option<Profile>> {
        let mut current = if start.as_os_str().is_empty() {
            env::current_dir()?
        } else if start.is_file() {
            start.parent().unwrap_or(start).to_path_buf()
        } else {
            start.to_path_buf()
        };
        if current.is_relative() {
            current = env::current_dir()?.join(current);
        }

        loop {
            for marker in [".cswitch-profile", ".claudeprofile"] {
                let marker_path = current.join(marker);
                if let Some(query) = Self::read_profile_marker(&marker_path)? {
                    let (_, profile) = self.find_profile(&query).with_context(|| {
                        format!(
                            "Project marker '{}' references unknown profile '{}'.",
                            marker_path.display(),
                            query
                        )
                    })?;
                    return Ok(Some(profile));
                }
            }

            if !current.pop() {
                break;
            }
        }

        Ok(None)
    }

    pub fn statusline_info(
        &self,
        profile_query: Option<&str>,
        project_dir: Option<&Path>,
    ) -> Result<StatuslineInfo> {
        let registry = self.load_registry()?;
        let (profile, project_marker) = if let Some(query) = profile_query {
            let (_, profile) = Self::find_profile_in_registry(&registry, query)?;
            (Some(profile), false)
        } else if let Some(project_dir) = project_dir {
            (self.resolve_project_profile(project_dir)?, true)
        } else {
            (None, false)
        };

        let Some(profile) = profile else {
            return Ok(StatuslineInfo {
                profile_id: None,
                profile_name: None,
                profile_alias: None,
                kind: None,
                provider_name: None,
                provider_id: None,
                key_name: None,
                key_id: None,
                mcp_servers: 0,
                mcp_names: Vec::new(),
                project_marker: false,
            });
        };

        let (provider_name, key_name) = profile
            .provider_id
            .as_ref()
            .and_then(|provider_id| {
                registry.providers.get(provider_id).map(|provider| {
                    let key_name = profile
                        .key_id
                        .as_ref()
                        .and_then(|key_id| provider.keys.get(key_id))
                        .map(|key| key.name.clone());
                    (Some(provider.name.clone()), key_name)
                })
            })
            .unwrap_or((None, None));
        let mut mcp_names = profile
            .mcp_server_ids
            .iter()
            .filter_map(|id| registry.mcp_servers.get(id))
            .map(|server| server.name.clone())
            .collect::<Vec<_>>();
        mcp_names.sort();

        Ok(StatuslineInfo {
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.name.clone()),
            profile_alias: profile.alias.clone(),
            kind: Some(profile.kind.clone()),
            provider_name,
            provider_id: profile.provider_id.clone(),
            key_name,
            key_id: profile.key_id.clone(),
            mcp_servers: mcp_names.len(),
            mcp_names,
            project_marker,
        })
    }

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
        let mut registry = self.load_registry()?;
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

    pub fn plan_shim_recovery(&self, shim_dir: &Path, replace: bool) -> Result<ShimRecoveryPlan> {
        Ok(self.build_shim_recovery_state(shim_dir, replace)?.plan)
    }

    pub fn recover_shims(&self, shim_dir: &Path, replace: bool) -> Result<ShimRecoverySummary> {
        let state = self.build_shim_recovery_state(shim_dir, replace)?;
        if state.plan.conflict_count() > 0 {
            bail!(
                "Shim recovery has {} conflicted profile(s). Use --replace to update them.",
                state.plan.conflict_count()
            );
        }

        let backup_path = if self.registry_path.exists() {
            let backup_path = self.registry_backup_path();
            fs::copy(&self.registry_path, &backup_path).with_context(|| {
                format!(
                    "Failed to create registry backup '{}'.",
                    backup_path.display()
                )
            })?;
            Some(backup_path)
        } else {
            None
        };

        let mut registry = state.registry;
        self.migrate_providers(&mut registry)?;
        self.save_registry(&registry)?;
        Ok(ShimRecoverySummary {
            plan: state.plan,
            backup_path,
        })
    }

    fn build_shim_recovery_state(
        &self,
        shim_dir: &Path,
        replace: bool,
    ) -> Result<ShimRecoveryState> {
        if !shim_dir.exists() {
            bail!("Shim directory '{}' does not exist.", shim_dir.display());
        }
        if !shim_dir.is_dir() {
            bail!("Shim path '{}' is not a directory.", shim_dir.display());
        }

        let mut registry = self.load_registry()?;
        let mut plan = ShimRecoveryPlan {
            shim_dir: shim_dir.to_path_buf(),
            ..Default::default()
        };
        let mut provider_names = registry
            .providers
            .values()
            .map(|provider| provider.name.clone())
            .collect::<HashSet<_>>();
        let mut entries = fs::read_dir(shim_dir)
            .with_context(|| format!("Failed to read shim directory '{}'.", shim_dir.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("Failed to list shim directory '{}'.", shim_dir.display()))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if !Self::is_recoverable_shim_file_name(&file_name) {
                continue;
            }

            plan.files_scanned += 1;
            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings
                        .push(format!("{}: failed to read shim: {}", file_name, err));
                    continue;
                }
            };
            let recovered = match Self::parse_recoverable_shim(&file_name, &content) {
                Ok(profile) => profile,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings.push(format!("{}: {}", file_name, err));
                    continue;
                }
            };
            plan.files_recoverable += 1;

            let existing_profile_id = match Self::find_profile_conflict_id(
                &registry,
                &recovered.name,
                Some(&recovered.alias),
            ) {
                Ok(id) => id,
                Err(err) => {
                    plan.files_skipped += 1;
                    plan.warnings.push(format!("{}: {}", file_name, err));
                    continue;
                }
            };
            if existing_profile_id.is_some() && !replace {
                plan.profiles_conflicted += 1;
                plan.profiles_conflict.push(format!(
                    "{} ({}) from {}",
                    recovered.name, recovered.alias, recovered.file_name
                ));
                continue;
            }

            let existing_profile = existing_profile_id
                .as_ref()
                .and_then(|id| registry.profiles.get(id))
                .cloned();
            let (provider_id, key_id) = Self::ensure_recovered_provider_key(
                &mut registry,
                &mut plan,
                &mut provider_names,
                &recovered,
            );
            let profile = Self::build_recovered_profile(
                existing_profile_id.clone(),
                &recovered,
                provider_id,
                key_id,
                existing_profile.as_ref(),
            );

            if let Some(id) = existing_profile_id {
                registry.profiles.insert(id, profile.clone());
                plan.profiles_updated += 1;
                plan.profiles_update.push(format!(
                    "{} ({})",
                    profile.name,
                    profile.alias.as_deref().unwrap_or("")
                ));
            } else {
                registry
                    .profiles
                    .insert(profile.id.clone(), profile.clone());
                plan.profiles_added += 1;
                plan.profiles_add.push(format!(
                    "{} ({})",
                    profile.name,
                    profile.alias.as_deref().unwrap_or("")
                ));
            }
        }

        plan.profiles_add.sort();
        plan.profiles_update.sort();
        plan.profiles_conflict.sort();
        plan.providers_add.sort();
        plan.provider_keys_add.sort();
        plan.warnings.sort();

        Ok(ShimRecoveryState { plan, registry })
    }

    // ── Provider resolution ─────────────────────────────────────────────────

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

    // ── Provider CRUD ───────────────────────────────────────────────────────

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

    // ── Key CRUD ──────────────────────────────────────────────────────────

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
        };

        let mut registry = self.load_registry()?;
        registry
            .profiles
            .insert(profile.id.clone(), profile.clone());
        self.save_registry(&registry)?;
        Ok(profile)
    }

    // ── Launch ───────────────────────────────────────────────────────────────

    pub fn launch_claude(&self, query: &str, args: &[String], use_stored_args: bool) -> Result<()> {
        let (id, profile) = self.find_profile(query)?;

        // Update last_used
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(&id) {
            p.last_used = Some(Utc::now());
        }
        self.save_registry(&registry)?;

        let mut cmd = std::process::Command::new("claude");
        if use_stored_args && let Some(ref stored) = profile.launch_args {
            cmd.args(stored);
        }
        cmd.args(args);

        if profile.kind == ProfileKind::Lightweight {
            if let Some(ref env) = profile.env {
                let (resolved_token, resolved_url) = self.resolve_credentials(&profile)?;
                let tool_shell = native_tinyfish_tool_shell();
                let artifacts = build_lightweight_runtime_artifacts(
                    env,
                    resolved_token.as_deref(),
                    resolved_url.as_deref(),
                    tool_shell,
                )?;

                cmd.arg("--settings");
                if artifacts.tinyfish_mode != TinyfishMode::None && tinyfish_available() {
                    let settings_json = artifacts
                        .tinyfish_settings_json
                        .as_deref()
                        .context("TinyFish settings missing for non-native mode")?;
                    let prompt_text = artifacts
                        .tinyfish_prompt_text
                        .as_deref()
                        .context("TinyFish prompt missing for non-native mode")?;
                    let plugin_hooks_json = artifacts
                        .tinyfish_plugin_hooks_json
                        .as_deref()
                        .context("TinyFish plugin hooks missing for non-native mode")?;
                    let plugin_manifest_json =
                        artifacts
                            .tinyfish_plugin_manifest_json
                            .as_deref()
                            .context("TinyFish plugin manifest missing for non-native mode")?;
                    let (plugin_root, prompt_path) = self.upsert_local_tinyfish_artifacts(
                        artifacts.tinyfish_mode,
                        tool_shell,
                        plugin_manifest_json,
                        plugin_hooks_json,
                        prompt_text,
                    )?;
                    cmd.arg(settings_json);
                    cmd.arg("--plugin-dir");
                    cmd.arg(plugin_root);
                    cmd.arg("--append-system-prompt-file");
                    cmd.arg(prompt_path);
                } else {
                    cmd.arg(&artifacts.base_settings_json);
                }

                let mcp_servers = self.profile_mcp_servers(&profile)?;
                if !mcp_servers.is_empty() {
                    let plugin_root =
                        self.upsert_local_profile_mcp_plugin(&profile, &mcp_servers)?;
                    cmd.arg("--plugin-dir");
                    cmd.arg(plugin_root);
                }
            }
        } else {
            let profile_dir = self.profile_dir(&profile);
            if !profile_dir.exists() {
                bail!(
                    "Profile directory for '{}' not found. Re-add it with: cswitch add --full {}",
                    profile.name,
                    profile.name
                );
            }
            cmd.env("CLAUDE_CONFIG_DIR", &profile_dir);
        }

        let status = cmd
            .status()
            .context("Failed to launch claude. Is it installed and in your PATH?")?;
        std::process::exit(status.code().unwrap_or(0));
    }

    // ── Aliases ──────────────────────────────────────────────────────────────

    #[cfg(target_os = "windows")]
    pub fn generate_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        // Always sync — even when empty (cleans up stale .cmd files)
        let cmd_report = self.sync_cmd_aliases()?;
        if profiles.is_empty() {
            return Ok(format!(
                "# No profiles found. Add one with: cswitch add <name>\n\n{}",
                cmd_report
            ));
        }
        let ps = self.generate_powershell_aliases(&profiles)?;
        Ok(format!("{}\n\n{}", ps, cmd_report))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn generate_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        if profiles.is_empty() {
            return Ok("# No profiles found. Add one with: cswitch add <name>".to_string());
        }
        // If ~/.varusers/bin exists, sync self-contained shell scripts there.
        // Otherwise fall back to bash-aliases / bashrc.d.
        if let Ok(bin) = Self::sh_bin_dir() {
            if bin.exists() && bin.is_dir() {
                let report = self.sync_sh_scripts()?;
                return Ok(format!(
                    "# Shell scripts synced to {}\n{}",
                    bin.display(),
                    report
                ));
            }
        }
        self.generate_shell_aliases_with_bashrc_d(&profiles)
    }

    #[cfg(not(target_os = "windows"))]
    fn generate_shell_aliases_with_bashrc_d(&self, profiles: &[Profile]) -> Result<String> {
        let aliases_content = self.generate_shell_aliases(profiles)?;
        let home = Self::home_dir()?;
        let bashrc_d = home.join(".bashrc.d");

        if bashrc_d.exists() && bashrc_d.is_dir() {
            fs::create_dir_all(&bashrc_d)?;
            let alias_file = bashrc_d.join("38-claude-switch.sh");
            fs::write(&alias_file, &aliases_content)?;
            Ok(format!(
                "{}\n\n# Aliases written to: ~/.bashrc.d/38-claude-switch.sh",
                aliases_content
            ))
        } else {
            Ok(aliases_content)
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn generate_shell_aliases(&self, profiles: &[Profile]) -> Result<String> {
        let mut lines = vec![
            "# claude-switch aliases — auto-generated by `cswitch aliases`".to_string(),
            String::new(),
        ];
        for p in profiles {
            if p.kind == ProfileKind::Lightweight {
                lines.push(format!(
                    "# Profile '{}' (lightweight): use 'cswitch use {}'",
                    p.name, p.name
                ));
                continue;
            }
            let dir = self.profile_dir(p);
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            lines.push(format!(
                "alias claude-{}=\"CLAUDE_CONFIG_DIR='{}' claude\"",
                alias_name,
                dir.display(),
            ));
        }
        Ok(lines.join("\n"))
    }

    fn generate_powershell_aliases(&self, profiles: &[Profile]) -> Result<String> {
        let mut lines = vec![
            "# claude-switch aliases — add to your PowerShell profile".to_string(),
            "# Run: notepad $PROFILE  to edit your profile".to_string(),
            "# Generated by: cswitch aliases".to_string(),
            String::new(),
        ];
        for p in profiles {
            if p.kind == ProfileKind::Lightweight {
                lines.push(format!(
                    "# Profile '{}' (lightweight): set env vars before running claude",
                    p.name
                ));
                continue;
            }
            let dir = self.profile_dir(p);
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            lines.push(format!(
                "function claude-{} {{ $env:CLAUDE_CONFIG_DIR='{}'; claude @args }}",
                alias_name,
                dir.display(),
            ));
        }
        Ok(lines.join("\n"))
    }

    pub fn sync_remote_aliases_with_progress<F>(
        &self,
        host: &str,
        verbose: bool,
        mut progress: F,
    ) -> Result<String>
    where
        F: FnMut(&str),
    {
        let profiles = self.list_profiles()?;
        let mut skipped_full_profiles = Vec::new();
        if verbose {
            progress(&format!(
                "[remote:{host}] probing remote OS and home via sftp pwd for {} profile(s)...",
                profiles.len()
            ));
        }
        let (remote_os, remote_home) = Self::probe_remote_os_and_home(host)?;
        let remote_bin_dir = match remote_os {
            RemoteOs::Unix => format!("{}/.varusers/bin", remote_home.trim_end_matches('/')),
            RemoteOs::Windows => {
                format!("{}\\.local\\bin", remote_home.trim_end_matches(['\\', '/']))
            }
        };
        let remote_generated_root = Self::remote_generated_root_dir(&remote_home, remote_os);
        let remote_prompts_dir =
            Self::join_remote_path(&remote_generated_root, remote_os, "prompts");
        let remote_plugins_dir =
            Self::join_remote_path(&remote_generated_root, remote_os, "plugins");
        if verbose {
            progress(&format!(
                "[remote:{host}] detected {:?}, home: {}, shim dir: {}",
                remote_os, remote_home, remote_bin_dir
            ));
        }

        if verbose {
            progress(&format!(
                "[remote:{host}] ensuring remote directory exists..."
            ));
        }
        Self::ensure_remote_dir(host, &remote_bin_dir)?;

        let remote_tool_shell = match remote_os {
            RemoteOs::Unix => TinyfishToolShell::Bash,
            RemoteOs::Windows => TinyfishToolShell::PowerShell,
        };
        let mut desired_shims: Vec<(String, String)> = Vec::new();
        let mut desired_prompts: Vec<(String, String)> = Vec::new();
        let mut desired_plugins: Vec<(String, String)> = Vec::new();
        let mut desired_mcps: Vec<(String, String)> = Vec::new();
        let mut desired_prompt_names = std::collections::HashSet::new();
        let mut desired_plugin_names = std::collections::HashSet::new();
        let mut desired_mcp_names = std::collections::HashSet::new();
        for profile in &profiles {
            let alias_name = profile.alias.as_deref().unwrap_or(&profile.name);
            let Some(file_name) = Self::remote_shim_file_name(profile, remote_os) else {
                skipped_full_profiles.push(alias_name.to_string());
                if verbose {
                    progress(&format!(
                        "[remote:{host}] skipping full profile for remote sync: {}",
                        alias_name
                    ));
                }
                continue;
            };
            let content = match remote_os {
                RemoteOs::Windows => self.generate_cmd_content(profile)?,
                RemoteOs::Unix => self.generate_sh_content(profile)?,
            };
            desired_shims.push((file_name, content));

            if profile.kind == ProfileKind::Lightweight
                && let Some(env) = profile.env.as_ref()
            {
                let (token, url) = self.resolve_credentials(profile)?;
                let artifacts = build_lightweight_runtime_artifacts(
                    env,
                    token.as_deref(),
                    url.as_deref(),
                    remote_tool_shell,
                )?;
                let tinyfish_mode = artifacts.tinyfish_mode;
                if let (Some(plugin_manifest_json), Some(plugin_hooks_json), Some(prompt_text)) = (
                    artifacts.tinyfish_plugin_manifest_json,
                    artifacts.tinyfish_plugin_hooks_json,
                    artifacts.tinyfish_prompt_text,
                ) {
                    let prompt_name =
                        Self::tinyfish_prompt_file_name(tinyfish_mode, remote_tool_shell)
                            .expect("TinyFish prompt file name should exist for non-native mode");
                    if desired_prompt_names.insert(prompt_name.clone()) {
                        desired_prompts.push((prompt_name, prompt_text));
                    }
                    let plugin_name = Self::tinyfish_plugin_dir_name(tinyfish_mode)
                        .expect("TinyFish plugin dir name should exist for non-native mode");
                    if desired_plugin_names.insert(plugin_name) {
                        desired_plugins.push((
                            Self::tinyfish_plugin_manifest_relative_path(tinyfish_mode, remote_os),
                            plugin_manifest_json,
                        ));
                        desired_plugins.push((
                            Self::tinyfish_plugin_hooks_relative_path(tinyfish_mode, remote_os),
                            plugin_hooks_json,
                        ));
                    }
                }

                let mcp_servers = self.profile_mcp_servers(profile)?;
                if !mcp_servers.is_empty() {
                    let mcp_plugin_name = Self::profile_mcp_plugin_dir_name(profile);
                    if desired_mcp_names.insert(mcp_plugin_name) {
                        desired_mcps.push((
                            Self::profile_mcp_manifest_relative_path(profile, remote_os),
                            Self::profile_mcp_plugin_manifest(profile)?,
                        ));
                        let mcp_config = Self::profile_mcp_config(&mcp_servers)?;
                        for config_path in
                            Self::profile_mcp_config_relative_paths(profile, remote_os)
                        {
                            desired_mcps.push((config_path, mcp_config.clone()));
                        }
                    }
                }
            }
        }
        if verbose {
            progress(&format!(
                "[remote:{host}] building {} remote shim(s), {} TinyFish prompt file(s), {} TinyFish plugin file(s), {} MCP plugin file(s); skipping {} full profile(s)...",
                desired_shims.len(),
                desired_prompts.len(),
                desired_plugins.len(),
                desired_mcps.len(),
                skipped_full_profiles.len()
            ));
        }

        if verbose {
            progress(&format!(
                "[remote:{host}] listing existing files in remote shim directory..."
            ));
        }
        let existing_shims = Self::list_remote_files_if_present(host, &remote_bin_dir, remote_os)?;
        let existing_shims_total = existing_shims.len();
        let managed_existing_shims: std::collections::HashSet<String> = existing_shims
            .into_iter()
            .filter(|name| Self::is_managed_remote_name(remote_os, name))
            .collect();
        let existing_shims_managed_count = managed_existing_shims.len();
        let ignored_shims_count = existing_shims_total.saturating_sub(existing_shims_managed_count);
        let existing_prompts =
            Self::list_remote_files_if_present(host, &remote_prompts_dir, remote_os)?;
        let existing_prompts_total = existing_prompts.len();
        let managed_existing_prompts: std::collections::HashSet<String> = existing_prompts
            .into_iter()
            .filter(|name| Self::is_managed_generated_prompt_name(name))
            .collect();
        let ignored_prompts_count =
            existing_prompts_total.saturating_sub(managed_existing_prompts.len());
        let existing_plugins =
            Self::list_remote_files_if_present(host, &remote_plugins_dir, remote_os)?;
        let existing_plugins_total = existing_plugins.len();
        let managed_existing_plugins: std::collections::HashSet<String> = existing_plugins
            .into_iter()
            .filter(|name| Self::is_managed_generated_plugin_dir_name(name))
            .collect();
        let ignored_plugins_count =
            existing_plugins_total.saturating_sub(managed_existing_plugins.len());
        let remote_mcps_dir = Self::join_remote_path(&remote_generated_root, remote_os, "mcps");
        let existing_mcps = Self::list_remote_files_if_present(host, &remote_mcps_dir, remote_os)?;
        let existing_mcps_total = existing_mcps.len();
        let managed_existing_mcps: std::collections::HashSet<String> = existing_mcps
            .into_iter()
            .filter(|name| Self::is_managed_generated_mcp_dir_name(name))
            .collect();
        let ignored_mcps_count = existing_mcps_total.saturating_sub(managed_existing_mcps.len());
        let ignored_count = ignored_shims_count
            + ignored_prompts_count
            + ignored_plugins_count
            + ignored_mcps_count;

        if verbose {
            progress(&format!(
                "[remote:{host}] found {} managed shim(s), {} managed prompt file(s), {} managed TinyFish plugin dir(s), {} managed MCP plugin dir(s); ignoring {} unrelated file(s)",
                existing_shims_managed_count,
                managed_existing_prompts.len(),
                managed_existing_plugins.len(),
                managed_existing_mcps.len(),
                ignored_count
            ));
        }

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut removed = 0usize;
        let mut details = Vec::new();

        if !desired_prompts.is_empty() || !managed_existing_prompts.is_empty() {
            Self::ensure_remote_dir(host, &remote_prompts_dir)?;
        }
        if !desired_plugins.is_empty() || !managed_existing_plugins.is_empty() {
            Self::ensure_remote_dir(host, &remote_plugins_dir)?;
        }
        if !desired_mcps.is_empty() || !managed_existing_mcps.is_empty() {
            Self::ensure_remote_dir(host, &remote_mcps_dir)?;
        }

        if verbose && !desired_shims.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} shim(s) via sftp batch...",
                desired_shims.len()
            ));
        }
        if !desired_shims.is_empty() {
            Self::upload_remote_files(host, &remote_bin_dir, remote_os, &desired_shims, true)?;
        }
        if verbose && !desired_prompts.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} shared TinyFish prompt file(s)...",
                desired_prompts.len()
            ));
        }
        if !desired_prompts.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_prompts_dir,
                remote_os,
                &desired_prompts,
                false,
            )?;
        }
        if verbose && !desired_plugins.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} shared TinyFish plugin file(s)...",
                desired_plugins.len()
            ));
        }
        if !desired_plugins.is_empty() {
            Self::upload_remote_files(
                host,
                &remote_plugins_dir,
                remote_os,
                &desired_plugins,
                false,
            )?;
        }
        if verbose && !desired_mcps.is_empty() {
            progress(&format!(
                "[remote:{host}] uploading {} MCP plugin file(s)...",
                desired_mcps.len()
            ));
        }
        if !desired_mcps.is_empty() {
            Self::upload_remote_files(host, &remote_mcps_dir, remote_os, &desired_mcps, false)?;
        }

        for (file_name, _) in &desired_shims {
            let remote_path = Self::join_remote_path(&remote_bin_dir, remote_os, file_name);
            if managed_existing_shims.contains(file_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in &desired_prompts {
            let remote_path = Self::join_remote_path(&remote_prompts_dir, remote_os, file_name);
            if managed_existing_prompts.contains(file_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in &desired_plugins {
            let remote_path = Self::join_remote_path(&remote_plugins_dir, remote_os, file_name);
            let plugin_dir_name = file_name
                .split(['/', '\\'])
                .next()
                .expect("plugin file path should include root dir");
            if managed_existing_plugins.contains(plugin_dir_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        for (file_name, _) in &desired_mcps {
            let remote_path = Self::join_remote_path(&remote_mcps_dir, remote_os, file_name);
            let plugin_dir_name = file_name
                .split(['/', '\\'])
                .next()
                .expect("MCP plugin file path should include root dir");
            if managed_existing_mcps.contains(plugin_dir_name) {
                updated += 1;
                if verbose {
                    details.push(format!("  = {}:{}", host, remote_path));
                }
            } else {
                added += 1;
                if verbose {
                    details.push(format!("  + {}:{}", host, remote_path));
                }
            }
        }

        let desired_shim_names: std::collections::HashSet<&str> = desired_shims
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let stale_shims: Vec<String> = managed_existing_shims
            .iter()
            .filter(|name| !desired_shim_names.contains(name.as_str()))
            .cloned()
            .collect();
        let stale_prompt_count = managed_existing_prompts
            .iter()
            .filter(|name| !desired_prompts.iter().any(|(desired, _)| desired == *name))
            .count();
        let stale_plugin_count = managed_existing_plugins
            .iter()
            .filter(|name| !desired_plugin_names.contains(*name))
            .count();
        let stale_mcp_count = managed_existing_mcps
            .iter()
            .filter(|name| !desired_mcp_names.contains(*name))
            .count();
        if verbose {
            progress(&format!(
                "[remote:{host}] checking {} stale shim(s), {} stale prompt file(s), {} stale TinyFish plugin dir(s), {} stale MCP plugin dir(s)...",
                stale_shims.len(),
                stale_prompt_count,
                stale_plugin_count,
                stale_mcp_count
            ));
        }
        for stale in stale_shims {
            let remote_path = Self::join_remote_path(&remote_bin_dir, remote_os, &stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] inspecting stale managed shim: {}",
                    remote_path
                ));
            }
            if Self::remote_file_has_marker(host, &remote_path, remote_os)? {
                if verbose {
                    progress(&format!(
                        "[remote:{host}] removing stale managed shim: {}",
                        remote_path
                    ));
                    details.push(format!("  - {}:{} (stale)", host, remote_path));
                }
                Self::remove_remote_file(host, &remote_path, remote_os)?;
                removed += 1;
            }
        }

        let desired_prompt_names: std::collections::HashSet<&str> = desired_prompts
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        for stale in managed_existing_prompts
            .iter()
            .filter(|name| !desired_prompt_names.contains(name.as_str()))
        {
            let remote_path = Self::join_remote_path(&remote_prompts_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale TinyFish prompt file: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_file(host, &remote_path, remote_os)?;
            removed += 1;
        }

        for stale in managed_existing_plugins
            .iter()
            .filter(|name| !desired_plugin_names.contains(*name))
        {
            let remote_path = Self::join_remote_path(&remote_plugins_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale TinyFish plugin dir: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_plugin_dir(host, &remote_path, remote_os)?;
            removed += 1;
        }

        for stale in managed_existing_mcps
            .iter()
            .filter(|name| !desired_mcp_names.contains(*name))
        {
            let remote_path = Self::join_remote_path(&remote_mcps_dir, remote_os, stale);
            if verbose {
                progress(&format!(
                    "[remote:{host}] removing stale MCP plugin dir: {}",
                    remote_path
                ));
                details.push(format!("  - {}:{} (stale)", host, remote_path));
            }
            Self::remove_remote_mcp_plugin_dir(host, &remote_path, remote_os)?;
            removed += 1;
        }

        if verbose {
            progress(&format!("[remote:{host}] remote shim sync complete"));
        }

        let summary = format!(
            "# Remote aliases synced to {} on {} ({:?}): {} added, {} updated, {} removed{}{}",
            remote_bin_dir,
            host,
            remote_os,
            added,
            updated,
            removed,
            if ignored_count > 0 {
                format!(", {} unrelated files ignored", ignored_count)
            } else {
                String::new()
            },
            if skipped_full_profiles.is_empty() {
                String::new()
            } else {
                format!(", {} full profile(s) skipped", skipped_full_profiles.len())
            }
        );
        if verbose {
            let mut output = vec![summary];
            if !skipped_full_profiles.is_empty() {
                output.extend(skipped_full_profiles.iter().map(|profile| {
                    format!("  ! skipped full profile for remote sync: {}", profile)
                }));
            }
            if !details.is_empty() {
                output.extend(details);
            }
            Ok(output.join("\n"))
        } else {
            Ok(summary)
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    pub fn base_dir(&self) -> PathBuf {
        self.registry_path
            .parent()
            .expect("registry path should always live under a base directory")
            .to_path_buf()
    }

    fn diagnostic(
        level: DiagnosticLevel,
        area: impl Into<String>,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> DiagnosticItem {
        DiagnosticItem {
            level,
            area: area.into(),
            message: message.into(),
            hint,
        }
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

    fn registry_backup_path(&self) -> PathBuf {
        let timestamp = Utc::now().format("%Y%m%d%H%M%S");
        self.registry_path
            .with_file_name(format!("registry.json.bak-{timestamp}"))
    }

    fn is_recoverable_shim_file_name(file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        (lower.starts_with("claude-") && lower.ends_with(".cmd"))
            || (lower.starts_with("claude-") && !lower.contains('.'))
    }

    fn parse_recoverable_shim(file_name: &str, content: &str) -> Result<RecoveredShimProfile> {
        if !content.contains(CMD_MARKER) && !content.contains(SH_MARKER) {
            bail!("not a cswitch generated shim");
        }
        let alias = Self::alias_from_shim_file_name(file_name)?;
        let (name, kind) = Self::parse_shim_profile_header(content)
            .with_context(|| "missing generated profile header".to_string())?;
        if kind != ProfileKind::Lightweight {
            bail!("only lightweight shims can be recovered");
        }
        let settings = Self::extract_shim_settings(content)
            .with_context(|| "missing recoverable --settings JSON".to_string())?;
        let env_object = settings
            .get("env")
            .and_then(serde_json::Value::as_object)
            .with_context(|| "settings JSON does not contain an env object".to_string())?;
        let token = Self::json_env_string(env_object, "ANTHROPIC_AUTH_TOKEN")?
            .with_context(|| "settings env is missing ANTHROPIC_AUTH_TOKEN".to_string())?;
        let base_url = Self::json_env_string(env_object, "ANTHROPIC_BASE_URL")?
            .with_context(|| "settings env is missing ANTHROPIC_BASE_URL".to_string())?;
        let mut extras = Vec::new();
        for (key, value) in env_object {
            if Self::known_lightweight_env_key(key) {
                continue;
            }
            if let Some(value) = value.as_str() {
                extras.push(format!("{key}={value}"));
            }
        }
        extras.sort();
        let env = LightweightEnv {
            auth_token: None,
            base_url: None,
            default_opus_model: Self::json_env_string(env_object, "ANTHROPIC_DEFAULT_OPUS_MODEL")?,
            default_sonnet_model: Self::json_env_string(
                env_object,
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
            )?,
            default_haiku_model: Self::json_env_string(
                env_object,
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            )?,
            model: Self::json_env_string(env_object, "ANTHROPIC_MODEL")?,
            subagent_model: Self::json_env_string(env_object, "CLAUDE_CODE_SUBAGENT_MODEL")?,
            extras,
        };
        Ok(RecoveredShimProfile {
            file_name: file_name.to_string(),
            name,
            alias,
            token,
            base_url,
            env,
            launch_args: Self::extract_shim_launch_args(content),
        })
    }

    fn alias_from_shim_file_name(file_name: &str) -> Result<String> {
        let stem = file_name
            .strip_suffix(".cmd")
            .or_else(|| file_name.strip_suffix(".CMD"))
            .unwrap_or(file_name);
        let alias = stem
            .strip_prefix("claude-")
            .with_context(|| format!("shim '{}' does not use the claude- prefix", file_name))?;
        if alias.trim().is_empty() {
            bail!("shim '{}' has an empty alias", file_name);
        }
        Self::validate_alias(alias)?;
        Ok(alias.to_string())
    }

    fn parse_shim_profile_header(content: &str) -> Option<(String, ProfileKind)> {
        for line in content.lines() {
            let trimmed = line.trim();
            let header = trimmed
                .strip_prefix(":: Profile: ")
                .or_else(|| trimmed.strip_prefix("# Profile: "));
            let Some(header) = header else {
                continue;
            };
            let (name, kind) = header.rsplit_once(" (")?;
            let kind = kind.strip_suffix(')')?;
            let kind = match kind {
                "lightweight" => ProfileKind::Lightweight,
                "full" => ProfileKind::Full,
                _ => return None,
            };
            return Some((name.to_string(), kind));
        }
        None
    }

    fn extract_shim_settings(content: &str) -> Result<serde_json::Value> {
        let candidates = [
            Self::extract_cmd_var(content, "_SETTINGS"),
            Self::extract_cmd_var(content, "_TF_SETTINGS"),
            Self::extract_legacy_inline_cmd_settings(content),
            Self::extract_shell_settings_env(content),
        ];
        for candidate in candidates.into_iter().flatten() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&candidate)
                && value.get("env").is_some()
            {
                return Ok(value);
            }
        }
        bail!("no parseable settings JSON found")
    }

    fn extract_cmd_var(content: &str, var_name: &str) -> Option<String> {
        let prefix = format!("set \"{var_name}=");
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix('"'))
            {
                return Some(Self::unescape_cmd_json_fragment(value));
            }
        }
        None
    }

    fn extract_legacy_inline_cmd_settings(content: &str) -> Option<String> {
        let marker = "claude --settings \"";
        let start = content.find(marker)? + marker.len();
        let rest = &content[start..];
        let end = Self::find_cmd_quoted_value_end(rest)?;
        Some(Self::unescape_cmd_json_fragment(&rest[..end]))
    }

    fn find_cmd_quoted_value_end(value: &str) -> Option<usize> {
        let mut backslashes = 0usize;
        for (idx, ch) in value.char_indices() {
            if ch == '\\' {
                backslashes += 1;
                continue;
            }
            if ch == '"' && backslashes.is_multiple_of(2) {
                return Some(idx);
            }
            backslashes = 0;
        }
        None
    }

    fn extract_shell_settings_env(content: &str) -> Option<String> {
        for line in content.lines() {
            let line = line.trim();
            let Some(value) = line.strip_prefix("SETTINGS_ENV=") else {
                continue;
            };
            let value = value.strip_prefix('\'')?.strip_suffix('\'')?;
            let mut settings = Self::unescape_shell_single_quoted_value(value);
            settings.push('}');
            return Some(settings);
        }
        None
    }

    fn unescape_cmd_json_fragment(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' if chars.peek() == Some(&'"') => {
                    chars.next();
                    out.push('"');
                }
                '^' => {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                '%' if chars.peek() == Some(&'%') => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push(ch),
            }
        }
        out
    }

    fn unescape_shell_single_quoted_value(value: &str) -> String {
        value.replace("'\\''", "'")
    }

    fn json_env_string(
        object: &serde_json::Map<String, serde_json::Value>,
        key: &str,
    ) -> Result<Option<String>> {
        match object.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => bail!("settings env field '{}' must be a string", key),
        }
    }

    fn known_lightweight_env_key(key: &str) -> bool {
        matches!(
            key,
            "ANTHROPIC_AUTH_TOKEN"
                | "ANTHROPIC_BASE_URL"
                | "ANTHROPIC_DEFAULT_OPUS_MODEL"
                | "ANTHROPIC_DEFAULT_SONNET_MODEL"
                | "ANTHROPIC_DEFAULT_HAIKU_MODEL"
                | "ANTHROPIC_MODEL"
                | "CLAUDE_CODE_SUBAGENT_MODEL"
        )
    }

    fn extract_shim_launch_args(content: &str) -> Option<Vec<String>> {
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line
                .strip_prefix("set \"_LAUNCH_ARGS=")
                .and_then(|value| value.strip_suffix('"'))
            {
                return Some(Self::split_recovered_launch_args(value));
            }
        }
        None
    }

    fn split_recovered_launch_args(value: &str) -> Vec<String> {
        value
            .split_whitespace()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect()
    }

    fn find_profile_conflict_id(
        registry: &Registry,
        name: &str,
        alias: Option<&str>,
    ) -> Result<Option<String>> {
        let by_name = registry
            .profiles
            .iter()
            .find(|(_, profile)| profile.name == name)
            .map(|(id, _)| id.clone());
        let by_alias = alias.and_then(|alias| {
            registry
                .profiles
                .iter()
                .find(|(_, profile)| profile.alias.as_deref() == Some(alias))
                .map(|(id, _)| id.clone())
        });
        match (by_name, by_alias) {
            (Some(left), Some(right)) if left != right => bail!(
                "name '{}' and alias '{}' match different existing profiles",
                name,
                alias.unwrap_or_default()
            ),
            (Some(id), _) | (_, Some(id)) => Ok(Some(id)),
            (None, None) => Ok(None),
        }
    }

    fn ensure_recovered_provider_key(
        registry: &mut Registry,
        plan: &mut ShimRecoveryPlan,
        provider_names: &mut HashSet<String>,
        recovered: &RecoveredShimProfile,
    ) -> (String, String) {
        if let Some((provider_id, key_id)) = Self::find_provider_key_by_url_and_token(
            registry,
            &recovered.base_url,
            &recovered.token,
        ) {
            plan.provider_keys_reused += 1;
            return (provider_id, key_id);
        }

        let provider_id = if let Some((id, _)) = registry
            .providers
            .iter()
            .find(|(_, provider)| provider.base_url == recovered.base_url)
        {
            id.clone()
        } else {
            let id = Self::new_unique_provider_id(registry);
            let name = Self::unique_recovered_provider_name(provider_names, &recovered.base_url);
            registry.providers.insert(
                id.clone(),
                Provider {
                    id: id.clone(),
                    name: name.clone(),
                    base_url: recovered.base_url.clone(),
                    keys: HashMap::new(),
                    api_key: String::new(),
                },
            );
            plan.providers_added += 1;
            plan.providers_add
                .push(format!("{} ({})", name, recovered.base_url));
            id
        };

        let key_id = Self::new_unique_key_id(
            registry
                .providers
                .get(&provider_id)
                .expect("provider was just created or found"),
        );
        let key_name = Self::unique_recovered_key_name(
            registry
                .providers
                .get(&provider_id)
                .expect("provider was just created or found"),
            &recovered.alias,
        );
        let provider = registry
            .providers
            .get_mut(&provider_id)
            .expect("provider was just created or found");
        provider.keys.insert(
            key_id.clone(),
            ProviderKey {
                id: key_id.clone(),
                name: key_name.clone(),
                api_key: recovered.token.clone(),
            },
        );
        plan.provider_keys_added += 1;
        plan.provider_keys_add.push(format!(
            "{} / {} from {}",
            provider.name, key_name, recovered.file_name
        ));
        (provider_id, key_id)
    }

    fn find_provider_key_by_url_and_token(
        registry: &Registry,
        base_url: &str,
        token: &str,
    ) -> Option<(String, String)> {
        for (provider_id, provider) in &registry.providers {
            if provider.base_url != base_url {
                continue;
            }
            for (key_id, key) in &provider.keys {
                if key.api_key == token {
                    return Some((provider_id.clone(), key_id.clone()));
                }
            }
        }
        None
    }

    fn new_unique_provider_id(registry: &Registry) -> String {
        loop {
            let id = format!("prov_{}", &Uuid::new_v4().to_string()[..8]);
            if !registry.providers.contains_key(&id) {
                return id;
            }
        }
    }

    fn new_unique_key_id(provider: &Provider) -> String {
        loop {
            let id = format!("key_{}", &Uuid::new_v4().to_string()[..8]);
            if !provider.keys.contains_key(&id) {
                return id;
            }
        }
    }

    fn unique_recovered_provider_name(names: &mut HashSet<String>, base_url: &str) -> String {
        let mut base = "Recovered provider".to_string();
        if let Some(host) = Self::host_from_url(base_url)
            && !host.is_empty()
        {
            base = format!("Recovered {host}");
        }
        let mut candidate = base.clone();
        let mut index = 2usize;
        while names.contains(&candidate) {
            candidate = format!("{base} {index}");
            index += 1;
        }
        names.insert(candidate.clone());
        candidate
    }

    fn host_from_url(url: &str) -> Option<String> {
        let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
        let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
        let host = authority.split(':').next()?.trim();
        if host.is_empty() {
            None
        } else {
            Some(host.to_string())
        }
    }

    fn unique_recovered_key_name(provider: &Provider, alias: &str) -> String {
        let base = format!("Recovered {alias}");
        let names = provider
            .keys
            .values()
            .map(|key| key.name.as_str())
            .collect::<HashSet<_>>();
        if !names.contains(base.as_str()) {
            return base;
        }
        let mut index = 2usize;
        loop {
            let candidate = format!("{base} {index}");
            if !names.contains(candidate.as_str()) {
                return candidate;
            }
            index += 1;
        }
    }

    fn build_recovered_profile(
        existing_id: Option<String>,
        recovered: &RecoveredShimProfile,
        provider_id: String,
        key_id: String,
        existing: Option<&Profile>,
    ) -> Profile {
        Profile {
            id: existing_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: recovered.name.clone(),
            alias: Some(recovered.alias.clone()),
            added: existing
                .map(|profile| profile.added)
                .unwrap_or_else(Utc::now),
            last_used: existing.and_then(|profile| profile.last_used),
            kind: ProfileKind::Lightweight,
            env: Some(recovered.env.clone()),
            launch_args: recovered.launch_args.clone(),
            provider_id: Some(provider_id),
            key_id: Some(key_id),
            mcp_server_ids: existing
                .map(|profile| profile.mcp_server_ids.clone())
                .unwrap_or_default(),
        }
    }

    fn selected_mcp_servers_in_registry(
        registry: &Registry,
        queries: &[String],
        all: bool,
    ) -> Result<Vec<McpServer>> {
        let mut servers = Vec::new();
        if all || queries.is_empty() {
            servers.extend(registry.mcp_servers.values().cloned());
        } else {
            let mut seen = std::collections::HashSet::new();
            for query in queries {
                let (id, server) = Self::find_mcp_server_in_registry(registry, query)?;
                if seen.insert(id) {
                    servers.push(server);
                }
            }
        }
        servers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        Ok(servers)
    }

    fn mcp_issue(
        level: DiagnosticLevel,
        server: &McpServer,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> McpValidationIssue {
        McpValidationIssue {
            level,
            server_id: server.id.clone(),
            server_name: server.name.clone(),
            message: message.into(),
            hint,
        }
    }

    fn validate_mcp_server_config(server: &McpServer) -> Vec<McpValidationIssue> {
        let mut issues = Vec::new();
        if server.name.trim().is_empty() {
            issues.push(Self::mcp_issue(
                DiagnosticLevel::Error,
                server,
                "name is empty",
                Some("rename or recreate this MCP server".to_string()),
            ));
        }

        let server_type = server.server_type.trim().to_ascii_lowercase();
        match server_type.as_str() {
            "stdio" => {
                if server
                    .command
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Error,
                        server,
                        "stdio server is missing command",
                        Some("set --command for this MCP server".to_string()),
                    ));
                } else if let Some(command) = &server.command
                    && !Self::looks_like_variable(command)
                    && !Self::command_exists(command)
                {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Warn,
                        server,
                        format!("command '{}' is not currently on PATH", command),
                        Some(
                            "install the command or rely on Claude-time environment setup"
                                .to_string(),
                        ),
                    ));
                }
                if server.url.is_some() {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Warn,
                        server,
                        "stdio server also has a URL field",
                        Some("remove --url unless this is intentional metadata".to_string()),
                    ));
                }
                if let Some(cwd) = &server.cwd
                    && !Self::looks_like_variable(cwd)
                    && !Path::new(cwd).exists()
                {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Warn,
                        server,
                        format!("cwd '{}' does not exist", cwd),
                        Some("create the directory or update --cwd".to_string()),
                    ));
                }
            }
            "http" | "streamable-http" | "sse" => {
                if server
                    .url
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Error,
                        server,
                        format!("{} server is missing URL", server_type),
                        Some("set --url for this MCP server".to_string()),
                    ));
                }
                if server.command.is_some() || !server.args.is_empty() || server.cwd.is_some() {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Warn,
                        server,
                        "remote server has stdio-only fields",
                        Some(
                            "clear command, args, and cwd unless they are intentional metadata"
                                .to_string(),
                        ),
                    ));
                }
                if server_type == "sse" {
                    issues.push(Self::mcp_issue(
                        DiagnosticLevel::Warn,
                        server,
                        "sse transport is deprecated",
                        Some(
                            "prefer streamable-http when the remote server supports it".to_string(),
                        ),
                    ));
                }
            }
            _ => issues.push(Self::mcp_issue(
                DiagnosticLevel::Error,
                server,
                format!("type '{}' is invalid", server.server_type),
                Some("use stdio, http, streamable-http, or sse".to_string()),
            )),
        }

        for field in [&server.env, &server.headers] {
            if field.keys().any(|key| key.trim().is_empty()) {
                issues.push(Self::mcp_issue(
                    DiagnosticLevel::Error,
                    server,
                    "env or header map contains an empty key",
                    Some("remove the empty key from this MCP server".to_string()),
                ));
            }
        }
        if server.timeout == Some(0) {
            issues.push(Self::mcp_issue(
                DiagnosticLevel::Warn,
                server,
                "timeout is 0 ms",
                Some("set a positive timeout or clear the timeout field".to_string()),
            ));
        }
        if server.disabled == Some(true) {
            issues.push(Self::mcp_issue(
                DiagnosticLevel::Warn,
                server,
                "server is disabled",
                Some(
                    "clear disabled or set it to false before expecting tools to load".to_string(),
                ),
            ));
        }
        issues
    }

    fn looks_like_variable(value: &str) -> bool {
        value.contains("${") || value.contains('%')
    }

    fn command_exists(command: &str) -> bool {
        let command = command.trim();
        if command.is_empty() {
            return false;
        }
        if command.contains('/') || command.contains('\\') {
            return Path::new(command).is_file();
        }
        let Some(paths) = env::var_os("PATH") else {
            return false;
        };

        #[cfg(target_os = "windows")]
        {
            let has_extension = Path::new(command).extension().is_some();
            let pathext = env::var_os("PATHEXT")
                .map(|value| {
                    value
                        .to_string_lossy()
                        .split(';')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|values| !values.is_empty())
                .unwrap_or_else(|| {
                    vec![".COM".into(), ".EXE".into(), ".BAT".into(), ".CMD".into()]
                });
            for dir in env::split_paths(&paths) {
                if dir.join(command).is_file() {
                    return true;
                }
                if !has_extension {
                    for ext in &pathext {
                        if dir.join(format!("{command}{ext}")).is_file() {
                            return true;
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            for dir in env::split_paths(&paths) {
                if dir.join(command).is_file() {
                    return true;
                }
            }
        }

        false
    }

    fn read_profile_marker(path: &Path) -> Result<Option<String>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read project marker '{}'.", path.display()))?;
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                return Ok(Some(line.to_string()));
            }
        }
        bail!(
            "Project marker '{}' does not contain a profile name.",
            path.display()
        )
    }

    fn managed_entry_names(dir: &Path, predicate: fn(&str) -> bool) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(ToString::to_string))
            .filter(|name| predicate(name))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn count_named_entries(dir: &Path, predicate: fn(&str) -> bool) -> usize {
        Self::managed_entry_names(dir, predicate).len()
    }

    fn generated_root_dir(&self) -> PathBuf {
        self.base_dir().join("generated")
    }

    fn generated_prompts_dir(&self) -> PathBuf {
        self.generated_root_dir().join("prompts")
    }

    fn generated_plugins_dir(&self) -> PathBuf {
        self.generated_root_dir().join("plugins")
    }

    fn generated_mcps_dir(&self) -> PathBuf {
        self.generated_root_dir().join("mcps")
    }

    fn profile_mcp_plugin_dir_name(profile: &Profile) -> String {
        let suffix: String = profile
            .id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(12)
            .collect();
        let suffix = if suffix.is_empty() {
            "unknown".to_string()
        } else {
            suffix
        };
        format!("cswitch-mcp-profile-{suffix}")
    }

    fn local_profile_mcp_plugin_root(&self, profile: &Profile) -> PathBuf {
        self.generated_mcps_dir()
            .join(Self::profile_mcp_plugin_dir_name(profile))
    }

    fn home_relative_profile_mcp_plugin_root(profile: &Profile, target_os: RemoteOs) -> String {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/mcps/{dir_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\mcps\\{dir_name}")
            }
        }
    }

    fn profile_mcp_manifest_relative_path(profile: &Profile, remote_os: RemoteOs) -> String {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.claude-plugin/plugin.json"),
            RemoteOs::Windows => format!("{dir_name}\\.claude-plugin\\plugin.json"),
        }
    }

    fn profile_mcp_config_relative_paths(profile: &Profile, remote_os: RemoteOs) -> [String; 2] {
        let dir_name = Self::profile_mcp_plugin_dir_name(profile);
        match remote_os {
            RemoteOs::Unix => [
                format!("{dir_name}/.mcp.json"),
                format!("{dir_name}/mcp.json"),
            ],
            RemoteOs::Windows => [
                format!("{dir_name}\\.mcp.json"),
                format!("{dir_name}\\mcp.json"),
            ],
        }
    }

    fn is_managed_generated_mcp_dir_name(file_name: &str) -> bool {
        file_name.starts_with("cswitch-mcp-profile-")
    }

    fn tinyfish_plugin_dir_name(mode: TinyfishMode) -> Option<String> {
        match mode {
            TinyfishMode::None => None,
            TinyfishMode::SearchOnly => Some("tinyfish-search-only".to_string()),
            TinyfishMode::FetchOnly => Some("tinyfish-fetch-only".to_string()),
            TinyfishMode::Full => Some("tinyfish-full".to_string()),
        }
    }

    fn tinyfish_prompt_file_name(
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
    ) -> Option<String> {
        let mode_name = match mode {
            TinyfishMode::None => return None,
            TinyfishMode::SearchOnly => "search-only",
            TinyfishMode::FetchOnly => "fetch-only",
            TinyfishMode::Full => "full",
        };
        let shell_name = match tool_shell {
            TinyfishToolShell::Bash => "bash",
            TinyfishToolShell::PowerShell => "powershell",
        };
        Some(format!("tinyfish-{mode_name}.{shell_name}.txt"))
    }

    fn local_tinyfish_plugin_root(&self, mode: TinyfishMode) -> PathBuf {
        self.generated_plugins_dir().join(
            Self::tinyfish_plugin_dir_name(mode)
                .expect("plugin path is only valid for TinyFish modes"),
        )
    }

    fn local_tinyfish_plugin_hooks_path(&self, mode: TinyfishMode) -> PathBuf {
        self.local_tinyfish_plugin_root(mode)
            .join("hooks")
            .join("hooks.json")
    }

    fn local_tinyfish_plugin_manifest_path(&self, mode: TinyfishMode) -> PathBuf {
        self.local_tinyfish_plugin_root(mode)
            .join(".claude-plugin")
            .join("plugin.json")
    }

    fn local_tinyfish_prompt_path(
        &self,
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
    ) -> PathBuf {
        self.generated_prompts_dir().join(
            Self::tinyfish_prompt_file_name(mode, tool_shell)
                .expect("prompt path is only valid for TinyFish modes"),
        )
    }

    fn home_relative_tinyfish_prompt_path(mode: TinyfishMode, target_os: RemoteOs) -> String {
        let file_name = Self::tinyfish_prompt_file_name(
            mode,
            match target_os {
                RemoteOs::Unix => TinyfishToolShell::Bash,
                RemoteOs::Windows => TinyfishToolShell::PowerShell,
            },
        )
        .expect("prompt path is only valid for TinyFish modes");
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/prompts/{file_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\prompts\\{file_name}")
            }
        }
    }

    fn home_relative_tinyfish_plugin_root(mode: TinyfishMode, target_os: RemoteOs) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match target_os {
            RemoteOs::Unix => format!("$HOME/.claude-switch/generated/plugins/{dir_name}"),
            RemoteOs::Windows => {
                format!("%USERPROFILE%\\.claude-switch\\generated\\plugins\\{dir_name}")
            }
        }
    }

    fn tinyfish_plugin_hooks_relative_path(mode: TinyfishMode, remote_os: RemoteOs) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/hooks/hooks.json"),
            RemoteOs::Windows => format!("{dir_name}\\hooks\\hooks.json"),
        }
    }

    fn tinyfish_plugin_manifest_relative_path(mode: TinyfishMode, remote_os: RemoteOs) -> String {
        let dir_name = Self::tinyfish_plugin_dir_name(mode)
            .expect("plugin path is only valid for TinyFish modes");
        match remote_os {
            RemoteOs::Unix => format!("{dir_name}/.claude-plugin/plugin.json"),
            RemoteOs::Windows => format!("{dir_name}\\.claude-plugin\\plugin.json"),
        }
    }

    fn remote_generated_root_dir(remote_home: &str, remote_os: RemoteOs) -> String {
        match remote_os {
            RemoteOs::Unix => {
                format!(
                    "{}/.claude-switch/generated",
                    remote_home.trim_end_matches('/')
                )
            }
            RemoteOs::Windows => format!(
                "{}\\.claude-switch\\generated",
                remote_home.trim_end_matches(['\\', '/'])
            ),
        }
    }

    fn is_managed_generated_prompt_name(file_name: &str) -> bool {
        file_name.starts_with("tinyfish-") && file_name.ends_with(".txt")
    }

    fn is_managed_generated_plugin_dir_name(file_name: &str) -> bool {
        matches!(
            file_name,
            "tinyfish-full" | "tinyfish-search-only" | "tinyfish-fetch-only"
        )
    }

    fn write_if_changed(path: &Path, content: &str) -> Result<()> {
        let needs_write = match fs::read_to_string(path) {
            Ok(existing) => existing != content,
            Err(_) => true,
        };
        if needs_write {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)?;
        }
        Ok(())
    }

    fn mcp_server_config_value(server: &McpServer) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(
            "type".into(),
            serde_json::Value::String(server.server_type.clone()),
        );
        if let Some(command) = &server.command {
            object.insert("command".into(), serde_json::Value::String(command.clone()));
        }
        if !server.args.is_empty() {
            object.insert("args".into(), serde_json::json!(server.args));
        }
        if !server.env.is_empty() {
            object.insert("env".into(), serde_json::json!(server.env));
        }
        if let Some(cwd) = &server.cwd {
            object.insert("cwd".into(), serde_json::Value::String(cwd.clone()));
        }
        if let Some(url) = &server.url {
            object.insert("url".into(), serde_json::Value::String(url.clone()));
        }
        if !server.headers.is_empty() {
            object.insert("headers".into(), serde_json::json!(server.headers));
        }
        if let Some(oauth) = &server.oauth {
            object.insert("oauth".into(), oauth.clone());
        }
        if let Some(headers_helper) = &server.headers_helper {
            object.insert(
                "headersHelper".into(),
                serde_json::Value::String(headers_helper.clone()),
            );
        }
        if let Some(timeout) = server.timeout {
            object.insert("timeout".into(), serde_json::json!(timeout));
        }
        if let Some(always_load) = server.always_load {
            object.insert("alwaysLoad".into(), serde_json::json!(always_load));
        }
        if let Some(disabled) = server.disabled {
            object.insert("disabled".into(), serde_json::json!(disabled));
        }
        serde_json::Value::Object(object)
    }

    fn profile_mcp_servers(&self, profile: &Profile) -> Result<Vec<McpServer>> {
        if profile.mcp_server_ids.is_empty() {
            return Ok(Vec::new());
        }
        if profile.kind != ProfileKind::Lightweight {
            bail!("MCP servers can only be linked to lightweight profiles.");
        }
        let registry = self.load_registry()?;
        let mut servers = Vec::new();
        for mcp_id in &profile.mcp_server_ids {
            let server = registry.mcp_servers.get(mcp_id).with_context(|| {
                format!(
                    "Profile '{}' references missing MCP '{}'.",
                    profile.name, mcp_id
                )
            })?;
            servers.push(server.clone());
        }
        Ok(servers)
    }

    fn profile_mcp_plugin_manifest(profile: &Profile) -> Result<String> {
        serde_json::to_string_pretty(&serde_json::json!({
            "name": Self::profile_mcp_plugin_dir_name(profile),
            "displayName": format!("claude-switch MCPs for {}", profile.name),
            "description": "Generated by claude-switch to attach selected MCP servers to this profile.",
        }))
        .context("Failed to serialize MCP plugin manifest JSON")
    }

    fn profile_mcp_config(servers: &[McpServer]) -> Result<String> {
        let mut mcp_servers = serde_json::Map::new();
        for server in servers {
            if mcp_servers.contains_key(&server.name) {
                bail!(
                    "Duplicate MCP server name '{}' in profile selection.",
                    server.name
                );
            }
            mcp_servers.insert(server.name.clone(), Self::mcp_server_config_value(server));
        }
        let root = serde_json::json!({
            "$schema": "https://json.schemastore.org/claude-code-settings.json",
            "mcpServers": mcp_servers,
        });
        serde_json::to_string_pretty(&root).context("Failed to serialize MCP config JSON")
    }

    fn upsert_local_profile_mcp_plugin(
        &self,
        profile: &Profile,
        servers: &[McpServer],
    ) -> Result<PathBuf> {
        let plugin_root = self.local_profile_mcp_plugin_root(profile);
        let manifest_path = plugin_root.join(".claude-plugin").join("plugin.json");
        let mcp_config = Self::profile_mcp_config(servers)?;
        Self::write_if_changed(&manifest_path, &Self::profile_mcp_plugin_manifest(profile)?)?;
        Self::write_if_changed(&plugin_root.join(".mcp.json"), &mcp_config)?;
        Self::write_if_changed(&plugin_root.join("mcp.json"), &mcp_config)?;
        Ok(plugin_root)
    }

    fn sync_local_mcp_artifacts(&self, profiles: &[Profile]) -> Result<()> {
        let mut desired = std::collections::HashSet::new();
        for profile in profiles {
            if profile.kind != ProfileKind::Lightweight || profile.mcp_server_ids.is_empty() {
                continue;
            }
            let servers = self.profile_mcp_servers(profile)?;
            self.upsert_local_profile_mcp_plugin(profile, &servers)?;
            desired.insert(Self::profile_mcp_plugin_dir_name(profile));
        }

        let mcps_dir = self.generated_mcps_dir();
        if let Ok(entries) = fs::read_dir(&mcps_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_mcp_dir_name(&file_name)
                    && !desired.contains(&file_name)
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
        Ok(())
    }

    fn remove_local_tinyfish_artifacts(&self, _profile_id: &str) -> Result<()> {
        Ok(())
    }

    fn upsert_local_tinyfish_artifacts(
        &self,
        mode: TinyfishMode,
        tool_shell: TinyfishToolShell,
        plugin_manifest_json: &str,
        plugin_hooks_json: &str,
        prompt_text: &str,
    ) -> Result<(PathBuf, PathBuf)> {
        let plugin_root = self.local_tinyfish_plugin_root(mode);
        let manifest_path = self.local_tinyfish_plugin_manifest_path(mode);
        let hooks_path = self.local_tinyfish_plugin_hooks_path(mode);
        let prompt_path = self.local_tinyfish_prompt_path(mode, tool_shell);
        Self::write_if_changed(&manifest_path, plugin_manifest_json)?;
        Self::write_if_changed(&hooks_path, plugin_hooks_json)?;
        Self::write_if_changed(&prompt_path, prompt_text)?;
        Ok((plugin_root, prompt_path))
    }

    fn sync_local_tinyfish_artifacts(&self, profiles: &[Profile]) -> Result<()> {
        let tool_shell = native_tinyfish_tool_shell();
        let mut desired_prompts = std::collections::HashSet::new();
        let mut desired_plugins = std::collections::HashSet::new();

        for profile in profiles {
            if profile.kind != ProfileKind::Lightweight {
                continue;
            }
            let Some(env) = profile.env.as_ref() else {
                self.remove_local_tinyfish_artifacts(&profile.id)?;
                continue;
            };
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                tool_shell,
            )?;
            match (
                artifacts.tinyfish_plugin_manifest_json.as_deref(),
                artifacts.tinyfish_plugin_hooks_json.as_deref(),
                artifacts.tinyfish_prompt_text.as_deref(),
            ) {
                (Some(plugin_manifest_json), Some(plugin_hooks_json), Some(prompt_text)) => {
                    self.upsert_local_tinyfish_artifacts(
                        artifacts.tinyfish_mode,
                        tool_shell,
                        plugin_manifest_json,
                        plugin_hooks_json,
                        prompt_text,
                    )?;
                    desired_prompts.insert(
                        Self::tinyfish_prompt_file_name(artifacts.tinyfish_mode, tool_shell)
                            .expect("prompt file name should exist for TinyFish modes"),
                    );
                    desired_plugins.insert(
                        Self::tinyfish_plugin_dir_name(artifacts.tinyfish_mode)
                            .expect("plugin dir name should exist for TinyFish modes"),
                    );
                }
                _ => self.remove_local_tinyfish_artifacts(&profile.id)?,
            }
        }

        let prompts_dir = self.generated_prompts_dir();
        if let Ok(entries) = fs::read_dir(&prompts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_prompt_name(&file_name)
                    && !desired_prompts.contains(&file_name)
                {
                    let _ = fs::remove_file(path);
                }
            }
        }

        let plugins_dir = self.generated_plugins_dir();
        if let Ok(entries) = fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                else {
                    continue;
                };
                if Self::is_managed_generated_plugin_dir_name(&file_name)
                    && !desired_plugins.contains(&file_name)
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn cmd_bin_dir() -> Result<PathBuf> {
        let home = Self::home_dir()?;
        Ok(Self::cmd_bin_dir_for_home(&home))
    }

    #[cfg(target_os = "windows")]
    fn cmd_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".local").join("bin")
    }

    fn build_sh_settings_env_prefix(
        env: &LightweightEnv,
        token: Option<&str>,
        url: Option<&str>,
    ) -> String {
        // The tail added by `build_sh_settings_tail` closes the root settings object.
        let prefix = build_lightweight_settings_env_prefix(env, token, url);
        format!("'{}'", Self::escape_sh_value(&prefix))
    }

    fn build_sh_settings_tail(mode: TinyfishMode, tool_shell: TinyfishToolShell) -> String {
        if let Some(allowlist) = tinyfish_permissions_allowlist(mode, tool_shell) {
            let permissions_json = serde_json::json!({
                "allow": allowlist,
            })
            .to_string();
            return format!(
                "'{}'",
                Self::escape_sh_value(&format!(",\"permissions\":{permissions_json}}}"))
            );
        }
        "'}'".to_string()
    }

    /// Generate the content of a self-contained `.cmd` file for a profile.
    fn generate_cmd_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile.launch_args.as_ref().is_some_and(|a| !a.is_empty());

        let mut lines: Vec<String> = Vec::new();
        lines.push("@echo off".into());
        lines.push("setlocal EnableExtensions DisableDelayedExpansion".into());
        lines.push(CMD_MARKER.into());
        lines.push(format!(":: Profile: {} ({})", profile.name, kind_label));

        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("set \"CLAUDE_CONFIG_DIR={}\"", dir.display()));
        }

        let cmd_tool_shell = TinyfishToolShell::PowerShell;
        let mut cmd_runtime = None;

        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                cmd_tool_shell,
            )?;
            cmd_runtime = Some(artifacts);
        }

        let tf_mode = cmd_runtime
            .as_ref()
            .map(|artifacts| artifacts.tinyfish_mode)
            .unwrap_or(TinyfishMode::None);
        if tf_mode != TinyfishMode::None {
            lines.push("set \"_TF=\"".into());
            lines.push("where tinyfish >nul 2>&1 && set \"_TF=1\"".into());
            lines.push(
                "set \"_TF_PLUGIN_DIR=".to_string()
                    + &Self::home_relative_tinyfish_plugin_root(tf_mode, RemoteOs::Windows)
                    + "\"",
            );
            lines.push(format!(
                "set \"_TF_PROMPT_FILE={}\"",
                Self::home_relative_tinyfish_prompt_path(tf_mode, RemoteOs::Windows)
            ));
        }
        if has_launch {
            let args_str = profile.launch_args.as_ref().unwrap().join(" ");
            lines.push(format!("set \"_LAUNCH_ARGS={args_str}\""));
        }

        lines.push("set \"_E=1\"".into());
        lines.push("set \"_R=\"".into());
        lines.push(":loop".into());
        lines.push("if \"%~1\"==\"\" goto build_settings".into());
        lines.push("if /i \"%~1\"==\"--no-extras\" (".into());
        lines.push("    set \"_E=\"".into());
        lines.push("    shift".into());
        lines.push("    goto loop".into());
        lines.push(")".into());
        lines.push("set \"_R=%_R% %1\"".into());
        lines.push("shift".into());
        lines.push("goto loop".into());
        lines.push(":build_settings".into());

        let mcp_servers = self.profile_mcp_servers(profile)?;
        let mcp_plugin_enabled = !mcp_servers.is_empty();
        if mcp_plugin_enabled {
            lines.push(
                "set \"_MCP_PLUGIN_DIR=".to_string()
                    + &Self::home_relative_profile_mcp_plugin_root(profile, RemoteOs::Windows)
                    + "\"",
            );
        }

        if let Some(runtime) = cmd_runtime.as_ref() {
            assign_cmd_json_var(&mut lines, "_SETTINGS", &runtime.base_settings_json);
            if tf_mode != TinyfishMode::None {
                let tinyfish_settings_json = runtime
                    .tinyfish_settings_json
                    .as_deref()
                    .context("TinyFish settings missing for non-native mode")?;
                assign_cmd_json_var(&mut lines, "_TF_SETTINGS", tinyfish_settings_json);
                if has_launch {
                    lines.push("if defined _TF if defined _E goto launch_with_hooks_extras".into());
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    lines.push("if defined _E goto launch_with_extras".into());
                    lines.push("goto launch_plain".into());
                } else {
                    lines.push("if defined _TF goto launch_with_hooks_plain".into());
                    lines.push("goto launch_plain".into());
                }
            } else if has_launch {
                lines.push("if defined _E goto launch_with_extras".into());
                lines.push("goto launch_plain".into());
            } else {
                lines.push("goto launch_plain".into());
            }
        } else if has_launch {
            lines.push("if defined _E goto launch_with_extras".into());
            lines.push("goto launch_plain".into());
        } else {
            lines.push("goto launch_plain".into());
        }

        let settings_prefix = if cmd_runtime.is_some() {
            "claude --settings \"%_SETTINGS%\""
        } else {
            "claude"
        };
        let mcp_plugin_part = if mcp_plugin_enabled {
            " --plugin-dir \"%_MCP_PLUGIN_DIR%\""
        } else {
            ""
        };

        if has_launch {
            if tf_mode != TinyfishMode::None {
                lines.push(":launch_with_hooks_extras".into());
                lines.push(format!("claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\"{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"));
                lines.push("exit /b %errorlevel%".into());
            }
            lines.push(":launch_with_extras".into());
            lines.push(format!(
                "{settings_prefix}{mcp_plugin_part} %_LAUNCH_ARGS% %_R%"
            ));
            lines.push("exit /b %errorlevel%".into());
        }

        if tf_mode != TinyfishMode::None {
            lines.push(":launch_with_hooks_plain".into());
            lines.push(format!("claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\"{mcp_plugin_part} %_R%"));
            lines.push("exit /b %errorlevel%".into());
        }

        lines.push(":launch_plain".into());
        lines.push(format!("{settings_prefix}{mcp_plugin_part} %_R%"));
        lines.push("exit /b %errorlevel%".into());

        Ok(lines.join("\r\n") + "\r\n")
    }

    /// Synchronize self-contained `.cmd` aliases into `~/.local/bin`.
    /// Creates new files, updates existing ones, and removes stale files
    /// whose profiles no longer exist.
    #[cfg(target_os = "windows")]
    pub fn sync_cmd_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        self.sync_local_tinyfish_artifacts(&profiles)?;
        self.sync_local_mcp_artifacts(&profiles)?;
        let bin_dir = Self::cmd_bin_dir()?;
        fs::create_dir_all(&bin_dir)?;

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut report = Vec::new();

        for p in &profiles {
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            let cmd_name = format!("claude-{}.cmd", alias_name);
            let cmd_path = bin_dir.join(&cmd_name);
            let content = self.generate_cmd_content(p)?;
            let needs_write = match fs::read_to_string(&cmd_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                fs::write(&cmd_path, &content)?;
                report.push(format!("  + {}", cmd_path.display()));
            } else {
                report.push(format!("  = {}", cmd_path.display()));
            }
            written.insert(cmd_name.to_lowercase());
        }

        // Remove stale cmd files (have marker but no matching profile)
        if let Ok(entries) = fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "cmd") {
                    let fname = path.file_name().unwrap().to_string_lossy().to_lowercase();
                    if !written.contains(&fname) {
                        // Check for marker before removing
                        if let Ok(content) = fs::read_to_string(&path)
                            && content.contains(CMD_MARKER)
                        {
                            let _ = fs::remove_file(&path);
                            report.push(format!("  - {} (stale)", path.display()));
                        }
                    }
                }
            }
        }

        let bin_str = bin_dir.display().to_string();
        Ok(format!(
            "# CMD aliases synced to {} ({} profiles)\n{}",
            bin_str,
            profiles.len(),
            report.join("\n")
        ))
    }

    // ── Shell-script aliases (Linux, ~/.varusers/bin) ─────────────────────

    #[cfg(not(target_os = "windows"))]
    fn sh_bin_dir() -> Result<PathBuf> {
        let home = Self::home_dir()?;
        Ok(Self::sh_bin_dir_for_home(&home))
    }

    #[cfg(not(target_os = "windows"))]
    fn sh_bin_dir_for_home(home: &Path) -> PathBuf {
        home.join(".varusers").join("bin")
    }

    fn run_local_command(program: &str, args: &[&str]) -> Result<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("Failed to run {}", program))?;
        if !output.status.success() {
            bail!(
                "{} failed: {}",
                program,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn run_remote_sftp_commands(host: &str, stdin: &str) -> Result<String> {
        let mut child = Command::new("sftp")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ForwardX11=no",
                "-b",
                "-",
                host,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn sftp")?;

        use std::io::Write;
        {
            let mut stdin_handle = child.stdin.take().unwrap();
            let _ = stdin_handle.write_all(stdin.as_bytes());
            // stdin_handle is dropped here, closing the pipe so sftp sees EOF.
        }

        let output = child.wait_with_output().context("Failed to wait on sftp")?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            // sftp exits non-zero on some benign glitches (e.g. -mkdir on
            // an existing dir).  Only bail when there is actual error output.
            if !stderr.is_empty() {
                bail!("sftp error: {}", stderr);
            }
            if stdout.is_empty() {
                bail!("sftp failed silently");
            }
            // Some sftp implementations print errors to stdout with empty stderr.
            // Bail if stdout contains known error patterns.
            let lower = stdout.to_lowercase();
            if lower.contains("no such file")
                || lower.contains("not found")
                || lower.contains("permission denied")
                || lower.contains("failure")
                || lower.contains("couldn't")
                || lower.contains("cannot")
            {
                bail!("sftp error (stdout): {}", stdout);
            }
        }
        Ok(stdout)
    }

    fn run_remote_sftp_batch(host: &str, batch_path: &str) -> Result<String> {
        Self::run_local_command(
            "sftp",
            &[
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ForwardX11=no",
                "-b",
                batch_path,
                host,
            ],
        )
    }

    fn probe_remote_os_and_home(host: &str) -> Result<(RemoteOs, String)> {
        let output = Self::run_remote_sftp_commands(host, "pwd\n")?;
        // sftp pwd prints: "Remote working directory: /home/alice" (Unix)
        // or               "Remote working directory: /C:/Users/alice" (Windows)
        let home = output
            .lines()
            .find(|line| line.contains("Remote working directory:"))
            .and_then(|line| line.split(": ").nth(1))
            .map(str::trim)
            .map(str::to_string)
            .context("sftp pwd did not produce a usable directory")?;

        // Windows sftp pwd looks like /C:/Users/alice — detect any drive letter
        let bytes = home.as_bytes();
        let is_windows = bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        let remote_os = if is_windows {
            RemoteOs::Windows
        } else if home.starts_with('/') {
            RemoteOs::Unix
        } else {
            bail!(
                "Could not determine remote OS for '{}' from sftp pwd output: {}",
                host,
                home
            );
        };
        Ok((remote_os, home))
    }

    /// Wrap a path in double quotes for sftp batch mode (handles spaces).
    fn sftp_quote(path: &str) -> String {
        format!("\"{}\"", path.replace('"', "\\\""))
    }

    fn ensure_remote_dir(host: &str, remote_bin_dir: &str) -> Result<()> {
        // Normalize backslashes to forward slashes for sftp
        let dir = remote_bin_dir.replace('\\', "/");
        // Build -mkdir commands for every prefix so parents are created
        let mut cmds = String::new();
        let mut accumulated = String::new();
        for component in dir.split('/') {
            if component.is_empty() {
                if dir.starts_with('/') {
                    accumulated.push('/');
                }
                continue;
            }
            if accumulated == "/" {
                accumulated.push_str(component);
            } else if accumulated.is_empty() {
                accumulated = component.to_string();
            } else {
                accumulated.push('/');
                accumulated.push_str(component);
            }
            cmds.push_str(&format!("-mkdir {}\n", Self::sftp_quote(&accumulated)));
        }
        if !cmds.is_empty() {
            // Errors are benign (dirs already exist); stderr will be empty thanks to -prefix
            let _ = Self::run_remote_sftp_commands(host, &cmds);
        }
        Ok(())
    }

    fn list_remote_files(
        host: &str,
        remote_bin_dir: &str,
        remote_os: RemoteOs,
    ) -> Result<Vec<String>> {
        let sftp_dir = if matches!(remote_os, RemoteOs::Windows) {
            remote_bin_dir.replace('\\', "/")
        } else {
            remote_bin_dir.to_string()
        };
        let output = Self::run_remote_sftp_commands(
            host,
            &format!("ls -1 {}\n", Self::sftp_quote(&format!("{}/", sftp_dir))),
        )?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let name = line.trim();
                if name.is_empty() || name.starts_with("sftp>") {
                    None
                } else {
                    // Strip leading path prefix to get just the filename
                    name.rsplit('/').next().map(str::to_string)
                }
            })
            .collect())
    }

    fn list_remote_files_if_present(
        host: &str,
        remote_dir: &str,
        remote_os: RemoteOs,
    ) -> Result<Vec<String>> {
        match Self::list_remote_files(host, remote_dir, remote_os) {
            Ok(files) => Ok(files),
            Err(err) => {
                let msg = err.to_string().to_lowercase();
                if msg.contains("no such file") || msg.contains("not found") {
                    Ok(Vec::new())
                } else {
                    Err(err)
                }
            }
        }
    }

    fn remote_file_has_marker(host: &str, remote_path: &str, remote_os: RemoteOs) -> Result<bool> {
        let sftp_path = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let local_tmp = std::env::temp_dir().join(format!("cswitch-marker-{}", Uuid::new_v4()));
        let sftp_cmd = format!(
            "get {} {}\n",
            Self::sftp_quote(&sftp_path),
            Self::sftp_quote(&local_tmp.display().to_string()),
        );
        let get_result = Self::run_remote_sftp_commands(host, &sftp_cmd);
        // Clean up temp file regardless of success/failure
        let content = match &get_result {
            Ok(_) => fs::read_to_string(&local_tmp).context("Failed to read temp marker file")?,
            Err(_) => String::new(),
        };
        let _ = fs::remove_file(&local_tmp);
        // If the remote file doesn't exist, sftp get fails — treat as "no marker"
        if get_result.is_err() {
            return Ok(false);
        }
        get_result?;
        Ok(content.contains(CMD_MARKER) || content.contains(SH_MARKER))
    }

    fn remove_remote_file(host: &str, remote_path: &str, remote_os: RemoteOs) -> Result<()> {
        let sftp_path = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        Self::run_remote_sftp_commands(host, &format!("rm {}\n", Self::sftp_quote(&sftp_path)))?;
        Ok(())
    }

    fn is_benign_sftp_missing_error(error: &anyhow::Error) -> bool {
        let message = error.to_string().to_ascii_lowercase();
        message.contains("no such file")
            || message.contains("not found")
            || message.contains("couldn't stat remote file")
    }

    fn remove_remote_plugin_dir(host: &str, remote_path: &str, remote_os: RemoteOs) -> Result<()> {
        Self::remove_remote_plugin_dir_with_runner(host, remote_path, remote_os, |stdin| {
            Self::run_remote_sftp_commands(host, stdin)
        })
    }

    fn remove_remote_mcp_plugin_dir(
        host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
    ) -> Result<()> {
        Self::remove_remote_mcp_plugin_dir_with_runner(host, remote_path, remote_os, |stdin| {
            Self::run_remote_sftp_commands(host, stdin)
        })
    }

    fn remove_remote_mcp_plugin_dir_with_runner<F>(
        _host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
        mut run_sftp: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let manifest_dir = Self::join_remote_path(remote_path, remote_os, ".claude-plugin");
        let manifest_json = Self::join_remote_path(&manifest_dir, remote_os, "plugin.json");
        let dot_mcp_json = Self::join_remote_path(remote_path, remote_os, ".mcp.json");
        let mcp_json = Self::join_remote_path(remote_path, remote_os, "mcp.json");
        let manifest_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_json.replace('\\', "/")
        } else {
            manifest_json
        };
        let manifest_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_dir.replace('\\', "/")
        } else {
            manifest_dir
        };
        let dot_mcp_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            dot_mcp_json.replace('\\', "/")
        } else {
            dot_mcp_json
        };
        let mcp_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            mcp_json.replace('\\', "/")
        } else {
            mcp_json
        };
        let plugin_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let cmds = format!(
            "rm {}\nrm {}\nrm {}\nrmdir {}\nrmdir {}\n",
            Self::sftp_quote(&manifest_json_sftp),
            Self::sftp_quote(&dot_mcp_json_sftp),
            Self::sftp_quote(&mcp_json_sftp),
            Self::sftp_quote(&manifest_dir_sftp),
            Self::sftp_quote(&plugin_dir_sftp),
        );
        match run_sftp(&cmds) {
            Ok(_) => Ok(()),
            Err(error) if Self::is_benign_sftp_missing_error(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn remove_remote_plugin_dir_with_runner<F>(
        _host: &str,
        remote_path: &str,
        remote_os: RemoteOs,
        mut run_sftp: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> Result<String>,
    {
        let manifest_dir = Self::join_remote_path(remote_path, remote_os, ".claude-plugin");
        let manifest_json = Self::join_remote_path(&manifest_dir, remote_os, "plugin.json");
        let hooks_dir = Self::join_remote_path(remote_path, remote_os, "hooks");
        let hooks_json = Self::join_remote_path(&hooks_dir, remote_os, "hooks.json");
        let manifest_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_json.replace('\\', "/")
        } else {
            manifest_json
        };
        let manifest_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            manifest_dir.replace('\\', "/")
        } else {
            manifest_dir
        };
        let hooks_json_sftp = if matches!(remote_os, RemoteOs::Windows) {
            hooks_json.replace('\\', "/")
        } else {
            hooks_json
        };
        let hooks_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            hooks_dir.replace('\\', "/")
        } else {
            hooks_dir
        };
        let plugin_dir_sftp = if matches!(remote_os, RemoteOs::Windows) {
            remote_path.replace('\\', "/")
        } else {
            remote_path.to_string()
        };
        let cmds = format!(
            "rm {}\nrm {}\nrmdir {}\nrmdir {}\nrmdir {}\n",
            Self::sftp_quote(&manifest_json_sftp),
            Self::sftp_quote(&hooks_json_sftp),
            Self::sftp_quote(&manifest_dir_sftp),
            Self::sftp_quote(&hooks_dir_sftp),
            Self::sftp_quote(&plugin_dir_sftp),
        );
        match run_sftp(&cmds) {
            Ok(_) => Ok(()),
            Err(error) if Self::is_benign_sftp_missing_error(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn join_remote_path(remote_dir: &str, remote_os: RemoteOs, file_name: &str) -> String {
        match remote_os {
            RemoteOs::Unix => format!("{}/{}", remote_dir.trim_end_matches('/'), file_name),
            RemoteOs::Windows => format!(
                "{}\\{}",
                remote_dir.trim_end_matches(['\\', '/']),
                file_name
            ),
        }
    }

    fn remote_shim_file_name(profile: &Profile, remote_os: RemoteOs) -> Option<String> {
        if profile.kind == ProfileKind::Full {
            return None;
        }
        let alias_name = profile.alias.as_deref().unwrap_or(&profile.name);
        Some(match remote_os {
            RemoteOs::Unix => format!("claude-{}", alias_name),
            RemoteOs::Windows => format!("claude-{}.cmd", alias_name),
        })
    }

    fn is_managed_remote_name(remote_os: RemoteOs, file_name: &str) -> bool {
        match remote_os {
            RemoteOs::Unix => file_name.starts_with("claude-"),
            RemoteOs::Windows => file_name.starts_with("claude-") && file_name.ends_with(".cmd"),
        }
    }

    fn build_remote_upload_batch(
        temp_root: &Path,
        remote_dir: &str,
        remote_os: RemoteOs,
        desired: &[(String, String)],
        chmod_unix: bool,
    ) -> String {
        let mut batch = String::new();
        for (file_name, _) in desired {
            let local_path = temp_root.join(file_name);
            let remote_path = Self::join_remote_path(remote_dir, remote_os, file_name);
            batch.push_str(&format!(
                "put {} {}\n",
                Self::sftp_quote(&local_path.display().to_string()),
                Self::sftp_quote(&remote_path),
            ));
            if matches!(remote_os, RemoteOs::Unix) && chmod_unix {
                batch.push_str(&format!("chmod 755 {}\n", Self::sftp_quote(&remote_path),));
            }
        }
        batch
    }

    fn remote_parent_dirs(
        remote_dir: &str,
        remote_os: RemoteOs,
        relative_path: &str,
    ) -> Vec<String> {
        let normalized = relative_path.replace('\\', "/");
        let mut components: Vec<&str> = normalized.split('/').collect();
        if components.len() <= 1 {
            return Vec::new();
        }
        components.pop();
        let mut dirs = Vec::new();
        let mut current = remote_dir.trim_end_matches(['\\', '/']).to_string();
        for component in components {
            current = match remote_os {
                RemoteOs::Unix => format!("{}/{}", current.trim_end_matches('/'), component),
                RemoteOs::Windows => {
                    format!("{}\\{}", current.trim_end_matches(['\\', '/']), component)
                }
            };
            dirs.push(current.clone());
        }
        dirs
    }

    fn upload_remote_files(
        host: &str,
        remote_dir: &str,
        remote_os: RemoteOs,
        desired: &[(String, String)],
        chmod_unix: bool,
    ) -> Result<()> {
        let temp_root = std::env::temp_dir().join(format!("cswitch-remote-{}", Uuid::new_v4()));
        let batch_path =
            std::env::temp_dir().join(format!("cswitch-remote-{}.sftp", Uuid::new_v4()));
        fs::create_dir_all(&temp_root).context("Failed to create temp shim directory")?;

        let mut remote_parent_dirs = std::collections::BTreeSet::new();

        for (file_name, content) in desired {
            let local_path = temp_root.join(file_name);
            if let Some(parent) = local_path.parent() {
                fs::create_dir_all(parent).context("Failed to create temp shim subdirectory")?;
            }
            fs::write(&local_path, content).context("Failed to write temp shim file")?;
            for parent_dir in Self::remote_parent_dirs(remote_dir, remote_os, file_name) {
                remote_parent_dirs.insert(parent_dir);
            }
        }
        for parent_dir in remote_parent_dirs {
            Self::ensure_remote_dir(host, &parent_dir)?;
        }
        let batch =
            Self::build_remote_upload_batch(&temp_root, remote_dir, remote_os, desired, chmod_unix);

        fs::write(&batch_path, batch).context("Failed to write temp sftp batch")?;
        let batch_path_str = batch_path.to_string_lossy().to_string();
        let result = Self::run_remote_sftp_batch(host, &batch_path_str);
        let _ = fs::remove_dir_all(&temp_root);
        let _ = fs::remove_file(&batch_path);
        result?;
        Ok(())
    }

    /// Escape a string value for embedding inside a bash single-quoted string.
    /// Replaces `'` with `'\''` (end quote, escaped literal quote, resume quote).
    fn escape_sh_value(s: &str) -> String {
        s.replace('\'', "'\\''")
    }

    /// Generate a self-contained bash script for a profile.
    fn generate_sh_content(&self, profile: &Profile) -> Result<String> {
        let kind_label = if profile.kind == ProfileKind::Full {
            "full"
        } else {
            "lightweight"
        };
        let has_launch = profile.launch_args.as_ref().is_some_and(|a| !a.is_empty());
        let launch_str = profile
            .launch_args
            .as_ref()
            .map(|a| a.join(" "))
            .unwrap_or_default();

        let mut lines: Vec<String> = Vec::new();

        lines.push("#!/usr/bin/env bash".into());
        lines.push(SH_MARKER.into());
        lines.push(format!("# Profile: {} ({})", profile.name, kind_label));
        lines.push("set -euo pipefail".into());
        lines.push(String::new());

        // Full profile: export CLAUDE_CONFIG_DIR
        if profile.kind == ProfileKind::Full {
            let dir = self.profile_dir(profile);
            lines.push(format!("export CLAUDE_CONFIG_DIR=\"{}\"", dir.display()));
        }

        let mut settings_enabled = false;
        let mut tinyfish_mode_for_profile = TinyfishMode::None;
        let sh_tool_shell = TinyfishToolShell::Bash;

        if profile.kind == ProfileKind::Lightweight
            && let Some(ref env) = profile.env
        {
            let (token, url) = self.resolve_credentials(profile)?;
            let artifacts = build_lightweight_runtime_artifacts(
                env,
                token.as_deref(),
                url.as_deref(),
                sh_tool_shell,
            )?;
            tinyfish_mode_for_profile = artifacts.tinyfish_mode;
            lines.push(format!(
                "SETTINGS_ENV={}",
                Self::build_sh_settings_env_prefix(env, token.as_deref(), url.as_deref())
            ));
            lines.push(format!(
                "BASE_SETTINGS=\"${{SETTINGS_ENV}}\"{}",
                Self::build_sh_settings_tail(TinyfishMode::None, sh_tool_shell)
            ));
            settings_enabled = true;
            if tinyfish_mode_for_profile != TinyfishMode::None {
                lines.push(format!(
                    "TF_SETTINGS=\"${{SETTINGS_ENV}}\"{}",
                    Self::build_sh_settings_tail(tinyfish_mode_for_profile, sh_tool_shell)
                ));
                lines.push(format!(
                    "TF_PROMPT_FILE=\"{}\"",
                    Self::home_relative_tinyfish_prompt_path(
                        tinyfish_mode_for_profile,
                        RemoteOs::Unix
                    )
                ));
                lines.push(format!(
                    "TF_PLUGIN_DIR=\"{}\"",
                    Self::home_relative_tinyfish_plugin_root(
                        tinyfish_mode_for_profile,
                        RemoteOs::Unix
                    )
                ));
            }
        }

        if settings_enabled {
            lines.push("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")".into());
        }

        // TinyFish runtime detection + conditional file-based prompt/settings
        if tinyfish_mode_for_profile != TinyfishMode::None {
            lines.push(String::new());
            lines.push("# Check if tinyfish is available for web search/fetch".into());
            lines.push("if command -v tinyfish >/dev/null 2>&1; then".into());
            lines.push("    TF_PLUGIN_ARGS=(--plugin-dir \"$TF_PLUGIN_DIR\")".into());
            lines.push("    TF_SP_ARGS=(--append-system-prompt-file \"$TF_PROMPT_FILE\")".into());
            lines.push("    SETTINGS_ARG=(--settings \"$TF_SETTINGS\")".into());
            lines.push("else".into());
            lines.push("    TF_PLUGIN_ARGS=()".into());
            lines.push("    TF_SP_ARGS=()".into());
            lines.push("fi".into());
        } else {
            lines.push("TF_PLUGIN_ARGS=()".into());
            lines.push("TF_SP_ARGS=()".into());
        }

        let mcp_servers = self.profile_mcp_servers(profile)?;
        if !mcp_servers.is_empty() {
            lines.push(format!(
                "MCP_PLUGIN_ARGS=(--plugin-dir \"{}\")",
                Self::home_relative_profile_mcp_plugin_root(profile, RemoteOs::Unix)
            ));
        } else {
            lines.push("MCP_PLUGIN_ARGS=()".into());
        }

        lines.push(String::new());
        lines.push("EXTRA=true".into());
        lines.push("ARGS=()".into());
        lines.push("while [[ $# -gt 0 ]]; do".into());
        lines.push("    case \"$1\" in".into());
        lines.push("        --no-extras) EXTRA=false; shift ;;".into());
        lines.push("        *) ARGS+=(\"$1\"); shift ;;".into());
        lines.push("    esac".into());
        lines.push("done".into());

        // Build the claude invocation
        let settings_part = if settings_enabled {
            " \"${SETTINGS_ARG[@]}\""
        } else {
            ""
        };
        let launch_part = if has_launch {
            &format!(" {}", launch_str)
        } else {
            ""
        };

        if has_launch {
            lines.push(format!(
                "if $EXTRA; then exec claude{0}{1} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; else exec claude{0} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"; fi",
                settings_part, launch_part
            ));
        } else {
            lines.push(format!(
                "exec claude{0} \"${{TF_PLUGIN_ARGS[@]}}\" \"${{TF_SP_ARGS[@]}}\" \"${{MCP_PLUGIN_ARGS[@]}}\" \"${{ARGS[@]}}\"",
                settings_part
            ));
        }

        Ok(lines.join("\n") + "\n")
    }

    /// Synchronize self-contained shell scripts into `~/.varusers/bin`.
    #[cfg(not(target_os = "windows"))]
    pub fn sync_sh_scripts(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        self.sync_local_tinyfish_artifacts(&profiles)?;
        self.sync_local_mcp_artifacts(&profiles)?;
        let bin_dir = Self::sh_bin_dir()?;
        // Only operate if the directory already exists (opt-in)
        if !bin_dir.exists() || !bin_dir.is_dir() {
            return Ok(String::new());
        }

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut report = Vec::new();

        for p in &profiles {
            let alias_name = p.alias.as_deref().unwrap_or(&p.name);
            let sh_name = format!("claude-{}", alias_name);
            let sh_path = bin_dir.join(&sh_name);
            let content = self.generate_sh_content(p)?;
            let needs_write = match fs::read_to_string(&sh_path) {
                Ok(existing) => existing != content,
                Err(_) => true,
            };
            if needs_write {
                fs::write(&sh_path, &content)?;
                // Make executable
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&sh_path)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&sh_path, perms)?;
                }
                report.push(format!("  + {}", sh_path.display()));
            } else {
                report.push(format!("  = {}", sh_path.display()));
            }
            written.insert(sh_name);
        }

        // Remove stale scripts (have marker but no matching profile)
        if let Ok(entries) = fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let fname = path.file_name().unwrap().to_string_lossy();
                if !written.contains(fname.as_ref()) {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if content.contains(SH_MARKER) {
                            let _ = fs::remove_file(&path);
                            report.push(format!("  - {} (stale)", path.display()));
                        }
                    }
                }
            }
        }

        Ok(format!(
            "{} profiles\n{}",
            profiles.len(),
            report.join("\n")
        ))
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

// ── Free helpers ──────────────────────────────────────────────────────────────

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    let dir = match fs::read_dir(src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Warning: cannot read '{}': {}", src.display(), e);
            return Ok(());
        }
    };
    for entry in dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: cannot read entry in '{}': {}", src.display(), e);
                continue;
            }
        };
        let dest_path = dst.join(entry.file_name());

        let is_symlink = entry
            .path()
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        if is_symlink {
            if let Ok(target) = fs::read_link(entry.path())
                && !copy_symlink(&target, &dest_path)
            {
                if target.is_dir() {
                    copy_dir_all(&target, &dest_path)?;
                } else if let Err(e) = fs::copy(&target, &dest_path) {
                    eprintln!("Warning: cannot copy '{}': {}", entry.path().display(), e);
                }
            }
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                eprintln!("Warning: cannot stat '{}': {}", entry.path().display(), e);
                continue;
            }
        };

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else if let Err(e) = fs::copy(entry.path(), &dest_path) {
            eprintln!("Warning: cannot copy '{}': {}", entry.path().display(), e);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn copy_symlink(target: &Path, dest: &Path) -> bool {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let result = if target.is_dir() {
        symlink_dir(target, dest)
    } else {
        symlink_file(target, dest)
    };
    result.is_ok()
}

#[cfg(not(target_os = "windows"))]
fn copy_symlink(target: &Path, dest: &Path) -> bool {
    std::os::unix::fs::symlink(target, dest).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelDiscoveryFailureKind {
    Auth,
    EndpointNotFound,
    Timeout,
    Transport,
    Parse,
    Unsupported,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDiscoverySuccess {
    pub models: Vec<String>,
    pub endpoint_used: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDiscoveryFailure {
    pub kind: ModelDiscoveryFailureKind,
    pub message: String,
    pub last_endpoint: Option<String>,
    pub tried_endpoints: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOs {
    Unix,
    Windows,
}

pub fn build_model_discovery_candidates(base_url: &str) -> Result<Vec<String>> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("Base URL is empty");
    }

    let mut candidates = Vec::new();
    if trimmed.ends_with("/v1") {
        candidates.push(format!("{trimmed}/models"));
    } else {
        candidates.push(format!("{trimmed}/v1/models"));
    }

    if let Some(stripped) = strip_compat_suffix(trimmed) {
        let root = stripped.trim_end_matches('/');
        if !root.is_empty() && root.contains("://") {
            candidates.push(format!("{root}/v1/models"));
            candidates.push(format!("{root}/models"));
        }
    }

    let mut unique = Vec::with_capacity(candidates.len());
    for url in candidates {
        if !unique.iter().any(|existing| existing == &url) {
            unique.push(url);
        }
    }
    Ok(unique)
}

pub fn discover_models_with_timeout(
    base_url: &str,
    auth_token: &str,
    timeout: Duration,
) -> std::result::Result<ModelDiscoverySuccess, ModelDiscoveryFailure> {
    let candidates = match build_model_discovery_candidates(base_url) {
        Ok(candidates) => candidates,
        Err(e) => {
            return Err(ModelDiscoveryFailure {
                kind: ModelDiscoveryFailureKind::Other,
                message: e.to_string(),
                last_endpoint: None,
                tried_endpoints: vec![],
            });
        }
    };
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();
    let mut last_not_found_endpoint = None;
    let mut last_not_found_message = None;
    let mut tried_endpoints: Vec<String> = Vec::new();

    for url in candidates {
        tried_endpoints.push(url.clone());
        let request = agent
            .get(&url)
            .header("x-api-key", auth_token)
            .header("Authorization", &format!("Bearer {}", auth_token));
        match request.call() {
            Ok(resp) => {
                let json: serde_json::Value = match resp.into_body().read_json() {
                    Ok(json) => json,
                    Err(e) => {
                        return Err(ModelDiscoveryFailure {
                            kind: ModelDiscoveryFailureKind::Parse,
                            message: format!("Invalid JSON response from models endpoint: {e}"),
                            last_endpoint: Some(url),
                            tried_endpoints: tried_endpoints.clone(),
                        });
                    }
                };
                let data = match json.get("data").and_then(|d| d.as_array()) {
                    Some(data) => data,
                    None => {
                        match json.get("models").and_then(|m| m.as_array()) {
                            Some(models) => models,
                            None => {
                                return Err(ModelDiscoveryFailure {
                                kind: ModelDiscoveryFailureKind::Unsupported,
                                message: "Unexpected response format (missing 'data' or 'models' array)".to_string(),
                                last_endpoint: Some(url),
                                tried_endpoints: tried_endpoints.clone(),
                            });
                            }
                        }
                    }
                };
                let mut models: Vec<String> = data
                    .iter()
                    .filter_map(|entry| {
                        entry.get("id").and_then(|id| id.as_str()).map(String::from)
                    })
                    .collect();
                models.sort();
                models.dedup();
                return Ok(ModelDiscoverySuccess {
                    models,
                    endpoint_used: url,
                });
            }
            Err(ureq::Error::StatusCode(status)) => {
                if status == 401 || status == 403 {
                    return Err(ModelDiscoveryFailure {
                        kind: ModelDiscoveryFailureKind::Auth,
                        message: format!("HTTP {status}: API key is invalid or lacks permission"),
                        last_endpoint: Some(url),
                        tried_endpoints: tried_endpoints.clone(),
                    });
                }
                if status == 404 || status == 405 {
                    last_not_found_endpoint = Some(url.clone());
                    last_not_found_message =
                        Some(format!("HTTP {status}: models endpoint unavailable"));
                    continue;
                }
                return Err(ModelDiscoveryFailure {
                    kind: ModelDiscoveryFailureKind::Other,
                    message: format!("HTTP {status}: model discovery failed"),
                    last_endpoint: Some(url),
                    tried_endpoints: tried_endpoints.clone(),
                });
            }
            Err(ureq::Error::Io(e)) => {
                let kind = if e.kind() == std::io::ErrorKind::TimedOut {
                    ModelDiscoveryFailureKind::Timeout
                } else {
                    ModelDiscoveryFailureKind::Transport
                };
                return Err(ModelDiscoveryFailure {
                    kind,
                    message: format!("Failed to reach models endpoint: {e}"),
                    last_endpoint: Some(url),
                    tried_endpoints: tried_endpoints.clone(),
                });
            }
            Err(e) => {
                return Err(ModelDiscoveryFailure {
                    kind: ModelDiscoveryFailureKind::Other,
                    message: format!("Failed to fetch models: {e}"),
                    last_endpoint: Some(url),
                    tried_endpoints: tried_endpoints.clone(),
                });
            }
        }
    }

    Err(ModelDiscoveryFailure {
        kind: ModelDiscoveryFailureKind::EndpointNotFound,
        message: last_not_found_message
            .unwrap_or_else(|| "No reachable models endpoint found".to_string()),
        last_endpoint: last_not_found_endpoint,
        tried_endpoints,
    })
}

pub fn discover_models(
    base_url: &str,
    auth_token: &str,
) -> std::result::Result<ModelDiscoverySuccess, ModelDiscoveryFailure> {
    discover_models_with_timeout(
        base_url,
        auth_token,
        Duration::from_secs(API_TEST_TIMEOUT_SECS),
    )
}

/// Fetch available model IDs from provider-aware model discovery.
pub fn fetch_models(base_url: &str, auth_token: &str) -> Result<Vec<String>> {
    discover_models(base_url, auth_token)
        .map(|result| result.models)
        .map_err(|failure| anyhow::anyhow!(failure.message))
}

fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    for suffix in KNOWN_COMPAT_SUFFIXES {
        if base_url.ends_with(*suffix) {
            return Some(&base_url[..base_url.len() - suffix.len()]);
        }
    }
    None
}

fn build_message_candidates(base_url: &str) -> Result<Vec<String>> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("Base URL is empty");
    }
    let mut candidates = Vec::new();
    if trimmed.ends_with("/v1") {
        candidates.push(format!("{trimmed}/messages"));
    } else {
        candidates.push(format!("{trimmed}/v1/messages"));
    }
    if let Some(stripped) = strip_compat_suffix(trimmed) {
        let root = stripped.trim_end_matches('/');
        if !root.is_empty() && root.contains("://") {
            candidates.push(format!("{root}/v1/messages"));
            candidates.push(format!("{root}/messages"));
        }
    }
    let mut unique = Vec::with_capacity(candidates.len());
    for url in candidates {
        if !unique.iter().any(|existing| existing == &url) {
            unique.push(url);
        }
    }
    Ok(unique)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicTestResult {
    pub text: String,
    pub endpoint_used: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Send one non-streaming Anthropic-compatible /v1/messages request,
/// trying multiple endpoint candidates derived from the base URL.
pub fn test_anthropic_message_with_timeout(
    base_url: &str,
    auth_token: &str,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<AnthropicTestResult> {
    let candidates = build_message_candidates(base_url)?;
    let body_json = serde_json::json!({
        "model": model,
        "max_tokens": 64,
        "messages": [
            {
                "role": "user",
                "content": prompt,
            }
        ],
    });
    let api_key_header = ureq::http::HeaderValue::from_bytes(auth_token.as_bytes())
        .context("Invalid API key for x-api-key header")?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();

    let mut last_err: Option<String> = None;
    for url in &candidates {
        let send_body =
            ureq::SendBody::from_json(&body_json).context("Failed to encode request JSON")?;
        let request = ureq::http::Request::builder()
            .method(ureq::http::Method::POST)
            .uri(url.as_str())
            .header("content-type", "application/json; charset=utf-8")
            .header("x-api-key", api_key_header.clone())
            .header("Authorization", &format!("Bearer {}", auth_token))
            .header("anthropic-version", "2023-06-01")
            .body(send_body)
            .context("Failed to build Anthropic test request")?;
        let resp = request
            .with_agent(&agent)
            .configure()
            .http_status_as_error(false)
            .run();
        let mut resp = match resp {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(format!("Failed to call {url}: {e:#}"));
                continue;
            }
        };
        let status = resp.status();
        let raw = resp
            .body_mut()
            .read_to_string()
            .context("Failed to read Anthropic /v1/messages response")?;
        if status == ureq::http::StatusCode::NOT_FOUND
            || status == ureq::http::StatusCode::METHOD_NOT_ALLOWED
        {
            continue;
        }
        if status == ureq::http::StatusCode::UNAUTHORIZED
            || status == ureq::http::StatusCode::FORBIDDEN
        {
            let body = raw.trim();
            let body = if body.is_empty() {
                "(empty body)"
            } else {
                body
            };
            bail!(
                "Anthropic test failed with HTTP {} at {url}: {}",
                status.as_u16(),
                body
            );
        }
        if !status.is_success() {
            let body = raw.trim();
            let body = if body.is_empty() {
                "(empty body)"
            } else {
                body
            };
            last_err = Some(format!("HTTP {} at {url}: {body}", status.as_u16()));
            continue;
        }
        return parse_anthropic_message_response(&raw, url);
    }
    match last_err {
        Some(err) => bail!("All message endpoint candidates failed. Last error: {err}"),
        None => bail!("No message endpoint candidates available for base URL: {base_url}"),
    }
}

pub fn test_anthropic_message(
    base_url: &str,
    auth_token: &str,
    model: &str,
    prompt: &str,
) -> Result<AnthropicTestResult> {
    test_anthropic_message_with_timeout(
        base_url,
        auth_token,
        model,
        prompt,
        Duration::from_secs(API_TEST_TIMEOUT_SECS),
    )
}

fn parse_anthropic_message_response(raw: &str, endpoint_used: &str) -> Result<AnthropicTestResult> {
    let json: serde_json::Value =
        serde_json::from_str(raw).context("Invalid JSON response from /v1/messages")?;
    let text = json
        .get("content")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("text"))
                .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty())
        .context("Unexpected response format from /v1/messages (missing text content)")?;
    let input_tokens = json.pointer("/usage/input_tokens").and_then(|v| v.as_u64());
    let output_tokens = json
        .pointer("/usage/output_tokens")
        .and_then(|v| v.as_u64());
    Ok(AnthropicTestResult {
        text,
        endpoint_used: endpoint_used.to_string(),
        input_tokens,
        output_tokens,
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    #[cfg(not(windows))]
    const TINYFISH_TIMEOUT_TEST_PROGRAM: &str = "sh";
    #[cfg(windows)]
    const TINYFISH_TIMEOUT_TEST_PROGRAM: &str = "cmd";
    use std::thread;
    use tempfile::TempDir;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn make_manager(tmp: &TempDir) -> ProfileManager {
        let base_dir = tmp.path().join(".claude-switch");
        let profiles_dir = base_dir.join("profiles");
        let registry_path = base_dir.join("registry.json");
        fs::create_dir_all(&profiles_dir).unwrap();
        ProfileManager {
            profiles_dir,
            registry_path,
        }
    }

    #[test]
    fn home_dir_layout_keeps_registry_and_generated_shims_under_same_root() {
        let tmp = TempDir::new().unwrap();
        let mgr = ProfileManager::new_in_home_dir(tmp.path()).unwrap();

        assert_eq!(mgr.base_dir(), tmp.path().join(".claude-switch"));
        assert_eq!(
            mgr.registry_path,
            tmp.path().join(".claude-switch").join("registry.json")
        );
        assert_eq!(
            mgr.profiles_dir,
            tmp.path().join(".claude-switch").join("profiles")
        );
        assert!(mgr.profiles_dir.exists());

        #[cfg(target_os = "windows")]
        assert_eq!(
            ProfileManager::cmd_bin_dir_for_home(tmp.path()),
            tmp.path().join(".local").join("bin")
        );

        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            ProfileManager::sh_bin_dir_for_home(tmp.path()),
            tmp.path().join(".varusers").join("bin")
        );
    }

    fn make_claude_dir(root: &Path) -> PathBuf {
        let dir = root.to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        let claude_json = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "test@example.com",
                "accountUuid": "uuid-0000-test"
            },
            "someOtherConfig": true
        });
        fs::write(
            dir.join(".claude.json"),
            serde_json::to_string_pretty(&claude_json).unwrap(),
        )
        .unwrap();
        let creds_json = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "access_tok",
                "refreshToken": "refresh_tok",
                "expiresAt": 9_999_999_999_u64,
                "scopes": ["user:inference"],
                "subscriptionType": "max"
            }
        });
        fs::write(
            dir.join(".credentials.json"),
            serde_json::to_string_pretty(&creds_json).unwrap(),
        )
        .unwrap();
        dir
    }

    fn unquote_single_quoted_shell_literal(value: &str) -> String {
        let inner = value
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .expect("expected single-quoted shell literal");
        inner.replace("'\\''", "'")
    }

    fn find_line<'a>(content: &'a str, prefix: &str) -> &'a str {
        content
            .lines()
            .find(|line| line.starts_with(prefix))
            .expect("expected line to exist")
    }

    fn unescape_generated_cmd_set_value(value: &str) -> String {
        let mut out = String::new();
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' if chars.peek() == Some(&'\\') => {
                    chars.next();
                    out.push('\\');
                }
                '\\' if chars.peek() == Some(&'"') => {
                    chars.next();
                    out.push('"');
                }
                '%' if chars.peek() == Some(&'%') => {
                    chars.next();
                    out.push('%');
                }
                '^' if chars.peek() == Some(&'^') => {
                    chars.next();
                    out.push('^');
                }
                _ => out.push(ch),
            }
        }
        out
    }

    fn cmd_set_value<'a>(content: &'a str, var_name: &str) -> &'a str {
        let prefix = format!("set \"{var_name}=");
        let line = find_line(content, &prefix);
        line.trim_start_matches(&prefix)
            .strip_suffix('"')
            .expect("expected set assignment to end with a quote")
    }

    #[cfg(not(windows))]
    fn tinyfish_timeout_test_args() -> Vec<&'static str> {
        vec!["-c", "sleep 5"]
    }

    #[cfg(windows)]
    fn tinyfish_timeout_test_args() -> Vec<&'static str> {
        vec!["/c", "ping -n 6 127.0.0.1 >nul"]
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let mut header_end = None;
        let mut body_len = 0usize;

        loop {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none()
                && let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n")
            {
                header_end = Some(pos + 4);
                let headers = String::from_utf8_lossy(&buf[..pos + 4]);
                body_len = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
            }
            if let Some(end) = header_end
                && buf.len() >= end + body_len
            {
                break;
            }
        }

        String::from_utf8(buf).unwrap()
    }

    fn spawn_model_fetch_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut paths = Vec::new();
            for (status_line, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                paths.push(path);
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            paths
        });
        (format!("http://{}", addr), handle)
    }

    // ── copy_dir_all ──────────────────────────────────────────────────────────

    #[test]
    fn copy_dir_all_copies_flat_files() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("b.txt"), "world").unwrap();
        let dst = tmp.path().join("dst");
        copy_dir_all(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dst.join("b.txt")).unwrap(), "world");
    }

    #[test]
    fn copy_dir_all_copies_nested_directories() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("sub/deep")).unwrap();
        fs::write(src.join("root.txt"), "root").unwrap();
        fs::write(src.join("sub").join("mid.txt"), "mid").unwrap();
        fs::write(src.join("sub/deep").join("leaf.txt"), "leaf").unwrap();
        let dst = tmp.path().join("dst");
        copy_dir_all(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("root.txt")).unwrap(), "root");
        assert_eq!(fs::read_to_string(dst.join("sub/mid.txt")).unwrap(), "mid");
        assert_eq!(
            fs::read_to_string(dst.join("sub/deep/leaf.txt")).unwrap(),
            "leaf"
        );
    }

    // ── add_profile_from ──────────────────────────────────────────────────────

    #[test]
    fn load_registry_returns_empty_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let reg = mgr.load_registry().unwrap();
        assert!(reg.profiles.is_empty());
    }

    #[test]
    fn save_and_load_registry_round_trips() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let id = Uuid::new_v4().to_string();
        let mut reg = Registry::default();
        reg.profiles.insert(
            id.clone(),
            Profile {
                id,
                name: "work".into(),
                alias: None,
                added: Utc::now(),
                last_used: None,
                kind: ProfileKind::Full,
                env: None,
                launch_args: None,
                provider_id: None,
                key_id: None,
                mcp_server_ids: Vec::new(),
            },
        );
        mgr.save_registry(&reg).unwrap();
        let loaded = mgr.load_registry().unwrap();
        assert_eq!(loaded.profiles.len(), 1);
    }

    // ── add_profile_from ──────────────────────────────────────────────────────

    #[test]
    fn add_profile_copies_files_into_profiles_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        mgr.add_profile_from("work", None, &src).unwrap();
        let profile = mgr.get_profile("work").unwrap();
        let dest = mgr.profile_dir(&profile);
        assert!(dest.join(".claude.json").exists(), ".claude.json missing");
        assert!(
            dest.join(".credentials.json").exists(),
            ".credentials.json missing"
        );
    }

    #[test]
    fn add_profile_records_entry_in_registry() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        mgr.add_profile_from("slot", None, &src).unwrap();
        let reg = mgr.load_registry().unwrap();
        let found = reg.profiles.values().any(|p| p.name == "slot");
        assert!(found);
    }

    #[test]
    fn add_profile_errors_on_nonexistent_source() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let err = mgr
            .add_profile_from("bad", None, &tmp.path().join("does-not-exist"))
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[test]
    fn add_profile_errors_on_duplicate_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        mgr.add_profile_from("dup", None, &src).unwrap();
        let err = mgr.add_profile_from("dup", None, &src).unwrap_err();
        assert!(err.to_string().contains("already in use"), "{err}");
    }

    #[test]
    fn add_profile_with_alias() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let p = mgr
            .add_profile_from("My Work Profile", Some("work"), &src)
            .unwrap();
        assert_eq!(p.name, "My Work Profile");
        assert_eq!(p.alias.as_deref(), Some("work"));
        // Lookup by alias
        let found = mgr.get_profile("work").unwrap();
        assert_eq!(found.id, p.id);
    }

    // ── find_profile ─────────────────────────────────────────────────────────

    #[test]
    fn find_profile_by_id_alias_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let p = mgr
            .add_profile_from("Display 名称", Some("short"), &src)
            .unwrap();

        // By id
        let (id, _) = mgr.find_profile(&p.id).unwrap();
        assert_eq!(id, p.id);
        // By alias
        let (id2, _) = mgr.find_profile("short").unwrap();
        assert_eq!(id2, p.id);
        // By name
        let (id3, _) = mgr.find_profile("Display 名称").unwrap();
        assert_eq!(id3, p.id);
        // Not found
        assert!(mgr.find_profile("nope").is_err());
    }

    #[test]
    fn find_profile_errors_on_ambiguous_alias() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let s1 = make_claude_dir(&tmp.path().join("c1"));
        let s2 = make_claude_dir(&tmp.path().join("c2"));
        mgr.add_profile_from("Profile One", Some("p"), &s1).unwrap();
        // Force-add second with same alias should remove the first (force behavior)
        // Actually, add_profile_from checks uniqueness, so second should fail
        let err = mgr
            .add_profile_from("Profile Two", Some("p"), &s2)
            .unwrap_err();
        assert!(err.to_string().contains("already in use"), "{err}");
    }

    // ── force add ────────────────────────────────────────────────────────────

    #[test]
    fn force_add_overwrites_existing_profile() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("v1"));
        mgr.add_profile_from("slot", None, &src).unwrap();
        let src2 = make_claude_dir(&tmp.path().join("v2"));
        mgr.add_profile_from_force("slot", None, &src2).unwrap();
        let p = mgr.get_profile("slot").unwrap();
        let dest = mgr.profile_dir(&p);
        assert!(dest.join(".claude.json").exists());
    }

    #[test]
    fn force_add_works_when_profile_does_not_yet_exist() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let p = mgr.add_profile_from_force("brand-new", None, &src).unwrap();
        assert_eq!(p.name, "brand-new");
    }

    // ── list_profiles ─────────────────────────────────────────────────────────

    #[test]
    fn list_profiles_returns_sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        for name in &["zebra", "alpha", "mango"] {
            let src = make_claude_dir(&tmp.path().join(format!("src-{name}")));
            mgr.add_profile_from(name, None, &src).unwrap();
        }
        let profiles = mgr.list_profiles().unwrap();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mango", "zebra"]);
    }

    // ── remove_profile ────────────────────────────────────────────────────────

    #[test]
    fn remove_profile_by_name_deletes_directory_and_entry() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let p = mgr.add_profile_from("to-delete", None, &src).unwrap();
        let dir = mgr.profile_dir(&p);
        mgr.remove_profile("to-delete").unwrap();
        assert!(!dir.exists());
        assert!(mgr.get_profile("to-delete").is_err());
    }

    #[test]
    fn remove_profile_by_alias() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        mgr.add_profile_from("Long Display Name", Some("del"), &src)
            .unwrap();
        mgr.remove_profile("del").unwrap();
        assert!(mgr.get_profile("del").is_err());
    }

    #[test]
    fn remove_profile_errors_when_profile_not_found() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let err = mgr.remove_profile("ghost").unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    // ── rename_profile ───────────────────────────────────────────────────────

    #[test]
    fn rename_profile_changes_name_and_alias() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let p = mgr.add_profile_from("old-name", Some("old"), &src).unwrap();
        let renamed = mgr.rename_profile(&p.id, "new-name", Some("new")).unwrap();
        assert_eq!(renamed.name, "new-name");
        assert_eq!(renamed.alias.as_deref(), Some("new"));
        assert_eq!(renamed.id, p.id); // id preserved
        // Old name no longer works
        assert!(mgr.get_profile("old-name").is_err());
        assert!(mgr.get_profile("old").is_err());
        // New name and alias work
        assert!(mgr.get_profile("new-name").is_ok());
        assert!(mgr.get_profile("new").is_ok());
    }

    #[test]
    fn rename_profile_errors_on_duplicate_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let s1 = make_claude_dir(&tmp.path().join("c1"));
        let s2 = make_claude_dir(&tmp.path().join("c2"));
        let p1 = mgr.add_profile_from("Profile A", Some("a"), &s1).unwrap();
        mgr.add_profile_from("Profile B", Some("b"), &s2).unwrap();
        let err = mgr.rename_profile(&p1.id, "Profile B", None).unwrap_err();
        assert!(err.to_string().contains("already in use"), "{err}");
    }

    // ── lightweight profiles ─────────────────────────────────────────────────

    #[test]
    fn create_and_launch_lightweight() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let env = LightweightEnv {
            auth_token: Some("tok".into()),
            base_url: Some("https://api.example.com".into()),
            ..Default::default()
        };
        let p = mgr
            .create_lightweight_profile("lite-prof", Some("lp"), env.clone())
            .unwrap();
        assert_eq!(p.name, "lite-prof");
        assert_eq!(p.alias.as_deref(), Some("lp"));
        assert_eq!(p.kind, ProfileKind::Lightweight);
        // Lookup by alias
        let found = mgr.get_profile("lp").unwrap();
        assert_eq!(found.id, p.id);
    }

    #[test]
    fn update_lightweight_preserves_id() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let env = LightweightEnv {
            auth_token: Some("old".into()),
            ..Default::default()
        };
        let p = mgr
            .create_lightweight_profile("test", Some("t"), env)
            .unwrap();
        let original_id = p.id.clone();

        let new_env = LightweightEnv {
            auth_token: Some("new".into()),
            ..Default::default()
        };
        let updated = mgr
            .update_lightweight(&original_id, "test-renamed", Some("tr"), new_env)
            .unwrap();
        assert_eq!(updated.id, original_id);
        assert_eq!(updated.name, "test-renamed");
        assert_eq!(updated.alias.as_deref(), Some("tr"));
        assert_eq!(
            updated.env.as_ref().unwrap().auth_token.as_deref(),
            Some("new")
        );
    }

    #[test]
    fn load_registry_keeps_standalone_inline_credentials() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let env = LightweightEnv {
            auth_token: Some("tok".into()),
            base_url: Some("https://api.example.com".into()),
            ..Default::default()
        };
        let profile = mgr
            .create_lightweight_profile("lite-prof", Some("lp"), env)
            .unwrap();

        let loaded = mgr.load_registry().unwrap();
        let migrated = loaded.profiles.get(&profile.id).unwrap();
        assert_eq!(migrated.provider_id, None);
        assert_eq!(migrated.key_id, None);
        assert!(loaded.providers.is_empty());

        let (token, url) = mgr.resolve_credentials(migrated).unwrap();
        assert_eq!(token.as_deref(), Some("tok"));
        assert_eq!(url.as_deref(), Some("https://api.example.com"));
    }

    #[test]
    fn unset_provider_persists_and_does_not_relink_on_reload() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let env = LightweightEnv {
            auth_token: Some("tok-inline".into()),
            base_url: Some("https://api.example.com".into()),
            ..Default::default()
        };
        let profile = mgr
            .create_lightweight_profile("lite-prof", Some("lp"), env)
            .unwrap();
        let provider = mgr
            .add_provider("Example", "https://api.example.com", "tok-provider")
            .unwrap();
        let key_id = provider.keys.keys().next().unwrap().clone();

        mgr.set_provider(&profile.id, &provider.id, &key_id)
            .unwrap();
        mgr.unset_provider(&profile.id).unwrap();

        let loaded = mgr.load_registry().unwrap();
        let stored = loaded.profiles.get(&profile.id).unwrap();
        assert_eq!(stored.provider_id, None);
        assert_eq!(stored.key_id, None);
        mgr.remove_provider(&provider.id).unwrap();
    }

    #[test]
    fn load_registry_keeps_distinct_providers_with_same_base_url() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let first = mgr
            .add_provider("First", "https://shared.example.invalid", "tok-one")
            .unwrap();
        let second = mgr
            .add_provider("Second", "https://shared.example.invalid", "tok-two")
            .unwrap();

        let loaded = mgr.load_registry().unwrap();
        assert!(loaded.providers.contains_key(&first.id));
        assert!(loaded.providers.contains_key(&second.id));
        assert_eq!(loaded.providers.len(), 2);
        assert_eq!(
            loaded
                .providers
                .get(&first.id)
                .map(|provider| provider.name.as_str()),
            Some("First")
        );
        assert_eq!(
            loaded
                .providers
                .get(&second.id)
                .map(|provider| provider.name.as_str()),
            Some("Second")
        );
    }

    #[test]
    fn build_model_discovery_candidates_strips_known_compat_suffixes() {
        let candidates =
            build_model_discovery_candidates("https://api.deepseek.com/anthropic").unwrap();
        assert_eq!(
            candidates,
            vec![
                "https://api.deepseek.com/anthropic/v1/models",
                "https://api.deepseek.com/v1/models",
                "https://api.deepseek.com/models",
            ]
        );
    }

    #[test]
    fn build_model_discovery_candidates_prefers_longest_suffix() {
        let candidates =
            build_model_discovery_candidates("https://api.z.ai/api/anthropic").unwrap();
        assert_eq!(
            candidates,
            vec![
                "https://api.z.ai/api/anthropic/v1/models",
                "https://api.z.ai/v1/models",
                "https://api.z.ai/models",
            ]
        );
    }

    #[test]
    fn discover_models_falls_back_to_root_models_endpoint() {
        let (base_url, handle) = spawn_model_fetch_server(vec![
            ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
            (
                "HTTP/1.1 200 OK",
                "{\"data\":[{\"id\":\"deepseek-chat\"},{\"id\":\"deepseek-reasoner\"}]}",
            ),
        ]);

        let result = discover_models(&format!("{base_url}/anthropic"), "sk-test").unwrap();
        let paths = handle.join().unwrap();

        assert_eq!(
            paths,
            vec!["/anthropic/v1/models".to_string(), "/v1/models".to_string()]
        );
        assert_eq!(result.endpoint_used, format!("{base_url}/v1/models"));
        assert_eq!(
            result.models,
            vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
        );
    }

    #[test]
    fn discover_models_classifies_auth_failure() {
        let (base_url, handle) = spawn_model_fetch_server(vec![(
            "HTTP/1.1 401 Unauthorized",
            "{\"error\":\"bad auth\"}",
        )]);

        let failure = discover_models(&base_url, "sk-test").unwrap_err();
        let paths = handle.join().unwrap();
        let expected_endpoint = format!("{base_url}/v1/models");

        assert_eq!(paths, vec!["/v1/models".to_string()]);
        assert_eq!(failure.kind, ModelDiscoveryFailureKind::Auth);
        assert_eq!(
            failure.last_endpoint.as_deref(),
            Some(expected_endpoint.as_str())
        );
    }

    #[test]
    fn discover_models_classifies_endpoint_not_found_after_candidates() {
        let (base_url, handle) = spawn_model_fetch_server(vec![
            ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
            ("HTTP/1.1 405 Method Not Allowed", "{\"error\":\"blocked\"}"),
            ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
        ]);

        let failure = discover_models(&format!("{base_url}/api/anthropic"), "sk-test").unwrap_err();
        let paths = handle.join().unwrap();

        assert_eq!(
            paths,
            vec![
                "/api/anthropic/v1/models".to_string(),
                "/v1/models".to_string(),
                "/models".to_string(),
            ]
        );
        assert_eq!(failure.kind, ModelDiscoveryFailureKind::EndpointNotFound);
    }

    #[test]
    fn discover_models_parses_models_field_fallback() {
        let (base_url, handle) = spawn_model_fetch_server(vec![(
            "HTTP/1.1 200 OK",
            "{\"object\":\"list\",\"models\":[{\"id\":\"llama3\"},{\"id\":\"qwen3-coder\"}]}",
        )]);

        let result = discover_models(&base_url, "sk-ollama").unwrap();
        let paths = handle.join().unwrap();

        assert_eq!(paths, vec!["/v1/models".to_string()]);
        assert_eq!(
            result.models,
            vec!["llama3".to_string(), "qwen3-coder".to_string()]
        );
    }

    #[test]
    fn migration_adds_key_id_for_single_key_provider_links() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider_id = "prov_old".to_string();
        let key_id = "key_old".to_string();
        let mut keys = HashMap::new();
        keys.insert(
            key_id.clone(),
            ProviderKey {
                id: key_id.clone(),
                name: "Default".into(),
                api_key: "tok".into(),
            },
        );
        let mut reg = Registry::default();
        reg.providers.insert(
            provider_id.clone(),
            Provider {
                id: provider_id.clone(),
                name: "Example".into(),
                base_url: "https://api.example.com".into(),
                keys,
                api_key: String::new(),
            },
        );
        reg.profiles.insert(
            "profile-1".into(),
            Profile {
                id: "profile-1".into(),
                name: "lite".into(),
                alias: None,
                added: Utc::now(),
                last_used: None,
                kind: ProfileKind::Lightweight,
                env: None,
                launch_args: None,
                provider_id: Some(provider_id.clone()),
                key_id: None,
                mcp_server_ids: Vec::new(),
            },
        );
        mgr.save_registry(&reg).unwrap();

        let loaded = mgr.load_registry().unwrap();
        let migrated = loaded.profiles.get("profile-1").unwrap();
        assert_eq!(migrated.key_id.as_deref(), Some(key_id.as_str()));
        let err = mgr.remove_key(&provider_id, &key_id).unwrap_err();
        assert!(err.to_string().contains("used by profiles"), "{err}");
    }

    #[test]
    fn list_profiles_using_key_returns_sorted_linked_profiles() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider = mgr
            .add_provider("Example", "https://api.example.com", "tok")
            .unwrap();
        let key_id = provider.keys.keys().next().unwrap().clone();
        let alpha = mgr
            .create_lightweight_profile("alpha", Some("alpha"), LightweightEnv::default())
            .unwrap();
        let beta = mgr
            .create_lightweight_profile("beta", Some("beta"), LightweightEnv::default())
            .unwrap();
        mgr.set_provider(&beta.id, &provider.id, &key_id).unwrap();
        mgr.set_provider(&alpha.id, &provider.id, &key_id).unwrap();

        let linked = mgr.list_profiles_using_key(&provider.id, &key_id).unwrap();

        assert_eq!(linked.len(), 2);
        assert_eq!(linked[0].name, "alpha");
        assert_eq!(linked[1].name, "beta");
    }

    #[test]
    fn load_registry_clears_invalid_provider_key_link() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider = mgr
            .add_provider("Example", "https://api.example.com", "tok")
            .unwrap();
        let profile = mgr
            .create_lightweight_profile("lite", None, LightweightEnv::default())
            .unwrap();
        let mut reg = mgr.load_registry().unwrap();
        let stored = reg.profiles.get_mut(&profile.id).unwrap();
        stored.provider_id = Some(provider.id.clone());
        stored.key_id = Some("missing-key".into());
        mgr.save_registry(&reg).unwrap();

        let loaded = mgr.load_registry().unwrap();
        let profile = loaded.profiles.get(&profile.id).unwrap();
        assert_eq!(profile.provider_id, None);
        assert_eq!(profile.key_id, None);
    }

    #[test]
    fn set_provider_rejects_full_profiles() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let full = mgr.add_profile_from("full", None, &src).unwrap();
        let provider = mgr
            .add_provider("Example", "https://api.example.com", "tok")
            .unwrap();
        let key_id = provider.keys.keys().next().unwrap();

        let err = mgr
            .set_provider(&full.id, &provider.id, key_id)
            .unwrap_err();
        assert!(err.to_string().contains("lightweight"), "{err}");
    }

    #[test]
    fn migration_moves_deprecated_api_key_even_when_keys_exist() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let raw = serde_json::json!({
            "profiles": {},
            "providers": {
                "prov_old": {
                    "id": "prov_old",
                    "name": "Example",
                    "base_url": "https://api.example.com",
                    "keys": {
                        "key_existing": {
                            "id": "key_existing",
                            "name": "Existing",
                            "api_key": "existing-token"
                        }
                    },
                    "api_key": "deprecated-token"
                }
            }
        });
        fs::write(
            &mgr.registry_path,
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();

        let loaded = mgr.load_registry().unwrap();
        let provider = loaded.providers.get("prov_old").unwrap();
        assert!(provider.api_key.is_empty());
        assert!(
            provider
                .keys
                .values()
                .any(|key| key.api_key == "deprecated-token")
        );
        assert!(
            provider
                .keys
                .values()
                .any(|key| key.api_key == "existing-token")
        );
    }

    #[test]
    fn provider_serialization_omits_deprecated_api_key_field() {
        let mut keys = HashMap::new();
        keys.insert(
            "key_existing".into(),
            ProviderKey {
                id: "key_existing".into(),
                name: "Existing".into(),
                api_key: "sk-test-generated-key-777777777777777777777777".into(),
            },
        );
        let provider = Provider {
            id: "prov_generated".into(),
            name: "Generated".into(),
            base_url: "https://generated-provider.invalid".into(),
            keys,
            api_key: "deprecated-should-not-serialize".into(),
        };

        let value: serde_json::Value = serde_json::to_value(&provider).unwrap();
        assert!(value.get("api_key").is_none(), "{value}");
        assert_eq!(
            value
                .pointer("/keys/key_existing/api_key")
                .and_then(|entry| entry.as_str()),
            Some("sk-test-generated-key-777777777777777777777777")
        );
    }

    #[test]
    fn test_anthropic_message_sends_expected_request_and_parses_response() {
        // For a bare host like http://127.0.0.1:PORT, build_message_candidates
        // produces only one candidate: {host}/v1/messages. So a single-response
        // test server is sufficient.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let response_body = serde_json::json!({
                "content": [
                    { "type": "text", "text": "Hello from generated test server" }
                ],
                "usage": {
                    "input_tokens": 7,
                    "output_tokens": 11
                }
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
            request
        });

        let result = test_anthropic_message(
            &format!("http://{}", addr),
            "sk-test-generated-key-555555555555555555555555",
            "claude-test-generated-model",
            "Hello",
        )
        .unwrap();
        let request = handle.join().unwrap();

        assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
        assert!(request.contains("content-type: application/json; charset=utf-8\r\n"));
        assert!(request.contains("x-api-key: sk-test-generated-key-555555555555555555555555\r\n"));
        assert!(
            request.contains(
                "authorization: Bearer sk-test-generated-key-555555555555555555555555\r\n"
            )
        );
        assert!(request.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(request.contains("claude-test-generated-model"));
        assert!(request.contains("\"content\": \"Hello\""));
        assert_eq!(result.text, "Hello from generated test server");
        assert!(result.endpoint_used.ends_with("/v1/messages"));
        assert_eq!(result.input_tokens, Some(7));
        assert_eq!(result.output_tokens, Some(11));
    }

    #[test]
    fn test_anthropic_message_surfaces_http_error_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _request = read_http_request(&mut stream);
            let response_body = serde_json::json!({
                "error": {
                    "message": "generated unauthorized"
                }
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
        });

        let err = test_anthropic_message(
            &format!("http://{}", addr),
            "sk-test-generated-key-666666666666666666666666",
            "claude-test-generated-model",
            "Hello",
        )
        .unwrap_err();
        handle.join().unwrap();

        let msg = err.to_string();
        assert!(msg.contains("HTTP 401"), "{msg}");
        assert!(msg.contains("generated unauthorized"), "{msg}");
    }

    // ── generate_aliases ──────────────────────────────────────────────────────

    #[test]
    fn generate_aliases_uses_alias_when_present() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        mgr.add_profile_from("Long Name", Some("ln"), &src).unwrap();
        let out = mgr.generate_aliases().unwrap();
        // Should use "ln" (alias) not "Long Name"
        assert!(out.contains("claude-ln"), "expected 'claude-ln' in:\n{out}");
    }

    #[test]
    fn generate_aliases_when_empty_returns_hint() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let out = mgr.generate_aliases().unwrap();
        assert!(out.contains("No profiles"), "{out}");
    }

    #[test]
    fn recover_shims_parses_legacy_cmd_and_groups_provider_keys() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let shim_dir = tmp.path().join("shims");
        fs::create_dir_all(&shim_dir).unwrap();
        let shim = r#"@echo off
setlocal
:: Generated by cswitch (claude-switch) — do not edit manually
:: Profile: 芒果-deepseek (lightweight)
set "_LAUNCH_ARGS=--dangerously-skip-permissions"
:launch
if defined _E (claude --settings "{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"sk-mango\",\"ANTHROPIC_BASE_URL\":\"https://aigc-llm.mgtv.com\",\"ANTHROPIC_DEFAULT_HAIKU_MODEL\":\"deepseek-v4-flash[1m]\",\"ANTHROPIC_DEFAULT_OPUS_MODEL\":\"deepseek-v4-pro[1m]\",\"ANTHROPIC_DEFAULT_SONNET_MODEL\":\"deepseek-v4-pro[1m]\",\"ANTHROPIC_MODEL\":\"deepseek-v4-pro[1m]\",\"CLAUDE_CODE_SUBAGENT_MODEL\":\"qwen3.7-max[1m]\",\"EXTRA_FLAG\":\"yes\"}}" %_LAUNCH_ARGS%!_R!)
"#;
        fs::write(shim_dir.join("claude-mg-ds.cmd"), shim).unwrap();

        let plan = mgr.plan_shim_recovery(&shim_dir, false).unwrap();
        assert_eq!(plan.files_scanned, 1);
        assert_eq!(plan.files_recoverable, 1);
        assert_eq!(plan.profiles_added, 1);
        assert_eq!(plan.providers_added, 1);
        assert_eq!(plan.provider_keys_added, 1);

        let summary = mgr.recover_shims(&shim_dir, false).unwrap();
        assert_eq!(summary.plan.profiles_added, 1);
        assert!(summary.backup_path.is_none());

        let registry = mgr.load_registry().unwrap();
        assert_eq!(registry.profiles.len(), 1);
        assert_eq!(registry.providers.len(), 1);
        let profile = registry.profiles.values().next().unwrap();
        assert_eq!(profile.name, "芒果-deepseek");
        assert_eq!(profile.alias.as_deref(), Some("mg-ds"));
        assert_eq!(
            profile.launch_args.as_deref(),
            Some(&vec!["--dangerously-skip-permissions".to_string()][..])
        );
        let env = profile.env.as_ref().unwrap();
        assert_eq!(env.auth_token, None);
        assert_eq!(env.base_url, None);
        assert_eq!(env.model.as_deref(), Some("deepseek-v4-pro[1m]"));
        assert_eq!(env.subagent_model.as_deref(), Some("qwen3.7-max[1m]"));
        assert_eq!(env.extras, vec!["EXTRA_FLAG=yes"]);
        let provider = registry
            .providers
            .get(profile.provider_id.as_ref().unwrap())
            .unwrap();
        assert_eq!(provider.base_url, "https://aigc-llm.mgtv.com");
        let key = provider.keys.get(profile.key_id.as_ref().unwrap()).unwrap();
        assert_eq!(key.api_key, "sk-mango");
    }

    #[test]
    fn recover_shims_parses_current_cmd_settings_variable() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let profile = Profile {
            id: Uuid::new_v4().to_string(),
            name: "current".into(),
            alias: Some("cur".into()),
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Lightweight,
            env: Some(LightweightEnv {
                auth_token: Some("sk-current".into()),
                base_url: Some("https://current.example.invalid".into()),
                model: Some("current-model".into()),
                ..Default::default()
            }),
            launch_args: Some(vec!["--dangerously-skip-permissions".into()]),
            provider_id: None,
            key_id: None,
            mcp_server_ids: Vec::new(),
        };
        let content = mgr.generate_cmd_content(&profile).unwrap();
        let recovered = ProfileManager::parse_recoverable_shim("claude-cur.cmd", &content).unwrap();
        assert_eq!(recovered.name, "current");
        assert_eq!(recovered.alias, "cur");
        assert_eq!(recovered.token, "sk-current");
        assert_eq!(recovered.base_url, "https://current.example.invalid");
        assert_eq!(recovered.env.model.as_deref(), Some("current-model"));
        assert_eq!(
            recovered.launch_args.as_deref(),
            Some(&vec!["--dangerously-skip-permissions".to_string()][..])
        );
    }

    #[test]
    fn recover_shims_parses_shell_settings_env() {
        let content = r#"#!/usr/bin/env bash
# Generated by cswitch (claude-switch) — do not edit manually
# Profile: shell profile (lightweight)
SETTINGS_ENV='{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-shell","ANTHROPIC_BASE_URL":"https://shell.example.invalid","ANTHROPIC_MODEL":"shell-model"}'
BASE_SETTINGS="${SETTINGS_ENV}"'}'
exec claude "${SETTINGS_ARG[@]}"
"#;
        let recovered =
            ProfileManager::parse_recoverable_shim("claude-shell-prof", content).unwrap();
        assert_eq!(recovered.name, "shell profile");
        assert_eq!(recovered.alias, "shell-prof");
        assert_eq!(recovered.token, "sk-shell");
        assert_eq!(recovered.base_url, "https://shell.example.invalid");
        assert_eq!(recovered.env.model.as_deref(), Some("shell-model"));
    }

    #[test]
    fn recover_shims_conflicts_until_replace() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        mgr.create_lightweight_profile("existing", Some("ex"), LightweightEnv::default())
            .unwrap();
        let shim_dir = tmp.path().join("shims");
        fs::create_dir_all(&shim_dir).unwrap();
        fs::write(
            shim_dir.join("claude-ex.cmd"),
            r#"@echo off
:: Generated by cswitch (claude-switch) — do not edit manually
:: Profile: existing (lightweight)
claude --settings "{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"sk-replace\",\"ANTHROPIC_BASE_URL\":\"https://replace.example.invalid\",\"ANTHROPIC_MODEL\":\"replace-model\"}}"
"#,
        )
        .unwrap();

        let plan = mgr.plan_shim_recovery(&shim_dir, false).unwrap();
        assert_eq!(plan.profiles_conflicted, 1);
        assert!(mgr.recover_shims(&shim_dir, false).is_err());

        let summary = mgr.recover_shims(&shim_dir, true).unwrap();
        assert_eq!(summary.plan.profiles_updated, 1);
        assert!(summary.backup_path.is_some());
        let (_, profile) = mgr.find_profile("ex").unwrap();
        assert_eq!(
            profile.env.as_ref().unwrap().model.as_deref(),
            Some("replace-model")
        );
        assert!(profile.provider_id.is_some());
        assert!(profile.key_id.is_some());
    }

    #[test]
    fn remote_path_join_matches_target_os_separator() {
        assert_eq!(
            ProfileManager::join_remote_path(
                "/home/test/.varusers/bin",
                RemoteOs::Unix,
                "claude-dev"
            ),
            "/home/test/.varusers/bin/claude-dev"
        );
        assert_eq!(
            ProfileManager::join_remote_path(
                "C:\\Users\\tester\\.local\\bin",
                RemoteOs::Windows,
                "claude-dev.cmd"
            ),
            "C:\\Users\\tester\\.local\\bin\\claude-dev.cmd"
        );
    }

    #[test]
    fn managed_remote_name_filter_only_matches_generated_prefix() {
        assert!(ProfileManager::is_managed_remote_name(
            RemoteOs::Unix,
            "claude-work"
        ));
        assert!(ProfileManager::is_managed_remote_name(
            RemoteOs::Windows,
            "claude-work.cmd"
        ));
        assert!(!ProfileManager::is_managed_remote_name(
            RemoteOs::Unix,
            "aria2c"
        ));
        assert!(!ProfileManager::is_managed_remote_name(
            RemoteOs::Windows,
            "aria2c.exe"
        ));
        assert!(!ProfileManager::is_managed_remote_name(
            RemoteOs::Unix,
            "xclaude-work"
        ));
    }

    #[test]
    fn remote_shim_file_name_skips_full_profiles() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"));
        let full = mgr
            .add_profile_from("full", Some("full-alias"), &src)
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-alias"), LightweightEnv::default())
            .unwrap();

        assert_eq!(
            ProfileManager::remote_shim_file_name(&full, RemoteOs::Unix),
            None
        );
        assert_eq!(
            ProfileManager::remote_shim_file_name(&lite, RemoteOs::Unix).as_deref(),
            Some("claude-lite-alias")
        );
        assert_eq!(
            ProfileManager::remote_shim_file_name(&lite, RemoteOs::Windows).as_deref(),
            Some("claude-lite-alias.cmd")
        );
    }

    #[test]
    fn remote_upload_batch_includes_chmod_for_unix() {
        let desired = vec![
            ("claude-work".to_string(), "content".to_string()),
            ("claude-play".to_string(), "content".to_string()),
        ];
        let batch = ProfileManager::build_remote_upload_batch(
            std::path::Path::new("/tmp/cswitch-remote"),
            "/share/home/shark/.varusers/bin",
            RemoteOs::Unix,
            &desired,
            true,
        );
        assert_eq!(batch.matches("put ").count(), 2);
        assert!(batch.contains("chmod 755 \"/share/home/shark/.varusers/bin/claude-work\""));
        assert!(batch.contains("chmod 755 \"/share/home/shark/.varusers/bin/claude-play\""));
    }

    #[test]
    fn remote_upload_batch_skips_chmod_for_sidecars() {
        let desired = vec![
            (
                "tinyfish-full/.claude-plugin/plugin.json".to_string(),
                "{\"name\":\"tinyfish-full\"}".to_string(),
            ),
            (
                "tinyfish-full/hooks/hooks.json".to_string(),
                "{\"hooks\":{}}".to_string(),
            ),
        ];
        let batch = ProfileManager::build_remote_upload_batch(
            std::path::Path::new("/tmp/cswitch-remote"),
            "/share/home/shark/.claude-switch/generated/plugins",
            RemoteOs::Unix,
            &desired,
            false,
        );
        assert_eq!(batch.matches("put ").count(), 2);
        assert!(batch.contains(
            "\"/share/home/shark/.claude-switch/generated/plugins/tinyfish-full/.claude-plugin/plugin.json\""
        ));
        assert!(batch.contains(
            "\"/share/home/shark/.claude-switch/generated/plugins/tinyfish-full/hooks/hooks.json\""
        ));
        assert!(!batch.contains("chmod 755"));
    }

    #[test]
    fn generate_cmd_content_available_for_remote_windows_shims_on_non_windows_hosts() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-alias"), LightweightEnv::default())
            .unwrap();

        let content = mgr.generate_cmd_content(&lite).unwrap();

        assert!(content.contains("@echo off"));
        assert!(content.contains(CMD_MARKER));
        assert!(content.contains("claude"));
    }

    #[test]
    fn generate_cmd_content_escapes_settings_for_remote_windows_shims() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "lite",
                Some("lite-alias"),
                LightweightEnv {
                    model: Some("claude-sonnet-4".into()),
                    extras: vec![
                        "PERCENT=value%with%percent".into(),
                        "BANG=value!with!bang".into(),
                    ],
                    ..Default::default()
                },
            )
            .unwrap();

        let content = mgr.generate_cmd_content(&lite).unwrap();

        assert!(content.contains("--settings "));
        assert!(content.contains("PERCENT"));
        assert!(content.contains("%%with%%percent"));
        assert!(content.contains("!with!bang"));
        assert!(!content.contains("^!with^!bang"));
        let json = unescape_generated_cmd_set_value(cmd_set_value(&content, "_SETTINGS"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["env"]["PERCENT"].as_str(),
            Some("value%with%percent")
        );
        assert_eq!(parsed["env"]["BANG"].as_str(), Some("value!with!bang"));
    }

    #[test]
    fn strip_compat_suffix_strips_new_entries() {
        let cases = vec![
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "https://dashscope.aliyuncs.com",
            ),
            ("https://api.openrouter.ai/api", "https://api.openrouter.ai"),
            (
                "https://proxy.example.com/api/v1",
                "https://proxy.example.com",
            ),
            ("https://example.com/v1", "https://example.com"),
            ("https://example.com/v1/messages", "https://example.com"),
            ("https://example.com/messages", "https://example.com"),
        ];
        for (url, expected_root) in cases {
            let stripped = strip_compat_suffix(url.trim_end_matches('/'));
            assert_eq!(stripped, Some(expected_root), "strip_compat_suffix({url})");
        }
    }

    #[test]
    fn build_message_candidates_basic() {
        let candidates = build_message_candidates("https://api.anthropic.com").unwrap();
        assert_eq!(candidates, vec!["https://api.anthropic.com/v1/messages"]);
    }

    #[test]
    fn build_message_candidates_with_compat_suffix() {
        let candidates = build_message_candidates("https://proxy.example.com/api").unwrap();
        assert_eq!(
            candidates,
            vec![
                "https://proxy.example.com/api/v1/messages",
                "https://proxy.example.com/v1/messages",
                "https://proxy.example.com/messages",
            ]
        );
    }

    #[test]
    fn build_message_candidates_v1_suffix() {
        let candidates = build_message_candidates("https://example.com/v1").unwrap();
        // /v1 is a KNOWN_COMPAT_SUFFIX, so strip yields root https://example.com
        assert_eq!(
            candidates,
            vec![
                "https://example.com/v1/messages",
                "https://example.com/messages",
            ]
        );
    }

    #[test]
    fn build_message_candidates_messages_suffix() {
        let candidates = build_message_candidates("https://example.com/v1/messages").unwrap();
        // /v1/messages is stripped, root is https://example.com
        assert_eq!(
            candidates,
            vec![
                "https://example.com/v1/messages/v1/messages",
                "https://example.com/v1/messages",
                "https://example.com/messages",
            ]
        );
    }

    #[test]
    fn build_message_candidates_empty_url() {
        assert!(build_message_candidates("").is_err());
        assert!(build_message_candidates("   ").is_err());
    }

    #[test]
    fn build_model_discovery_candidates_new_suffixes() {
        let cases = vec![
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                vec![
                    "https://dashscope.aliyuncs.com/compatible-mode/v1/models",
                    "https://dashscope.aliyuncs.com/v1/models",
                    "https://dashscope.aliyuncs.com/models",
                ],
            ),
            (
                "https://api.openrouter.ai/api",
                vec![
                    "https://api.openrouter.ai/api/v1/models",
                    "https://api.openrouter.ai/v1/models",
                    "https://api.openrouter.ai/models",
                ],
            ),
            (
                "https://proxy.example.com/api/v1",
                vec![
                    "https://proxy.example.com/api/v1/models",
                    "https://proxy.example.com/v1/models",
                    "https://proxy.example.com/models",
                ],
            ),
            (
                "http://localhost:1234/v1",
                vec![
                    "http://localhost:1234/v1/models",
                    "http://localhost:1234/models",
                ],
            ),
        ];
        for (url, expected) in cases {
            let candidates = build_model_discovery_candidates(url).unwrap();
            assert_eq!(
                candidates, expected,
                "build_model_discovery_candidates({url})"
            );
        }
    }

    #[test]
    fn url_matches_exact() {
        assert!(url_matches(
            "https://api.deepseek.com/anthropic",
            NATIVE_SEARCH_URLS
        ));
        assert!(!url_matches(
            "https://new-api.example.com",
            NATIVE_SEARCH_URLS
        ));
    }

    #[test]
    fn url_matches_trailing_slash() {
        assert!(url_matches(
            "https://api.deepseek.com/anthropic/",
            NATIVE_SEARCH_URLS
        ));
    }

    #[test]
    fn url_matches_canonical_scheme_host_and_default_https_port() {
        assert!(url_matches("HTTPS://API.ANTHROPIC.COM/", NATIVE_FETCH_URLS));
        assert!(url_matches(
            "https://api.anthropic.com:443",
            NATIVE_FETCH_URLS
        ));
        assert!(url_matches(
            "https://API.DEEPSEEK.COM:443/anthropic/v1/messages",
            NATIVE_SEARCH_URLS
        ));
        assert!(!url_matches(
            "https://api.anthropic.com:444",
            NATIVE_FETCH_URLS
        ));
    }

    #[test]
    fn url_matches_no() {
        assert!(!url_matches(
            "https://api.openrouter.ai/api",
            NATIVE_SEARCH_URLS
        ));
        assert!(!url_matches("http://localhost:11434", NATIVE_SEARCH_URLS));
    }

    #[test]
    fn deepseek_has_search_but_not_fetch() {
        let base = "https://api.deepseek.com/anthropic";
        assert!(url_matches(base, NATIVE_SEARCH_URLS));
        assert!(!url_matches(base, NATIVE_FETCH_URLS));
    }

    #[test]
    fn anyrouter_has_both() {
        assert!(url_matches("https://anyrouter.top", NATIVE_SEARCH_URLS));
        assert!(url_matches("https://anyrouter.top", NATIVE_FETCH_URLS));
    }

    #[test]
    fn proxy_has_neither() {
        let base = "https://new-api.example.com";
        assert!(!url_matches(base, NATIVE_SEARCH_URLS));
        assert!(!url_matches(base, NATIVE_FETCH_URLS));
    }

    #[test]
    fn empty_base_url_uses_native_provider_defaults() {
        assert_eq!(tinyfish_mode(""), TinyfishMode::None);
        assert_eq!(tinyfish_mode("   "), TinyfishMode::None);
    }

    #[test]
    fn tinyfish_mode_accepts_canonical_native_urls() {
        assert_eq!(
            tinyfish_mode("HTTPS://API.ANTHROPIC.COM:443/"),
            TinyfishMode::None
        );
        assert_eq!(
            tinyfish_mode("https://API.DEEPSEEK.COM:443/anthropic/"),
            TinyfishMode::FetchOnly
        );
    }

    #[test]
    fn tinyfish_mode_uses_search_only_for_fetch_native_only() {
        assert_eq!(
            tinyfish_mode_for_capabilities(false, true),
            TinyfishMode::SearchOnly
        );
        let hooks =
            tinyfish_plugin_hooks(TinyfishMode::SearchOnly, TinyfishToolShell::PowerShell).unwrap();
        assert!(hooks.contains("WebSearch"));
        assert!(!hooks.contains("WebFetch"));
        let manifest = tinyfish_plugin_manifest(TinyfishMode::SearchOnly).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert_eq!(manifest["name"].as_str(), Some("tinyfish-search-only"));
    }

    #[test]
    fn tinyfish_mode_can_be_disabled_via_reserved_extra() {
        let env = LightweightEnv {
            base_url: Some("https://new-api.example.com".into()),
            extras: vec!["CLAUDE_SWITCH_TINYFISH=off".into()],
            ..Default::default()
        };
        let artifacts = build_lightweight_runtime_artifacts(
            &env,
            Some("sk-test"),
            env.base_url.as_deref(),
            TinyfishToolShell::PowerShell,
        )
        .unwrap();
        assert_eq!(artifacts.tinyfish_mode, TinyfishMode::None);
        assert!(artifacts.tinyfish_plugin_hooks_json.is_none());
        assert!(artifacts.tinyfish_plugin_manifest_json.is_none());
    }

    #[test]
    fn tinyfish_mode_disable_extra_is_case_insensitive() {
        let env = LightweightEnv {
            base_url: Some("https://new-api.example.com".into()),
            extras: vec!["CLAUDE_SWITCH_TINYFISH=FALSE".into()],
            ..Default::default()
        };
        let artifacts = build_lightweight_runtime_artifacts(
            &env,
            Some("sk-test"),
            env.base_url.as_deref(),
            TinyfishToolShell::PowerShell,
        )
        .unwrap();
        assert_eq!(artifacts.tinyfish_mode, TinyfishMode::None);
    }

    #[test]
    fn reserved_tinyfish_extra_is_not_forwarded_to_env() {
        let env = LightweightEnv {
            extras: vec!["CLAUDE_SWITCH_TINYFISH=off".into(), "FOO=bar".into()],
            ..Default::default()
        };
        let settings = build_lightweight_settings(
            &env,
            Some("sk-test"),
            Some("https://new-api.example.com"),
            TinyfishMode::Full,
            TinyfishToolShell::PowerShell,
        );
        let env_map = settings["env"].as_object().unwrap();
        assert!(!env_map.contains_key("CLAUDE_SWITCH_TINYFISH"));
        assert_eq!(env_map["FOO"].as_str(), Some("bar"));
    }

    #[test]
    fn tinyfish_full_hooks_use_requested_tool_shell() {
        let hooks = tinyfish_full_hooks(TinyfishToolShell::PowerShell);
        let pre_tool = hooks["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool.len(), 2);
        let matchers: Vec<&str> = pre_tool
            .iter()
            .map(|h| h["matcher"].as_str().unwrap())
            .collect();
        assert!(matchers.contains(&"WebSearch"));
        assert!(matchers.contains(&"WebFetch"));
        let search_hook = pre_tool
            .iter()
            .find(|h| h["matcher"].as_str() == Some("WebSearch"))
            .unwrap();
        assert!(
            search_hook["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("which search provider to use")
        );
        assert!(search_hook["hooks"][0]["shell"].as_str().unwrap() == "powershell");
        let fetch_hook = pre_tool
            .iter()
            .find(|h| h["matcher"].as_str() == Some("WebFetch"))
            .unwrap();
        assert!(
            fetch_hook["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("which fetch provider to use")
        );
        assert!(fetch_hook["hooks"][0]["shell"].as_str().unwrap() == "powershell");
        let subagent = hooks["hooks"]["SubagentStart"].as_array().unwrap();
        assert_eq!(subagent.len(), 1);
        let subagent_cmd = subagent[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(subagent_cmd.contains("tinyfish search query \\\"<QUERY>\\\""));
        assert!(subagent_cmd.contains("tinyfish fetch content get \\\"<URL>\\\""));
        assert!(!subagent_cmd.contains("tinyfish search query QUERY"));
        assert!(!subagent_cmd.contains("tinyfish fetch content get URL"));
        assert!(subagent_cmd.contains("PowerShell tool"));
    }

    #[test]
    fn tinyfish_fetch_only_hooks_use_requested_tool_shell() {
        let hooks = tinyfish_fetch_only_hooks(TinyfishToolShell::PowerShell);
        let pre_tool = hooks["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool.len(), 1);
        assert_eq!(pre_tool[0]["matcher"].as_str().unwrap(), "WebFetch");
        assert!(
            pre_tool[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("which fetch provider to use")
        );
        assert!(pre_tool[0]["hooks"][0]["shell"].as_str().unwrap() == "powershell");
        let subagent = hooks["hooks"]["SubagentStart"].as_array().unwrap();
        assert_eq!(subagent.len(), 1);
        let subagent_cmd = subagent[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(subagent_cmd.contains("tinyfish fetch content get \\\"<URL>\\\""));
        assert!(!subagent_cmd.contains("tinyfish search query \\\"<QUERY>\\\""));
        assert!(!subagent_cmd.contains("tinyfish fetch content get URL"));
        assert!(!subagent_cmd.contains("tinyfish search query QUERY"));
        assert!(subagent_cmd.contains("PowerShell tool"));
    }

    #[test]
    fn tinyfish_bash_hook_command_escapes_apostrophes() {
        let command = tinyfish_hook_command(
            TinyfishToolShell::Bash,
            "PreToolUse",
            Some("allow"),
            "don't break",
        );
        assert!(command.starts_with("printf '%s\\n' '"));
        assert!(command.contains("don'\\''t break"));
    }

    #[test]
    fn tinyfish_powershell_hook_command_escapes_apostrophes() {
        let command = tinyfish_hook_command(
            TinyfishToolShell::PowerShell,
            "PreToolUse",
            Some("allow"),
            "don't break",
        );
        assert!(command.starts_with("Write-Output '"));
        assert!(command.contains("don''t break"));
    }

    #[test]
    fn tinyfish_available_probe_times_out() {
        let started = std::time::Instant::now();
        let ok = tinyfish_command_succeeds_with_timeout(
            TINYFISH_TIMEOUT_TEST_PROGRAM,
            &tinyfish_timeout_test_args(),
            Duration::from_millis(100),
        );
        assert!(!ok);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn tinyfish_prompt_variants_are_platform_specific() {
        let bash_prompt = tinyfish_prompt(TinyfishMode::Full, TinyfishToolShell::Bash).unwrap();
        let powershell_prompt =
            tinyfish_prompt(TinyfishMode::Full, TinyfishToolShell::PowerShell).unwrap();
        assert!(bash_prompt.contains("run via the Bash tool"));
        assert!(!bash_prompt.contains("PowerShell"));
        assert!(powershell_prompt.contains("run via the PowerShell tool"));
        assert!(!powershell_prompt.contains("run via Bash"));
    }

    #[test]
    fn tinyfish_prompt_file_names_are_shared_by_mode_and_shell() {
        assert_eq!(
            ProfileManager::tinyfish_prompt_file_name(
                TinyfishMode::Full,
                TinyfishToolShell::PowerShell
            )
            .as_deref(),
            Some("tinyfish-full.powershell.txt")
        );
        assert_eq!(
            ProfileManager::tinyfish_prompt_file_name(
                TinyfishMode::FetchOnly,
                TinyfishToolShell::Bash
            )
            .as_deref(),
            Some("tinyfish-fetch-only.bash.txt")
        );
        assert_eq!(
            ProfileManager::tinyfish_prompt_file_name(
                TinyfishMode::SearchOnly,
                TinyfishToolShell::PowerShell
            )
            .as_deref(),
            Some("tinyfish-search-only.powershell.txt")
        );
        assert_eq!(
            ProfileManager::tinyfish_prompt_file_name(
                TinyfishMode::None,
                TinyfishToolShell::PowerShell
            ),
            None
        );
        assert!(ProfileManager::is_managed_generated_prompt_name(
            "tinyfish-full.powershell.txt"
        ));
        assert!(ProfileManager::is_managed_generated_prompt_name(
            "tinyfish-fetch-only.bash.txt"
        ));
        assert!(ProfileManager::is_managed_generated_prompt_name(
            "tinyfish-search-only.powershell.txt"
        ));
        assert!(!ProfileManager::is_managed_generated_prompt_name(
            "notes.tinyfish.txt"
        ));
        assert!(!ProfileManager::is_managed_generated_prompt_name(
            "tinyfish-full.json"
        ));
    }

    #[test]
    fn build_lightweight_settings_windows_tinyfish_allows_bash_and_powershell() {
        let settings = build_lightweight_settings(
            &LightweightEnv::default(),
            Some("sk-test"),
            Some("https://new-api.example.com"),
            TinyfishMode::Full,
            TinyfishToolShell::PowerShell,
        );
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        let allow_values: Vec<&str> = allow.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(allow_values.contains(&"Bash(tinyfish:*)"));
        assert!(allow_values.contains(&"PowerShell(tinyfish:*)"));
        assert!(settings.get("hooks").is_none());
    }

    #[test]
    fn build_lightweight_settings_unix_tinyfish_allows_only_bash() {
        let settings = build_lightweight_settings(
            &LightweightEnv::default(),
            Some("sk-test"),
            Some("https://new-api.example.com"),
            TinyfishMode::Full,
            TinyfishToolShell::Bash,
        );
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        let allow_values: Vec<&str> = allow.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(allow_values.contains(&"Bash(tinyfish:*)"));
        assert!(!allow_values.contains(&"PowerShell(tinyfish:*)"));
        assert!(settings.get("hooks").is_none());
    }

    #[test]
    fn build_lightweight_settings_native_provider_omits_tinyfish_permissions() {
        let settings = build_lightweight_settings(
            &LightweightEnv::default(),
            Some("sk-test"),
            Some("https://anyrouter.top"),
            TinyfishMode::None,
            TinyfishToolShell::PowerShell,
        );
        assert!(settings.get("permissions").is_none());
    }

    #[test]
    fn sync_local_tinyfish_artifacts_writes_shared_plugins_and_prompt() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        mgr.create_lightweight_profile(
            "proxy-one",
            Some("proxy-one"),
            LightweightEnv {
                auth_token: Some("sk-one".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
        mgr.create_lightweight_profile(
            "proxy-two",
            Some("proxy-two"),
            LightweightEnv {
                auth_token: Some("sk-two".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-opus".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let profiles = mgr.list_profiles().unwrap();

        mgr.sync_local_tinyfish_artifacts(&profiles).unwrap();

        let prompt_path =
            mgr.local_tinyfish_prompt_path(TinyfishMode::Full, native_tinyfish_tool_shell());
        assert!(prompt_path.exists());
        let plugin_path = mgr.local_tinyfish_plugin_root(TinyfishMode::Full);
        assert!(plugin_path.exists());
        assert!(
            plugin_path
                .join(".claude-plugin")
                .join("plugin.json")
                .exists()
        );
        assert!(plugin_path.join("hooks").join("hooks.json").exists());
        let manifest: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(plugin_path.join(".claude-plugin").join("plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"].as_str(), Some("tinyfish-full"));
        assert_eq!(manifest["displayName"].as_str(), Some("TinyFish Full"));
        let prompt_files: Vec<_> = fs::read_dir(mgr.generated_prompts_dir())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(prompt_files.len(), 1);
        assert_eq!(
            prompt_files[0],
            ProfileManager::tinyfish_prompt_file_name(
                TinyfishMode::Full,
                native_tinyfish_tool_shell()
            )
            .unwrap()
        );
    }

    #[test]
    fn sync_local_tinyfish_artifacts_removes_stale_managed_plugin_dirs() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let stale_plugin = mgr.generated_plugins_dir().join("tinyfish-full");
        fs::create_dir_all(stale_plugin.join("hooks")).unwrap();
        fs::write(stale_plugin.join("hooks").join("hooks.json"), "{}").unwrap();
        let unmanaged_plugin = mgr.generated_plugins_dir().join("notes");
        fs::create_dir_all(&unmanaged_plugin).unwrap();

        mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

        assert!(!stale_plugin.exists());
        assert!(unmanaged_plugin.exists());
    }

    #[test]
    fn sync_local_tinyfish_artifacts_keeps_unmanaged_tinyfish_prefixed_dirs() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let custom_plugin = mgr.generated_plugins_dir().join("tinyfish-custom");
        fs::create_dir_all(&custom_plugin).unwrap();

        mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

        assert!(custom_plugin.exists());
    }

    #[test]
    fn sync_local_tinyfish_artifacts_keeps_legacy_tinyfish_settings_files() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let legacy_dir = mgr.base_dir().join("generated").join("settings");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_file = legacy_dir.join("config.tinyfish.json");
        fs::write(&legacy_file, "{}").unwrap();

        mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

        assert!(legacy_file.exists());
    }

    #[test]
    fn generated_plugin_file_names_are_shared_by_mode_and_shell() {
        assert_eq!(
            ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::Full).as_deref(),
            Some("tinyfish-full")
        );
        assert_eq!(
            ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::FetchOnly).as_deref(),
            Some("tinyfish-fetch-only")
        );
        assert_eq!(
            ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::SearchOnly).as_deref(),
            Some("tinyfish-search-only")
        );
        assert_eq!(
            ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::None),
            None
        );
        assert!(ProfileManager::is_managed_generated_plugin_dir_name(
            "tinyfish-full"
        ));
        assert!(ProfileManager::is_managed_generated_plugin_dir_name(
            "tinyfish-fetch-only"
        ));
        assert!(ProfileManager::is_managed_generated_plugin_dir_name(
            "tinyfish-search-only"
        ));
        assert!(!ProfileManager::is_managed_generated_plugin_dir_name(
            "notes"
        ));
        assert!(!ProfileManager::is_managed_generated_plugin_dir_name(
            "tinyfish-custom"
        ));
    }

    #[test]
    fn mcp_server_crud_links_only_lightweight_profiles() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "codex-sessions".into(),
                server_type: "stdio".into(),
                command: Some("codex-sessions-mcp".into()),
                ..Default::default()
            })
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-mcp"), LightweightEnv::default())
            .unwrap();
        let full_src = make_claude_dir(&tmp.path().join("fake-claude-mcp"));
        let full = mgr
            .add_profile_from("full", Some("full-mcp"), &full_src)
            .unwrap();

        let linked = mgr
            .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
            .unwrap();
        assert_eq!(linked.mcp_server_ids, vec![server.id.clone()]);
        assert!(
            mgr.set_profile_mcps(&full.id, std::slice::from_ref(&server.id))
                .unwrap_err()
                .to_string()
                .contains("lightweight")
        );
        assert!(
            mgr.remove_mcp_server(&server.id)
                .unwrap_err()
                .to_string()
                .contains("used by profiles")
        );
        let refs = mgr.list_profiles_using_mcp(&server.id).unwrap();
        assert_eq!(refs[0].name, "lite");
    }

    #[test]
    fn mcp_plugin_generation_writes_mcp_json_and_manifest() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let mut env = HashMap::new();
        env.insert("GITHUB_TOKEN".into(), "${GITHUB_TOKEN}".into());
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "github".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
                env,
                always_load: Some(false),
                disabled: Some(false),
                ..Default::default()
            })
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-mcp-json"), LightweightEnv::default())
            .unwrap();
        let linked = mgr
            .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
            .unwrap();
        let servers = mgr.profile_mcp_servers(&linked).unwrap();
        let plugin_root = mgr
            .upsert_local_profile_mcp_plugin(&linked, &servers)
            .unwrap();
        assert!(
            plugin_root
                .join(".claude-plugin")
                .join("plugin.json")
                .exists()
        );
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(plugin_root.join(".mcp.json")).unwrap())
                .unwrap();
        let compat_config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(plugin_root.join("mcp.json")).unwrap())
                .unwrap();
        assert_eq!(config, compat_config);
        assert_eq!(
            config["$schema"].as_str(),
            Some("https://json.schemastore.org/claude-code-settings.json")
        );
        assert_eq!(
            config["mcpServers"]["github"]["command"].as_str(),
            Some("npx")
        );
        assert_eq!(
            config["mcpServers"]["github"]["env"]["GITHUB_TOKEN"].as_str(),
            Some("${GITHUB_TOKEN}")
        );
        assert_eq!(
            config["mcpServers"]["github"]["alwaysLoad"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn mcp_export_import_and_replace_round_trip() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "github".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
                always_load: Some(true),
                ..Default::default()
            })
            .unwrap();
        let exported = mgr
            .export_mcp_config(std::slice::from_ref(&server.id), false)
            .unwrap();
        let config: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(
            config["mcpServers"]["github"]["command"].as_str(),
            Some("npx")
        );

        let other_tmp = TempDir::new().unwrap();
        let other = make_manager(&other_tmp);
        let imported = other.import_mcp_config(&exported, false).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].name, "github");
        assert_eq!(
            imported[0].args,
            vec!["-y", "@modelcontextprotocol/server-github"]
        );

        let replacement = serde_json::json!({
            "mcpServers": {
                "github": {
                    "type": "stdio",
                    "command": "node",
                    "args": ["server.js"]
                }
            }
        });
        other
            .import_mcp_config(&replacement.to_string(), true)
            .unwrap();
        let updated = other.get_mcp_server("github").unwrap();
        assert_eq!(updated.command.as_deref(), Some("node"));
        assert_eq!(updated.args, vec!["server.js"]);
    }

    #[test]
    fn mcp_validate_reports_missing_runtime_command() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let command = format!("missing-cswitch-command-{}", Uuid::new_v4());
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "missing".into(),
                server_type: "stdio".into(),
                command: Some(command.clone()),
                ..Default::default()
            })
            .unwrap();
        let issues = mgr
            .validate_mcp_servers(std::slice::from_ref(&server.id), false)
            .unwrap();
        assert!(
            issues
                .iter()
                .any(|issue| issue.level == DiagnosticLevel::Warn
                    && issue.message.contains(&command))
        );
    }

    #[test]
    fn inspect_config_counts_registry_and_generated_artifacts() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "filesystem".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                ..Default::default()
            })
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-inspect"), LightweightEnv::default())
            .unwrap();
        let linked = mgr
            .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
            .unwrap();
        let servers = mgr.profile_mcp_servers(&linked).unwrap();
        mgr.upsert_local_profile_mcp_plugin(&linked, &servers)
            .unwrap();

        let inspection = mgr.inspect_config().unwrap();
        assert_eq!(inspection.profiles, 1);
        assert_eq!(inspection.lightweight_profiles, 1);
        assert_eq!(inspection.mcp_servers, 1);
        assert_eq!(inspection.linked_mcp_refs, 1);
        assert_eq!(inspection.generated_mcp_plugins, 1);
    }

    #[test]
    fn doctor_reports_stale_mcp_plugin_state() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "filesystem".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                ..Default::default()
            })
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lite-doctor"), LightweightEnv::default())
            .unwrap();
        mgr.set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
            .unwrap();
        let report = mgr.doctor_report().unwrap();
        assert!(report.items.iter().any(|item| {
            item.level == DiagnosticLevel::Warn
                && item.area == "mcp"
                && item.message.contains("artifacts have not been generated")
        }));
    }

    #[test]
    fn resolve_project_profile_reads_parent_marker() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let profile = mgr
            .create_lightweight_profile("project profile", Some("proj"), LightweightEnv::default())
            .unwrap();
        let project_dir = tmp.path().join("project");
        let nested_dir = project_dir.join("src").join("bin");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(project_dir.join(".cswitch-profile"), "proj\n").unwrap();

        let selected = mgr
            .resolve_project_profile(&nested_dir)
            .unwrap()
            .expect("marker should select profile");
        assert_eq!(selected.id, profile.id);
        assert!(mgr.resolve_project_profile(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn statusline_info_reports_profile_provider_and_mcps() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider = mgr
            .add_provider_with_key_name(
                "OpenRouter",
                "https://openrouter.example.invalid/api",
                "Team",
                "sk-test",
            )
            .unwrap();
        let key_id = provider.keys.keys().next().cloned().unwrap();
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "filesystem".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                ..Default::default()
            })
            .unwrap();
        let profile = mgr
            .create_lightweight_profile("work", Some("wrk"), LightweightEnv::default())
            .unwrap();
        mgr.set_provider(&profile.id, &provider.id, &key_id)
            .unwrap();
        mgr.set_profile_mcps(&profile.id, std::slice::from_ref(&server.id))
            .unwrap();

        let info = mgr.statusline_info(Some("wrk"), None).unwrap();
        assert_eq!(info.profile_name.as_deref(), Some("work"));
        assert_eq!(info.profile_alias.as_deref(), Some("wrk"));
        assert_eq!(info.provider_name.as_deref(), Some("OpenRouter"));
        assert_eq!(info.key_name.as_deref(), Some("Team"));
        assert_eq!(info.mcp_names, vec!["filesystem"]);
        assert!(!info.project_marker);

        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join(".cswitch-profile"), "wrk\n").unwrap();
        let project_info = mgr.statusline_info(None, Some(&project_dir)).unwrap();
        assert_eq!(project_info.profile_name.as_deref(), Some("work"));
        assert!(project_info.project_marker);
    }

    #[test]
    fn config_bundle_export_redacts_secrets_and_imports_with_replace() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider = mgr
            .add_provider_with_key_name(
                "Provider",
                "https://provider.example.invalid",
                "Default",
                "secret-key",
            )
            .unwrap();
        let key_id = provider.keys.keys().next().cloned().unwrap();
        let mut mcp_env = HashMap::new();
        mcp_env.insert("GITHUB_TOKEN".into(), "ghp-secret-token".into());
        mcp_env.insert("TOKENIZERS_PARALLELISM".into(), "false".into());
        let mut mcp_headers = HashMap::new();
        mcp_headers.insert("Authorization".into(), "Bearer mcp-header-secret".into());
        mcp_headers.insert("X-Mode".into(), "portable".into());
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "github".into(),
                server_type: "stdio".into(),
                command: Some("npx".into()),
                env: mcp_env,
                headers: mcp_headers,
                oauth: Some(serde_json::json!({
                    "clientId": "client-id",
                    "clientSecret": "oauth-secret",
                    "scopes": ["read", "write"]
                })),
                ..Default::default()
            })
            .unwrap();
        let profile = mgr
            .create_lightweight_profile(
                "bundle",
                Some("bun"),
                LightweightEnv {
                    auth_token: Some("profile-secret".into()),
                    base_url: Some("https://provider.example.invalid".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        mgr.set_provider(&profile.id, &provider.id, &key_id)
            .unwrap();
        mgr.set_profile_mcps(&profile.id, std::slice::from_ref(&server.id))
            .unwrap();

        let redacted = mgr.export_config_bundle(&[], false).unwrap();
        assert!(!redacted.contains("secret-key"));
        assert!(!redacted.contains("profile-secret"));
        assert!(!redacted.contains("ghp-secret-token"));
        assert!(!redacted.contains("mcp-header-secret"));
        assert!(!redacted.contains("oauth-secret"));
        let bundle: ConfigBundle = serde_json::from_str(&redacted).unwrap();
        assert!(!bundle.secrets_included);
        assert_eq!(bundle.profiles.len(), 1);
        assert_eq!(
            bundle.providers[0].keys.values().next().unwrap().api_key,
            ""
        );
        assert_eq!(bundle.mcp_servers[0].env["GITHUB_TOKEN"], "");
        assert_eq!(bundle.mcp_servers[0].env["TOKENIZERS_PARALLELISM"], "false");
        assert_eq!(bundle.mcp_servers[0].headers["Authorization"], "");
        assert_eq!(bundle.mcp_servers[0].headers["X-Mode"], "portable");
        assert_eq!(
            bundle.mcp_servers[0].oauth.as_ref().unwrap()["clientSecret"].as_str(),
            Some("")
        );
        assert_eq!(
            bundle.mcp_servers[0].oauth.as_ref().unwrap()["clientId"].as_str(),
            Some("client-id")
        );

        let with_secrets = mgr.export_config_bundle(&[], true).unwrap();
        assert!(with_secrets.contains("secret-key"));
        assert!(with_secrets.contains("ghp-secret-token"));
        assert!(with_secrets.contains("mcp-header-secret"));
        assert!(with_secrets.contains("oauth-secret"));

        let other_tmp = TempDir::new().unwrap();
        let other = make_manager(&other_tmp);
        let plan = other
            .plan_config_bundle_import(&with_secrets, false)
            .unwrap();
        assert_eq!(plan.summary.profiles_added, 1);
        assert_eq!(plan.summary.providers_added, 1);
        assert_eq!(plan.summary.mcp_servers_added, 1);
        assert_eq!(plan.profiles_add.len(), 1);
        assert!(plan.profiles_update.is_empty());
        assert!(other.list_profiles().unwrap().is_empty());

        let summary = other.import_config_bundle(&with_secrets, false).unwrap();
        assert_eq!(summary.profiles_added, 1);
        assert_eq!(summary.providers_added, 1);
        assert_eq!(summary.mcp_servers_added, 1);
        assert_eq!(other.list_profiles().unwrap()[0].name, "bundle");

        let plan = other
            .plan_config_bundle_import(&with_secrets, true)
            .unwrap();
        assert_eq!(plan.summary.profiles_updated, 1);
        assert_eq!(plan.summary.providers_updated, 1);
        assert_eq!(plan.summary.mcp_servers_updated, 1);
        assert!(plan.profiles_add.is_empty());
        assert_eq!(plan.profiles_update.len(), 1);

        let conflict_plan = other
            .plan_config_bundle_import(&with_secrets, false)
            .unwrap();
        assert_eq!(conflict_plan.conflict_count(), 3);
        assert_eq!(conflict_plan.summary.profiles_conflicted, 1);
        assert_eq!(conflict_plan.summary.providers_conflicted, 1);
        assert_eq!(conflict_plan.summary.mcp_servers_conflicted, 1);
        assert!(
            other
                .import_config_bundle(&with_secrets, false)
                .unwrap_err()
                .to_string()
                .contains("Use --replace")
        );

        let summary = other.import_config_bundle(&with_secrets, true).unwrap();
        assert_eq!(summary.profiles_updated, 1);
        assert_eq!(summary.providers_updated, 1);
        assert_eq!(summary.mcp_servers_updated, 1);

        let redacted_from_other = other.export_config_bundle(&[], false).unwrap();
        other
            .import_config_bundle(&redacted_from_other, true)
            .unwrap();
        let preserved_provider = other.get_provider(&provider.id).unwrap();
        assert_eq!(
            preserved_provider
                .keys
                .get(&key_id)
                .map(|key| key.api_key.as_str()),
            Some("secret-key")
        );
        let preserved_mcp = other.get_mcp_server(&server.id).unwrap();
        assert_eq!(
            preserved_mcp.env.get("GITHUB_TOKEN").map(String::as_str),
            Some("ghp-secret-token")
        );
        assert_eq!(
            preserved_mcp
                .env
                .get("TOKENIZERS_PARALLELISM")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            preserved_mcp
                .headers
                .get("Authorization")
                .map(String::as_str),
            Some("Bearer mcp-header-secret")
        );
        assert_eq!(
            preserved_mcp.headers.get("X-Mode").map(String::as_str),
            Some("portable")
        );
        assert_eq!(
            preserved_mcp.oauth.as_ref().unwrap()["clientSecret"].as_str(),
            Some("oauth-secret")
        );

        let scoped = mgr
            .export_config_bundle(std::slice::from_ref(&profile.id), false)
            .unwrap();
        let scoped_bundle: ConfigBundle = serde_json::from_str(&scoped).unwrap();
        assert_eq!(scoped_bundle.profiles.len(), 1);
        assert_eq!(scoped_bundle.providers.len(), 1);
        assert_eq!(scoped_bundle.mcp_servers.len(), 1);
        assert_eq!(scoped_bundle.profiles[0].id, profile.id);
        assert_eq!(scoped_bundle.providers[0].keys.len(), 1);
        assert!(scoped_bundle.providers[0].keys.contains_key(&key_id));

        let validation = mgr.validate_config_bundle(&scoped).unwrap();
        assert_eq!(validation.profiles, 1);
        assert_eq!(validation.error_count(), 0);
    }

    #[test]
    fn scoped_config_export_includes_only_selected_provider_keys() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let provider = mgr
            .add_provider_with_key_name(
                "Shared",
                "https://shared.example.invalid",
                "Team A",
                "secret-a",
            )
            .unwrap();
        let key_a = provider.keys.keys().next().cloned().unwrap();
        let key_b = mgr.add_key(&provider.id, "Team B", "secret-b").unwrap().id;
        let first = mgr
            .create_lightweight_profile("first", Some("first"), LightweightEnv::default())
            .unwrap();
        let second = mgr
            .create_lightweight_profile("second", Some("second"), LightweightEnv::default())
            .unwrap();
        mgr.set_provider(&first.id, &provider.id, &key_a).unwrap();
        mgr.set_provider(&second.id, &provider.id, &key_b).unwrap();

        let scoped = mgr
            .export_config_bundle(std::slice::from_ref(&first.id), true)
            .unwrap();
        assert!(scoped.contains("secret-a"));
        assert!(!scoped.contains("secret-b"));
        let bundle: ConfigBundle = serde_json::from_str(&scoped).unwrap();
        assert_eq!(bundle.providers.len(), 1);
        assert_eq!(bundle.providers[0].keys.len(), 1);
        assert!(bundle.providers[0].keys.contains_key(&key_a));
        assert!(!bundle.providers[0].keys.contains_key(&key_b));
    }

    #[test]
    fn config_import_rejects_profiles_with_missing_provider_references() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let bundle = ConfigBundle {
            schema: "https://github.com/m2selfA/claude-switch/config-bundle/v1".into(),
            exported_at: Utc::now(),
            profiles: vec![Profile {
                id: Uuid::new_v4().to_string(),
                name: "broken".into(),
                alias: Some("broken".into()),
                added: Utc::now(),
                last_used: None,
                kind: ProfileKind::Lightweight,
                env: Some(LightweightEnv::default()),
                launch_args: None,
                provider_id: Some("missing-provider".into()),
                key_id: Some("missing-key".into()),
                mcp_server_ids: Vec::new(),
            }],
            providers: Vec::new(),
            mcp_servers: Vec::new(),
            secrets_included: true,
        };
        let content = serde_json::to_string(&bundle).unwrap();

        let err = mgr
            .plan_config_bundle_import(&content, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing provider"), "{err}");
        let err = mgr
            .import_config_bundle(&content, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing provider"), "{err}");
        assert!(mgr.load_registry().unwrap().profiles.is_empty());
    }

    #[test]
    fn generated_launchers_include_mcp_plugin_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let server = mgr
            .add_mcp_server(McpServerInput {
                name: "codex-sessions".into(),
                server_type: "stdio".into(),
                command: Some("codex-sessions-mcp".into()),
                ..Default::default()
            })
            .unwrap();
        let lite = mgr
            .create_lightweight_profile("lite", Some("lmcp"), LightweightEnv::default())
            .unwrap();
        let linked = mgr
            .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
            .unwrap();

        let cmd = mgr.generate_cmd_content(&linked).unwrap();
        assert!(
            cmd.contains("%USERPROFILE%\\.claude-switch\\generated\\mcps\\cswitch-mcp-profile-")
        );
        assert!(cmd.contains("--plugin-dir \"%_MCP_PLUGIN_DIR%\""));

        let sh = mgr.generate_sh_content(&linked).unwrap();
        assert!(sh.contains("$HOME/.claude-switch/generated/mcps/cswitch-mcp-profile-"));
        assert!(sh.contains("MCP_PLUGIN_ARGS=(--plugin-dir"));
    }

    #[test]
    fn generate_cmd_content_uses_plugin_dir_and_inline_tf_settings() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://new-api.example.com".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_cmd_content(&lite).unwrap();
        assert!(content.contains("setlocal EnableExtensions DisableDelayedExpansion"));
        assert!(content.contains("goto build_settings"));
        assert!(content.contains(":build_settings"));
        assert!(content.contains("if defined _TF goto launch_with_hooks_plain"));
        assert!(content.contains("set \"_SETTINGS={\\\"env\\\":{"));
        assert!(content.contains("set \"_TF_SETTINGS={\\\"env\\\":{"));
        assert!(content.contains("set \"_TF=\""));
        assert!(content.contains("set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-full\""));
        assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-full.powershell.txt\""));
        assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
        assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
        assert!(!content.contains("SubagentStart"));
        assert!(!content.contains("PreToolUse"));
        assert!(content.contains("PowerShell(tinyfish:*)"));
        assert_eq!(content.matches("\\\"ANTHROPIC_AUTH_TOKEN\\\"").count(), 2);
        assert!(content.contains("--settings \"%_SETTINGS%\""));
        assert!(!content.contains("_TF_SETTINGS_FILE="));
    }

    #[test]
    fn generate_cmd_content_assigns_parseable_json_settings() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp"),
                LightweightEnv {
                    auth_token: Some("sk-test!bang%20caret^value".into()),
                    base_url: Some("https://new-api.example.com/path!section/%5E/^v2".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_cmd_content(&lite).unwrap();

        for var_name in ["_SETTINGS", "_TF_SETTINGS"] {
            let json = unescape_generated_cmd_set_value(cmd_set_value(&content, var_name));
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(
                parsed["env"]["ANTHROPIC_AUTH_TOKEN"].as_str(),
                Some("sk-test!bang%20caret^value")
            );
            assert_eq!(
                parsed["env"]["ANTHROPIC_BASE_URL"].as_str(),
                Some("https://new-api.example.com/path!section/%5E/^v2")
            );
            assert!(!json.contains("^!"));
        }
        assert!(!content.contains("call claude --settings"));
    }

    #[test]
    fn generate_cmd_content_includes_tf_prompt() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://new-api.example.com".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_cmd_content(&lite).unwrap();
        assert!(content.contains("where tinyfish >nul 2>&1 && set \"_TF=1\""));
        assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-full.powershell.txt\""));
        assert!(content.contains(
            "set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-full\""
        ));
        assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
        assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
        assert!(!content.contains("--append-system-prompt \"%_TF_PROMPT%\""));
        assert!(!content.contains("rate limited by tinyfish"));
        assert!(!content.contains("run via Bash"));
    }

    #[test]
    fn generate_sh_content_switches_between_base_and_hook_settings() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://new-api.example.com".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_sh_content(&lite).unwrap();
        assert!(content.contains("command -v tinyfish"));
        assert!(content.contains("TF_SP_ARGS=("));
        assert!(content.contains("SETTINGS_ENV="));
        assert!(content.contains("BASE_SETTINGS="));
        assert!(content.contains("TF_SETTINGS="));
        assert!(content.contains("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")"));
        assert!(
            content
                .contains("TF_PLUGIN_DIR=\"$HOME/.claude-switch/generated/plugins/tinyfish-full\"")
        );
        assert!(content.contains(
            "TF_PROMPT_FILE=\"$HOME/.claude-switch/generated/prompts/tinyfish-full.bash.txt\""
        ));
        assert!(content.contains("TF_PLUGIN_ARGS=(--plugin-dir \"$TF_PLUGIN_DIR\")"));
        assert!(content.contains("TF_SP_ARGS=(--append-system-prompt-file \"$TF_PROMPT_FILE\")"));
        assert!(content.contains("SETTINGS_ARG=(--settings \"$TF_SETTINGS\")"));
        assert!(content.contains("BASE_SETTINGS=\"${SETTINGS_ENV}\""));
        assert!(!content.contains("HOOK_SETTINGS="));
        assert!(content.contains("Bash(tinyfish:*)"));
        assert_eq!(content.matches("\"ANTHROPIC_AUTH_TOKEN\"").count(), 1);
        assert!(!content.contains("run via Bash"));
        assert!(!content.contains("PowerShell tool"));

        let settings_env_line = find_line(&content, "SETTINGS_ENV=");
        let settings_env = unquote_single_quoted_shell_literal(
            settings_env_line.trim_start_matches("SETTINGS_ENV="),
        );
        let base_settings_line = find_line(&content, "BASE_SETTINGS=");
        let base_tail = unquote_single_quoted_shell_literal(
            base_settings_line.trim_start_matches("BASE_SETTINGS=\"${SETTINGS_ENV}\""),
        );
        let tf_settings_line = find_line(&content, "TF_SETTINGS=");
        let tf_tail = unquote_single_quoted_shell_literal(
            tf_settings_line.trim_start_matches("TF_SETTINGS=\"${SETTINGS_ENV}\""),
        );

        let base_settings_json = format!("{settings_env}{base_tail}");
        let tf_settings_json = format!("{settings_env}{tf_tail}");
        let base_json: serde_json::Value = serde_json::from_str(&base_settings_json).unwrap();
        let tf_json: serde_json::Value = serde_json::from_str(&tf_settings_json).unwrap();
        assert!(base_json.get("permissions").is_none());
        let allow = tf_json["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow.len(), 1);
        assert_eq!(allow[0].as_str(), Some("Bash(tinyfish:*)"));
    }

    #[test]
    fn generate_cmd_content_deepseek_fetch_only_prompt() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "ds-prof",
                Some("ds"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://api.deepseek.com/anthropic".into()),
                    model: Some("deepseek-v4".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_cmd_content(&lite).unwrap();
        assert!(content.contains(
            "set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-fetch-only\""
        ));
        assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-fetch-only.powershell.txt\""));
        assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
        assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
        assert!(!content.contains("WebFetch"));
        assert!(!content.contains("WebSearch"));
    }

    #[test]
    fn generate_cmd_content_native_provider_skips_tinyfish() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "native-prof",
                Some("native"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://anyrouter.top".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let content = mgr.generate_cmd_content(&lite).unwrap();
        assert!(!content.contains("_TF_PLUGIN_DIR="));
        assert!(!content.contains("_TF_PROMPT_FILE="));
        assert!(!content.contains("tinyfish:*)"));
    }

    #[test]
    fn generate_cmd_content_respects_no_extras_when_tinyfish_missing() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let mut lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp-noextras"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://new-api.example.com".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        lite.launch_args = Some(vec!["--dangerously-skip-permissions".into()]);

        let content = mgr.generate_cmd_content(&lite).unwrap();

        assert!(content.contains("if defined _TF if defined _E goto launch_with_hooks_extras"));
        assert!(content.contains("if defined _TF goto launch_with_hooks_plain"));
        assert!(content.contains("if defined _E goto launch_with_extras"));
    }

    #[cfg(windows)]
    #[test]
    fn generated_cmd_subagent_hook_does_not_trigger_cmd_parse_error() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let lite = mgr
            .create_lightweight_profile(
                "proxy-prof",
                Some("pp-win-cmd"),
                LightweightEnv {
                    auth_token: Some("sk-test".into()),
                    base_url: Some("https://new-api.example.com".into()),
                    model: Some("claude-sonnet".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut content = mgr.generate_cmd_content(&lite).unwrap();
        content = content.replace(
            "claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\" %_LAUNCH_ARGS% %_R%",
            "echo launched hooks extras",
        );
        content = content.replace(
            "claude --settings \"%_SETTINGS%\" %_LAUNCH_ARGS% %_R%",
            "echo launched extras",
        );
        content = content.replace(
            "claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\" %_R%",
            "echo launched hooks plain",
        );
        content = content.replace(
            "claude --settings \"%_SETTINGS%\" %_R%",
            "echo launched plain",
        );

        let shim_path = tmp.path().join("claude-cstcloud.cmd");
        fs::write(&shim_path, content).unwrap();

        let output = std::process::Command::new("cmd")
            .args(["/c", shim_path.to_string_lossy().as_ref(), "--help"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");

        assert!(output.status.success());
        assert!(combined.contains("launched"));
        assert!(!combined.contains("The system cannot find the file specified."));
    }

    #[test]
    fn remove_remote_plugin_dir_reports_runner_errors() {
        let result = ProfileManager::remove_remote_plugin_dir_with_runner(
            "host",
            "/tmp/tinyfish-full",
            RemoteOs::Unix,
            |_| anyhow::bail!("permission denied"),
        );
        assert!(result.is_err());
    }
}
