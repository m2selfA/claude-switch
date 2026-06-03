use super::*;

impl App {
    pub(super) fn handle_provider_smart_paste(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.reset_provider_smart_input();
            self.mode = if self.page == Page::Provider {
                Mode::Normal
            } else {
                Mode::ProviderList
            };
            return Ok(());
        }

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.reset_provider_smart_input();
                self.mode = if self.page == Page::Provider {
                    Mode::Normal
                } else {
                    Mode::ProviderList
                };
            }
            KeyCode::Enter => {
                let raw = self.provider_smart_paste_buf.trim();
                if raw.is_empty() {
                    return Ok(());
                }
                match parse_provider_smart_paste(raw) {
                    Ok(parsed) => self.apply_provider_smart_paste(parsed)?,
                    Err(e) => {
                        self.provider_smart_paste_error = Some(e.to_string());
                        self.cursor_pos = self.provider_smart_paste_buf.len();
                    }
                }
            }
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.provider_smart_paste_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    pub(super) fn reset_provider_smart_input(&mut self) {
        self.provider_add_existing_id = None;
        self.provider_name_buf.clear();
        self.provider_url_buf.clear();
        self.provider_key_name_buf.clear();
        self.provider_key_buf.clear();
        self.provider_smart_paste_buf.clear();
        self.provider_smart_paste_error = None;
        self.input_buffer.clear();
        self.cursor_pos = 0;
    }

    pub(super) fn start_provider_smart_input(&mut self) -> Result<()> {
        self.reset_provider_smart_input();
        self.providers_cache = self.manager.list_providers().unwrap_or_default();
        match Clipboard::new().and_then(|mut clip| clip.get_text()) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    self.mode = Mode::ProviderSmartPaste;
                    return Ok(());
                }
                match parse_provider_smart_paste(trimmed) {
                    Ok(parsed) => self.apply_provider_smart_paste(parsed),
                    Err(e) => {
                        self.provider_smart_paste_buf = text;
                        self.provider_smart_paste_error = Some(e.to_string());
                        self.cursor_pos = self.provider_smart_paste_buf.len();
                        self.mode = Mode::ProviderSmartPaste;
                        Ok(())
                    }
                }
            }
            Err(e) => {
                self.provider_smart_paste_error = Some(format!(
                    "Could not read clipboard: {}. Paste provider data manually and press Enter.",
                    e
                ));
                self.mode = Mode::ProviderSmartPaste;
                Ok(())
            }
        }
    }

    pub(super) fn apply_provider_smart_paste(&mut self, parsed: SmartProviderPaste) -> Result<()> {
        let provider_name = if parsed.name.trim().is_empty() {
            inferred_provider_name(&parsed.base_url)
        } else {
            parsed.name
        };
        let key_name = if parsed.key_name.trim().is_empty() {
            "Default".to_string()
        } else {
            parsed.key_name
        };

        if let Some(existing) = self
            .providers_cache
            .iter()
            .find(|p| p.base_url == parsed.base_url)
            .cloned()
        {
            if existing
                .keys
                .values()
                .any(|key| key.api_key == parsed.api_key)
            {
                self.reset_provider_smart_input();
                self.mode = Mode::Message(
                    format!(
                        "Provider '{}' already has this key. Nothing added.",
                        existing.name
                    ),
                    true,
                );
                return Ok(());
            }

            self.provider_add_existing_id = Some(existing.id);
            self.provider_name_buf = existing.name;
            self.provider_url_buf = existing.base_url;
            self.provider_key_name_buf = key_name;
            self.provider_key_buf = parsed.api_key;
            self.cursor_pos = self.provider_key_name_buf.len();
            self.mode = Mode::ProviderAdd { step: 0 };
            return Ok(());
        }

        self.reset_provider_smart_input();
        self.provider_add_existing_id = None;
        self.provider_name_buf = provider_name;
        self.provider_url_buf = parsed.base_url;
        self.provider_key_name_buf = key_name;
        self.provider_key_buf = parsed.api_key;
        self.cursor_pos = self.provider_name_buf.len();
        self.mode = Mode::ProviderAdd { step: 0 };
        Ok(())
    }
}
