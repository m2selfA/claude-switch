use super::*;

impl App {
    fn runtime_session_hot_switch_eligible(&self, session: &RuntimeSessionInfo) -> bool {
        self.settings_allow_local_runtime_hot_switch
            || !crate::profile::is_local_runtime_base_url(&session.state.base_url)
    }

    fn provider_hot_switch_blocked_message(&self, base_url: &str) -> Option<String> {
        crate::profile::is_local_runtime_base_url(base_url).then(|| {
            format!(
                "Local/self-hosted API '{}' uses direct lightweight launch, an inline apiKeyHelper, and cannot use dynamic hot switch.",
                base_url.trim()
            )
        })
    }

    fn refresh_process_switch_sessions(&mut self) -> Result<()> {
        self.runtime_sessions_cache = self
            .manager
            .list_runtime_sessions()?
            .into_iter()
            .filter(|session| session.active)
            .filter(|session| self.runtime_session_hot_switch_eligible(session))
            .collect();
        if self.runtime_sessions_cache.is_empty() {
            self.runtime_session_selected = 0;
        } else {
            self.runtime_session_selected = self
                .runtime_session_selected
                .min(self.runtime_sessions_cache.len().saturating_sub(1));
        }
        Ok(())
    }

    pub(super) fn start_process_switch_picker(
        &mut self,
        provider_id: String,
        key_id: String,
        return_mode: Mode,
    ) -> Result<()> {
        let provider = self.manager.get_provider(&provider_id)?;
        if let Some(message) = self.provider_hot_switch_blocked_message(&provider.base_url) {
            self.show_message(message, true, Some(return_mode));
            return Ok(());
        }
        self.refresh_process_switch_sessions()?;
        if self.runtime_sessions_cache.is_empty() {
            self.show_message(
                "No hot-switch-eligible runtime-managed Claude sessions found.".to_string(),
                true,
                Some(return_mode),
            );
            return Ok(());
        }
        self.runtime_session_selected = 0;
        self.mode = Mode::ProcessSwitchPicker {
            provider_id,
            key_id,
            return_mode: Box::new(return_mode),
        };
        Ok(())
    }

    pub(super) fn selected_runtime_session(&self) -> Option<&RuntimeSessionInfo> {
        self.runtime_sessions_cache
            .get(self.runtime_session_selected)
    }

    pub(super) fn handle_process_switch_picker(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (provider_id, key_id, return_mode) = match &self.mode {
            Mode::ProcessSwitchPicker {
                provider_id,
                key_id,
                return_mode,
            } => (provider_id.clone(), key_id.clone(), *return_mode.clone()),
            _ => return Ok(()),
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = return_mode;
            }
            KeyCode::Enter => {
                let Some(session) = self.selected_runtime_session().cloned() else {
                    return Ok(());
                };
                self.runtime_switch_model_buf = self.provider_test_model_buf.clone();
                self.cursor_pos = self.runtime_switch_model_buf.len();
                self.mode = Mode::ProcessSwitchModelConfirm {
                    session_id: session.state.session_id,
                    provider_id,
                    key_id,
                    return_mode: Box::new(return_mode),
                };
            }
            _ if Self::is_prev_list_key(code, modifiers) => {
                if self.runtime_session_selected == 0 {
                    self.runtime_session_selected =
                        self.runtime_sessions_cache.len().saturating_sub(1);
                } else {
                    self.runtime_session_selected -= 1;
                }
            }
            _ if Self::is_next_list_key(code, modifiers) => {
                self.runtime_session_selected =
                    (self.runtime_session_selected + 1) % self.runtime_sessions_cache.len();
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_process_switch_model_confirm(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (session_id, provider_id, key_id, return_mode) = match &self.mode {
            Mode::ProcessSwitchModelConfirm {
                session_id,
                provider_id,
                key_id,
                return_mode,
            } => (
                session_id.clone(),
                provider_id.clone(),
                key_id.clone(),
                *return_mode.clone(),
            ),
            _ => return Ok(()),
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = Mode::ProcessSwitchPicker {
                    provider_id,
                    key_id,
                    return_mode: Box::new(return_mode),
                };
            }
            KeyCode::Enter => {
                let model = self.runtime_switch_model_buf.trim().to_string();
                if model.is_empty() {
                    return Ok(());
                }
                let updated = match self.manager.switch_runtime_session(
                    &session_id,
                    &provider_id,
                    &key_id,
                    &model,
                ) {
                    Ok(updated) => updated,
                    Err(error) => {
                        self.refresh_process_switch_sessions()?;
                        let next_mode = if self.runtime_sessions_cache.is_empty() {
                            return_mode.clone()
                        } else {
                            Mode::ProcessSwitchPicker {
                                provider_id: provider_id.clone(),
                                key_id: key_id.clone(),
                                return_mode: Box::new(return_mode.clone()),
                            }
                        };
                        self.show_message(error.to_string(), true, Some(next_mode));
                        return Ok(());
                    }
                };
                let cwd = updated
                    .cwd
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "—".to_string());
                let summary = format!(
                    "Switched {} (pid {}) to {} / {} with model {}.\nCWD: {}",
                    updated.session_id,
                    updated
                        .pid
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    updated.provider_name.as_deref().unwrap_or("inline"),
                    updated.key_name.as_deref().unwrap_or("no-key"),
                    updated.model.as_deref().unwrap_or(&model),
                    cwd,
                );
                self.show_message(summary, false, Some(return_mode));
            }
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.runtime_switch_model_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }
}
