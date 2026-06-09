use anyhow::{Context, Result, bail};
use std::time::Duration;

use super::url_match::{
    ANYROUTER_URLS, NATIVE_FETCH_URLS, NATIVE_SEARCH_URLS, is_local_runtime_base_url, url_matches,
};
use super::{LightweightEnv, LocalGatewayToolMode, ProfileManager};

const TINYFISH_CONTROL_EXTRA_KEY: &str = "CLAUDE_SWITCH_TINYFISH";
const TINYFISH_CONTROL_MODE_ENV: &str = "CLAUDE_SWITCH_TINYFISH_MODE";
const TINYFISH_BASH_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)"];
const TINYFISH_WINDOWS_ALLOWLIST: &[&str] = &["Bash(tinyfish:*)", "PowerShell(tinyfish:*)"];
const TINYFISH_ROUTER_PLUGIN_ROOT_NAME: &str = "tinyfish-router";
const TINYFISH_FULL_PLUGIN_ROOT_NAME: &str = "tinyfish-full";
const TINYFISH_FETCH_ONLY_PLUGIN_ROOT_NAME: &str = "tinyfish-fetch-only";
const TINYFISH_BASH_HOOK_SCRIPT_NAME: &str = "hook-router.sh";
const TINYFISH_BASH_STATUSLINE_SCRIPT_NAME: &str = "statusline.sh";
const TINYFISH_POWERSHELL_HOOK_SCRIPT_NAME: &str = "hook-router.ps1";
const TINYFISH_POWERSHELL_STATUSLINE_SCRIPT_NAME: &str = "statusline.ps1";

const TINYFISH_BASH_HOOK_SCRIPT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

event_name="${1:-}"
tool_name="${2:-}"
payload="$(cat)"
single_line="$(printf '%s' "$payload" | tr -d '\r\n')"

extract_string_field() {
    local key="$1"
    printf '%s' "$single_line" | sed -n "s/.*\"${key}\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"
}

to_lower() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

strip_1m_suffix() {
    local value="${1%[[:space:]]}"
    if [[ "$value" == *"[1m]" ]]; then
        value="${value%\[1m\]}"
    fi
    printf '%s' "$value"
}

model_family() {
    local value
    value="$(to_lower "$(strip_1m_suffix "$1")")"
    case "$value" in
        claude*) printf 'claude' ;;
        deepseek*) printf 'deepseek' ;;
        *) printf 'unknown' ;;
    esac
}

provider_capabilities() {
    local value
    value="$(to_lower "$1")"
    case "$value" in
        *"://api.deepseek.com/anthropic"*)
            printf '1 0\n'
            ;;
        *"://api.anthropic.com"*|*"://anyrouter.top"*|*"://a-ocnfniawgw.cn-shanghai.fcapp.run"*)
            printf '1 1\n'
            ;;
        "")
            printf '1 1\n'
            ;;
        *)
            printf '0 0\n'
            ;;
    esac
}

control_mode_routes() {
    case "$(to_lower "${CLAUDE_SWITCH_TINYFISH_MODE:-}")" in
        search-fetch) printf 'tinyfish tinyfish\n' ;;
        fetch-only) printf 'native tinyfish\n' ;;
        gateway-only) printf 'native native\n' ;;
        *) return 1 ;;
    esac
}

compute_routes() {
    local base_url="$1"
    local model="$2"
    local override_search override_fetch
    if read -r override_search override_fetch < <(control_mode_routes); then
        printf '%s %s\n' "$override_search" "$override_fetch"
        return
    fi
    local desired_search desired_fetch
    case "$(model_family "$model")" in
        claude)
            desired_search="native"
            desired_fetch="native"
            ;;
        deepseek)
            desired_search="native"
            desired_fetch="tinyfish"
            ;;
        *)
            desired_search="tinyfish"
            desired_fetch="tinyfish"
            ;;
    esac

    local cap_search cap_fetch search_route fetch_route
    read -r cap_search cap_fetch < <(provider_capabilities "$base_url")
    search_route="tinyfish"
    fetch_route="tinyfish"
    if [[ "$desired_search" == "native" && "$cap_search" == "1" ]]; then
        search_route="native"
    fi
    if [[ "$desired_fetch" == "native" && "$cap_fetch" == "1" ]]; then
        fetch_route="native"
    fi
    printf '%s %s\n' "$search_route" "$fetch_route"
}

state_dir="${CLAUDE_PLUGIN_DATA:-${TMPDIR:-/tmp}/tinyfish-router}"
mkdir -p "$state_dir"
session_id="$(extract_string_field session_id)"
state_file="$state_dir/session-${session_id:-unknown}.env"

write_state() {
    local search_route="$1"
    local fetch_route="$2"
    {
        printf 'search_route=%s\n' "$search_route"
        printf 'fetch_route=%s\n' "$fetch_route"
    } > "$state_file"
}

emit_hook_json() {
    local hook_event="$1"
    local decision="$2"
    local context="$3"

    printf '{"hookSpecificOutput":{"hookEventName":"%s"' "$hook_event"
    if [[ -n "$decision" ]]; then
        printf ',"permissionDecision":"%s"' "$decision"
    fi
    printf ',"additionalContext":"%s"}}\n' "$context"
}

status_context() {
    local search_route="$1"
    local fetch_route="$2"
    printf 'TinyFish router active: search=%s fetch=%s.' "$search_route" "$fetch_route"
}

tool_context() {
    local route_kind="$1"
    case "$route_kind" in
        search)
            printf 'Use tinyfish search query "<query>" via the Bash tool instead of WebSearch. Native WebSearch is only a fallback when TinyFish is unavailable or rate-limited.'
            ;;
        fetch)
            printf 'Use tinyfish fetch content get "<url>" via the Bash tool instead of WebFetch. Native WebFetch is only a fallback when TinyFish is unavailable or rate-limited.'
            ;;
    esac
}

subagent_context() {
    local search_route="$1"
    local fetch_route="$2"
    if [[ "$search_route" == "tinyfish" && "$fetch_route" == "tinyfish" ]]; then
        printf 'Subagents should use tinyfish search query "<query>" for search and tinyfish fetch content get "<url>" for fetch via the Bash tool. Native web tools are fallbacks only.'
    elif [[ "$fetch_route" == "tinyfish" ]]; then
        printf 'Subagents may use native WebSearch, but WebFetch should use tinyfish fetch content get "<url>" via the Bash tool. Native fetch is fallback only.'
    elif [[ "$search_route" == "tinyfish" ]]; then
        printf 'Subagents should use tinyfish search query "<query>" via the Bash tool for search. Native WebFetch may stay native.'
    else
        printf 'Subagent web routing is currently native. If a hook later blocks a native web tool, immediately switch to the TinyFish replacement command.'
    fi
}

read -r current_search_route current_fetch_route < <(compute_routes "${ANTHROPIC_BASE_URL:-}" "${ANTHROPIC_MODEL:-}")

