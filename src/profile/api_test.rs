use anyhow::{Context, Result, bail};
use std::time::Duration;
use ureq::RequestExt;

use super::url_match::{ANYROUTER_URLS, url_matches};

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

const ANYROUTER_TEST_BETA_CANDIDATES: &[&str] = &[
    "claude-code-20250219",
    "interleaved-thinking-2025-05-14",
    "context-1m-2025-08-07",
    "redact-thinking-2026-02-12",
    "context-management-2025-06-27",
    "prompt-caching-scope-2026-01-05",
    "advanced-tool-use-2025-11-20",
    "effort-2025-11-24",
    "fast-mode-2026-02-01",
];

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

pub(super) fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    for suffix in KNOWN_COMPAT_SUFFIXES {
        if base_url.ends_with(*suffix) {
            return Some(&base_url[..base_url.len() - suffix.len()]);
        }
    }
    None
}

pub(super) fn build_message_candidates(base_url: &str) -> Result<Vec<String>> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AnthropicTestRequest {
    pub(super) body: serde_json::Value,
    pub(super) anthropic_beta: Option<String>,
    pub(super) anyrouter_non_haiku: bool,
}

pub(super) fn is_anyrouter_url(base_url: &str) -> bool {
    url_matches(base_url, ANYROUTER_URLS)
}

fn is_haiku_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("haiku")
}

fn anyrouter_beta_required_field(beta: &str) -> Option<&'static str> {
    match beta {
        "interleaved-thinking-2025-05-14"
        | "context-1m-2025-08-07"
        | "redact-thinking-2026-02-12" => Some("thinking"),
        "context-management-2025-06-27" => Some("context_management"),
        "effort-2025-11-24" => Some("output_config"),
        _ => None,
    }
}

fn anyrouter_body_has_truthy_field(body: &serde_json::Value, field: &str) -> bool {
    match body.get(field) {
        Some(serde_json::Value::Null) | None => false,
        Some(serde_json::Value::String(value)) => !value.is_empty(),
        Some(_) => true,
    }
}

pub(super) fn patch_anyrouter_beta_header(
    candidates: &[&str],
    body: &serde_json::Value,
) -> Option<String> {
    let kept: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|flag| {
            if *flag == "structured-outputs-2025-12-15" {
                return false;
            }
            anyrouter_beta_required_field(flag)
                .is_none_or(|field| anyrouter_body_has_truthy_field(body, field))
        })
        .collect();

    if kept.is_empty() {
        None
    } else {
        Some(kept.join(","))
    }
}

pub(super) fn build_anthropic_test_request(
    base_url: &str,
    model: &str,
    prompt: &str,
) -> AnthropicTestRequest {
    let anyrouter_non_haiku = is_anyrouter_url(base_url) && !is_haiku_model(model);
    let max_tokens = if anyrouter_non_haiku { 1200 } else { 64 };
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [
            {
                "role": "user",
                "content": prompt,
            }
        ],
    });

    if anyrouter_non_haiku && let Some(object) = body.as_object_mut() {
        object.insert(
            "thinking".into(),
            serde_json::json!({
                "type": "enabled",
                "budget_tokens": 1024,
            }),
        );
    }

    let anthropic_beta = if anyrouter_non_haiku {
        patch_anyrouter_beta_header(ANYROUTER_TEST_BETA_CANDIDATES, &body)
    } else {
        None
    };

    AnthropicTestRequest {
        body,
        anthropic_beta,
        anyrouter_non_haiku,
    }
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
    let test_request = build_anthropic_test_request(base_url, model, prompt);
    let api_key_header = ureq::http::HeaderValue::from_bytes(auth_token.as_bytes())
        .context("Invalid API key for x-api-key header")?;
    let beta_header = test_request
        .anthropic_beta
        .as_deref()
        .map(ureq::http::HeaderValue::from_str)
        .transpose()
        .context("Invalid anthropic-beta header for AnyRouter test request")?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();

    let mut last_err: Option<String> = None;
    for url in &candidates {
        let send_body = ureq::SendBody::from_json(&test_request.body)
            .context("Failed to encode request JSON")?;
        let mut request = ureq::http::Request::builder()
            .method(ureq::http::Method::POST)
            .uri(url.as_str())
            .header("content-type", "application/json; charset=utf-8")
            .header("x-api-key", api_key_header.clone())
            .header("Authorization", &format!("Bearer {}", auth_token))
            .header("anthropic-version", "2023-06-01");
        if let Some(value) = beta_header.as_ref() {
            request = request.header("anthropic-beta", value.clone());
        }
        let request = request
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
        if test_request.anyrouter_non_haiku && raw.trim().is_empty() {
            last_err = Some(format!(
                "HTTP {} at {url}: AnyRouter returned an empty success body",
                status.as_u16()
            ));
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
