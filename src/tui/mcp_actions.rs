use super::*;

impl App {
    pub(super) fn handle_mcp_page_normal_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        match code {
            _ if Self::is_prev_list_key(code, modifiers) => self.move_mcp_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_mcp_down(),
            KeyCode::Char('a') => {
                self.reset_mcp_editor();
                self.mode = Mode::McpAdd { step: 0 };
            }
            KeyCode::Char('e') => {
                if let Some(mcp) = self.selected_mcp().cloned() {
                    self.load_mcp_editor(&mcp);
                    self.mode = Mode::McpEdit {
                        mcp_id: mcp.id,
                        step: 0,
                    };
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(mcp) = self.selected_mcp() {
                    self.mode = Mode::ConfirmDeleteMcp {
                        mcp_id: mcp.id.clone(),
                        name: mcp.name.clone(),
                    };
                }
            }
            KeyCode::Enter => {
                self.refresh_mcp_profile_links();
                self.show_message(
                    format!("{} linked profile(s)", self.mcp_profile_links_cache.len()),
                    false,
                    Some(Mode::Normal),
                );
            }
            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_mcp_smart_paste();
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_mcp_editor(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (is_edit, step) = match &self.mode {
            Mode::McpAdd { step } => (false, *step),
            Mode::McpEdit { step, .. } => (true, *step),
            _ => return Ok(()),
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_next_field_key(code, modifiers) => {
                let next = (step + 1) % MCP_EDITOR_STEPS;
                self.cursor_pos = self.mcp_editor_cursor_pos(next);
                self.mode = match &self.mode {
                    Mode::McpEdit { mcp_id, .. } => Mode::McpEdit {
                        mcp_id: mcp_id.clone(),
                        step: next,
                    },
                    _ => Mode::McpAdd { step: next },
                };
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                let next = (step + MCP_EDITOR_STEPS - 1) % MCP_EDITOR_STEPS;
                self.cursor_pos = self.mcp_editor_cursor_pos(next);
                self.mode = match &self.mode {
                    Mode::McpEdit { mcp_id, .. } => Mode::McpEdit {
                        mcp_id: mcp_id.clone(),
                        step: next,
                    },
                    _ => Mode::McpAdd { step: next },
                };
            }
            KeyCode::Tab => match step {
                1 => {
                    self.mcp_type_idx = (self.mcp_type_idx + 1) % MCP_TYPES.len();
                }
                11 => {
                    self.mcp_always_load = next_optional_bool(self.mcp_always_load);
                }
                12 => {
                    self.mcp_disabled = next_optional_bool(self.mcp_disabled);
                }
                _ => {
                    let next = (step + 1) % MCP_EDITOR_STEPS;
                    self.cursor_pos = self.mcp_editor_cursor_pos(next);
                    self.mode = match &self.mode {
                        Mode::McpEdit { mcp_id, .. } => Mode::McpEdit {
                            mcp_id: mcp_id.clone(),
                            step: next,
                        },
                        _ => Mode::McpAdd { step: next },
                    };
                }
            },
            KeyCode::Enter => match step {
                3 => {
                    let value = self.input_buffer.trim().to_string();
                    if !value.is_empty() {
                        self.mcp_args.push(value);
                        self.input_buffer.clear();
                        self.cursor_pos = 0;
                    }
                }
                4 => {
                    let value = self.input_buffer.trim().to_string();
                    if value.contains('=') {
                        self.mcp_env.push(value);
                        self.input_buffer.clear();
                        self.cursor_pos = 0;
                    }
                }
                7 => {
                    let value = self.input_buffer.trim().to_string();
                    if value.contains('=') {
                        self.mcp_headers.push(value);
                        self.input_buffer.clear();
                        self.cursor_pos = 0;
                    }
                }
                _ => {
                    let input = self.current_mcp_input()?;
                    let result = if is_edit {
                        let mcp_id = match &self.mode {
                            Mode::McpEdit { mcp_id, .. } => mcp_id.clone(),
                            _ => return Ok(()),
                        };
                        self.manager.update_mcp_server(
                            &mcp_id,
                            McpServerUpdate {
                                name: Some(input.name),
                                server_type: Some(input.server_type),
                                command: Some(input.command),
                                args: Some(input.args),
                                env: Some(input.env),
                                cwd: Some(input.cwd),
                                url: Some(input.url),
                                headers: Some(input.headers),
                                oauth: Some(input.oauth),
                                headers_helper: Some(input.headers_helper),
                                timeout: Some(input.timeout),
                                always_load: Some(input.always_load),
                                disabled: Some(input.disabled),
                            },
                        )
                    } else {
                        self.manager.add_mcp_server(input)
                    };
                    match result {
                        Ok(server) => {
                            self.sync_shims();
                            self.mcps_cache = self.manager.list_mcp_servers().unwrap_or_default();
                            if let Some(idx) =
                                self.mcps_cache.iter().position(|m| m.id == server.id)
                            {
                                self.mcp_list_state.select(Some(idx));
                            }
                            self.refresh_mcp_profile_links();
                            self.mode =
                                Mode::Message(format!("MCP '{}' saved.", server.name), false);
                        }
                        Err(error) => self.mode = Mode::Message(error.to_string(), true),
                    }
                }
            },
            KeyCode::Backspace => match step {
                3 if self.input_buffer.is_empty() => {
                    self.mcp_args.pop();
                }
                4 if self.input_buffer.is_empty() => {
                    self.mcp_env.pop();
                }
                7 if self.input_buffer.is_empty() => {
                    self.mcp_headers.pop();
                }
                _ => self.edit_mcp_text_field(code, modifiers, step),
            },
            _ => self.edit_mcp_text_field(code, modifiers, step),
        }
        Ok(())
    }

    pub(super) fn edit_mcp_text_field(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        step: usize,
    ) {
        match step {
            0 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_name_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            2 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_command_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            3 | 4 | 7 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.input_buffer,
                    &mut self.cursor_pos,
                    true,
                );
            }
            5 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_cwd_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            6 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_url_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            8 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_oauth_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            9 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_headers_helper_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            10 => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_timeout_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
            _ => {}
        }
    }