case "$event_name" in
    SessionStart)
        startup_model="$(extract_string_field model)"
        read -r current_search_route current_fetch_route < <(compute_routes "${ANTHROPIC_BASE_URL:-}" "$startup_model")
        write_state "$current_search_route" "$current_fetch_route"
        if [[ -n "${CLAUDE_ENV_FILE:-}" ]]; then
            {
                printf 'export CLAUDE_SWITCH_TINYFISH_STATE_FILE=%q\n' "$state_file"
                printf 'export CLAUDE_SWITCH_TINYFISH_STARTUP_MODEL=%q\n' "$startup_model"
            } >> "$CLAUDE_ENV_FILE"
        fi
        emit_hook_json "$event_name" "" "$(status_context "$current_search_route" "$current_fetch_route")"
        ;;
    PreToolUse)
        if [[ -f "$state_file" ]]; then
            # State files are agent-owned and contain only route keys with safe values.
            # shellcheck disable=SC1090
            . "$state_file"
            current_search_route="${search_route:-$current_search_route}"
            current_fetch_route="${fetch_route:-$current_fetch_route}"
        fi
        case "$tool_name" in
            WebSearch)
                if [[ "$current_search_route" == "tinyfish" ]]; then
                    emit_hook_json "$event_name" "deny" "$(tool_context search)"
                fi
                ;;
            WebFetch)
                if [[ "$current_fetch_route" == "tinyfish" ]]; then
                    emit_hook_json "$event_name" "deny" "$(tool_context fetch)"
                fi
                ;;
        esac
        ;;
    SubagentStart)
        read -r current_search_route current_fetch_route < <(compute_routes "${ANTHROPIC_BASE_URL:-}" "${CLAUDE_CODE_SUBAGENT_MODEL:-}")
        emit_hook_json "$event_name" "" "$(subagent_context "$current_search_route" "$current_fetch_route")"
        ;;
esac
"#;

const TINYFISH_BASH_STATUSLINE_SCRIPT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

payload="$(cat)"
single_line="$(printf '%s' "$payload" | tr -d '\r\n')"

extract_string_field() {
    local key="$1"
    printf '%s' "$single_line" | sed -n "s/.*\"${key}\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p"
}

extract_model_id() {
    printf '%s' "$single_line" | sed -n 's/.*"model"[[:space:]]*:[[:space:]]*{[^}]*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}

to_lower() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

strip_1m_suffix() {
    local value="${1%[[:space:]]}"
    if [[ "$value" == *"[1m]" ]]; then
        value="${value%\[1m\]}"
    fi
    printf '%s' "$value"
}

model_family() {
    local value
    value="$(to_lower "$(strip_1m_suffix "$1")")"
    case "$value" in
        claude*) printf 'claude' ;;
        deepseek*) printf 'deepseek' ;;
        *) printf 'unknown' ;;
    esac
}

provider_capabilities() {
    local value
    value="$(to_lower "$1")"
    case "$value" in
        *"://api.deepseek.com/anthropic"*)
            printf '1 0\n'
            ;;
        *"://api.anthropic.com"*|*"://anyrouter.top"*|*"://a-ocnfniawgw.cn-shanghai.fcapp.run"*)
            printf '1 1\n'
            ;;
        "")
            printf '1 1\n'
            ;;
        *)
            printf '0 0\n'
            ;;
    esac
}

control_mode_routes() {
    case "$(to_lower "${CLAUDE_SWITCH_TINYFISH_MODE:-}")" in
        search-fetch) printf 'tinyfish tinyfish\n' ;;
        fetch-only) printf 'native tinyfish\n' ;;
        gateway-only) printf 'native native\n' ;;
        *) return 1 ;;
    esac
}

compute_routes() {
    local base_url="$1"
    local model="$2"
    local override_search override_fetch
    if read -r override_search override_fetch < <(control_mode_routes); then
        printf '%s %s\n' "$override_search" "$override_fetch"
        return
    fi
    local desired_search desired_fetch
    case "$(model_family "$model")" in
        claude)
            desired_search="native"
            desired_fetch="native"
            ;;
        deepseek)
            desired_search="native"
            desired_fetch="tinyfish"
            ;;
        *)
            desired_search="tinyfish"
            desired_fetch="tinyfish"
            ;;
    esac

    local cap_search cap_fetch search_route fetch_route
    read -r cap_search cap_fetch < <(provider_capabilities "$base_url")
    search_route="tinyfish"
    fetch_route="tinyfish"
    if [[ "$desired_search" == "native" && "$cap_search" == "1" ]]; then
        search_route="native"
    fi
    if [[ "$desired_fetch" == "native" && "$cap_fetch" == "1" ]]; then
        fetch_route="native"
    fi
    printf '%s %s\n' "$search_route" "$fetch_route"
}

session_id="$(extract_string_field session_id)"
model_id="$(extract_model_id)"
if [[ -z "$model_id" ]]; then
    model_id="${ANTHROPIC_MODEL:-}"
fi
read -r search_route fetch_route < <(compute_routes "${ANTHROPIC_BASE_URL:-}" "$model_id")

state_dir="${CLAUDE_PLUGIN_DATA:-${TMPDIR:-/tmp}/tinyfish-router}"
mkdir -p "$state_dir"
state_file="$state_dir/session-${session_id:-unknown}.env"
{
    printf 'search_route=%s\n' "$search_route"
    printf 'fetch_route=%s\n' "$fetch_route"
} > "$state_file"

short_model="$(strip_1m_suffix "$model_id")"
if [[ -z "$short_model" ]]; then
    short_model="unknown"
fi
printf 'tf s=%s f=%s m=%s\n' "$search_route" "$fetch_route" "$short_model"
"#;

const TINYFISH_POWERSHELL_HOOK_SCRIPT: &str = r#"
param(
    [string]$EventName = "",
    [string]$ToolName = ""
)

$raw = [Console]::In.ReadToEnd()
$payload = $null
if (-not [string]::IsNullOrWhiteSpace($raw)) {
    try {
        $payload = $raw | ConvertFrom-Json -Depth 20
    } catch {
        $payload = $null
    }
}

function Strip-1mSuffix([string]$Model) {
    if ([string]::IsNullOrWhiteSpace($Model)) {
        return ""
    }
    $trimmed = $Model.Trim()
    if ($trimmed.EndsWith("[1m]")) {
        return $trimmed.Substring(0, $trimmed.Length - 4).TrimEnd()
    }
    return $trimmed
}

function Get-ModelFamily([string]$Model) {
    $lower = Strip-1mSuffix($Model).ToLowerInvariant()
    if ($lower.StartsWith("claude")) {
        return "claude"
    }
    if ($lower.StartsWith("deepseek")) {
        return "deepseek"
    }
    return "unknown"
}

function Get-ProviderCapabilities([string]$BaseUrl) {
    $lower = ($BaseUrl ?? "").ToLowerInvariant()
    if ($lower -like "*://api.deepseek.com/anthropic*") {
        return @{ Search = $true; Fetch = $false }
    }
    if ($lower -like "*://api.anthropic.com*" -or $lower -like "*://anyrouter.top*" -or $lower -like "*://a-ocnfniawgw.cn-shanghai.fcapp.run*") {
        return @{ Search = $true; Fetch = $true }
    }
    if ([string]::IsNullOrWhiteSpace($lower)) {
        return @{ Search = $true; Fetch = $true }
    }
    return @{ Search = $false; Fetch = $false }
}

function Get-ControlModeRoutes() {
    $mode = (($env:CLAUDE_SWITCH_TINYFISH_MODE ?? "").Trim()).ToLowerInvariant()
    switch ($mode) {
        "search-fetch" { return @{ Search = "tinyfish"; Fetch = "tinyfish" } }
        "fetch-only" { return @{ Search = "native"; Fetch = "tinyfish" } }
        "gateway-only" { return @{ Search = "native"; Fetch = "native" } }
        default { return $null }
    }
}

