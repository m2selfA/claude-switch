use super::*;

impl ProfileManager {
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

    pub(super) fn validate_mcp_server_config(server: &McpServer) -> Vec<McpValidationIssue> {
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
}
