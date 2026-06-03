use super::*;

pub(super) const PUBLIC_SITE_TEST_DEFAULT_PROMPT: &str = "Hello";
pub(super) const PUBLIC_SITE_TEST_GROUP_GAP_MS: u64 = 1500;
const PUBLIC_SITE_TEST_TIMEOUT_SECS: u64 = 16;
pub(super) const PUBLIC_SITE_TEST_PAGE_SIZE: usize = 5;
pub(super) const PUBLIC_SITE_EVENT_POLL_MS: u64 = 100;
pub(super) const PUBLIC_SITE_DETAIL_SCROLL_STEP: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum PublicSiteModelSource {
    None,
    DefaultHaiku,
    DefaultOpus,
    DefaultSonnet,
    ExplicitModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicSiteProviderTestSlot {
    Haiku,
    Sonnet,
    Opus,
    Model,
    Subagent,
}

impl PublicSiteProviderTestSlot {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Haiku => "haiku",
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
            Self::Model => "model",
            Self::Subagent => "subagent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicSiteTarget {
    pub(super) provider_id: String,
    pub(super) provider_name: String,
    pub(super) key_id: String,
    pub(super) key_name: String,
    pub(super) base_url: String,
    pub(super) profile_id: String,
    pub(super) profile_name: String,
    pub(super) api_key: String,
    pub(super) preflight_error: Option<String>,
    pub(super) configured_model: Option<String>,
    pub(super) model_source: PublicSiteModelSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PublicSiteRequestKey {
    pub(super) base_url: String,
    pub(super) provider_identity: String,
    pub(super) key_identity: String,
    pub(super) model_identity: String,
    pub(super) prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicSiteRequestPlan {
    pub(super) key: PublicSiteRequestKey,
    pub(super) request_target: PublicSiteTarget,
    pub(super) consumers: Vec<PublicSiteTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicSiteTestResult {
    pub(super) provider_name: String,
    pub(super) key_name: String,
    pub(super) base_url: String,
    pub(super) profile_id: String,
    pub(super) profile_name: String,
    pub(super) model: String,
    pub(super) first_char: Option<String>,
    pub(super) response_preview: Option<String>,
    pub(super) endpoint_used: Option<String>,
    pub(super) latency_ms: Option<u128>,
    pub(super) input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
    pub(super) is_success: bool,
    pub(super) error: Option<String>,
}

#[derive(Debug)]
pub(super) enum PublicSiteWorkerEvent {
    Result(PublicSiteTestResult),
}

pub(super) fn normalize_base_url_key(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn base_url_host(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    let host = without_scheme
        .split('/')
        .next()?
        .rsplit('@')
        .next()?
        .split(':')
        .next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

pub(super) fn is_public_test_excluded_base_url(base_url: &str) -> bool {
    matches!(
        base_url_host(base_url).as_deref(),
        Some("api.anthropic.com" | "api.deepseek.com")
    )
}

fn normalized_public_site_model(model: &str) -> Option<String> {
    let trimmed = trim_model_context_suffix(model).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn public_site_model_from_profile(
    profile: &Profile,
) -> (PublicSiteModelSource, Option<String>) {
    let Some(env) = profile.env.as_ref() else {
        return (PublicSiteModelSource::None, None);
    };
    if let Some(model) = env
        .default_haiku_model
        .as_deref()
        .and_then(normalized_public_site_model)
    {
        return (PublicSiteModelSource::DefaultHaiku, Some(model));
    }
    if let Some(model) = env.model.as_deref().and_then(normalized_public_site_model) {
        return (PublicSiteModelSource::ExplicitModel, Some(model));
    }
    if let Some(model) = env
        .default_sonnet_model
        .as_deref()
        .and_then(normalized_public_site_model)
    {
        return (PublicSiteModelSource::DefaultSonnet, Some(model));
    }
    if let Some(model) = env
        .default_opus_model
        .as_deref()
        .and_then(normalized_public_site_model)
    {
        return (PublicSiteModelSource::DefaultOpus, Some(model));
    }
    (PublicSiteModelSource::None, None)
}

pub(super) fn public_site_provider_test_slot_from_key(
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<PublicSiteProviderTestSlot> {
    if !modifiers.is_empty() {
        return None;
    }
    match code {
        KeyCode::Char('h') => Some(PublicSiteProviderTestSlot::Haiku),
        KeyCode::Char('s') => Some(PublicSiteProviderTestSlot::Sonnet),
        KeyCode::Char('o') => Some(PublicSiteProviderTestSlot::Opus),
        KeyCode::Char('m') => Some(PublicSiteProviderTestSlot::Model),
        KeyCode::Char('a') => Some(PublicSiteProviderTestSlot::Subagent),
        _ => None,
    }
}

pub(super) fn public_site_provider_test_model_from_profile(
    profile: &Profile,
    slot: PublicSiteProviderTestSlot,
) -> Option<String> {
    let env = profile.env.as_ref()?;
    let model = match slot {
        PublicSiteProviderTestSlot::Haiku => env.default_haiku_model.as_deref(),
        PublicSiteProviderTestSlot::Sonnet => env.default_sonnet_model.as_deref(),
        PublicSiteProviderTestSlot::Opus => env.default_opus_model.as_deref(),
        PublicSiteProviderTestSlot::Model => env.model.as_deref(),
        PublicSiteProviderTestSlot::Subagent => env.subagent_model.as_deref(),
    }?;
    normalized_public_site_model(model)
}

pub(super) fn sort_public_site_results(results: &mut [PublicSiteTestResult]) {
    results.sort_by(|left, right| {
        right
            .is_success
            .cmp(&left.is_success)
            .then_with(|| left.latency_ms.cmp(&right.latency_ms))
            .then_with(|| left.first_char.cmp(&right.first_char))
            .then_with(|| left.provider_name.cmp(&right.provider_name))
            .then_with(|| left.key_name.cmp(&right.key_name))
    });
}

fn public_site_first_char(text: &str) -> Option<String> {
    text.trim().chars().next().map(|ch| ch.to_string())
}

fn public_site_preview(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut preview = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            preview.push_str("...");
            break;
        }
        preview.push(ch);
    }
    Some(preview)
}

pub(super) fn public_site_request_timeout() -> Duration {
    Duration::from_secs(PUBLIC_SITE_TEST_TIMEOUT_SECS)
}

pub(super) fn public_site_target_preflight_result(
    target: &PublicSiteTarget,
) -> PublicSiteTestResult {
    PublicSiteTestResult {
        provider_name: target.provider_name.clone(),
        key_name: target.key_name.clone(),
        base_url: target.base_url.clone(),
        profile_id: target.profile_id.clone(),
        profile_name: target.profile_name.clone(),
        model: target.configured_model.clone().unwrap_or_default(),
        first_char: None,
        response_preview: None,
        endpoint_used: None,
        latency_ms: None,
        input_tokens: None,
        output_tokens: None,
        is_success: false,
        error: Some(
            target
                .preflight_error
                .clone()
                .unwrap_or_else(|| "Unknown public-test preflight error.".into()),
        ),
    }
}

pub(super) fn public_site_result_detail(result: &PublicSiteTestResult) -> String {
    let latency = result
        .latency_ms
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "n/a".into());
    let endpoint = result.endpoint_used.as_deref().unwrap_or("(none)");
    if result.is_success {
        format!(
            "Provider: {}\nKey: {}\nBase URL: {}\nProfile: {}\nModel: {}\nLatency: {}\nEndpoint: {}\nReply: {}",
            result.provider_name,
            result.key_name,
            result.base_url,
            result.profile_name,
            result.model,
            latency,
            endpoint,
            result.response_preview.as_deref().unwrap_or("(empty)")
        )
    } else {
        format!(
            "Error: {}\n\nProvider: {}\nKey: {}\nBase URL: {}\nProfile: {}\nModel: {}\nLatency: {}",
            result.error.as_deref().unwrap_or("Unknown error"),
            result.provider_name,
            result.key_name,
            result.base_url,
            result.profile_name,
            if result.model.is_empty() {
                "(unresolved)"
            } else {
                &result.model
            },
            latency
        )
    }
}

pub(super) fn public_site_result_detail_lines(result: &PublicSiteTestResult) -> Vec<String> {
    public_site_result_detail(result)
        .lines()
        .map(|line| line.to_string())
        .collect()
}

fn wrapped_visual_line_count(text: &str, width: usize) -> usize {
    let width = width.max(1);
    UnicodeWidthStr::width(text).max(1).div_ceil(width)
}

pub(super) fn public_site_detail_scroll_limit(
    detail_lines: &[String],
    width: u16,
    height: u16,
) -> u16 {
    let visible_height = height.max(1) as usize;
    let width = width.max(1) as usize;
    let total_lines = detail_lines
        .iter()
        .map(|line| wrapped_visual_line_count(line, width))
        .sum::<usize>()
        .max(1);
    total_lines.saturating_sub(visible_height) as u16
}

fn public_site_request_key(target: &PublicSiteTarget, prompt: &str) -> PublicSiteRequestKey {
    PublicSiteRequestKey {
        base_url: normalize_base_url_key(&target.base_url),
        provider_identity: if target.provider_id.is_empty() {
            "inline".to_string()
        } else {
            target.provider_id.clone()
        },
        key_identity: if target.key_id.is_empty() {
            format!("inline:{}", target.api_key)
        } else {
            target.key_id.clone()
        },
        model_identity: target
            .configured_model
            .clone()
            .unwrap_or_else(|| "__DISCOVER__".to_string()),
        prompt: prompt.to_string(),
    }
}

pub(super) fn build_public_site_request_plans(
    targets: &[PublicSiteTarget],
    prompt: &str,
) -> Vec<PublicSiteRequestPlan> {
    let mut plans: BTreeMap<PublicSiteRequestKey, PublicSiteRequestPlan> = BTreeMap::new();

    for target in targets {
        if target.preflight_error.is_some() {
            continue;
        }
        let key = public_site_request_key(target, prompt);
        plans
            .entry(key.clone())
            .and_modify(|plan| plan.consumers.push(target.clone()))
            .or_insert_with(|| PublicSiteRequestPlan {
                key,
                request_target: target.clone(),
                consumers: vec![target.clone()],
            });
    }

    plans.into_values().collect()
}

pub(super) fn fan_out_public_site_result(
    template: &PublicSiteTestResult,
    target: &PublicSiteTarget,
) -> PublicSiteTestResult {
    let mut result = template.clone();
    result.provider_name = target.provider_name.clone();
    result.key_name = target.key_name.clone();
    result.base_url = target.base_url.clone();
    result.profile_id = target.profile_id.clone();
    result.profile_name = target.profile_name.clone();
    result
}

pub(super) fn ellipsize(value: &str, max_chars: usize) -> String {
    let mut shortened = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            shortened.push_str("...");
            return shortened;
        }
        shortened.push(ch);
    }
    shortened
}

pub(super) fn execute_public_site_target_with_timeout(
    target: &PublicSiteTarget,
    prompt: &str,
    timeout: Duration,
) -> PublicSiteTestResult {
    if let Some(error) = target.preflight_error.as_ref() {
        return PublicSiteTestResult {
            provider_name: target.provider_name.clone(),
            key_name: target.key_name.clone(),
            base_url: target.base_url.clone(),
            profile_id: target.profile_id.clone(),
            profile_name: target.profile_name.clone(),
            model: target.configured_model.clone().unwrap_or_default(),
            first_char: None,
            response_preview: None,
            endpoint_used: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            is_success: false,
            error: Some(error.clone()),
        };
    }
    let mut model = target.configured_model.clone();
    if model.is_none() {
        match discover_models_with_timeout(&target.base_url, &target.api_key, timeout) {
            Ok(discovery) => {
                model = discovery
                    .models
                    .into_iter()
                    .find(|candidate| !candidate.trim().is_empty());
                if model.is_some() {
                    thread::sleep(Duration::from_millis(PUBLIC_SITE_TEST_GROUP_GAP_MS));
                }
            }
            Err(failure) => {
                return PublicSiteTestResult {
                    provider_name: target.provider_name.clone(),
                    key_name: target.key_name.clone(),
                    base_url: target.base_url.clone(),
                    profile_id: target.profile_id.clone(),
                    profile_name: target.profile_name.clone(),
                    model: String::new(),
                    first_char: None,
                    response_preview: None,
                    endpoint_used: None,
                    latency_ms: None,
                    input_tokens: None,
                    output_tokens: None,
                    is_success: false,
                    error: Some(format!(
                        "No configured model and model discovery failed: {}",
                        failure.message
                    )),
                };
            }
        }
    }

    let Some(model) = model else {
        return PublicSiteTestResult {
            provider_name: target.provider_name.clone(),
            key_name: target.key_name.clone(),
            base_url: target.base_url.clone(),
            profile_id: target.profile_id.clone(),
            profile_name: target.profile_name.clone(),
            model: String::new(),
            first_char: None,
            response_preview: None,
            endpoint_used: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            is_success: false,
            error: Some("No configured model and discovery returned no models.".into()),
        };
    };

    let started = Instant::now();
    match test_anthropic_message_with_timeout(
        &target.base_url,
        &target.api_key,
        &model,
        prompt,
        timeout,
    ) {
        Ok(response) => PublicSiteTestResult {
            provider_name: target.provider_name.clone(),
            key_name: target.key_name.clone(),
            base_url: target.base_url.clone(),
            profile_id: target.profile_id.clone(),
            profile_name: target.profile_name.clone(),
            model,
            first_char: public_site_first_char(&response.text),
            response_preview: public_site_preview(&response.text, 40),
            endpoint_used: Some(response.endpoint_used),
            latency_ms: Some(started.elapsed().as_millis()),
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            is_success: true,
            error: None,
        },
        Err(error) => PublicSiteTestResult {
            provider_name: target.provider_name.clone(),
            key_name: target.key_name.clone(),
            base_url: target.base_url.clone(),
            profile_id: target.profile_id.clone(),
            profile_name: target.profile_name.clone(),
            model,
            first_char: None,
            response_preview: None,
            endpoint_used: None,
            latency_ms: Some(started.elapsed().as_millis()),
            input_tokens: None,
            output_tokens: None,
            is_success: false,
            error: Some(error.to_string()),
        },
    }
}

pub(super) fn execute_public_site_target(
    target: &PublicSiteTarget,
    prompt: &str,
) -> PublicSiteTestResult {
    execute_public_site_target_with_timeout(target, prompt, public_site_request_timeout())
}
