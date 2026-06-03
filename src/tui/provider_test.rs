use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ProviderTestSource {
    Page,
    KeyList,
    TestKeyList,
    PublicSite,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ProviderTestKeySelection {
    NoKeys,
    Single(ProviderKey),
    Multiple,
}

pub(super) fn provider_test_key_selection(keys: &[ProviderKey]) -> ProviderTestKeySelection {
    match keys {
        [] => ProviderTestKeySelection::NoKeys,
        [key] => ProviderTestKeySelection::Single(key.clone()),
        _ => ProviderTestKeySelection::Multiple,
    }
}

fn provider_test_return_mode(source: ProviderTestSource, provider_id: &str) -> Option<Mode> {
    match source {
        ProviderTestSource::Page => None,
        ProviderTestSource::KeyList => Some(Mode::ProviderKeyList {
            provider_id: provider_id.to_string(),
        }),
        ProviderTestSource::TestKeyList => Some(Mode::ProviderTestKeyList {
            provider_id: provider_id.to_string(),
        }),
        ProviderTestSource::PublicSite => Some(Mode::PublicSiteResults),
    }
}

pub(super) fn provider_test_outcome_next_mode(
    code: KeyCode,
    modifiers: KeyModifiers,
    provider_id: &str,
    key_id: &str,
    source: ProviderTestSource,
    field: usize,
) -> Mode {
    if matches!(code, KeyCode::Char('q') | KeyCode::Esc)
        || (code == KeyCode::Char('g') && modifiers.contains(KeyModifiers::CONTROL))
    {
        provider_test_return_mode(source, provider_id).unwrap_or(Mode::Normal)
    } else {
        Mode::ProviderAnthropicTest {
            provider_id: provider_id.to_string(),
            key_id: key_id.to_string(),
            source,
            field,
        }
    }
}

pub(super) fn model_fetch_state_for_models(models: &[String]) -> ModelFetchState {
    if models.is_empty() {
        ModelFetchState::Empty
    } else {
        ModelFetchState::Loaded
    }
}

pub(super) fn model_fetch_unavailable_message(error: &str) -> String {
    format!(
        "/v1/models unavailable: {}. Manual model entry still works.",
        error
    )
}

pub(super) fn complete_provider_test_model(models: &[String], current: &str) -> Option<String> {
    let needle = current.trim();
    if needle.is_empty() {
        return models.first().cloned();
    }

    let needle_lower = needle.to_lowercase();
    models
        .iter()
        .find(|model| model.eq_ignore_ascii_case(needle))
        .cloned()
        .or_else(|| {
            models
                .iter()
                .find(|model| model.to_lowercase().contains(&needle_lower))
                .cloned()
        })
}

fn is_prev_provider_test_model_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() && matches!(code, KeyCode::Up)
}

fn is_next_provider_test_model_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() && matches!(code, KeyCode::Down)
}

fn is_prev_provider_test_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
    code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL)
}

fn is_next_provider_test_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
    code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL)
}

impl App {
    pub(super) fn set_provider_test_models_from_result(
        &mut self,
        fetched: std::result::Result<ModelDiscoverySuccess, String>,
    ) {
        match fetched {
            Ok(discovery) => {
                self.provider_test_models = discovery
                    .models
                    .into_iter()
                    .map(|model| trim_model_context_suffix(&model).to_string())
                    .collect();
                self.provider_test_models.sort();
                self.provider_test_models.dedup();
                self.provider_test_model_fetch_state =
                    model_fetch_state_for_models(&self.provider_test_models);
            }
            Err(e) => {
                self.provider_test_models.clear();
                self.provider_test_model_fetch_state =
                    ModelFetchState::Unavailable(model_fetch_unavailable_message(&e.to_string()));
            }
        }
    }

    pub(super) fn sync_provider_test_model_selection_from_buffer(&mut self) {
        if let Some(index) = self
            .provider_test_models
            .iter()
            .position(|model| model == &self.provider_test_model_buf)
        {
            self.provider_test_model_selected = index;
        }
    }

    pub(super) fn start_selected_provider_test(&mut self) -> Result<()> {
        let provider = self
            .provider_list_state
            .selected()
            .and_then(|i| self.providers_cache.get(i))
            .cloned();
        let Some(provider) = provider else {
            self.show_message("Select a provider first.".into(), true, None);
            return Ok(());
        };

        self.provider_keys_cache = self.manager.list_keys(&provider.id).unwrap_or_default();
        match provider_test_key_selection(&self.provider_keys_cache) {
            ProviderTestKeySelection::NoKeys => {
                self.show_message(
                    format!("Provider '{}' has no keys to test.", provider.name),
                    true,
                    None,
                );
            }
            ProviderTestKeySelection::Single(key) => {
                self.start_provider_test_popup(&provider, &key, ProviderTestSource::Page)?;
            }
            ProviderTestKeySelection::Multiple => {
                self.provider_key_selected = 0;
                self.mode = Mode::ProviderTestKeyList {
                    provider_id: provider.id,
                };
            }
        }
        Ok(())
    }