function Get-Routes([string]$BaseUrl, [string]$Model) {
    $override = Get-ControlModeRoutes
    if ($null -ne $override) {
        return $override
    }
    $family = Get-ModelFamily $Model
    switch ($family) {
        "claude" {
            $desiredSearch = "native"
            $desiredFetch = "native"
        }
        "deepseek" {
            $desiredSearch = "native"
            $desiredFetch = "tinyfish"
        }
        default {
            $desiredSearch = "tinyfish"
            $desiredFetch = "tinyfish"
        }
    }

    $caps = Get-ProviderCapabilities $BaseUrl
    $searchRoute = if ($desiredSearch -eq "native" -and $caps.Search) { "native" } else { "tinyfish" }
    $fetchRoute = if ($desiredFetch -eq "native" -and $caps.Fetch) { "native" } else { "tinyfish" }

    return @{ Search = $searchRoute; Fetch = $fetchRoute }
}

function Get-StateDir() {
    if ($env:CLAUDE_PLUGIN_DATA) {
        return $env:CLAUDE_PLUGIN_DATA
    }
    return (Join-Path $env:TEMP "tinyfish-router")
}

function Get-StateFile([string]$SessionId) {
    $stateDir = Get-StateDir
    New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
    $suffix = if ([string]::IsNullOrWhiteSpace($SessionId)) { "unknown" } else { $SessionId }
    return (Join-Path $stateDir "session-$suffix.env")
}

function Write-State([string]$StateFile, $Routes) {
    @(
        "search_route=$($Routes.Search)"
        "fetch_route=$($Routes.Fetch)"
    ) | Set-Content -Encoding UTF8 -Path $StateFile
}

function Read-State([string]$StateFile) {
    $map = @{}
    if (-not (Test-Path $StateFile)) {
        return $map
    }
    foreach ($line in Get-Content -Path $StateFile) {
        if ($line -match '^([^=]+)=(.*)$') {
            $map[$matches[1]] = $matches[2]
        }
    }
    return $map
}

