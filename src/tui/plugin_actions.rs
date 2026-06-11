use super::*;

impl App {
    pub(super) fn refresh_plugin_state(&mut self) {
        self.plugins_cache = self.manager.list_installed_plugins().unwrap_or_default();
        if self.plugins_cache.is_empty() {
            self.plugin_list_state.select(None);
            self.plugin_profile_links_cache.clear();
            return;
        }
        let selected = self
            .plugin_list_state
            .selected()
            .unwrap_or(0)
            .min(self.plugins_cache.len() - 1);
        self.plugin_list_state.select(Some(selected));
        self.refresh_plugin_profile_links();
    }

    pub(super) fn refresh_plugin_profile_links(&mut self) {
        self.plugin_profile_links_cache = self
            .selected_plugin()
            .and_then(|plugin| self.manager.list_profiles_using_plugin(&plugin.id).ok())
            .unwrap_or_default();
    }

    pub(super) fn selected_plugin(&self) -> Option<&InstalledPlugin> {
        self.plugin_list_state
            .selected()
            .and_then(|idx| self.plugins_cache.get(idx))
            .or_else(|| self.plugins_cache.first())
    }

    pub(super) fn filtered_plugin_catalog_indices(&self) -> Vec<usize> {
        let q = self.plugin_filter_buf.to_lowercase();
        self.plugin_catalog_cache
            .iter()
            .enumerate()
            .filter(|(_, plugin)| {
                q.is_empty()
                    || plugin.id.to_lowercase().contains(&q)
                    || plugin
                        .display_name
                        .as_deref()
                        .map(|value| value.to_lowercase().contains(&q))
                        .unwrap_or(false)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    pub(super) fn filtered_installed_plugin_indices(&self) -> Vec<usize> {
        let q = self.plugin_filter_buf.to_lowercase();
        self.plugins_cache
            .iter()
            .enumerate()
            .filter(|(_, plugin)| {
                q.is_empty()
                    || plugin.id.to_lowercase().contains(&q)
                    || plugin.plugin_name.to_lowercase().contains(&q)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    pub(super) fn handle_plugin_page_normal_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        match code {
            _ if Self::is_prev_list_key(code, modifiers) => {
                if self.plugins_cache.is_empty() {
                    return Ok(false);
                }
                let next = match self.plugin_list_state.selected() {
                    Some(0) | None => self.plugins_cache.len() - 1,
                    Some(idx) => idx - 1,
                };
                self.plugin_list_state.select(Some(next));
                self.refresh_plugin_profile_links();
            }
            _ if Self::is_next_list_key(code, modifiers) => {
                if self.plugins_cache.is_empty() {
                    return Ok(false);
                }
                let next = match self.plugin_list_state.selected() {
                    Some(idx) => (idx + 1) % self.plugins_cache.len(),
                    None => 0,
                };
                self.plugin_list_state.select(Some(next));
                self.refresh_plugin_profile_links();
            }
            KeyCode::Char('a') => {
                self.plugin_catalog_cache = self.manager.list_hosted_plugin_catalog(None)?;
                if self.plugin_catalog_cache.is_empty() {
                    self.show_message(
                        "No hosted plugins found. Add a marketplace from the CLI first.".into(),
                        true,
                        None,
                    );
                } else {
                    self.plugin_filter_buf.clear();
                    self.cursor_pos = 0;
                    self.plugin_list_state.select(Some(0));
                    self.mode = Mode::PluginInstallPicker;
                }
            }
            KeyCode::Char('u') => {
                let Some(plugin) = self.selected_plugin().cloned() else {
                    return Ok(false);
                };
                match self.manager.update_installed_plugin(&plugin.id) {
                    Ok(updated) => {
                        self.refresh_plugin_state();
                        self.sync_shims();
                        self.show_message(
                            format!("Hosted plugin '{}' updated.", updated.id),
                            false,
                            Some(Mode::Normal),
                        );
                    }
                    Err(error) => self.show_message(error.to_string(), true, Some(Mode::Normal)),
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let Some(plugin) = self.selected_plugin().cloned() else {
                    return Ok(false);
                };
                match self.manager.uninstall_installed_plugin(&plugin.id, true) {
                    Ok(_) => {
                        self.refresh_plugin_state();
                        self.sync_shims();
                        self.show_message(
                            format!("Hosted plugin '{}' removed.", plugin.id),
                            false,
                            Some(Mode::Normal),
                        );
                    }
                    Err(error) => self.show_message(error.to_string(), true, Some(Mode::Normal)),
                }
            }
            KeyCode::Enter => {
                self.refresh_plugin_profile_links();
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_plugin_install_picker(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let filtered = self.filtered_plugin_catalog_indices();
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_prev_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.plugin_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                let next = if pos == 0 {
                    filtered.len() - 1
                } else {
                    pos - 1
                };
                self.plugin_list_state.select(Some(filtered[next]));
            }
            _ if Self::is_next_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.plugin_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                self.plugin_list_state
                    .select(Some(filtered[(pos + 1) % filtered.len()]));
            }
            KeyCode::Tab => {
                if let Some(idx) = filtered.first()
                    && let Some(plugin) = self.plugin_catalog_cache.get(*idx)
                {
                    self.plugin_filter_buf = plugin.id.clone();
                    self.cursor_pos = self.plugin_filter_buf.len();
                    self.plugin_list_state.select(Some(*idx));
                }
            }
            KeyCode::Enter => {
                let Some(idx) = self.plugin_list_state.selected() else {
                    return Ok(());
                };
                let Some(plugin) = self.plugin_catalog_cache.get(idx).cloned() else {
                    return Ok(());
                };
                match self.manager.install_hosted_plugin(&plugin.id, true) {
                    Ok(installed) => {
                        self.refresh_plugin_state();
                        self.sync_shims();
                        self.mode = Mode::Message(
                            format!("Hosted plugin '{}' installed.", installed.id),
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
                    &mut self.plugin_filter_buf,
                    &mut self.cursor_pos,
                    true,
                );
                if let Some(idx) = self.filtered_plugin_catalog_indices().first() {
                    self.plugin_list_state.select(Some(*idx));
                }
            }
        }
        Ok(())
    }

    pub(super) fn start_selected_profile_plugin_picker(&mut self) -> Result<()> {
        let Some(profile) = self.selected_profile().cloned() else {
            return Ok(());
        };
        self.plugins_cache = self.manager.list_installed_plugins().unwrap_or_default();
        if self.plugins_cache.is_empty() {
            self.show_message(
                "No hosted plugins installed. Install one in the Plugin page or CLI first.".into(),
                true,
                None,
            );
            return Ok(());
        }
        self.plugin_selected_ids = profile.plugin_ids.clone();
        self.plugin_filter_buf.clear();
        self.cursor_pos = 0;
        self.plugin_list_state = ListState::default();
        self.plugin_list_state.select(Some(0));
        self.mode = Mode::PluginProfilePicker {
            profile_id: profile.id,
        };
        Ok(())
    }

    pub(super) fn handle_plugin_profile_picker(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let profile_id = match &self.mode {
            Mode::PluginProfilePicker { profile_id } => profile_id.clone(),
            _ => return Ok(()),
        };
        let filtered = self.filtered_installed_plugin_indices();
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_prev_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.plugin_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                let next = if pos == 0 {
                    filtered.len() - 1
                } else {
                    pos - 1
                };
                self.plugin_list_state.select(Some(filtered[next]));
            }
            _ if Self::is_next_filter_list_key(code, modifiers) && !filtered.is_empty() => {
                let current = self.plugin_list_state.selected().unwrap_or(0);
                let pos = filtered.iter().position(|idx| *idx == current).unwrap_or(0);
                self.plugin_list_state
                    .select(Some(filtered[(pos + 1) % filtered.len()]));
            }
            KeyCode::Char(' ') => {
                if let Some(idx) = self.plugin_list_state.selected()
                    && let Some(plugin) = self.plugins_cache.get(idx)
                {
                    if self.plugin_selected_ids.iter().any(|id| id == &plugin.id) {
                        self.plugin_selected_ids.retain(|id| id != &plugin.id);
                    } else {
                        self.plugin_selected_ids.push(plugin.id.clone());
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(idx) = filtered.first()
                    && let Some(plugin) = self.plugins_cache.get(*idx)
                {
                    self.plugin_filter_buf = plugin.id.clone();
                    self.cursor_pos = self.plugin_filter_buf.len();
                    self.plugin_list_state.select(Some(*idx));
                }
            }
            KeyCode::Enter => {
                let selected = self.plugin_selected_ids.clone();
                match self.manager.set_profile_plugins(&profile_id, &selected) {
                    Ok(profile) => {
                        self.sync_shims();
                        self.refresh()?;
                        self.select_by_id(&profile.id);
                        self.mode = Mode::Message(
                            format!(
                                "Profile '{}' now has {} hosted plugin(s).",
                                profile.name,
                                profile.plugin_ids.len()
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
                    &mut self.plugin_filter_buf,
                    &mut self.cursor_pos,
                    true,
                );
                if let Some(idx) = self.filtered_installed_plugin_indices().first() {
                    self.plugin_list_state.select(Some(*idx));
                }
            }
        }
        Ok(())
    }
}