    pub(super) fn filtered_mcp_indices(&self) -> Vec<usize> {
        let q = self.mcp_filter_buf.to_lowercase();
        self.mcps_cache
            .iter()
            .enumerate()
            .filter(|(_, mcp)| {
                q.is_empty()
                    || mcp.name.to_lowercase().contains(&q)
                    || mcp.id.to_lowercase().contains(&q)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    pub(super) fn handle_mcp_profile_picker(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let profile_id = match &self.mode {
            Mode::McpProfilePicker { profile_id } => profile_id.clone(),
            _ => return Ok(()),
        };
        let filtered = self.filtered_mcp_indices();
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_prev_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.mcp_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                let next = if pos == 0 {
                    filtered.len() - 1
                } else {
                    pos - 1
                };
                self.mcp_list_state.select(Some(filtered[next]));
            }
            _ if Self::is_next_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.mcp_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                self.mcp_list_state
                    .select(Some(filtered[(pos + 1) % filtered.len()]));
            }
            KeyCode::Char(' ') => {
                if let Some(idx) = self.mcp_list_state.selected()
                    && let Some(mcp) = self.mcps_cache.get(idx)
                {
                    if self.mcp_selected_ids.iter().any(|id| id == &mcp.id) {
                        self.mcp_selected_ids.retain(|id| id != &mcp.id);
                    } else {
                        self.mcp_selected_ids.push(mcp.id.clone());
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(idx) = filtered.first()
                    && let Some(mcp) = self.mcps_cache.get(*idx)
                {
                    self.mcp_filter_buf = mcp.name.clone();
                    self.cursor_pos = self.mcp_filter_buf.len();
                    self.mcp_list_state.select(Some(*idx));
                }
            }
            KeyCode::Enter => {
                let selected = self.mcp_selected_ids.clone();
                match self.manager.set_profile_mcps(&profile_id, &selected) {
                    Ok(profile) => {
                        self.sync_shims();
                        self.refresh()?;
                        self.select_by_id(&profile.id);
                        self.mode = Mode::Message(
                            format!(
                                "Profile '{}' now has {} MCP server(s).",
                                profile.name,
                                profile.mcp_server_ids.len()
                            ),
                            false,
                        );
                    }
                    Err(error) => self.mode = Mode::Message(error.to_string(), true),
                }
            }
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_filter_buf,
                    &mut self.cursor_pos,
                    true,
                );
                if let Some(idx) = self.filtered_mcp_indices().first() {
                    self.mcp_list_state.select(Some(*idx));
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_mcp_smart_paste(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            KeyCode::Enter => match parse_mcp_smart_paste(&self.mcp_oauth_buf) {
                Ok(inputs) => match self.manager.import_mcp_servers_skip_existing(inputs) {
                    Ok(result) => {
                        self.sync_shims();
                        self.mcps_cache = self.manager.list_mcp_servers().unwrap_or_default();
                        if let Some(server) = result.imported.first()
                            && let Some(idx) =
                                self.mcps_cache.iter().position(|m| m.id == server.id)
                        {
                            self.mcp_list_state.select(Some(idx));
                        }
                        self.refresh_mcp_profile_links();
                        self.mode = Mode::Message(Self::mcp_smart_paste_summary(&result), false);
                    }
                    Err(error) => self.mode = Mode::Message(error.to_string(), true),
                },
                Err(error) => self.mode = Mode::Message(error.to_string(), true),
            },
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.mcp_oauth_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    fn mcp_smart_paste_summary(result: &crate::profile::McpSmartPasteImportResult) -> String {
        let skipped_preview = |names: &[String]| {
            let preview = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if names.len() > 3 {
                format!("{preview}, +{} more", names.len() - 3)
            } else {
                preview
            }
        };

        match (
            result.imported.as_slice(),
            result.skipped_existing.as_slice(),
        ) {
            ([server], []) => format!("MCP '{}' imported.", server.name),
            ([], skipped) => format!(
                "Skipped {} existing MCP(s): {}.",
                skipped.len(),
                skipped_preview(skipped)
            ),
            (imported, []) => format!("Imported {} MCP(s).", imported.len()),
            (imported, skipped) => format!(
                "Imported {} MCP(s); skipped {} existing: {}.",
                imported.len(),
                skipped.len(),
                skipped_preview(skipped)
            ),
        }
    }

    pub(super) fn handle_confirm_delete_mcp(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let mcp_id = match &self.mode {
                    Mode::ConfirmDeleteMcp { mcp_id, .. } => mcp_id.clone(),
                    _ => return Ok(()),
                };
                match self.manager.remove_mcp_server(&mcp_id) {
                    Ok(_) => {
                        self.sync_shims();
                        self.mcps_cache = self.manager.list_mcp_servers().unwrap_or_default();
                        if self.mcp_list_state.selected().unwrap_or(0) >= self.mcps_cache.len()
                            && !self.mcps_cache.is_empty()
                        {
                            self.mcp_list_state.select(Some(self.mcps_cache.len() - 1));
                        }
                        self.refresh_mcp_profile_links();
                        self.mode = Mode::Normal;
                    }
                    Err(error) => self.mode = Mode::Message(error.to_string(), true),
                }
            }
            _ => self.mode = Mode::Normal,
        }
        Ok(())
    }

    pub(super) fn selected_mcp(&self) -> Option<&McpServer> {
        self.mcp_list_state
            .selected()
            .and_then(|idx| self.mcps_cache.get(idx))
            .or_else(|| self.mcps_cache.first())
    }

    pub(super) fn move_mcp_up(&mut self) {
        if self.mcps_cache.is_empty() {
            return;
        }
        let i = match self.mcp_list_state.selected() {
            Some(0) | None => self.mcps_cache.len() - 1,
            Some(i) => i - 1,
        };
        self.mcp_list_state.select(Some(i));
        self.mcp_list_scroll = self.mcp_list_scroll.position(i);
        self.refresh_mcp_profile_links();
    }

    pub(super) fn move_mcp_down(&mut self) {
        if self.mcps_cache.is_empty() {
            return;
        }
        let i = match self.mcp_list_state.selected() {
            Some(i) => (i + 1) % self.mcps_cache.len(),
            None => 0,
        };
        self.mcp_list_state.select(Some(i));
        self.mcp_list_scroll = self.mcp_list_scroll.position(i);
        self.refresh_mcp_profile_links();
    }

    pub(super) fn refresh_mcp_profile_links(&mut self) {
        self.mcp_profile_links_cache = self
            .selected_mcp()
            .and_then(|mcp| self.manager.list_profiles_using_mcp(&mcp.id).ok())
            .unwrap_or_default();
    }

    pub(super) fn reset_mcp_editor(&mut self) {
        self.mcp_name_buf.clear();
        self.mcp_type_idx = 0;
        self.mcp_command_buf.clear();
        self.mcp_args.clear();
        self.mcp_env.clear();
        self.mcp_cwd_buf.clear();
        self.mcp_url_buf.clear();
        self.mcp_headers.clear();
        self.mcp_oauth_buf.clear();
        self.mcp_headers_helper_buf.clear();
        self.mcp_timeout_buf.clear();
        self.mcp_always_load = None;
        self.mcp_disabled = None;
        self.input_buffer.clear();
        self.cursor_pos = 0;
    }

    pub(super) fn load_mcp_editor(&mut self, server: &McpServer) {
        self.mcp_name_buf = server.name.clone();
        self.mcp_type_idx = mcp_type_index(&server.server_type);
        self.mcp_command_buf = server.command.clone().unwrap_or_default();
        self.mcp_args = server.args.clone();
        self.mcp_env = map_to_entries(&server.env);
        self.mcp_cwd_buf = server.cwd.clone().unwrap_or_default();
        self.mcp_url_buf = server.url.clone().unwrap_or_default();
        self.mcp_headers = map_to_entries(&server.headers);
        self.mcp_oauth_buf = server
            .oauth
            .as_ref()
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_default())
            .unwrap_or_default();
        self.mcp_headers_helper_buf = server.headers_helper.clone().unwrap_or_default();
        self.mcp_timeout_buf = server
            .timeout
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.mcp_always_load = server.always_load;
        self.mcp_disabled = server.disabled;
        self.input_buffer.clear();
        self.cursor_pos = self.mcp_name_buf.len();
    }

    pub(super) fn mcp_editor_cursor_pos(&self, step: usize) -> usize {
        match step {
            0 => self.mcp_name_buf.len(),
            2 => self.mcp_command_buf.len(),
            3 | 4 | 7 => self.input_buffer.len(),
            5 => self.mcp_cwd_buf.len(),
            6 => self.mcp_url_buf.len(),
            8 => self.mcp_oauth_buf.len(),
            9 => self.mcp_headers_helper_buf.len(),
            10 => self.mcp_timeout_buf.len(),
            _ => 0,
        }
    }

    pub(super) fn paste_into_mcp_editor(&mut self, text: &str) {
        let step = match &self.mode {
            Mode::McpAdd { step } | Mode::McpEdit { step, .. } => *step,
            _ => return,
        };
        match step {
            0 => insert_str_at_cursor(&mut self.mcp_name_buf, &mut self.cursor_pos, text),
            2 => insert_str_at_cursor(&mut self.mcp_command_buf, &mut self.cursor_pos, text),
            3 | 4 | 7 => insert_str_at_cursor(&mut self.input_buffer, &mut self.cursor_pos, text),
            5 => insert_str_at_cursor(&mut self.mcp_cwd_buf, &mut self.cursor_pos, text),
            6 => insert_str_at_cursor(&mut self.mcp_url_buf, &mut self.cursor_pos, text),
            8 => insert_str_at_cursor(&mut self.mcp_oauth_buf, &mut self.cursor_pos, text),
            9 => insert_str_at_cursor(&mut self.mcp_headers_helper_buf, &mut self.cursor_pos, text),
            10 => insert_str_at_cursor(&mut self.mcp_timeout_buf, &mut self.cursor_pos, text),
            _ => {}
        }
    }

    pub(super) fn current_mcp_input(&self) -> Result<McpServerInput> {
        let oauth = if self.mcp_oauth_buf.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(self.mcp_oauth_buf.trim())
                    .map_err(|err| anyhow::anyhow!("OAuth JSON is invalid: {}", err))?,
            )
        };
        let timeout = if self.mcp_timeout_buf.trim().is_empty() {
            None
        } else {
            Some(
                self.mcp_timeout_buf
                    .trim()
                    .parse::<u64>()
                    .map_err(|err| anyhow::anyhow!("Timeout must be a number: {}", err))?,
            )
        };
        Ok(McpServerInput {
            name: self.mcp_name_buf.trim().to_string(),
            server_type: MCP_TYPES[self.mcp_type_idx % MCP_TYPES.len()].to_string(),
            command: optional_non_empty(self.mcp_command_buf.trim()),
            args: self.mcp_args.clone(),
            env: entries_to_map(&self.mcp_env, "env")?,
            cwd: optional_non_empty(self.mcp_cwd_buf.trim()),
            url: optional_non_empty(self.mcp_url_buf.trim()),
            headers: entries_to_map(&self.mcp_headers, "headers")?,
            oauth,
            headers_helper: optional_non_empty(self.mcp_headers_helper_buf.trim()),
            timeout,
            always_load: self.mcp_always_load,
            disabled: self.mcp_disabled,
        })
    }