function Emit-HookJson([string]$HookEventName, [string]$PermissionDecision, [string]$AdditionalContext) {
    $output = @{
        hookSpecificOutput = @{
            hookEventName = $HookEventName
            additionalContext = $AdditionalContext
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($PermissionDecision)) {
        $output.hookSpecificOutput.permissionDecision = $PermissionDecision
    }
    $output | ConvertTo-Json -Compress -Depth 6
}

function Get-ToolContext([string]$Kind) {
    switch ($Kind) {
        "search" { return "Use tinyfish search query ""<query>"" via the PowerShell tool instead of WebSearch. Native WebSearch is only a fallback when TinyFish is unavailable or rate-limited." }
        "fetch" { return "Use tinyfish fetch content get ""<url>"" via the PowerShell tool instead of WebFetch. Native WebFetch is only a fallback when TinyFish is unavailable or rate-limited." }
    }
}

function Get-SubagentContext($Routes) {
    if ($Routes.Search -eq "tinyfish" -and $Routes.Fetch -eq "tinyfish") {
        return "Subagents should use tinyfish search query ""<query>"" for search and tinyfish fetch content get ""<url>"" for fetch via the PowerShell tool. Native web tools are fallbacks only."
    }
    if ($Routes.Fetch -eq "tinyfish") {
        return "Subagents may use native WebSearch, but WebFetch should use tinyfish fetch content get ""<url>"" via the PowerShell tool. Native fetch is fallback only."
    }
    if ($Routes.Search -eq "tinyfish") {
        return "Subagents should use tinyfish search query ""<query>"" via the PowerShell tool for search. Native WebFetch may stay native."
    }
    return "Subagent web routing is currently native. If a hook later blocks a native web tool, immediately switch to the TinyFish replacement command."
}

$sessionId = if ($payload) { [string]$payload.session_id } else { "" }
$stateFile = Get-StateFile $sessionId
$routes = Get-Routes $env:ANTHROPIC_BASE_URL $env:ANTHROPIC_MODEL

switch ($EventName) {
    "SessionStart" {
        $startupModel = if ($payload) { [string]$payload.model } else { "" }
        $routes = Get-Routes $env:ANTHROPIC_BASE_URL $startupModel
        Write-State $stateFile $routes
        if ($env:CLAUDE_ENV_FILE) {
            @(
                "Set-Item Env:CLAUDE_SWITCH_TINYFISH_STATE_FILE '$stateFile'"
                "Set-Item Env:CLAUDE_SWITCH_TINYFISH_STARTUP_MODEL '$startupModel'"
            ) | Add-Content -Path $env:CLAUDE_ENV_FILE
        }
        Emit-HookJson $EventName "" "TinyFish router active: search=$($routes.Search) fetch=$($routes.Fetch)." | Write-Output
        break
    }
    "PreToolUse" {
        $state = Read-State $stateFile
        if ($state.ContainsKey("search_route")) {
            $routes.Search = $state["search_route"]
        }
        if ($state.ContainsKey("fetch_route")) {
            $routes.Fetch = $state["fetch_route"]
        }
        if ($ToolName -eq "WebSearch" -and $routes.Search -eq "tinyfish") {
            Emit-HookJson $EventName "deny" (Get-ToolContext "search") | Write-Output
        } elseif ($ToolName -eq "WebFetch" -and $routes.Fetch -eq "tinyfish") {
            Emit-HookJson $EventName "deny" (Get-ToolContext "fetch") | Write-Output
        }
        break
    }
    "SubagentStart" {
        $routes = Get-Routes $env:ANTHROPIC_BASE_URL $env:CLAUDE_CODE_SUBAGENT_MODEL
        Emit-HookJson $EventName "" (Get-SubagentContext $routes) | Write-Output
        break
    }
}
"#;

const TINYFISH_POWERSHELL_STATUSLINE_SCRIPT: &str = r#"
$raw = [Console]::In.ReadToEnd()
$payload = $null
if (-not [string]::IsNullOrWhiteSpace($raw)) {
    try {
        $payload = $raw | ConvertFrom-Json -Depth 20
    } catch {
        $payload = $null
    }
}

function Strip-1mSuffix([string]$Model) {
    if ([string]::IsNullOrWhiteSpace($Model)) {
        return ""
    }
    $trimmed = $Model.Trim()
    if ($trimmed.EndsWith("[1m]")) {
        return $trimmed.Substring(0, $trimmed.Length - 4).TrimEnd()
    }
    return $trimmed
}

function Get-ModelFamily([string]$Model) {
    $lower = Strip-1mSuffix($Model).ToLowerInvariant()
    if ($lower.StartsWith("claude")) {
        return "claude"
    }
    if ($lower.StartsWith("deepseek")) {
        return "deepseek"
    }
    return "unknown"
}

function Get-ProviderCapabilities([string]$BaseUrl) {
    $lower = ($BaseUrl ?? "").ToLowerInvariant()
    if ($lower -like "*://api.deepseek.com/anthropic*") {
        return @{ Search = $true; Fetch = $false }
    }
    if ($lower -like "*://api.anthropic.com*" -or $lower -like "*://anyrouter.top*" -or $lower -like "*://a-ocnfniawgw.cn-shanghai.fcapp.run*") {
        return @{ Search = $true; Fetch = $true }
    }
    if ([string]::IsNullOrWhiteSpace($lower)) {
        return @{ Search = $true; Fetch = $true }
    }
    return @{ Search = $false; Fetch = $false }
}

function Get-ControlModeRoutes() {
    $mode = (($env:CLAUDE_SWITCH_TINYFISH_MODE ?? "").Trim()).ToLowerInvariant()
    switch ($mode) {
        "search-fetch" { return @{ Search = "tinyfish"; Fetch = "tinyfish" } }
        "fetch-only" { return @{ Search = "native"; Fetch = "tinyfish" } }
        "gateway-only" { return @{ Search = "native"; Fetch = "native" } }
        default { return $null }
    }
}

function Get-Routes([string]$BaseUrl, [string]$Model) {
    $override = Get-ControlModeRoutes
    if ($null -ne $override) {
        return $override
    }
    $family = Get-ModelFamily $Model
    switch ($family) {
        "claude" {
            $desiredSearch = "native"
            $desiredFetch = "native"
        }
        "deepseek" {
            $desiredSearch = "native"
            $desiredFetch = "tinyfish"
        }
        default {
            $desiredSearch = "tinyfish"
            $desiredFetch = "tinyfish"
        }
    }

    $caps = Get-ProviderCapabilities $BaseUrl
    $searchRoute = if ($desiredSearch -eq "native" -and $caps.Search) { "native" } else { "tinyfish" }
    $fetchRoute = if ($desiredFetch -eq "native" -and $caps.Fetch) { "native" } else { "tinyfish" }

    return @{ Search = $searchRoute; Fetch = $fetchRoute }
}

function Get-StateDir() {
    if ($env:CLAUDE_PLUGIN_DATA) {
        return $env:CLAUDE_PLUGIN_DATA
    }
    return (Join-Path $env:TEMP "tinyfish-router")
}

$sessionId = if ($payload) { [string]$payload.session_id } else { "" }
$modelId = if ($payload -and $payload.model) { [string]$payload.model.id } else { $env:ANTHROPIC_MODEL }
$routes = Get-Routes $env:ANTHROPIC_BASE_URL $modelId

$stateDir = Get-StateDir
New-Item -ItemType Directory -Force -Path $stateDir | Out-Null
$suffix = if ([string]::IsNullOrWhiteSpace($sessionId)) { "unknown" } else { $sessionId }
$stateFile = Join-Path $stateDir "session-$suffix.env"
@(
    "search_route=$($routes.Search)"
    "fetch_route=$($routes.Fetch)"
) | Set-Content -Encoding UTF8 -Path $stateFile

$shortModel = Strip-1mSuffix $modelId
if ([string]::IsNullOrWhiteSpace($shortModel)) {
    $shortModel = "unknown"
}

"tf s=$($routes.Search) f=$($routes.Fetch) m=$shortModel"
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TinyfishToolShell {
    Bash,
    PowerShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TinyfishRoute {
    Native,
    Tinyfish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TinyfishMode {
    None,
    SearchOnly,
    FetchOnly,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum TinyfishPluginVariant {
    Router,
    Full,
    FetchOnly,
}

impl TinyfishPluginVariant {
    pub(super) fn dir_name(self) -> &'static str {
        match self {
            Self::Router => TINYFISH_ROUTER_PLUGIN_ROOT_NAME,
            Self::Full => TINYFISH_FULL_PLUGIN_ROOT_NAME,
            Self::FetchOnly => TINYFISH_FETCH_ONLY_PLUGIN_ROOT_NAME,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Router => "TinyFish Router",
            Self::Full => "TinyFish Full",
            Self::FetchOnly => "TinyFish Fetch Only",
        }
    }

    fn manifest_description(self) -> &'static str {
        match self {
            Self::Router => {
                "Generated by claude-switch to route native web tools through TinyFish when the current model or provider requires it."
            }
            Self::Full => {
                "Generated by claude-switch to replace native web search and fetch with TinyFish for explicit localhost local-gateway launches."
            }
            Self::FetchOnly => {
                "Generated by claude-switch to replace native web fetch with TinyFish for explicit localhost local-gateway launches."
            }
        }
    }

    fn output_style_description(self) -> &'static str {
        match self {
            Self::Router => "Respect claude-switch TinyFish routing decisions",
            Self::Full => {
                "Respect claude-switch TinyFish full routing decisions for explicit localhost local-gateway launches"
            }
            Self::FetchOnly => {
                "Respect claude-switch TinyFish fetch-only routing decisions for explicit localhost local-gateway launches"
            }
        }
    }

    fn status_context(self) -> &'static str {
        match self {
            Self::Router => "TinyFish router active.",
            Self::Full => "TinyFish full mode active: search=tinyfish fetch=tinyfish.",
            Self::FetchOnly => "TinyFish fetch-only mode active: search=native fetch=tinyfish.",
        }
    }

    fn statusline_mode_label(self) -> &'static str {
        match self {
            Self::Router => "router",
            Self::Full => "full",
            Self::FetchOnly => "fetch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TinyfishModelFamily {
    Claude,
    DeepSeek,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TinyfishProviderCapabilities {
    search_native: bool,
    fetch_native: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LightweightRuntimeArtifacts {
    pub(super) base_settings_json: String,
    pub(super) local_gateway_mode: LocalGatewayToolMode,
    pub(super) tinyfish_enabled: bool,
    pub(super) tinyfish_mode: TinyfishMode,
    pub(super) tinyfish_plugin_variant: Option<TinyfishPluginVariant>,
    pub(super) tinyfish_plugin_hooks_json: Option<String>,
    pub(super) tinyfish_plugin_manifest_json: Option<String>,
    pub(super) tinyfish_output_style_text: Option<String>,
    pub(super) tinyfish_hook_script_text: Option<String>,
    pub(super) tinyfish_statusline_script_text: Option<String>,
}

fn strip_model_1m_suffix(model: &str) -> &str {
    let trimmed = model.trim();
    trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim_end()
}

fn tinyfish_model_family(model: &str) -> TinyfishModelFamily {
    let normalized = strip_model_1m_suffix(model).to_ascii_lowercase();
    if normalized.starts_with("claude") {
        TinyfishModelFamily::Claude
    } else if normalized.starts_with("deepseek") {
        TinyfishModelFamily::DeepSeek
    } else {
        TinyfishModelFamily::Unknown
    }
}

fn tinyfish_nonempty_model_family(model: Option<&str>) -> Option<TinyfishModelFamily> {
    let model = model?;
    (!strip_model_1m_suffix(model).is_empty()).then(|| tinyfish_model_family(model))
}

fn tinyfish_provider_default_model_family(base_url: &str) -> TinyfishModelFamily {
    if base_url.trim().is_empty() {
        return TinyfishModelFamily::Unknown;
    }
    if url_matches(base_url, NATIVE_SEARCH_URLS) {
        if url_matches(base_url, NATIVE_FETCH_URLS) {
            TinyfishModelFamily::Claude
        } else {
            TinyfishModelFamily::DeepSeek
        }
    } else {
        TinyfishModelFamily::Unknown
    }
}

fn tinyfish_main_model_family(env: &LightweightEnv, base_url: &str) -> TinyfishModelFamily {
    tinyfish_nonempty_model_family(env.model.as_deref())
        .or_else(|| tinyfish_nonempty_model_family(env.default_haiku_model.as_deref()))
        .or_else(|| tinyfish_nonempty_model_family(env.default_sonnet_model.as_deref()))
        .or_else(|| tinyfish_nonempty_model_family(env.default_opus_model.as_deref()))
        .unwrap_or_else(|| tinyfish_provider_default_model_family(base_url))
}

fn tinyfish_provider_capabilities(base_url: &str) -> TinyfishProviderCapabilities {
    TinyfishProviderCapabilities {
        search_native: url_matches(base_url, NATIVE_SEARCH_URLS) || base_url.trim().is_empty(),
        fetch_native: url_matches(base_url, NATIVE_FETCH_URLS) || base_url.trim().is_empty(),
    }
}

fn routes_for_local_gateway_mode(
    base_url: &str,
    local_gateway_mode: LocalGatewayToolMode,
) -> Result<Option<(TinyfishRoute, TinyfishRoute)>> {
    if local_gateway_mode.is_auto() {
        return Ok(None);
    }
    if !is_local_runtime_base_url(base_url) {
        bail!(
            "Local gateway mode '{}' only applies to localhost/LAN self-hosted APIs, got '{}'.",
            local_gateway_mode.as_cli_value(),
            base_url.trim()
        );
    }
    Ok(Some(match local_gateway_mode {
        LocalGatewayToolMode::Auto => unreachable!("auto handled above"),
        LocalGatewayToolMode::SearchFetch => (TinyfishRoute::Tinyfish, TinyfishRoute::Tinyfish),
        LocalGatewayToolMode::FetchOnly => (TinyfishRoute::Native, TinyfishRoute::Tinyfish),
        LocalGatewayToolMode::GatewayOnly => (TinyfishRoute::Native, TinyfishRoute::Native),
    }))
}

fn desired_routes_for_model_family(family: TinyfishModelFamily) -> (TinyfishRoute, TinyfishRoute) {
    match family {
        TinyfishModelFamily::Claude => (TinyfishRoute::Native, TinyfishRoute::Native),
        TinyfishModelFamily::DeepSeek => (TinyfishRoute::Native, TinyfishRoute::Tinyfish),
        TinyfishModelFamily::Unknown => (TinyfishRoute::Tinyfish, TinyfishRoute::Tinyfish),
    }
}

fn tinyfish_routes_for_model_family(
    base_url: &str,
    family: TinyfishModelFamily,
) -> (TinyfishRoute, TinyfishRoute) {
    let caps = tinyfish_provider_capabilities(base_url);
    let (desired_search, desired_fetch) = desired_routes_for_model_family(family);
    let search = if desired_search == TinyfishRoute::Native && caps.search_native {
        TinyfishRoute::Native
    } else {
        TinyfishRoute::Tinyfish
    };
    let fetch = if desired_fetch == TinyfishRoute::Native && caps.fetch_native {
        TinyfishRoute::Native
    } else {
        TinyfishRoute::Tinyfish
    };
    (search, fetch)
}

#[cfg(test)]
pub(super) fn tinyfish_routes(base_url: &str, model: &str) -> (TinyfishRoute, TinyfishRoute) {
    tinyfish_routes_for_model_family(base_url, tinyfish_model_family(model))
}

pub(super) fn tinyfish_mode_from_routes(
    search_route: TinyfishRoute,
    fetch_route: TinyfishRoute,
) -> TinyfishMode {
    match (search_route, fetch_route) {
        (TinyfishRoute::Native, TinyfishRoute::Native) => TinyfishMode::None,
        (TinyfishRoute::Tinyfish, TinyfishRoute::Native) => TinyfishMode::SearchOnly,
        (TinyfishRoute::Native, TinyfishRoute::Tinyfish) => TinyfishMode::FetchOnly,
        (TinyfishRoute::Tinyfish, TinyfishRoute::Tinyfish) => TinyfishMode::Full,
    }
}

fn tinyfish_mode_for_model_family(base_url: &str, family: TinyfishModelFamily) -> TinyfishMode {
    if base_url.trim().is_empty() {
        return TinyfishMode::None;
    }
    let (search_route, fetch_route) = tinyfish_routes_for_model_family(base_url, family);
    tinyfish_mode_from_routes(search_route, fetch_route)
}

#[cfg(test)]
pub(super) fn tinyfish_mode(base_url: &str, model: &str) -> TinyfishMode {
    tinyfish_mode_for_model_family(base_url, tinyfish_model_family(model))
}

fn tinyfish_enabled_for_profile(
    base_url: &str,
    main_model_family: TinyfishModelFamily,
    subagent_model: Option<&str>,
) -> bool {
    if base_url.trim().is_empty() {
        return false;
    }
    if url_matches(base_url, ANYROUTER_URLS) {
        return true;
    }
    let main_requires_router =
        tinyfish_mode_for_model_family(base_url, main_model_family) != TinyfishMode::None;
    let subagent_requires_router =
        tinyfish_nonempty_model_family(subagent_model).is_some_and(|family| {
            tinyfish_mode_for_model_family(base_url, family) != TinyfishMode::None
        });
    main_requires_router || subagent_requires_router
}

fn tinyfish_plugin_variant_for_local_gateway_mode(
    local_gateway_mode: LocalGatewayToolMode,
) -> Option<TinyfishPluginVariant> {
    match local_gateway_mode {
        LocalGatewayToolMode::Auto | LocalGatewayToolMode::GatewayOnly => None,
        LocalGatewayToolMode::SearchFetch => Some(TinyfishPluginVariant::Full),
        LocalGatewayToolMode::FetchOnly => Some(TinyfishPluginVariant::FetchOnly),
    }
}

pub(super) fn native_tinyfish_tool_shell() -> TinyfishToolShell {
    if cfg!(windows) {
        TinyfishToolShell::PowerShell
    } else {
        TinyfishToolShell::Bash
    }
}

pub(super) fn tinyfish_permissions_allowlist(
    enabled: bool,
    tool_shell: TinyfishToolShell,
) -> Option<&'static [&'static str]> {
    if !enabled {
        return None;
    }
    Some(match tool_shell {
        TinyfishToolShell::Bash => TINYFISH_BASH_ALLOWLIST,
        TinyfishToolShell::PowerShell => TINYFISH_WINDOWS_ALLOWLIST,
    })
}

fn tinyfish_statusline_command(script_path: &str, tool_shell: TinyfishToolShell) -> String {
    match tool_shell {
        TinyfishToolShell::Bash => format!("\"{}\"", script_path.replace('\\', "/")),
        TinyfishToolShell::PowerShell => {
            let path = script_path.replace('\\', "/").replace('"', "\\\"");
            format!("powershell -NoProfile -ExecutionPolicy Bypass -Command \"& \\\"{path}\\\"\"")
        }
    }
}

pub(super) fn build_lightweight_settings(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    tinyfish_enabled: bool,
    tool_shell: TinyfishToolShell,
    tinyfish_statusline_script_path: Option<&str>,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    build_lightweight_settings_with_local_gateway_mode(
        env,
        token,
        url,
        tinyfish_enabled,
        tool_shell,
        tinyfish_statusline_script_path,
        LocalGatewayToolMode::Auto,
    )
}

pub(super) fn build_lightweight_settings_with_local_gateway_mode(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    tinyfish_enabled: bool,
    tool_shell: TinyfishToolShell,
    tinyfish_statusline_script_path: Option<&str>,
    local_gateway_mode: LocalGatewayToolMode,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut settings = serde_json::Map::new();
    let env_map = build_lightweight_env_map(env, url, local_gateway_mode);
    settings.insert("env".into(), serde_json::Value::Object(env_map));
    if let Some(token) = token {
        settings.insert(
            "apiKeyHelper".into(),
            serde_json::Value::String(ProfileManager::inline_api_key_helper_command(
                token, tool_shell,
            )?),
        );
    }
    if let Some(allowlist) = tinyfish_permissions_allowlist(tinyfish_enabled, tool_shell) {
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
    if let Some(script_path) = tinyfish_statusline_script_path {
        settings.insert(
            "statusLine".into(),
            serde_json::json!({
                "type": "command",
                "command": tinyfish_statusline_command(script_path, tool_shell),
            }),
        );
    }
    Ok(settings)
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
    url: Option<&str>,
    local_gateway_mode: LocalGatewayToolMode,
) -> Vec<(String, String)> {
    let mut entries = Vec::new();

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
    if !local_gateway_mode.is_auto() {
        entries.push((
            TINYFISH_CONTROL_MODE_ENV.into(),
            local_gateway_mode.as_cli_value().to_string(),
        ));
    }

    entries
}

fn build_lightweight_env_map(
    env: &LightweightEnv,
    url: Option<&str>,
    local_gateway_mode: LocalGatewayToolMode,
) -> serde_json::Map<String, serde_json::Value> {
    build_lightweight_env_entries(env, url, local_gateway_mode)
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect()
}

pub(super) fn tinyfish_plugin_script_file_name(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => TINYFISH_BASH_HOOK_SCRIPT_NAME,
        TinyfishToolShell::PowerShell => TINYFISH_POWERSHELL_HOOK_SCRIPT_NAME,
    }
}

pub(super) fn tinyfish_statusline_script_file_name(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => TINYFISH_BASH_STATUSLINE_SCRIPT_NAME,
        TinyfishToolShell::PowerShell => TINYFISH_POWERSHELL_STATUSLINE_SCRIPT_NAME,
    }
}

fn tinyfish_search_tool_context(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => {
            "Use tinyfish search query \"<query>\" via the Bash tool instead of WebSearch. Native WebSearch is only a fallback when TinyFish is unavailable or rate-limited."
        }
        TinyfishToolShell::PowerShell => {
            "Use tinyfish search query \"\"<query>\"\" via the PowerShell tool instead of WebSearch. Native WebSearch is only a fallback when TinyFish is unavailable or rate-limited."
        }
    }
}

