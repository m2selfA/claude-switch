use super::*;
use std::collections::HashMap;

impl ProfileManager {
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
        for (key, value) in map.iter_mut() {
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
                    incoming_items.iter_mut().zip(existing_items)
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

    pub(super) fn redact_mcp_server_secrets(server: &mut McpServer) {
        Self::redact_mcp_string_map_secrets(&mut server.env);
        Self::redact_mcp_string_map_secrets(&mut server.headers);
        if let Some(oauth) = &mut server.oauth {
            Self::redact_mcp_json_secrets(oauth, false);
        }
    }

    pub(super) fn mcp_server_has_secrets(server: &McpServer) -> bool {
        Self::mcp_string_map_has_secrets(&server.env)
            || Self::mcp_string_map_has_secrets(&server.headers)
            || server
                .oauth
                .as_ref()
                .is_some_and(|oauth| Self::mcp_json_has_secrets(oauth, false))
    }

    pub(super) fn preserve_mcp_server_secrets(incoming: &mut McpServer, existing: &McpServer) {
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
}
