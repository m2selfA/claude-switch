use super::*;
use std::collections::HashMap;

impl ProfileManager {
    fn normalize_generated_unix_home(home: &str) -> String {
        let mut normalized = home.replace('\\', "/");
        while normalized.ends_with('/') && normalized.len() > 1 {
            normalized.pop();
        }
        normalized
    }

    fn normalize_generated_windows_home(home: &str) -> String {
        let mut normalized = if home.len() >= 3
            && home.as_bytes()[0] == b'/'
            && home.as_bytes()[1].is_ascii_alphabetic()
            && home.as_bytes()[2] == b':'
        {
            home[1..].to_string()
        } else {
            home.to_string()
        };
        normalized = normalized.replace('/', "\\");
        while normalized.ends_with('\\')
            && !(normalized.len() == 3
                && normalized.as_bytes()[1] == b':'
                && normalized.as_bytes()[2] == b'\\')
        {
            normalized.pop();
        }
        normalized
    }

    fn expand_unix_generated_tilde(value: &str, unix_home: &str) -> String {
        if value == "~" {
            return unix_home.to_string();
        }

        let Some(rest) = value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\"))
        else {
            return value.to_string();
        };

        let suffix = rest.trim_start_matches(['/', '\\']).replace('\\', "/");
        if suffix.is_empty() {
            unix_home.to_string()
        } else {
            format!("{unix_home}/{suffix}")
        }
    }

    fn expand_windows_generated_tilde(value: &str, windows_home: &str) -> String {
        if value == "~" {
            return windows_home.to_string();
        }

        let Some(rest) = value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\"))
        else {
            return value.to_string();
        };

        let suffix = rest.trim_start_matches(['/', '\\']).replace('/', "\\");
        if suffix.is_empty() {
            windows_home.to_string()
        } else {
            format!("{windows_home}\\{suffix}")
        }
    }

    fn mcp_pathlike_value_for_target(
        value: &str,
        target_os: RemoteOs,
        target_home: Option<&str>,
    ) -> String {
        let should_expand = value == "~" || value.starts_with("~/") || value.starts_with("~\\");
        if !should_expand {
            return value.to_string();
        }

        match (target_os, target_home) {
            (RemoteOs::Unix, Some(home)) => Self::expand_unix_generated_tilde(value, home),
            (RemoteOs::Windows, Some(home)) => Self::expand_windows_generated_tilde(value, home),
            _ => value.to_string(),
        }
    }

    pub(crate) fn parse_mcp_smart_paste_inputs(raw: &str) -> Result<Vec<McpServerInput>> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("MCP JSON is empty.");
        }

        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(primary_err) => {
                let wrapped = format!("{{{trimmed}}}");
                serde_json::from_str(&wrapped)
                    .with_context(|| format!("Failed to parse MCP JSON: {primary_err}"))?
            }
        };

        Self::mcp_server_inputs_from_paste_value(&value)
    }

    pub(super) fn normalize_mcp_server_type(server_type: &str) -> Result<String> {
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

    fn looks_like_direct_mcp_server(object: &serde_json::Map<String, serde_json::Value>) -> bool {
        object.keys().any(|key| {
            matches!(
                key.as_str(),
                "name"
                    | "type"
                    | "command"
                    | "args"
                    | "env"
                    | "cwd"
                    | "url"
                    | "headers"
                    | "oauth"
                    | "headersHelper"
                    | "timeout"
                    | "alwaysLoad"
                    | "disabled"
            )
        })
    }

    fn mcp_server_inputs_from_named_map(
        object: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Vec<McpServerInput>> {
        if object.is_empty() {
            bail!("MCP JSON does not contain any servers.");
        }

        object
            .iter()
            .map(|(name, value)| Self::mcp_server_input_from_config(name, value))
            .collect()
    }

    fn mcp_server_inputs_from_paste_value(
        value: &serde_json::Value,
    ) -> Result<Vec<McpServerInput>> {
        let Some(object) = value.as_object() else {
            bail!("MCP JSON must be an object.");
        };

        if let Some(servers) = object.get("mcpServers").and_then(|value| value.as_object()) {
            return Self::mcp_server_inputs_from_named_map(servers);
        }

        if Self::looks_like_direct_mcp_server(object) {
            let name = Self::json_string_field(object, "name", "imported-mcp")?
                .unwrap_or_else(|| "imported-mcp".to_string());
            return Ok(vec![Self::mcp_server_input_from_config(&name, value)?]);
        }

        if object.values().all(serde_json::Value::is_object) {
            return Self::mcp_server_inputs_from_named_map(object);
        }

        bail!(
            "Paste a single MCP server object, a named MCP entry, or an object containing mcpServers."
        )
    }

    pub(super) fn mcp_server_input_from_config(
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

    pub(super) fn build_mcp_server(id: String, input: McpServerInput) -> Result<McpServer> {
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

    fn mcp_server_config_value_for_target(
        server: &McpServer,
        target_os: RemoteOs,
        target_home: Option<&str>,
    ) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(
            "type".into(),
            serde_json::Value::String(server.server_type.clone()),
        );
        if let Some(command) = &server.command {
            object.insert(
                "command".into(),
                serde_json::Value::String(Self::mcp_pathlike_value_for_target(
                    command,
                    target_os,
                    target_home,
                )),
            );
        }
        if !server.args.is_empty() {
            object.insert(
                "args".into(),
                serde_json::json!(
                    server
                        .args
                        .iter()
                        .map(|arg| Self::mcp_pathlike_value_for_target(arg, target_os, target_home))
                        .collect::<Vec<_>>()
                ),
            );
        }
        if !server.env.is_empty() {
            let env = server
                .env
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        Self::mcp_pathlike_value_for_target(value, target_os, target_home),
                    )
                })
                .collect::<HashMap<_, _>>();
            object.insert("env".into(), serde_json::json!(env));
        }
        if let Some(cwd) = &server.cwd {
            object.insert(
                "cwd".into(),
                serde_json::Value::String(Self::mcp_pathlike_value_for_target(
                    cwd,
                    target_os,
                    target_home,
                )),
            );
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
                serde_json::Value::String(Self::mcp_pathlike_value_for_target(
                    headers_helper,
                    target_os,
                    target_home,
                )),
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

    pub(super) fn profile_mcp_servers(&self, profile: &Profile) -> Result<Vec<McpServer>> {
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

    pub(super) fn profile_mcp_config(servers: &[McpServer]) -> Result<String> {
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

    pub(super) fn profile_mcp_config_for_target(
        servers: &[McpServer],
        target_os: RemoteOs,
        target_home: Option<&str>,
    ) -> Result<String> {
        let normalized_target_home = match target_os {
            RemoteOs::Unix => target_home.map(Self::normalize_generated_unix_home),
            RemoteOs::Windows => target_home.map(Self::normalize_generated_windows_home),
        };
        let target_home = normalized_target_home.as_deref();

        let mut mcp_servers = serde_json::Map::new();
        for server in servers {
            if mcp_servers.contains_key(&server.name) {
                bail!(
                    "Duplicate MCP server name '{}' in profile selection.",
                    server.name
                );
            }
            mcp_servers.insert(
                server.name.clone(),
                Self::mcp_server_config_value_for_target(server, target_os, target_home),
            );
        }
        let root = serde_json::json!({
            "$schema": "https://json.schemastore.org/claude-code-settings.json",
            "mcpServers": mcp_servers,
        });
        serde_json::to_string_pretty(&root).context("Failed to serialize MCP config JSON")
    }
}