fn tinyfish_fetch_tool_context(tool_shell: TinyfishToolShell) -> &'static str {
    match tool_shell {
        TinyfishToolShell::Bash => {
            "Use tinyfish fetch content get \"<url>\" via the Bash tool instead of WebFetch. Native WebFetch is only a fallback when TinyFish is unavailable or rate-limited."
        }
        TinyfishToolShell::PowerShell => {
            "Use tinyfish fetch content get \"\"<url>\"\" via the PowerShell tool instead of WebFetch. Native WebFetch is only a fallback when TinyFish is unavailable or rate-limited."
        }
    }
}

fn tinyfish_static_subagent_context(
    variant: TinyfishPluginVariant,
    tool_shell: TinyfishToolShell,
) -> &'static str {
    match (variant, tool_shell) {
        (TinyfishPluginVariant::Full, TinyfishToolShell::Bash) => {
            "Subagents should use tinyfish search query \"<query>\" for search and tinyfish fetch content get \"<url>\" for fetch via the Bash tool. Native web tools are fallbacks only."
        }
        (TinyfishPluginVariant::Full, TinyfishToolShell::PowerShell) => {
            "Subagents should use tinyfish search query \"\"<query>\"\" for search and tinyfish fetch content get \"\"<url>\"\" for fetch via the PowerShell tool. Native web tools are fallbacks only."
        }
        (TinyfishPluginVariant::FetchOnly, TinyfishToolShell::Bash) => {
            "Subagents may use native WebSearch, but WebFetch should use tinyfish fetch content get \"<url>\" via the Bash tool. Native fetch is fallback only."
        }
        (TinyfishPluginVariant::FetchOnly, TinyfishToolShell::PowerShell) => {
            "Subagents may use native WebSearch, but WebFetch should use tinyfish fetch content get \"\"<url>\"\" via the PowerShell tool. Native fetch is fallback only."
        }
        (TinyfishPluginVariant::Router, _) => {
            "Subagent web routing is currently native. If a hook later blocks a native web tool, immediately switch to the TinyFish replacement command."
        }
    }
}

