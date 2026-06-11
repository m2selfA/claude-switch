use super::*;

impl App {
    pub(super) fn refresh(&mut self) -> Result<()> {
        self.profiles = self.manager.list_profiles()?;
        self.apply_filter();
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
            self.list_scroll = ScrollbarState::default();
        } else {
            let idx = self.list_state.selected().unwrap_or(0);
            self.list_state
                .select(Some(idx.min(self.filtered_indices.len() - 1)));
            self.list_scroll = self
                .list_scroll
                .content_length(self.filtered_indices.len())
                .position(idx);
        }
        Ok(())
    }

    pub(super) fn apply_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.filtered_indices = (0..self.profiles.len()).collect();
        } else {
            self.filtered_indices = self
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.name.to_lowercase().contains(&q)
                        || p.alias
                            .as_deref()
                            .map(|a| a.to_lowercase().contains(&q))
                            .unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
            self.list_scroll = ScrollbarState::default();
        } else {
            let sel = self.list_state.selected().unwrap_or(0);
            self.list_state
                .select(Some(sel.min(self.filtered_indices.len() - 1)));
            self.list_scroll = self
                .list_scroll
                .content_length(self.filtered_indices.len())
                .position(sel.min(self.filtered_indices.len() - 1));
        }
    }

    pub(super) fn is_manager_switch_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::BackTab)
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT))
    }

    pub(super) fn mode_allows_manager_switch(&self) -> bool {
        matches!(self.mode, Mode::Normal | Mode::Search | Mode::ProviderList)
    }

    pub(super) fn switch_manager_page(&mut self) -> Result<()> {
        self.page = match self.page {
            Page::Profile => {
                self.providers_cache = self.manager.list_providers().unwrap_or_default();
                self.provider_list_state = ListState::default();
                if !self.providers_cache.is_empty() {
                    self.provider_list_state.select(Some(0));
                }
                Page::Provider
            }
            Page::Provider => {
                self.mcps_cache = self.manager.list_mcp_servers().unwrap_or_default();
                self.mcp_list_state = ListState::default();
                if !self.mcps_cache.is_empty() {
                    self.mcp_list_state.select(Some(0));
                }
                self.refresh_mcp_profile_links();
                Page::Mcp
            }
            Page::Mcp => {
                self.refresh_plugin_state();
                Page::Plugin
            }
            Page::Plugin => {
                self.settings_allow_local_runtime_hot_switch = self
                    .manager
                    .allow_local_runtime_hot_switch()
                    .unwrap_or(false);
                Page::Settings
            }
            Page::Settings => {
                self.refresh()?;
                Page::Profile
            }
        };
        self.mode = Mode::Normal;
        Ok(())
    }

    pub(super) fn handle_manager_switch_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        if !Self::is_manager_switch_key(code, modifiers) || !self.mode_allows_manager_switch() {
            return Ok(false);
        }
        self.switch_manager_page()?;
        Ok(true)
    }

    pub(super) fn is_cancel_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Esc)
            || (code == KeyCode::Char('g') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_prev_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up | KeyCode::Char('k'))
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_next_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down | KeyCode::Char('j'))
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_prev_filter_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up)
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_next_filter_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down)
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_prev_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up)
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn is_next_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down)
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    pub(super) fn select_by_id(&mut self, id: &str) {
        if let Some(fi) = self
            .filtered_indices
            .iter()
            .position(|&i| self.profiles[i].id == id)
        {
            self.list_state.select(Some(fi));
            self.list_scroll = self.list_scroll.position(fi);
        }
    }

    pub(super) fn selected_profile(&self) -> Option<&Profile> {
        self.list_state
            .selected()
            .and_then(|fi| self.filtered_indices.get(fi))
            .and_then(|&i| self.profiles.get(i))
    }

    pub fn run(mut self) -> Result<()> {
        let mut terminal = ratatui::init();
        terminal.clear()?;
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        result
    }

    pub(super) fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            self.poll_public_site_worker_events();
            terminal.draw(|f| self.render(f))?;

            if !event::poll(Duration::from_millis(PUBLIC_SITE_EVENT_POLL_MS))? {
                continue;
            }

            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    if self.handle_manager_switch_key(key.code, key.modifiers)? {
                        continue;
                    }

                    match &self.mode.clone() {
                        Mode::FirstRun => {
                            if self.handle_first_run_key(key.code, key.modifiers)? {
                                return Ok(());
                            }
                        }
                        Mode::Normal => {
                            if self.handle_normal_key(key.code, key.modifiers)? {
                                return Ok(());
                            }
                        }
                        Mode::Search => {
                            if self.handle_search_key(key.code, key.modifiers)? {
                                return Ok(());
                            }
                        }
                        Mode::Help => {
                            self.mode = Mode::Normal;
                        }
                        Mode::ConfirmDelete => {
                            self.handle_confirm_delete(key.code)?;
                        }
                        Mode::AddFullName => {
                            self.handle_add_full_name(key.code, key.modifiers)?;
                        }
                        Mode::AddFullAlias => {
                            self.handle_add_full_alias(key.code, key.modifiers)?;
                        }
                        Mode::DuplicateProfileName { .. } => {
                            self.handle_duplicate_profile_name(key.code, key.modifiers)?;
                        }
                        Mode::DuplicateProfileAlias { .. } => {
                            self.handle_duplicate_profile_alias(key.code, key.modifiers)?;
                        }
                        Mode::LiteProviderSelect => {
                            lite::handle_lite_provider_select(self, key.code, key.modifiers)?;
                        }
                        Mode::LiteKeySelect { .. } => {
                            lite::handle_lite_key_select(self, key.code, key.modifiers)?;
                        }
                        Mode::LiteFetching => {
                            if Self::is_cancel_key(key.code, key.modifiers) {
                                self.mode = Mode::Normal;
                            }
                        }
                        Mode::LiteModelSelect { .. } => {
                            lite::handle_lite_model_select(self, key.code, key.modifiers)?;
                        }
                        Mode::LiteEdit { .. } => {
                            lite::handle_lite_model_select(self, key.code, key.modifiers)?;
                        }
                        Mode::ProviderAnthropicTest { .. } => {
                            self.handle_provider_anthropic_test(key.code, key.modifiers)?;
                        }
                        Mode::ProviderAnthropicOutcome { .. } => {
                            self.handle_provider_anthropic_outcome(key.code, key.modifiers)?;
                        }
                        Mode::ProcessSwitchPicker { .. } => {
                            self.handle_process_switch_picker(key.code, key.modifiers)?;
                        }
                        Mode::ProcessSwitchModelConfirm { .. } => {
                            self.handle_process_switch_model_confirm(key.code, key.modifiers)?;
                        }
                        Mode::LocalGatewayLaunchPicker { .. } => {
                            self.handle_local_gateway_launch_mode(key.code, key.modifiers)?;
                        }
                        Mode::EditProfile { .. } => {
                            self.handle_edit_profile(key.code, key.modifiers)?;
                        }
                        Mode::ProviderList => {
                            self.handle_provider_list(key.code, key.modifiers)?;
                        }
                        Mode::ProviderAdd { .. } => {
                            self.handle_provider_add(key.code, key.modifiers)?;
                        }
                        Mode::ProviderSmartPaste => {
                            self.handle_provider_smart_paste(key.code, key.modifiers)?;
                        }
                        Mode::ProviderEdit { .. } => {
                            self.handle_provider_edit(key.code, key.modifiers)?;
                        }
                        Mode::ProviderEditKeyInput { .. } => {
                            self.handle_provider_edit_key_input(key.code, key.modifiers)?;
                        }
                        Mode::ProviderKeyList { .. } => {
                            self.handle_provider_key_list(key.code, key.modifiers)?;
                        }
                        Mode::ProviderTestKeyList { .. } => {
                            self.handle_provider_test_key_list(key.code, key.modifiers)?;
                        }
                        Mode::ProviderKeyAdd { .. } => {
                            self.handle_provider_key_add(key.code, key.modifiers)?;
                        }
                        Mode::ProviderKeyEdit { .. } => {
                            self.handle_provider_key_edit(key.code, key.modifiers)?;
                        }
                        Mode::ProviderKeyRename { .. } => {
                            self.handle_provider_key_rename(key.code, key.modifiers)?;
                        }
                        Mode::ConfirmDeleteProvider { .. } => {
                            self.handle_confirm_delete_provider(key.code)?;
                        }
                        Mode::ConfirmDeleteKey { .. } => {
                            self.handle_confirm_delete_key(key.code)?;
                        }
                        Mode::ProviderKeyInUse { .. } => {
                            self.handle_provider_key_in_use(key.code, key.modifiers)?;
                        }
                        Mode::McpAdd { .. } | Mode::McpEdit { .. } => {
                            self.handle_mcp_editor(key.code, key.modifiers)?;
                        }
                        Mode::McpProfilePicker { .. } => {
                            self.handle_mcp_profile_picker(key.code, key.modifiers)?;
                        }
                        Mode::PluginInstallPicker => {
                            self.handle_plugin_install_picker(key.code, key.modifiers)?;
                        }
                        Mode::PluginProfilePicker { .. } => {
                            self.handle_plugin_profile_picker(key.code, key.modifiers)?;
                        }
                        Mode::McpSmartPaste => {
                            self.handle_mcp_smart_paste(key.code, key.modifiers)?;
                        }
                        Mode::ConfirmDeleteMcp { .. } => {
                            self.handle_confirm_delete_mcp(key.code)?;
                        }
                        Mode::PublicSitePrompt => {
                            self.handle_public_site_prompt(key.code, key.modifiers)?;
                        }
                        Mode::PublicSiteTesting => {
                            self.handle_public_site_results(key.code, key.modifiers)?;
                        }
                        Mode::PublicSiteResults => {
                            self.handle_public_site_results(key.code, key.modifiers)?;
                        }
                        Mode::Message(_, _) => {
                            self.mode = self.message_return_mode.take().unwrap_or(Mode::Normal);
                        }
                    }
                }
                Event::Paste(text) => {
                    self.handle_paste(&text)?;
                }
                _ => {}
            }
        }
    }

    pub(super) fn handle_paste(&mut self, text: &str) -> Result<()> {
        match self.mode.clone() {
            Mode::Search => {
                insert_str_at_cursor(&mut self.search_query, &mut self.cursor_pos, text);
                self.apply_filter();
            }
            Mode::AddFullName => {
                insert_str_at_cursor(&mut self.input_buffer, &mut self.cursor_pos, text);
            }
            Mode::AddFullAlias => {
                insert_filtered_str_at_cursor(
                    &mut self.input_buffer,
                    &mut self.cursor_pos,
                    text,
                    is_alias_char,
                );
            }
            Mode::DuplicateProfileName { .. } => {
                insert_str_at_cursor(&mut self.input_buffer, &mut self.cursor_pos, text);
            }
            Mode::DuplicateProfileAlias { .. } => {
                insert_filtered_str_at_cursor(
                    &mut self.input_buffer,
                    &mut self.cursor_pos,
                    text,
                    is_alias_char,
                );
            }
            Mode::EditProfile { step, .. } => match step {
                0 => insert_str_at_cursor(&mut self.lite_name, &mut self.cursor_pos, text),
                1 => insert_filtered_str_at_cursor(
                    &mut self.lite_alias,
                    &mut self.cursor_pos,
                    text,
                    is_alias_char,
                ),
                _ => insert_str_at_cursor(&mut self.lite_launch_args, &mut self.cursor_pos, text),
            },
            Mode::LiteModelSelect { .. } | Mode::LiteEdit { .. } => match self.lite_step {
                0 => insert_str_at_cursor(&mut self.lite_name, &mut self.cursor_pos, text),
                1 => insert_filtered_str_at_cursor(
                    &mut self.lite_alias,
                    &mut self.cursor_pos,
                    text,
                    is_alias_char,
                ),
                2 => insert_str_at_cursor(&mut self.lite_mod_opus, &mut self.cursor_pos, text),
                3 => insert_str_at_cursor(&mut self.lite_mod_sonnet, &mut self.cursor_pos, text),
                4 => insert_str_at_cursor(&mut self.lite_mod_haiku, &mut self.cursor_pos, text),
                5 => insert_str_at_cursor(&mut self.lite_mod_model, &mut self.cursor_pos, text),
                6 => insert_str_at_cursor(&mut self.lite_mod_subagent, &mut self.cursor_pos, text),
                7 => insert_str_at_cursor(&mut self.input_buffer, &mut self.cursor_pos, text),
                8 => insert_str_at_cursor(&mut self.lite_launch_args, &mut self.cursor_pos, text),
                _ => {}
            },
            Mode::ProviderSmartPaste => {
                insert_str_at_cursor(
                    &mut self.provider_smart_paste_buf,
                    &mut self.cursor_pos,
                    text,
                );
            }
            Mode::McpAdd { .. } | Mode::McpEdit { .. } => {
                self.paste_into_mcp_editor(text);
            }
            Mode::McpProfilePicker { .. } => {
                insert_str_at_cursor(&mut self.mcp_filter_buf, &mut self.cursor_pos, text);
            }
            Mode::PluginInstallPicker | Mode::PluginProfilePicker { .. } => {
                insert_str_at_cursor(&mut self.plugin_filter_buf, &mut self.cursor_pos, text);
            }
            Mode::McpSmartPaste => {
                insert_str_at_cursor(&mut self.mcp_oauth_buf, &mut self.cursor_pos, text);
            }
            Mode::ProviderAnthropicTest { field, .. } => {
                if field == 0 {
                    insert_str_at_cursor(
                        &mut self.provider_test_model_buf,
                        &mut self.cursor_pos,
                        text,
                    );
                    self.sync_provider_test_model_selection_from_buffer();
                } else {
                    insert_str_at_cursor(
                        &mut self.provider_test_prompt_buf,
                        &mut self.cursor_pos,
                        text,
                    );
                }
            }
            Mode::ProcessSwitchModelConfirm { .. } => {
                insert_str_at_cursor(
                    &mut self.runtime_switch_model_buf,
                    &mut self.cursor_pos,
                    text,
                );
            }
            Mode::PublicSitePrompt => {
                insert_str_at_cursor(&mut self.public_site_prompt_buf, &mut self.cursor_pos, text);
            }
            Mode::ProviderAdd { step } => {
                let buf = match step {
                    0 if self.provider_add_existing_id.is_some() => &mut self.provider_key_name_buf,
                    0 => &mut self.provider_name_buf,
                    1 => &mut self.provider_url_buf,
                    2 => &mut self.provider_key_name_buf,
                    _ => &mut self.provider_key_buf,
                };
                insert_str_at_cursor(buf, &mut self.cursor_pos, text);
            }
            Mode::ProviderEdit { step, .. } if step < 2 => {
                let buf = if step == 0 {
                    &mut self.provider_name_buf
                } else {
                    &mut self.provider_url_buf
                };
                insert_str_at_cursor(buf, &mut self.cursor_pos, text);
            }
            Mode::ProviderEditKeyInput { step, .. }
            | Mode::ProviderKeyAdd { step, .. }
            | Mode::ProviderKeyEdit { step, .. } => {
                let buf = if step == 0 {
                    &mut self.provider_key_name_buf
                } else {
                    &mut self.provider_key_buf
                };
                insert_str_at_cursor(buf, &mut self.cursor_pos, text);
            }
            Mode::ProviderKeyRename { .. } => {
                insert_str_at_cursor(&mut self.provider_key_name_buf, &mut self.cursor_pos, text);
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_normal_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            _ => {}
        }

        match self.page {
            Page::Profile => self.handle_profile_page_key(code, modifiers),
            Page::Provider => self.handle_provider_page_normal_key(code, modifiers),
            Page::Mcp => self.handle_mcp_page_normal_key(code, modifiers),
            Page::Plugin => self.handle_plugin_page_normal_key(code, modifiers),
            Page::Settings => self.handle_settings_page_normal_key(code, modifiers),
        }
    }

    fn handle_settings_page_normal_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        if matches!(code, KeyCode::Enter | KeyCode::Char(' ')) && modifiers.is_empty() {
            self.settings_allow_local_runtime_hot_switch =
                !self.settings_allow_local_runtime_hot_switch;
            self.manager
                .set_allow_local_runtime_hot_switch(self.settings_allow_local_runtime_hot_switch)?;
            let status = if self.settings_allow_local_runtime_hot_switch {
                "enabled"
            } else {
                "disabled"
            };
            self.show_message(
                format!("Local/self-hosted runtime hot switch {}.", status),
                false,
                Some(Mode::Normal),
            );
        }
        Ok(false)
    }

    pub(super) fn sync_shims(&self) {
        #[cfg(test)]
        if std::env::var_os("CSWITCH_TEST_DISABLE_SHIM_SYNC").is_some() {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            if let Err(e) = self.manager.sync_cmd_aliases() {
                eprintln!("Note: failed to sync CMD aliases: {}", e);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            if let Err(e) = self.manager.sync_sh_scripts() {
                eprintln!("Note: failed to sync shell scripts: {}", e);
            }
        }
    }

    pub(super) fn show_message(&mut self, msg: String, is_err: bool, return_mode: Option<Mode>) {
        self.message_return_mode = return_mode;
        self.mode = Mode::Message(msg, is_err);
    }
}
