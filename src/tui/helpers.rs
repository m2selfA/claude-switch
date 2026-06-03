use super::*;

pub(super) fn launch_args_from_str(s: &str) -> Option<Vec<String>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.split_whitespace().map(String::from).collect())
    }
}

pub(super) fn optional_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn optional_bool_label(value: Option<bool>) -> String {
    match value {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => "(default)".to_string(),
    }
}

pub(super) fn next_optional_bool(value: Option<bool>) -> Option<bool> {
    match value {
        None => Some(true),
        Some(true) => Some(false),
        Some(false) => None,
    }
}

pub(super) fn empty_label(value: &str) -> String {
    if value.is_empty() {
        "(empty)".to_string()
    } else {
        value.to_string()
    }
}

pub(super) fn mcp_type_index(value: &str) -> usize {
    MCP_TYPES
        .iter()
        .position(|server_type| *server_type == value)
        .unwrap_or(0)
}

pub(super) fn map_to_entries(map: &HashMap<String, String>) -> Vec<String> {
    let mut entries: Vec<_> = map
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    entries.sort();
    entries
}

pub(super) fn entries_to_map(entries: &[String], field: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for entry in entries {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("MCP {} entry must be KEY=VALUE: {}", field, entry))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("MCP {} entry has an empty key.", field);
        }
        map.insert(key.to_string(), value.trim().to_string());
    }
    Ok(map)
}

pub(super) fn parse_mcp_smart_paste(raw: &str) -> Result<McpServerInput> {
    let value: serde_json::Value = serde_json::from_str(raw.trim())?;
    if let Some(servers) = value.get("mcpServers").and_then(|value| value.as_object()) {
        if servers.len() != 1 {
            bail!("Import expects exactly one mcpServers entry.");
        }
        let (name, config) = servers.iter().next().expect("len checked above");
        return mcp_input_from_json(name, config);
    }
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("imported-mcp");
    mcp_input_from_json(name, &value)
}

fn mcp_input_from_json(name: &str, value: &serde_json::Value) -> Result<McpServerInput> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("MCP JSON must be an object."))?;
    let server_type = object
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("stdio")
        .to_string();
    let args = object
        .get("args")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerInput {
        name: name.to_string(),
        server_type,
        command: object
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        args,
        env: json_string_map(object.get("env"))?,
        cwd: object
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        url: object
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        headers: json_string_map(object.get("headers"))?,
        oauth: object.get("oauth").cloned(),
        headers_helper: object
            .get("headersHelper")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        timeout: object.get("timeout").and_then(|value| value.as_u64()),
        always_load: object.get("alwaysLoad").and_then(|value| value.as_bool()),
        disabled: object.get("disabled").and_then(|value| value.as_bool()),
    })
}

fn json_string_map(value: Option<&serde_json::Value>) -> Result<HashMap<String, String>> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected an object with string values."))?;
    let mut map = HashMap::new();
    for (key, value) in object {
        let Some(value) = value.as_str() else {
            bail!("Value for '{}' must be a string.", key);
        };
        map.insert(key.clone(), value.to_string());
    }
    Ok(map)
}

pub(super) fn replace_last_word(s: &str, replacement: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return replacement.to_string();
    }
    if let Some(last_space) = trimmed.rfind(char::is_whitespace) {
        format!("{} {}", &trimmed[..last_space], replacement)
    } else {
        replacement.to_string()
    }
}

pub(super) fn trim_model_context_suffix(model: &str) -> &str {
    strip_model_1m_suffix(model)
}

pub(super) fn model_has_1m_suffix(model: &str) -> bool {
    model.trim_end().ends_with("[1m]")
}

pub(super) fn strip_model_1m_suffix(model: &str) -> &str {
    let trimmed = model.trim_end();
    trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim_end()
}

pub(super) fn apply_model_1m_flag(model: &str, enabled: bool) -> String {
    let base = strip_model_1m_suffix(model).trim_end().to_string();
    if base.is_empty() {
        return base;
    }
    if enabled {
        format!("{}[1m]", base)
    } else {
        base
    }
}

pub(super) fn display_ellipsize(s: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }

    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let target_width = max_width - 3;
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > target_width {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out.push_str("...");
    out
}

pub(super) fn display_pad(s: &str, width: usize) -> String {
    let value = display_ellipsize(s, width);
    let value_width = UnicodeWidthStr::width(value.as_str());
    if value_width >= width {
        value
    } else {
        format!("{}{}", value, " ".repeat(width - value_width))
    }
}

pub(super) fn mask_api_key(api_key: &str) -> String {
    if api_key.chars().count() <= 12 {
        return api_key.to_string();
    }

    let prefix: String = api_key.chars().take(6).collect();
    let suffix: String = api_key
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

pub(super) fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width: w,
        height: height.min(area.height),
    }
}