fn tinyfish_output_style_text(variant: TinyfishPluginVariant) -> String {
    format!(
        "---\nname: {}\ndescription: {}\nforce-for-plugin: true\nkeep-coding-instructions: true\n---\n\nIf a hook blocks a native WebSearch or WebFetch call, immediately use the TinyFish replacement\ncommand described by the hook instead of retrying the blocked native web tool.\n",
        variant.display_name(),
        variant.output_style_description()
    )
}

fn tinyfish_static_bash_hook_script(variant: TinyfishPluginVariant) -> String {
    let pretool_cases = match variant {
        TinyfishPluginVariant::Full => format!(
            "            WebSearch)\n                emit_hook_json \"$event_name\" \"deny\" '{}'\n                ;;\n            WebFetch)\n                emit_hook_json \"$event_name\" \"deny\" '{}'\n                ;;\n",
            tinyfish_search_tool_context(TinyfishToolShell::Bash),
            tinyfish_fetch_tool_context(TinyfishToolShell::Bash)
        ),
        TinyfishPluginVariant::FetchOnly => format!(
            "            WebFetch)\n                emit_hook_json \"$event_name\" \"deny\" '{}'\n                ;;\n",
            tinyfish_fetch_tool_context(TinyfishToolShell::Bash)
        ),
        TinyfishPluginVariant::Router => {
            unreachable!("router variant uses dedicated dynamic script")
        }
    };
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

event_name="${{1:-}}"
tool_name="${{2:-}}"
cat >/dev/null

emit_hook_json() {{
    local hook_event="$1"
    local decision="$2"
    local context="$3"

    printf '{{"hookSpecificOutput":{{"hookEventName":"%s"' "$hook_event"
    if [[ -n "$decision" ]]; then
        printf ',"permissionDecision":"%s"' "$decision"
    fi
    printf ',"additionalContext":"%s"}}}}\n' "$context"
}}

case "$event_name" in
    SessionStart)
        emit_hook_json "$event_name" "" '{}'
        ;;
    PreToolUse)
        case "$tool_name" in
{pretool_cases}        esac
        ;;
    SubagentStart)
        emit_hook_json "$event_name" "" '{}'
        ;;
esac
"#,
        variant.status_context(),
        tinyfish_static_subagent_context(variant, TinyfishToolShell::Bash)
    )
}

