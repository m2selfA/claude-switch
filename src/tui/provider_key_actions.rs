use super::*;

impl App {
    pub(super) fn handle_provider_edit_key_input(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let pid = match &self.mode {
            Mode::ProviderEditKeyInput { provider_id, .. } => provider_id.clone(),
            _ => return Ok(()),
        };
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                self.provider_key_selected = 0;
                self.cursor_pos =
                    provider_edit_cursor_pos(2, &self.provider_name_buf, &self.provider_url_buf);
                self.mode = Mode::ProviderEdit {
                    provider_id: pid,
                    step: 2,
                };
            }
            KeyCode::Tab | KeyCode::Char('n')
                if code == KeyCode::Tab || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let step = match &self.mode {
                    Mode::ProviderEditKeyInput { step, .. } => *step,
                    _ => 0,
                };
                let next_step = (step + 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderEditKeyInput {
                    provider_id: pid,
                    step: next_step,
                };
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                let step = match &self.mode {
                    Mode::ProviderEditKeyInput { step, .. } => *step,
                    _ => 0,
                };
                let next_step = (step + 2 - 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderEditKeyInput {
                    provider_id: pid,
                    step: next_step,
                };
            }
            KeyCode::Enter => {
                let step = match &self.mode {
                    Mode::ProviderEditKeyInput { step, .. } => *step,
                    _ => 0,
                };
                if step == 1 {
                    let name = self.provider_key_name_buf.trim().to_string();
                    let key = self.provider_key_buf.trim().to_string();
                    if name.is_empty() || key.is_empty() {
                        return Ok(());
                    }
                    let _ = self.manager.add_key(&pid, &name, &key);
                    self.sync_shims();
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    self.provider_key_selected = self.provider_keys_cache.len().saturating_sub(1);
                    self.cursor_pos = provider_edit_cursor_pos(
                        2,
                        &self.provider_name_buf,
                        &self.provider_url_buf,
                    );
                    self.mode = Mode::ProviderEdit {
                        provider_id: pid,
                        step: 2,
                    };
                } else {
                    let next_step = step + 1;
                    self.cursor_pos = provider_key_cursor_pos(
                        next_step,
                        &self.provider_key_name_buf,
                        &self.provider_key_buf,
                    );
                    self.mode = Mode::ProviderEditKeyInput {
                        provider_id: pid,
                        step: next_step,
                    };
                }
            }
            _ => {
                let step = match &self.mode {
                    Mode::ProviderEditKeyInput { step, .. } => *step,
                    _ => 0,
                };
                let buf = match step {
                    0 => &mut self.provider_key_name_buf,
                    _ => &mut self.provider_key_buf,
                };
                emacs_edit(code, modifiers, buf, &mut self.cursor_pos, true);
            }
        }
        Ok(())
    }

    pub(super) fn handle_provider_key_list(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.providers_cache = self.manager.list_providers().unwrap_or_default();
                if self.page == Page::Provider {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
            }
            _ if Self::is_prev_list_key(code, modifiers) => self.move_provider_key_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_provider_key_down(),
            KeyCode::Char('a') => {
                self.provider_key_name_buf.clear();
                self.provider_key_buf.clear();
                self.cursor_pos = 0;
                let pid = match &self.mode {
                    Mode::ProviderKeyList { provider_id } => provider_id.clone(),
                    _ => return Ok(()),
                };
                self.mode = Mode::ProviderKeyAdd {
                    provider_id: pid,
                    step: 0,
                };
            }
            KeyCode::Char('e') => {
                let kid = self
                    .selected_provider_key()
                    .map(|k| (k.id.clone(), k.name.clone(), k.api_key.clone()));
                if let Some((kid_val, name, key)) = kid {
                    let pid = match &self.mode {
                        Mode::ProviderKeyList { provider_id } => provider_id.clone(),
                        _ => return Ok(()),
                    };
                    let name_len = name.len();
                    self.provider_key_name_buf = name;
                    self.provider_key_buf = key;
                    self.cursor_pos = name_len;
                    self.mode = Mode::ProviderKeyEdit {
                        provider_id: pid,
                        key_id: kid_val,
                        step: 0,
                        source: KeyEditSource::ProviderKeyList,
                    };
                }
            }
            KeyCode::Char('r') => {
                let kid = self
                    .selected_provider_key()
                    .map(|k| (k.id.clone(), k.name.clone()));
                if let Some((kid_val, name)) = kid {
                    let pid = match &self.mode {
                        Mode::ProviderKeyList { provider_id } => provider_id.clone(),
                        _ => return Ok(()),
                    };
                    let name_len = name.len();
                    self.provider_key_name_buf = name;
                    self.provider_key_buf.clear();
                    self.cursor_pos = name_len;
                    self.mode = Mode::ProviderKeyRename {
                        provider_id: pid,
                        key_id: kid_val,
                        source: KeyEditSource::ProviderKeyList,
                    };
                }
            }
            KeyCode::Char('d') => {
                let kid = self
                    .selected_provider_key()
                    .map(|k| (k.id.clone(), k.name.clone()));
                if let Some((kid_val, name)) = kid {
                    let pid = match &self.mode {
                        Mode::ProviderKeyList { provider_id } => provider_id.clone(),
                        _ => return Ok(()),
                    };
                    self.mode = Mode::ConfirmDeleteKey {
                        provider_id: pid,
                        key_id: kid_val,
                        name,
                    };
                }
            }
            KeyCode::Char('t') => {
                self.start_provider_key_test()?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_provider_key_rename(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (pid, kid, source) = match &self.mode {
            Mode::ProviderKeyRename {
                provider_id,
                key_id,
                source,
            } => (provider_id.clone(), key_id.clone(), *source),
            _ => return Ok(()),
        };
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                match source {
                    KeyEditSource::ProviderEdit => {
                        self.mode = Mode::ProviderEdit {
                            provider_id: pid,
                            step: 2,
                        };
                    }
                    KeyEditSource::ProviderKeyList => {
                        self.mode = Mode::ProviderKeyList { provider_id: pid };
                    }
                }
            }
            KeyCode::Enter => {
                let name = self.provider_key_name_buf.trim().to_string();
                match self.manager.rename_key(&pid, &kid, &name) {
                    Ok(renamed) => {
                        self.sync_shims();
                        self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                        self.provider_key_selected = self
                            .provider_keys_cache
                            .iter()
                            .position(|key| key.id == renamed.id)
                            .unwrap_or_else(|| {
                                self.provider_key_selected
                                    .min(self.provider_keys_cache.len().saturating_sub(1))
                            });
                        match source {
                            KeyEditSource::ProviderEdit => {
                                self.mode = Mode::ProviderEdit {
                                    provider_id: pid,
                                    step: 2,
                                };
                            }
                            KeyEditSource::ProviderKeyList => {
                                self.mode = Mode::ProviderKeyList { provider_id: pid };
                            }
                        }
                    }
                    Err(e) => {
                        self.show_message(
                            e.to_string(),
                            true,
                            Some(Mode::ProviderKeyRename {
                                provider_id: pid,
                                key_id: kid,
                                source,
                            }),
                        );
                    }
                }
            }
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.provider_key_name_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_provider_key_add(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                let pid = match &self.mode {
                    Mode::ProviderKeyAdd { provider_id, .. } => provider_id.clone(),
                    _ => return Ok(()),
                };
                self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                self.mode = Mode::ProviderKeyList { provider_id: pid };
            }
            KeyCode::Tab | KeyCode::Char('n')
                if code == KeyCode::Tab || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let (pid, step) = match &self.mode {
                    Mode::ProviderKeyAdd { provider_id, step } => (provider_id.clone(), *step),
                    _ => return Ok(()),
                };
                let next_step = (step + 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderKeyAdd {
                    provider_id: pid,
                    step: next_step,
                };
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                let (pid, step) = match &self.mode {
                    Mode::ProviderKeyAdd { provider_id, step } => (provider_id.clone(), *step),
                    _ => return Ok(()),
                };
                let next_step = (step + 2 - 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderKeyAdd {
                    provider_id: pid,
                    step: next_step,
                };
            }
            KeyCode::Enter => {
                let (pid, step) = match &self.mode {
                    Mode::ProviderKeyAdd { provider_id, step } => (provider_id.clone(), *step),
                    _ => return Ok(()),
                };
                if step == 1 {
                    let name = self.provider_key_name_buf.trim().to_string();
                    let key = self.provider_key_buf.trim().to_string();
                    if name.is_empty() || key.is_empty() {
                        return Ok(());
                    }
                    match self.manager.add_key(&pid, &name, &key) {
                        Ok(_) => {
                            self.sync_shims();
                            self.provider_keys_cache =
                                self.manager.list_keys(&pid).unwrap_or_default();
                            self.mode = Mode::ProviderKeyList { provider_id: pid };
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                } else {
                    let next_step = step + 1;
                    self.cursor_pos = provider_key_cursor_pos(
                        next_step,
                        &self.provider_key_name_buf,
                        &self.provider_key_buf,
                    );
                    self.mode = Mode::ProviderKeyAdd {
                        provider_id: pid,
                        step: next_step,
                    };
                }
            }
            _ => {
                let step = match &self.mode {
                    Mode::ProviderKeyAdd { step, .. } => *step,
                    _ => 0,
                };
                let buf = match step {
                    0 => &mut self.provider_key_name_buf,
                    _ => &mut self.provider_key_buf,
                };
                emacs_edit(code, modifiers, buf, &mut self.cursor_pos, true);
            }
        }
        Ok(())
    }

    pub(super) fn handle_provider_key_edit(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (pid, kid, source) = match &self.mode {
            Mode::ProviderKeyEdit {
                provider_id,
                key_id,
                source,
                ..
            } => (provider_id.clone(), key_id.clone(), *source),
            _ => return Ok(()),
        };
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                match source {
                    KeyEditSource::ProviderEdit => {
                        self.mode = Mode::ProviderEdit {
                            provider_id: pid,
                            step: 2,
                        };
                    }
                    KeyEditSource::ProviderKeyList => {
                        self.mode = Mode::ProviderKeyList { provider_id: pid };
                    }
                }
            }
            KeyCode::Tab | KeyCode::Char('n')
                if code == KeyCode::Tab || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let step = match &self.mode {
                    Mode::ProviderKeyEdit { step, .. } => *step,
                    _ => 0,
                };
                let next_step = (step + 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderKeyEdit {
                    provider_id: pid,
                    key_id: kid,
                    step: next_step,
                    source,
                };
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                let step = match &self.mode {
                    Mode::ProviderKeyEdit { step, .. } => *step,
                    _ => 0,
                };
                let next_step = (step + 2 - 1) % 2;
                self.cursor_pos = provider_key_cursor_pos(
                    next_step,
                    &self.provider_key_name_buf,
                    &self.provider_key_buf,
                );
                self.mode = Mode::ProviderKeyEdit {
                    provider_id: pid,
                    key_id: kid,
                    step: next_step,
                    source,
                };
            }
            KeyCode::Enter => {
                let step = match &self.mode {
                    Mode::ProviderKeyEdit { step, .. } => *step,
                    _ => 0,
                };
                if step == 1 {
                    let name = self.provider_key_name_buf.trim().to_string();
                    let key = self.provider_key_buf.trim().to_string();
                    if name.is_empty() || key.is_empty() {
                        return Ok(());
                    }
                    match self.manager.update_key(&pid, &kid, &name, &key) {
                        Ok(_) => {
                            self.sync_shims();
                            self.provider_keys_cache =
                                self.manager.list_keys(&pid).unwrap_or_default();
                            match source {
                                KeyEditSource::ProviderEdit => {
                                    self.mode = Mode::ProviderEdit {
                                        provider_id: pid,
                                        step: 2,
                                    };
                                }
                                KeyEditSource::ProviderKeyList => {
                                    self.mode = Mode::ProviderKeyList { provider_id: pid };
                                }
                            }
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                } else {
                    let next_step = step + 1;
                    self.cursor_pos = provider_key_cursor_pos(
                        next_step,
                        &self.provider_key_name_buf,
                        &self.provider_key_buf,
                    );
                    self.mode = Mode::ProviderKeyEdit {
                        provider_id: pid,
                        key_id: kid,
                        step: next_step,
                        source,
                    };
                }
            }
            _ => {
                let step = match &self.mode {
                    Mode::ProviderKeyEdit { step, .. } => *step,
                    _ => 0,
                };
                let buf = match step {
                    0 => &mut self.provider_key_name_buf,
                    _ => &mut self.provider_key_buf,
                };
                emacs_edit(code, modifiers, buf, &mut self.cursor_pos, true);
            }
        }
        Ok(())
    }

    pub(super) fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (pid, kid, name) = match &self.mode {
                    Mode::ConfirmDeleteKey {
                        provider_id,
                        key_id,
                        name,
                    } => (provider_id.clone(), key_id.clone(), name.clone()),
                    _ => return Ok(()),
                };
                if self.open_provider_key_in_use_popup_if_needed(
                    &pid,
                    &kid,
                    &name,
                    Mode::ProviderKeyList {
                        provider_id: pid.clone(),
                    },
                )? {
                    return Ok(());
                }
                match self.manager.remove_key(&pid, &kid) {
                    Ok(_) => {
                        self.sync_shims();
                        self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                        self.mode = Mode::ProviderKeyList { provider_id: pid };
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            _ => {
                let pid = match &self.mode {
                    Mode::ConfirmDeleteKey { provider_id, .. } => provider_id.clone(),
                    _ => return Ok(()),
                };
                self.mode = Mode::ProviderKeyList { provider_id: pid };
            }
        }
        Ok(())
    }

    pub(super) fn open_provider_key_in_use_popup_if_needed(
        &mut self,
        provider_id: &str,
        key_id: &str,
        key_name: &str,
        return_mode: Mode,
    ) -> Result<bool> {
        let linked = self.manager.list_profiles_using_key(provider_id, key_id)?;
        if linked.is_empty() {
            return Ok(false);
        }
        self.provider_key_linked_profiles = linked;
        self.provider_key_linked_profile_selected = 0;
        self.mode = Mode::ProviderKeyInUse {
            provider_id: provider_id.to_string(),
            key_id: key_id.to_string(),
            name: key_name.to_string(),
            return_mode: Box::new(return_mode),
        };
        Ok(true)
    }

    pub(super) fn force_remove_provider_key(
        &mut self,
        provider_id: &str,
        key_id: &str,
        return_mode: Mode,
    ) -> Result<()> {
        let linked = self.manager.list_profiles_using_key(provider_id, key_id)?;
        for profile in linked {
            self.manager.unset_provider(&profile.id)?;
        }
        self.manager.remove_key(provider_id, key_id)?;
        self.sync_shims();
        self.refresh()?;
        self.provider_keys_cache = self.manager.list_keys(provider_id).unwrap_or_default();
        if self.provider_key_selected >= self.provider_keys_cache.len() {
            self.provider_key_selected = self.provider_key_selected.saturating_sub(1);
        }
        self.provider_key_linked_profiles.clear();
        self.provider_key_linked_profile_selected = 0;
        self.mode = return_mode;
        Ok(())
    }

    pub(super) fn handle_provider_key_in_use(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (pid, kid, return_mode) = match &self.mode {
            Mode::ProviderKeyInUse {
                provider_id,
                key_id,
                return_mode,
                ..
            } => (provider_id.clone(), key_id.clone(), *return_mode.clone()),
            _ => return Ok(()),
        };
        match code {
            _ if Self::is_prev_list_key(code, modifiers) => {
                self.move_provider_key_linked_profile_up();
            }
            _ if Self::is_next_list_key(code, modifiers) => {
                self.move_provider_key_linked_profile_down();
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let Some(profile) = self.selected_provider_key_linked_profile().cloned() else {
                    return Ok(());
                };
                self.manager.remove_profile(&profile.id)?;
                self.sync_shims();
                self.refresh()?;
                self.provider_key_linked_profiles =
                    self.manager.list_profiles_using_key(&pid, &kid)?;
                if self.provider_key_linked_profile_selected
                    >= self.provider_key_linked_profiles.len()
                    && !self.provider_key_linked_profiles.is_empty()
                {
                    self.provider_key_linked_profile_selected =
                        self.provider_key_linked_profiles.len().saturating_sub(1);
                }
                if self.provider_key_linked_profiles.is_empty() {
                    self.manager.remove_key(&pid, &kid)?;
                    self.sync_shims();
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    self.mode = return_mode;
                }
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.force_remove_provider_key(&pid, &kid, return_mode)?;
            }
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = return_mode;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn move_provider_key_up(&mut self) {
        if self.provider_keys_cache.is_empty() {
            return;
        }
        if self.provider_key_selected > 0 {
            self.provider_key_selected -= 1;
        } else {
            self.provider_key_selected = self.provider_keys_cache.len() - 1;
        }
    }

    pub(super) fn move_provider_key_down(&mut self) {
        if self.provider_keys_cache.is_empty() {
            return;
        }
        if self.provider_key_selected + 1 < self.provider_keys_cache.len() {
            self.provider_key_selected += 1;
        } else {
            self.provider_key_selected = 0;
        }
    }

    pub(super) fn selected_provider_key(&self) -> Option<&ProviderKey> {
        self.provider_keys_cache
            .get(self.provider_key_selected)
            .or_else(|| self.provider_keys_cache.first())
    }

    pub(super) fn move_provider_key_linked_profile_up(&mut self) {
        if self.provider_key_linked_profiles.is_empty() {
            return;
        }
        if self.provider_key_linked_profile_selected > 0 {
            self.provider_key_linked_profile_selected -= 1;
        } else {
            self.provider_key_linked_profile_selected = self.provider_key_linked_profiles.len() - 1;
        }
    }

    pub(super) fn move_provider_key_linked_profile_down(&mut self) {
        if self.provider_key_linked_profiles.is_empty() {
            return;
        }
        if self.provider_key_linked_profile_selected + 1 < self.provider_key_linked_profiles.len() {
            self.provider_key_linked_profile_selected += 1;
        } else {
            self.provider_key_linked_profile_selected = 0;
        }
    }

    pub(super) fn selected_provider_key_linked_profile(&self) -> Option<&Profile> {
        self.provider_key_linked_profiles
            .get(self.provider_key_linked_profile_selected)
            .or_else(|| self.provider_key_linked_profiles.first())
    }
}