    pub(super) fn start_selected_profile_mcp_picker(&mut self) -> Result<()> {
        let Some(profile) = self.selected_profile().cloned() else {
            return Ok(());
        };
        if profile.kind != ProfileKind::Lightweight {
            self.show_message(
                "MCP servers can only be linked to lightweight profiles.".into(),
                true,
                None,
            );
            return Ok(());
        }
        self.mcps_cache = self.manager.list_mcp_servers().unwrap_or_default();
        if self.mcps_cache.is_empty() {
            self.show_message(
                "No MCP servers found. Add one in MCP Manager first.".into(),
                true,
                None,
            );
            return Ok(());
        }
        self.mcp_selected_ids = profile.mcp_server_ids.clone();
        self.mcp_filter_buf.clear();
        self.cursor_pos = 0;
        self.mcp_list_state = ListState::default();
        self.mcp_list_state.select(Some(0));
        self.mode = Mode::McpProfilePicker {
            profile_id: profile.id,
        };
        Ok(())
    }

    pub(super) fn start_mcp_smart_paste(&mut self) {
        self.reset_mcp_editor();
        match Clipboard::new().and_then(|mut clip| clip.get_text()) {
            Ok(text) if !text.trim().is_empty() => {
                self.mcp_oauth_buf = text;
                self.cursor_pos = self.mcp_oauth_buf.len();
            }
            _ => {}
        }
        self.mode = Mode::McpSmartPaste;
    }
}