fn tinyfish_static_powershell_hook_script(variant: TinyfishPluginVariant) -> String {
    let pretool_branch = match variant {
        TinyfishPluginVariant::Full => format!(
            "        if ($ToolName -eq \"WebSearch\") {{\n            Emit-HookJson $EventName \"deny\" \"{}\" | Write-Output\n        }} elseif ($ToolName -eq \"WebFetch\") {{\n            Emit-HookJson $EventName \"deny\" \"{}\" | Write-Output\n        }}\n",
            tinyfish_search_tool_context(TinyfishToolShell::PowerShell),
            tinyfish_fetch_tool_context(TinyfishToolShell::PowerShell)
        ),
        TinyfishPluginVariant::FetchOnly => format!(
            "        if ($ToolName -eq \"WebFetch\") {{\n            Emit-HookJson $EventName \"deny\" \"{}\" | Write-Output\n        }}\n",
            tinyfish_fetch_tool_context(TinyfishToolShell::PowerShell)
        ),
        TinyfishPluginVariant::Router => {
            unreachable!("router variant uses dedicated dynamic script")
        }
    };
    format!(
        r#"
param(
    [string]$EventName = "",
    [string]$ToolName = ""
)

$null = [Console]::In.ReadToEnd()

function Emit-HookJson([string]$HookEventName, [string]$PermissionDecision, [string]$AdditionalContext) {{
    $output = @{{
        hookSpecificOutput = @{{
            hookEventName = $HookEventName
            additionalContext = $AdditionalContext
        }}
    }}
    if (-not [string]::IsNullOrWhiteSpace($PermissionDecision)) {{
        $output.hookSpecificOutput.permissionDecision = $PermissionDecision
    }}
    $output | ConvertTo-Json -Compress -Depth 6
}}

switch ($EventName) {{
    "SessionStart" {{
        Emit-HookJson $EventName "" "{}" | Write-Output
        break
    }}
    "PreToolUse" {{
{pretool_branch}        break
    }}
    "SubagentStart" {{
        Emit-HookJson $EventName "" "{}" | Write-Output
        break
    }}
}}
"#,
        variant.status_context(),
        tinyfish_static_subagent_context(variant, TinyfishToolShell::PowerShell)
    )
}

fn tinyfish_static_bash_statusline_script(variant: TinyfishPluginVariant) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

payload="$(cat)"
single_line="$(printf '%s' "$payload" | tr -d '\r\n')"

extract_model_id() {{
    printf '%s' "$single_line" | sed -n 's/.*"model"[[:space:]]*:[[:space:]]*{{[^}}]*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}}

strip_1m_suffix() {{
    local value="${{1%[[:space:]]}}"
    if [[ "$value" == *"[1m]" ]]; then
        value="${{value%\[1m\]}}"
    fi
    printf '%s' "$value"
}}

model_id="$(extract_model_id)"
if [[ -z "$model_id" ]]; then
    model_id="${{ANTHROPIC_MODEL:-}}"
fi
short_model="$(strip_1m_suffix "$model_id")"
if [[ -z "$short_model" ]]; then
    short_model="unknown"
fi

printf 'tf {} m=%s\n' "$short_model"
"#,
        variant.statusline_mode_label()
    )
}

fn tinyfish_static_powershell_statusline_script(variant: TinyfishPluginVariant) -> String {
    format!(
        r#"
$raw = [Console]::In.ReadToEnd()
$payload = $null
if (-not [string]::IsNullOrWhiteSpace($raw)) {{
    try {{
        $payload = $raw | ConvertFrom-Json -Depth 20
    }} catch {{
        $payload = $null
    }}
}}

function Strip-1mSuffix([string]$Model) {{
    if ([string]::IsNullOrWhiteSpace($Model)) {{
        return ""
    }}
    $trimmed = $Model.Trim()
    if ($trimmed.EndsWith("[1m]")) {{
        return $trimmed.Substring(0, $trimmed.Length - 4).TrimEnd()
    }}
    return $trimmed
}}

$modelId = if ($payload -and $payload.model) {{ [string]$payload.model.id }} else {{ $env:ANTHROPIC_MODEL }}
$shortModel = Strip-1mSuffix $modelId
if ([string]::IsNullOrWhiteSpace($shortModel)) {{
    $shortModel = "unknown"
}}

"tf {} m=$shortModel"
"#,
        variant.statusline_mode_label()
    )
}

pub(super) fn tinyfish_plugin_hooks(
    variant: TinyfishPluginVariant,
    tool_shell: TinyfishToolShell,
) -> String {
    let hook_script = format!(
        "${{CLAUDE_PLUGIN_ROOT}}/scripts/{}",
        tinyfish_plugin_script_file_name(tool_shell)
    );
    let pre_tool_matchers = match variant {
        TinyfishPluginVariant::Router | TinyfishPluginVariant::Full => {
            vec!["WebSearch", "WebFetch"]
        }
        TinyfishPluginVariant::FetchOnly => vec!["WebFetch"],
    };
    let mut hooks = serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": tinyfish_hook_command(tool_shell, &hook_script, "SessionStart", None),
                }]
            }],
            "PreToolUse": pre_tool_matchers.iter().map(|matcher| serde_json::json!({
                "matcher": matcher,
                "hooks": [{
                    "type": "command",
                    "command": tinyfish_hook_command(tool_shell, &hook_script, "PreToolUse", Some(matcher)),
                }]
            })).collect::<Vec<_>>(),
            "SubagentStart": [{
                "hooks": [{
                    "type": "command",
                    "command": tinyfish_hook_command(tool_shell, &hook_script, "SubagentStart", None),
                }]
            }]
        }
    });
    if matches!(tool_shell, TinyfishToolShell::PowerShell) {
        for hook_set in hooks["hooks"]
            .as_object_mut()
            .into_iter()
            .flat_map(|map| map.values_mut())
        {
            if let Some(items) = hook_set.as_array_mut() {
                for item in items {
                    if let Some(inner_hooks) = item["hooks"].as_array_mut() {
                        for hook in inner_hooks {
                            hook["shell"] = serde_json::Value::String("powershell".to_string());
                        }
                    }
                }
            }
        }
    }
    hooks.to_string()
}

pub(super) fn tinyfish_hook_command(
    tool_shell: TinyfishToolShell,
    script_path: &str,
    event_name: &str,
    tool_name: Option<&str>,
) -> String {
    match tool_shell {
        TinyfishToolShell::Bash => match tool_name {
            Some(tool_name) => format!("\"{script_path}\" {event_name} {tool_name}"),
            None => format!("\"{script_path}\" {event_name}"),
        },
        TinyfishToolShell::PowerShell => match tool_name {
            Some(tool_name) => format!("& \"{script_path}\" {event_name} {tool_name}"),
            None => format!("& \"{script_path}\" {event_name}"),
        },
    }
}

pub(super) fn tinyfish_plugin_manifest(variant: TinyfishPluginVariant) -> String {
    serde_json::json!({
        "name": variant.dir_name(),
        "displayName": variant.display_name(),
        "description": variant.manifest_description(),
        "outputStyles": "./output-styles",
    })
    .to_string()
}

pub(super) fn tinyfish_output_style(variant: TinyfishPluginVariant) -> String {
    tinyfish_output_style_text(variant)
}

