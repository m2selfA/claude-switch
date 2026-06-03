use super::*;

impl ProfileManager {
    pub(super) fn check_mcp_name_unique_in_registry(
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

    pub(super) fn selected_mcp_servers_in_registry(
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
}