    pub(super) fn start_provider_key_test(&mut self) -> Result<()> {
        let (provider_id, source) = match &self.mode {
            Mode::ProviderKeyList { provider_id } => {
                (provider_id.clone(), ProviderTestSource::KeyList)
            }
            Mode::ProviderTestKeyList { provider_id } => {
                (provider_id.clone(), ProviderTestSource::TestKeyList)
            }
            _ => return Ok(()),
        };
        let provider = self.manager.get_provider(&provider_id)?;
        let Some(key) = self.selected_provider_key().cloned() else {
            self.show_message("Select a provider key first.".into(), true, None);
            return Ok(());
        };
        self.start_provider_test_popup(&provider, &key, source)
    }

    pub(super) fn start_provider_test_popup(
        &mut self,
        provider: &Provider,
        key: &ProviderKey,
        source: ProviderTestSource,
    ) -> Result<()> {
        let fetched_models = discover_models(&provider.base_url, &key.api_key).map_err(|failure| {
            let tried = if failure.tried_endpoints.is_empty() {
                String::new()
            } else {
                format!("\n  Tried: {}", failure.tried_endpoints.join(" → "))
            };
            format!(
                "Provider '{}' key '{}' could not discover models: {}{}",
                provider.name, key.name, failure.message, tried
            )
        });
        self.set_provider_test_models_from_result(fetched_models);
        self.provider_test_model_selected = 0;
        self.provider_test_model_buf = self
            .provider_test_models
            .first()
            .cloned()
            .unwrap_or_default();
        self.provider_test_prompt_buf = "Hello".to_string();
        self.cursor_pos = self.provider_test_model_buf.len();
        self.mode = Mode::ProviderAnthropicTest {
            provider_id: provider.id.clone(),
            key_id: key.id.clone(),
            source,
            field: 0,
        };
        Ok(())
    }

