use anyhow::{Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SmartProviderPaste {
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) key_name: String,
    pub(super) api_key: String,
}

pub(super) fn parse_provider_smart_paste(raw: &str) -> Result<SmartProviderPaste> {
    let input = raw.trim();
    if input.starts_with("https://app.nextchat.dev/#/?settings=") {
        return parse_nextchat_settings_url(input);
    }
    if input.starts_with("opencat://team/join?") {
        return parse_opencat_join_url(input);
    }
    if input.starts_with("cherrystudio://providers/api-keys") {
        return parse_cherrystudio_provider_url(input);
    }
    parse_newapi_provider_json(input)
}

fn parse_newapi_provider_json(input: &str) -> Result<SmartProviderPaste> {
    let value: serde_json::Value = serde_json::from_str(input)?;
    let base_url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("Smart paste needs JSON fields 'url' and 'key'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn parse_cherrystudio_provider_url(input: &str) -> Result<SmartProviderPaste> {
    let data = input
        .split_once('?')
        .map(|(_, query)| query)
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                (key == "data").then_some(value)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Cherry Studio URL is missing data=."))?;
    let decoded_param = percent_decode(data)?;
    let decoded = URL_SAFE_NO_PAD.decode(decoded_param.as_bytes())?;
    let value: serde_json::Value = serde_json::from_slice(&decoded)?;

    let base_url = value
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("apiKey")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("Cherry Studio data needs 'baseUrl' and 'apiKey'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Default")
            .to_string(),
        api_key,
    })
}

fn parse_nextchat_settings_url(input: &str) -> Result<SmartProviderPaste> {
    let encoded = input
        .split_once("#/?settings=")
        .map(|(_, value)| value)
        .ok_or_else(|| anyhow::anyhow!("NextChat URL is missing settings=."))?;
    let decoded = percent_decode(encoded)?;
    let value: serde_json::Value = serde_json::from_str(&decoded)?;
    let base_url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("NextChat settings need 'url' and 'key'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn parse_opencat_join_url(input: &str) -> Result<SmartProviderPaste> {
    let query = input
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| anyhow::anyhow!("OpenCat URL is missing query params."))?;
    let mut base_url = String::new();
    let mut api_key = String::new();
    for part in query.split('&') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(value)?;
        match key {
            "domain" => base_url = decoded.trim().to_string(),
            "token" => api_key = decoded.trim().to_string(),
            _ => {}
        }
    }
    if base_url.is_empty() || api_key.is_empty() {
        bail!("OpenCat join URL needs 'domain' and 'token'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
                let value = u8::from_str_radix(hex, 16)?;
                out.push(value);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    Ok(String::from_utf8(out)?)
}

pub(super) fn inferred_provider_name(base_url: &str) -> String {
    base_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Provider")
        .to_string()
}
