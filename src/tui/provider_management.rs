use super::*;

impl App {
    pub(super) fn handle_provider_page_normal_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        match code {
            _ if Self::is_prev_list_key(code, modifiers) => self.move_provider_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_provider_down(),

            KeyCode::Enter => {
                if let Some(p) = self
                    .providers_cache
                    .get(self.provider_list_state.selected().unwrap_or(0))
                {
                    self.provider_keys_cache = self.manager.list_keys(&p.id).unwrap_or_default();
                    self.provider_key_selected = 0;
                    self.mode = Mode::ProviderKeyList {
                        provider_id: p.id.clone(),
                    };
                }
            }

            KeyCode::Char('a') => {
                self.provider_name_buf.clear();
                self.provider_url_buf.clear();
                self.provider_key_buf.clear();
                self.provider_key_name_buf = "Default".to_string();
                self.provider_add_existing_id = None;
                self.provider_smart_paste_buf.clear();
                self.provider_smart_paste_error = None;
                self.cursor_pos = 0;
                self.mode = Mode::ProviderAdd { step: 0 };
            }

            KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_provider_smart_input()?;
            }

            KeyCode::Char('t') => {
                self.start_selected_provider_test()?;
            }

            KeyCode::Char('e') => {
                if let Some(p) = self
                    .providers_cache
                    .get(self.provider_list_state.selected().unwrap_or(0))
                    .cloned()
                {
                    self.provider_name_buf = p.name.clone();
                    self.provider_url_buf = p.base_url.clone();
                    self.cursor_pos = p.name.len();
                    self.provider_keys_cache = self.manager.list_keys(&p.id).unwrap_or_default();
                    self.provider_key_selected = 0;
                    self.mode = Mode::ProviderEdit {
                        provider_id: p.id.clone(),
                        step: 0,
                    };
                }
            }

            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(p) = self
                    .providers_cache
                    .get(self.provider_list_state.selected().unwrap_or(0))
                {
                    let pid = p.id.clone();
                    let name = p.name.clone();
                    self.mode = Mode::ConfirmDeleteProvider {
                        provider_id: pid,
                        name,
                    };
                }
            }

            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }

            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_provider_list(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_prev_list_key(code, modifiers) => self.move_provider_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_provider_down(),
            KeyCode::Enter => {
                let pid = self
                    .provider_list_state
                    .selected()
                    .and_then(|i| self.providers_cache.get(i))
                    .map(|p| p.id.clone());
                if let Some(pid) = pid {
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    self.provider_key_selected = 0;
                    self.mode = Mode::ProviderKeyList { provider_id: pid };
                }
            }
            KeyCode::Char('a') => {
                self.provider_name_buf.clear();
                self.provider_url_buf.clear();
                self.provider_key_buf.clear();
                self.provider_key_name_buf = "Default".to_string();
                self.provider_add_existing_id = None;
                self.provider_smart_paste_buf.clear();
                self.provider_smart_paste_error = None;
                self.mode = Mode::ProviderAdd { step: 0 };
            }
            KeyCode::Char('e') => {
                let data = self
                    .provider_list_state
                    .selected()
                    .and_then(|i| self.providers_cache.get(i))
                    .map(|p| (p.id.clone(), p.name.clone(), p.base_url.clone()));
                if let Some((pid, name, url)) = data {
                    let name_len = name.len();
                    self.provider_name_buf = name;
                    self.provider_url_buf = url;
                    self.cursor_pos = name_len;
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    self.provider_key_selected = 0;
                    self.mode = Mode::ProviderEdit {
                        provider_id: pid,
                        step: 0,
                    };
                }
            }
            KeyCode::Char('d') => {
                let data = self
                    .provider_list_state
                    .selected()
                    .and_then(|i| self.providers_cache.get(i))
                    .map(|p| (p.id.clone(), p.name.clone()));
                if let Some((pid, name)) = data {
                    self.mode = Mode::ConfirmDeleteProvider {
                        provider_id: pid,
                        name,
                    };
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_provider_add(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let step = match &self.mode {
            Mode::ProviderAdd { step } => *step,
            _ => 0,
        };
        let total_steps = if self.provider_add_existing_id.is_some() {
            1
        } else {
            4
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                if self.page == Page::Provider {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
            }
            KeyCode::Tab | KeyCode::Char('n')
                if total_steps > 1
                    && (code == KeyCode::Tab || modifiers.contains(KeyModifiers::CONTROL)) =>
            {
                let next_step = (step + 1) % total_steps;
                self.cursor_pos = provider_add_cursor_pos(
                    next_step,
                    self.provider_add_existing_id.as_deref(),
                    &self.provider_name_buf,
                    &self.provider_url_buf,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderAdd { step: next_step };
            }
            _ if total_steps > 1 && Self::is_prev_field_key(code, modifiers) => {
                let next_step = (step + total_steps - 1) % total_steps;
                self.cursor_pos = provider_add_cursor_pos(
                    next_step,
                    self.provider_add_existing_id.as_deref(),
                    &self.provider_name_buf,
                    &self.provider_url_buf,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderAdd { step: next_step };
            }
            KeyCode::Enter => {
                if step + 1 == total_steps {
                    let name = self.provider_name_buf.trim().to_string();
                    let url = self.provider_url_buf.trim().to_string();
                    let key_name = self.provider_key_name_buf.trim().to_string();
                    let key = self.provider_key_buf.trim().to_string();
                    if (self.provider_add_existing_id.is_none() && name.is_empty())
                        || (self.provider_add_existing_id.is_none() && url.is_empty())
                        || key_name.is_empty()
                        || key.is_empty()
                    {
                        return Ok(());
                    }

                    let result = if let Some(provider_id) = self.provider_add_existing_id.clone() {
                        self.manager
                            .add_key(&provider_id, &key_name, &key)
                            .map(|_| ())
                    } else {
                        self.manager
                            .add_provider_with_key_name(&name, &url, &key_name, &key)
                            .map(|_| ())
                    };

                    match result {
                        Ok(_) => {
                            self.sync_shims();
                            self.providers_cache =
                                self.manager.list_providers().unwrap_or_default();
                            if self.page == Page::Provider {
                                self.mode = Mode::Normal;
                            } else {
                                self.mode = Mode::ProviderList;
                            }
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                } else {
                    let next_step = step + 1;
                    self.cursor_pos = provider_add_cursor_pos(
                        next_step,
                        self.provider_add_existing_id.as_deref(),
                        &self.provider_name_buf,
                        &self.provider_url_buf,
                        &self.provider_key_name_buf,
                        &self.provider_key_buf,
                    );
                    self.mode = Mode::ProviderAdd { step: next_step };
                }
            }
            _ => {
                let buf = match step {
                    0 if self.provider_add_existing_id.is_some() => &mut self.provider_key_name_buf,
                    0 => &mut self.provider_name_buf,
                    1 => &mut self.provider_url_buf,
                    2 => &mut self.provider_key_name_buf,
                    _ => &mut self.provider_key_buf,
                };
                emacs_edit(code, modifiers, buf, &mut self.cursor_pos, true);
            }
        }
        Ok(())
    }

    pub(super) fn handle_provider_edit(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (pid, step) = match &self.mode {
            Mode::ProviderEdit { provider_id, step } => (provider_id.clone(), *step),
            _ => return Ok(()),
        };
        let total_steps: usize = 3;

        if step < 2 && code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL) {
            let next_step = (step + 1) % total_steps;
            self.cursor_pos = provider_edit_cursor_pos(
                next_step,
                &self.provider_name_buf,
                &self.provider_url_buf,
            );
            self.mode = Mode::ProviderEdit {
                provider_id: pid,
                step: next_step,
            };
            return Ok(());
        }

        if step < 2 && code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL) {
            let next_step = (step + total_steps - 1) % total_steps;
            self.cursor_pos = provider_edit_cursor_pos(
                next_step,
                &self.provider_name_buf,
                &self.provider_url_buf,
            );
            self.mode = Mode::ProviderEdit {
                provider_id: pid,
                step: next_step,
            };
            return Ok(());
        }

        if step < 2
            && !matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Tab)
            && !(code == KeyCode::Char('g') && modifiers.contains(KeyModifiers::CONTROL))
            && emacs_edit(
                code,
                modifiers,
                if step == 0 {
                    &mut self.provider_name_buf
                } else {
                    &mut self.provider_url_buf
                },
                &mut self.cursor_pos,
                true,
            )
        {
            return Ok(());
        }

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.providers_cache = self.manager.list_providers().unwrap_or_default();
                if self.page == Page::Provider {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
            }
            KeyCode::Enter => {
                if step == 2 {
                    let name = self.provider_name_buf.trim().to_string();
                    if !name.is_empty() {
                        let _ =
                            self.manager
                                .update_provider(&pid, &name, self.provider_url_buf.trim());
                    }
                    self.sync_shims();
                    self.providers_cache = self.manager.list_providers().unwrap_or_default();
                    if self.page == Page::Provider {
                        self.mode = Mode::Normal;
                    } else {
                        self.mode = Mode::ProviderList;
                    }
                } else {
                    let next_step = (step + 1) % total_steps;
                    self.cursor_pos = provider_edit_cursor_pos(
                        next_step,
                        &self.provider_name_buf,
                        &self.provider_url_buf,
                    );
                    self.mode = Mode::ProviderEdit {
                        provider_id: pid,
                        step: next_step,
                    };
                }
            }
            KeyCode::Tab if step < 2 => {
                let next_step = step + 1;
                self.cursor_pos = provider_edit_cursor_pos(
                    next_step,
                    &self.provider_name_buf,
                    &self.provider_url_buf,
                );
                self.mode = Mode::ProviderEdit {
                    provider_id: pid,
                    step: next_step,
                };
            }
            KeyCode::Char('a') if step == 2 => {
                self.provider_key_name_buf.clear();
                self.provider_key_buf.clear();
                self.cursor_pos = 0;
                self.mode = Mode::ProviderEditKeyInput {
                    provider_id: pid,
                    step: 0,
                };
            }
            KeyCode::Char('d') if step == 2 => {
                if let Some(k) = self.selected_provider_key().cloned() {
                    if self.open_provider_key_in_use_popup_if_needed(
                        &pid,
                        &k.id,
                        &k.name,
                        Mode::ProviderEdit {
                            provider_id: pid.clone(),
                            step: 2,
                        },
                    )? {
                        return Ok(());
                    }
                    self.manager.remove_key(&pid, &k.id)?;
                    self.sync_shims();
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    if self.provider_key_selected >= self.provider_keys_cache.len() {
                        self.provider_key_selected = self.provider_key_selected.saturating_sub(1);
                    }
                }
            }
            KeyCode::Char('e') if step == 2 => {
                if let Some(k) = self.selected_provider_key().cloned() {
                    self.provider_key_name_buf = k.name.clone();
                    self.provider_key_buf = k.api_key.clone();
                    self.cursor_pos = k.name.len();
                    self.mode = Mode::ProviderKeyEdit {
                        provider_id: pid,
                        key_id: k.id,
                        step: 0,
                        source: KeyEditSource::ProviderEdit,
                    };
                }
            }
            KeyCode::Char('r') if step == 2 => {
                if let Some(k) = self.selected_provider_key().cloned() {
                    self.provider_key_name_buf = k.name.clone();
                    self.provider_key_buf.clear();
                    self.cursor_pos = k.name.len();
                    self.mode = Mode::ProviderKeyRename {
                        provider_id: pid,
                        key_id: k.id,
                        source: KeyEditSource::ProviderEdit,
                    };
                }
            }
            _ if step == 2
                && Self::is_prev_list_key(code, modifiers)
                && self.provider_key_selected > 0 =>
            {
                self.provider_key_selected -= 1;
            }
            _ if step == 2
                && Self::is_next_list_key(code, modifiers)
                && self.provider_key_selected + 1 < self.provider_keys_cache.len() =>
            {
                self.provider_key_selected += 1;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_confirm_delete_provider(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let pid = match &self.mode {
                    Mode::ConfirmDeleteProvider { provider_id, .. } => provider_id.clone(),
                    _ => return Ok(()),
                };
                match self.manager.remove_provider(&pid) {
                    Ok(_) => {
                        self.sync_shims();
                        self.providers_cache = self.manager.list_providers().unwrap_or_default();
                        if self.page == Page::Provider {
                            self.mode = Mode::Normal;
                        } else {
                            self.mode = Mode::ProviderList;
                        }
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            _ => {
                if self.page == Page::Provider {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
            }
        }
        Ok(())
    }

    pub(super) fn move_provider_up(&mut self) {
        if self.providers_cache.is_empty() {
            return;
        }
        let i = match self.provider_list_state.selected() {
            Some(0) | None => self.providers_cache.len() - 1,
            Some(i) => i - 1,
        };
        self.provider_list_state.select(Some(i));
        self.provider_list_scroll = self.provider_list_scroll.position(i);
    }

    pub(super) fn move_provider_down(&mut self) {
        if self.providers_cache.is_empty() {
            return;
        }
        let i = match self.provider_list_state.selected() {
            Some(i) => (i + 1) % self.providers_cache.len(),
            None => 0,
        };
        self.provider_list_state.select(Some(i));
        self.provider_list_scroll = self.provider_list_scroll.position(i);
    }
}