    pub(super) fn handle_provider_anthropic_test(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (provider_id, key_id, source, field) = match &self.mode {
            Mode::ProviderAnthropicTest {
                provider_id,
                key_id,
                source,
                field,
            } => (provider_id.clone(), key_id.clone(), *source, *field),
            _ => return Ok(()),
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = provider_test_return_mode(source, &provider_id).unwrap_or(Mode::Normal);
            }
            KeyCode::Enter => {
                let provider = self.manager.get_provider(&provider_id)?;
                let key = provider
                    .keys
                    .get(&key_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Key '{}' not found.", key_id))?;
                let model = self.provider_test_model_buf.trim().to_string();
                let prompt = self.provider_test_prompt_buf.trim().to_string();
                if model.is_empty() || prompt.is_empty() {
                    return Ok(());
                }
                match test_anthropic_message(&provider.base_url, &key.api_key, &model, &prompt) {
                    Ok(result) => {
                        self.mode = Mode::ProviderAnthropicOutcome {
                            provider_id,
                            key_id,
                            source,
                            field,
                            model,
                            endpoint_used: Some(result.endpoint_used),
                            input_tokens: result.input_tokens,
                            output_tokens: result.output_tokens,
                            body: result.text.trim().to_string(),
                            is_error: false,
                        };
                    }
                    Err(e) => {
                        self.mode = Mode::ProviderAnthropicOutcome {
                            provider_id,
                            key_id,
                            source,
                            field,
                            model,
                            endpoint_used: None,
                            input_tokens: None,
                            output_tokens: None,
                            body: e.to_string(),
                            is_error: true,
                        };
                    }
                }
            }
            KeyCode::PageUp => {
                if !self.provider_test_models.is_empty() {
                    self.provider_test_model_selected =
                        self.provider_test_model_selected.saturating_sub(5);
                    self.provider_test_model_buf =
                        self.provider_test_models[self.provider_test_model_selected].clone();
                    if field == 0 {
                        self.cursor_pos = self.provider_test_model_buf.len();
                    }
                }
            }
            KeyCode::PageDown => {
                if !self.provider_test_models.is_empty() {
                    let last = self.provider_test_models.len().saturating_sub(1);
                    self.provider_test_model_selected =
                        (self.provider_test_model_selected + 5).min(last);
                    self.provider_test_model_buf =
                        self.provider_test_models[self.provider_test_model_selected].clone();
                    if field == 0 {
                        self.cursor_pos = self.provider_test_model_buf.len();
                    }
                }
            }
            KeyCode::Tab if field == 0 => {
                if let Some(completed) = complete_provider_test_model(
                    &self.provider_test_models,
                    &self.provider_test_model_buf,
                ) {
                    self.provider_test_model_buf = completed;
                    self.cursor_pos = self.provider_test_model_buf.len();
                    self.sync_provider_test_model_selection_from_buffer();
                }
            }
            _ if field == 0 && is_prev_provider_test_model_key(code, modifiers) => {
                if !self.provider_test_models.is_empty() {
                    if self.provider_test_model_selected == 0 {
                        self.provider_test_model_selected = self.provider_test_models.len() - 1;
                    } else {
                        self.provider_test_model_selected -= 1;
                    }
                    self.provider_test_model_buf =
                        self.provider_test_models[self.provider_test_model_selected].clone();
                    self.cursor_pos = self.provider_test_model_buf.len();
                }
            }
            _ if field == 0 && is_next_provider_test_model_key(code, modifiers) => {
                if !self.provider_test_models.is_empty() {
                    self.provider_test_model_selected =
                        (self.provider_test_model_selected + 1) % self.provider_test_models.len();
                    self.provider_test_model_buf =
                        self.provider_test_models[self.provider_test_model_selected].clone();
                    self.cursor_pos = self.provider_test_model_buf.len();
                }
            }
            _ if is_next_provider_test_field_key(code, modifiers) => {
                let next_field = (field + 1) % 2;
                self.cursor_pos = if next_field == 0 {
                    self.provider_test_model_buf.len()
                } else {
                    self.provider_test_prompt_buf.len()
                };
                self.mode = Mode::ProviderAnthropicTest {
                    provider_id,
                    key_id,
                    source,
                    field: next_field,
                };
            }
            _ if is_prev_provider_test_field_key(code, modifiers) => {
                let next_field = if field == 0 { 1 } else { field - 1 };
                self.cursor_pos = if next_field == 0 {
                    self.provider_test_model_buf.len()
                } else {
                    self.provider_test_prompt_buf.len()
                };
                self.mode = Mode::ProviderAnthropicTest {
                    provider_id,
                    key_id,
                    source,
                    field: next_field,
                };
            }
            _ => {
                let consumed = if field == 0 {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.provider_test_model_buf,
                        &mut self.cursor_pos,
                        true,
                    )
                } else {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.provider_test_prompt_buf,
                        &mut self.cursor_pos,
                        true,
                    )
                };
                if consumed && field == 0 {
                    self.sync_provider_test_model_selection_from_buffer();
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_provider_anthropic_outcome(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (provider_id, key_id, source, field) = match &self.mode {
            Mode::ProviderAnthropicOutcome {
                provider_id,
                key_id,
                source,
                field,
                ..
            } => (provider_id.clone(), key_id.clone(), *source, *field),
            _ => return Ok(()),
        };

        self.mode =
            provider_test_outcome_next_mode(code, modifiers, &provider_id, &key_id, source, field);
        if matches!(self.mode, Mode::ProviderAnthropicTest { field: 0, .. }) {
            self.cursor_pos = self.provider_test_model_buf.len();
        } else if matches!(self.mode, Mode::ProviderAnthropicTest { field: 1, .. }) {
            self.cursor_pos = self.provider_test_prompt_buf.len();
        }
        Ok(())
    }

    pub(super) fn handle_provider_test_key_list(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = Mode::Normal;
            }
            _ if Self::is_prev_list_key(code, modifiers) => {
                if self.provider_key_selected > 0 {
                    self.provider_key_selected -= 1;
                } else if !self.provider_keys_cache.is_empty() {
                    self.provider_key_selected = self.provider_keys_cache.len() - 1;
                }
            }
            _ if Self::is_next_list_key(code, modifiers) => {
                if self.provider_key_selected + 1 < self.provider_keys_cache.len() {
                    self.provider_key_selected += 1;
                } else {
                    self.provider_key_selected = 0;
                }
            }
            KeyCode::Char('t') => {
                let provider_id = match &self.mode {
                    Mode::ProviderTestKeyList { provider_id } => provider_id.clone(),
                    _ => return Ok(()),
                };
                let provider = self.manager.get_provider(&provider_id)?;
                let Some(key) = self.selected_provider_key().cloned() else {
                    self.show_message("Select a provider key first.".into(), true, None);
                    return Ok(());
                };
                let return_mode =
                    provider_test_return_mode(ProviderTestSource::TestKeyList, &provider.id);
                match discover_models(&provider.base_url, &key.api_key) {
                    Ok(discovery) => {
                        let mut models: Vec<String> = discovery
                            .models
                            .into_iter()
                            .map(|model| trim_model_context_suffix(&model).to_string())
                            .collect();
                        models.sort();
                        models.dedup();
                        let preview = models
                            .iter()
                            .take(6)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        let summary = if models.is_empty() {
                            format!(
                                "Provider '{}' key '{}' returned no models.",
                                provider.name, key.name
                            )
                        } else {
                            format!(
                                "Provider '{}' key '{}': {} models via {} [{}]",
                                provider.name,
                                key.name,
                                models.len(),
                                discovery.endpoint_used,
                                preview
                            )
                        };
                        self.show_message(summary, false, return_mode);
                    }
                    Err(failure) => {
                        let tried = if failure.tried_endpoints.is_empty() {
                            String::new()
                        } else {
                            format!(". Tried: {}", failure.tried_endpoints.join(" → "))
                        };
                        self.show_message(
                            format!(
                                "Provider '{}' key '{}' could not discover models: {}{}. The provider may still work with a manually entered model name.",
                                provider.name, key.name, failure.message, tried
                            ),
                            false,
                            return_mode,
                        );
                    }
                }
            }
            KeyCode::Char('T') => {
                self.start_provider_key_test()?;
            }
            _ => {}
        }
        Ok(())
    }
}
