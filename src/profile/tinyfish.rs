use anyhow::{Context, Result};
use std::time::Duration;

use super::LightweightEnv;
use super::url_match::{NATIVE_FETCH_URLS, NATIVE_SEARCH_URLS, url_matches};

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

const TINYFISH_WEBSEARCH_PRETOOL_CONTEXT: &str =
    "Follow the instructions about which search provider to use, listed in your Claude.md file";
const TINYFISH_WEBFETCH_PRETOOL_CONTEXT: &str =
    "Follow the instructions about which fetch provider to use, listed in your Claude.md file";
const TINYFISH_CONTROL_EXTRA_KEY: &str = "CLAUDE_SWITCH_TINYFISH";
const TINYFISH_BASH_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)"];
const TINYFISH_WINDOWS_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)", "PowerShell(tinyfish:*)"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TinyfishToolShell {
    Bash,
    PowerShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TinyfishMode {
    None,
    SearchOnly,
    FetchOnly,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LightweightRuntimeArtifacts {
    pub(super) base_settings_json: String,
    pub(super) tinyfish_mode: TinyfishMode,
    pub(super) tinyfish_settings_json: Option<String>,
    pub(super) tinyfish_prompt_text: Option<String>,
    pub(super) tinyfish_plugin_hooks_json: Option<String>,
    pub(super) tinyfish_plugin_manifest_json: Option<String>,
}

pub(super) fn tinyfish_full_hooks(tool_shell: TinyfishToolShell) -> serde_json::Value {
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

pub(super) fn tinyfish_fetch_only_hooks(tool_shell: TinyfishToolShell) -> serde_json::Value {
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

pub(super) fn tinyfish_mode(base_url: &str) -> TinyfishMode {
    if base_url.trim().is_empty() {
        return TinyfishMode::None;
    }
    let has_search = url_matches(base_url, NATIVE_SEARCH_URLS);
    let has_fetch = url_matches(base_url, NATIVE_FETCH_URLS);
    tinyfish_mode_for_capabilities(has_search, has_fetch)
}

pub(super) fn tinyfish_mode_for_capabilities(has_search: bool, has_fetch: bool) -> TinyfishMode {
    match (has_search, has_fetch) {
        (true, true) => TinyfishMode::None,
        (true, false) => TinyfishMode::FetchOnly,
        (false, true) => TinyfishMode::SearchOnly,
        (false, false) => TinyfishMode::Full,
    }
}

pub(super) fn native_tinyfish_tool_shell() -> TinyfishToolShell {
    if cfg!(windows) {
        TinyfishToolShell::PowerShell
    } else {
        TinyfishToolShell::Bash
    }
}

pub(super) fn tinyfish_prompt(
    mode: TinyfishMode,
    tool_shell: TinyfishToolShell,
) -> Option<&'static str> {
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

pub(super) fn tinyfish_permissions_allowlist(
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

pub(super) fn tinyfish_hook_command(
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

pub(super) fn tinyfish_command_succeeds_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> bool {
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

pub(super) fn tinyfish_available() -> bool {
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

pub(super) fn build_lightweight_settings(
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

pub(super) fn build_lightweight_settings_env_prefix(
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

pub(super) fn tinyfish_plugin_hooks(
    mode: TinyfishMode,
    tool_shell: TinyfishToolShell,
) -> Option<String> {
    match mode {
        TinyfishMode::None => None,
        TinyfishMode::SearchOnly => Some(tinyfish_search_only_hooks(tool_shell).to_string()),
        TinyfishMode::FetchOnly => Some(tinyfish_fetch_only_hooks(tool_shell).to_string()),
        TinyfishMode::Full => Some(tinyfish_full_hooks(tool_shell).to_string()),
    }
}

pub(super) fn tinyfish_plugin_manifest(mode: TinyfishMode) -> Option<String> {
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

pub(super) fn build_lightweight_runtime_artifacts(
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