pub(super) fn tinyfish_hook_script(
    tool_shell: TinyfishToolShell,
    variant: TinyfishPluginVariant,
) -> String {
    match (tool_shell, variant) {
        (TinyfishToolShell::Bash, TinyfishPluginVariant::Router) => {
            TINYFISH_BASH_HOOK_SCRIPT.to_string()
        }
        (TinyfishToolShell::Bash, TinyfishPluginVariant::Full)
        | (TinyfishToolShell::Bash, TinyfishPluginVariant::FetchOnly) => {
            tinyfish_static_bash_hook_script(variant)
        }
        (TinyfishToolShell::PowerShell, TinyfishPluginVariant::Router) => {
            TINYFISH_POWERSHELL_HOOK_SCRIPT.to_string()
        }
        (TinyfishToolShell::PowerShell, TinyfishPluginVariant::Full)
        | (TinyfishToolShell::PowerShell, TinyfishPluginVariant::FetchOnly) => {
            tinyfish_static_powershell_hook_script(variant)
        }
    }
}

pub(super) fn tinyfish_statusline_script(
    tool_shell: TinyfishToolShell,
    variant: TinyfishPluginVariant,
) -> String {
    match (tool_shell, variant) {
        (TinyfishToolShell::Bash, TinyfishPluginVariant::Router) => {
            TINYFISH_BASH_STATUSLINE_SCRIPT.to_string()
        }
        (TinyfishToolShell::Bash, TinyfishPluginVariant::Full)
        | (TinyfishToolShell::Bash, TinyfishPluginVariant::FetchOnly) => {
            tinyfish_static_bash_statusline_script(variant)
        }
        (TinyfishToolShell::PowerShell, TinyfishPluginVariant::Router) => {
            TINYFISH_POWERSHELL_STATUSLINE_SCRIPT.to_string()
        }
        (TinyfishToolShell::PowerShell, TinyfishPluginVariant::Full)
        | (TinyfishToolShell::PowerShell, TinyfishPluginVariant::FetchOnly) => {
            tinyfish_static_powershell_statusline_script(variant)
        }
    }
}

pub(super) fn tinyfish_command_succeeds_with_timeout_with_path(
    program: &str,
    args: &[&str],
    timeout: Duration,
    path_override: Option<std::ffi::OsString>,
) -> bool {
    let child_path = path_override.clone();
    let candidate_paths = path_override.or_else(|| std::env::var_os("PATH"));
    let mut child =
        match super::with_local_command_candidates_for_paths(program, candidate_paths, |resolved| {
            let mut command = std::process::Command::new(resolved);
            command
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(path) = child_path.as_ref() {
                command.env("PATH", path);
            }
            command.spawn()
        }) {
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

pub(super) fn tinyfish_command_succeeds_with_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> bool {
    tinyfish_command_succeeds_with_timeout_with_path(program, args, timeout, None)
}

#[cfg(target_os = "windows")]
fn refreshed_windows_command_path() -> Option<std::ffi::OsString> {
    #[derive(serde::Deserialize)]
    struct PathSnapshot {
        machine: Option<String>,
        user: Option<String>,
    }

    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let powershell = if powershell.is_file() {
        powershell.into_os_string()
    } else {
        std::ffi::OsString::from("powershell.exe")
    };

    let output = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$paths = @{ machine = [Environment]::GetEnvironmentVariable('Path', 'Machine'); user = [Environment]::GetEnvironmentVariable('Path', 'User') }; $paths | ConvertTo-Json -Compress",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let snapshot: PathSnapshot = serde_json::from_slice(&output.stdout).ok()?;
    let mut merged = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for source in [
        std::env::var_os("PATH"),
        snapshot.machine.map(std::ffi::OsString::from),
        snapshot.user.map(std::ffi::OsString::from),
    ] {
        let Some(source) = source else {
            continue;
        };
        for entry in std::env::split_paths(&source) {
            let key = entry.to_string_lossy().to_ascii_lowercase();
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            merged.push(entry);
        }
    }

    (!merged.is_empty())
        .then(|| std::env::join_paths(merged).ok())
        .flatten()
}

pub(crate) fn tinyfish_available() -> bool {
    let timeout = Duration::from_secs(2);
    tinyfish_command_succeeds_with_timeout("tinyfish", &["--version"], timeout) || {
        #[cfg(target_os = "windows")]
        {
            refreshed_windows_command_path().is_some_and(|path| {
                tinyfish_command_succeeds_with_timeout_with_path(
                    "tinyfish",
                    &["--version"],
                    timeout,
                    Some(path),
                )
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }
}

pub(super) fn build_lightweight_runtime_artifacts(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    tool_shell: TinyfishToolShell,
) -> Result<LightweightRuntimeArtifacts> {
    build_lightweight_runtime_artifacts_with_local_gateway_mode(
        env,
        token,
        url,
        tool_shell,
        LocalGatewayToolMode::Auto,
    )
}

pub(super) fn build_lightweight_runtime_artifacts_with_local_gateway_mode(
    env: &LightweightEnv,
    token: Option<&str>,
    url: Option<&str>,
    tool_shell: TinyfishToolShell,
    local_gateway_mode: LocalGatewayToolMode,
) -> Result<LightweightRuntimeArtifacts> {
    let base_url = url.unwrap_or_default();
    let base_settings_json =
        serde_json::to_string(&build_lightweight_settings_with_local_gateway_mode(
            env,
            token,
            url,
            false,
            tool_shell,
            None,
            local_gateway_mode,
        )?)
        .context("Failed to serialize base lightweight settings JSON")?;

    let main_model_family = tinyfish_main_model_family(env, base_url);
    let explicit_routes = routes_for_local_gateway_mode(base_url, local_gateway_mode)?;
    let (tinyfish_enabled, tinyfish_mode, tinyfish_plugin_variant) =
        if let Some((search_route, fetch_route)) = explicit_routes {
            let tinyfish_mode = tinyfish_mode_from_routes(search_route, fetch_route);
            (
                tinyfish_mode != TinyfishMode::None,
                tinyfish_mode,
                tinyfish_plugin_variant_for_local_gateway_mode(local_gateway_mode),
            )
        } else if tinyfish_disabled_via_extra(&env.extras) {
            (false, TinyfishMode::None, None)
        } else {
            let tinyfish_enabled = tinyfish_enabled_for_profile(
                base_url,
                main_model_family,
                env.subagent_model.as_deref(),
            );
            let tinyfish_mode = if tinyfish_enabled {
                tinyfish_mode_for_model_family(base_url, main_model_family)
            } else {
                TinyfishMode::None
            };
            (
                tinyfish_enabled,
                tinyfish_mode,
                tinyfish_enabled.then_some(TinyfishPluginVariant::Router),
            )
        };

    Ok(LightweightRuntimeArtifacts {
        base_settings_json,
        local_gateway_mode,
        tinyfish_enabled,
        tinyfish_mode,
        tinyfish_plugin_variant,
        tinyfish_plugin_hooks_json: tinyfish_plugin_variant
            .map(|variant| tinyfish_plugin_hooks(variant, tool_shell)),
        tinyfish_plugin_manifest_json: tinyfish_plugin_variant.map(tinyfish_plugin_manifest),
        tinyfish_output_style_text: tinyfish_plugin_variant.map(tinyfish_output_style),
        tinyfish_hook_script_text: tinyfish_plugin_variant
            .map(|variant| tinyfish_hook_script(tool_shell, variant)),
        tinyfish_statusline_script_text: tinyfish_plugin_variant
            .map(|variant| tinyfish_statusline_script(tool_shell, variant)),
    })
}
