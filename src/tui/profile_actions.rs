use anyhow::Context;

use super::*;
use crate::profile::{
    LaunchOptions, LocalGatewayToolMode, RequestedLocalGatewayMode, tinyfish_available,
};

impl App {
    fn start_selected_profile_duplicate(&mut self) -> Result<()> {
        let Some(profile) = self.selected_profile().cloned() else {
            return Ok(());
        };
        self.lite_name = self.manager.suggest_duplicate_name(&profile.id)?;
        self.input_buffer = self.lite_name.clone();
        self.cursor_pos = self.input_buffer.len();
        self.mode = Mode::DuplicateProfileName {
            profile_id: profile.id,
        };
        Ok(())
    }

    fn open_local_gateway_launch_picker(
        &mut self,
        profile: &Profile,
        use_stored_args: bool,
    ) -> Result<bool> {
        let Some(base_url) = self.manager.resolved_local_gateway_base_url(profile)? else {
            return Ok(false);
        };
        self.local_gateway_mode_selected = 0;
        self.mode = Mode::LocalGatewayLaunchPicker {
            profile_id: profile.id.clone(),
            use_stored_args,
            base_url,
        };
        Ok(true)
    }

    fn launch_selected_profile(&mut self, profile: &Profile, use_stored_args: bool) -> Result<()> {
        ratatui::restore();
        println!(
            "Launching Claude with profile '{}' ({} extra args)...",
            profile.name,
            if use_stored_args { "with" } else { "without" }
        );
        self.manager.launch_claude(
            &profile.id,
            &[],
            LaunchOptions {
                use_stored_args,
                local_gateway_mode: RequestedLocalGatewayMode::Omitted,
            },
        )?;
        Ok(())
    }

    fn selected_local_gateway_launch_mode(&self) -> LocalGatewayToolMode {
        LocalGatewayToolMode::EXPLICIT[self
            .local_gateway_mode_selected
            .min(LocalGatewayToolMode::EXPLICIT.len().saturating_sub(1))]
    }

    pub(super) fn handle_local_gateway_launch_mode(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let (profile_id, use_stored_args, return_mode) = match &self.mode {
            Mode::LocalGatewayLaunchPicker {
                profile_id,
                use_stored_args,
                ..
            } => (profile_id.clone(), *use_stored_args, self.mode.clone()),
            _ => return Ok(()),
        };

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let local_gateway_mode = self.selected_local_gateway_launch_mode();
                if local_gateway_mode.requires_tinyfish() && !tinyfish_available() {
                    self.show_message(
                        "TinyFish is required for the selected local gateway mode but the 'tinyfish' command is unavailable.".into(),
                        true,
                        Some(return_mode),
                    );
                    return Ok(());
                }
                let profile = self
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .cloned()
                    .context("Selected profile no longer exists.")?;
                ratatui::restore();
                println!(
                    "Launching Claude with profile '{}' ({}) via {}...",
                    profile.name,
                    if use_stored_args {
                        "with extra args"
                    } else {
                        "without extra args"
                    },
                    local_gateway_mode.as_cli_value()
                );
                self.manager.launch_claude(
                    &profile.id,
                    &[],
                    LaunchOptions {
                        use_stored_args,
                        local_gateway_mode: RequestedLocalGatewayMode::Explicit(local_gateway_mode),
                    },
                )?;
            }
            _ if Self::is_prev_list_key(code, modifiers) => {
                if self.local_gateway_mode_selected == 0 {
                    self.local_gateway_mode_selected =
                        LocalGatewayToolMode::EXPLICIT.len().saturating_sub(1);
                } else {
                    self.local_gateway_mode_selected -= 1;
                }
            }
            _ if Self::is_next_list_key(code, modifiers) => {
                self.local_gateway_mode_selected =
                    (self.local_gateway_mode_selected + 1) % LocalGatewayToolMode::EXPLICIT.len();
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn handle_profile_page_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        match code {
            _ if Self::is_prev_list_key(code, modifiers) => self.move_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_down(),

            KeyCode::Char('/') => {
                self.search_query.clear();
                self.apply_filter();
                self.mode = Mode::Search;
            }

            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }

            KeyCode::Enter if modifiers.contains(KeyModifiers::SHIFT) => {
                if let Some(p) = self.selected_profile().cloned() {
                    self.launch_selected_profile(&p, false)?;
                }
            }

            KeyCode::Enter => {
                if let Some(p) = self.selected_profile().cloned() {
                    self.launch_selected_profile(&p, true)?;
                }
            }

            KeyCode::Char('t') => {
                lite::start_lite_profile_creation(self)?;
            }

            KeyCode::Char('T') => {
                self.start_public_site_prompt()?;
            }

            KeyCode::Char('M') => {
                self.start_selected_profile_mcp_picker()?;
            }

            KeyCode::Char('P') => {
                self.start_selected_profile_plugin_picker()?;
            }

            KeyCode::Char('g') => {
                if let Some(p) = self.selected_profile().cloned() {
                    self.open_local_gateway_launch_picker(&p, true)?;
                }
            }

            KeyCode::Char('a') => {
                self.mode = Mode::AddFullName;
                self.input_buffer.clear();
            }

            KeyCode::Char('c') if self.selected_profile().is_some() => {
                match self.start_selected_profile_duplicate() {
                    Ok(()) => {}
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }

            KeyCode::Char('d') | KeyCode::Delete if self.selected_profile().is_some() => {
                self.mode = Mode::ConfirmDelete;
            }

            KeyCode::Char('e') => {
                let profile = match self.selected_profile() {
                    Some(p) => p.clone(),
                    None => return Ok(false),
                };

                if profile.kind == ProfileKind::Lightweight {
                    if let Some(ref env) = profile.env {
                        let (resolved_token, resolved_url) = self
                            .manager
                            .resolve_credentials(&profile)
                            .unwrap_or_else(|_| {
                                (
                                    env.auth_token.clone(),
                                    env.base_url
                                        .clone()
                                        .or_else(|| Some("https://api.anthropic.com".to_string())),
                                )
                            });
                        self.lite_token = resolved_token.unwrap_or_default();
                        self.lite_url =
                            resolved_url.unwrap_or_else(|| "https://api.anthropic.com".to_string());
                        self.lite_provider_id = profile.provider_id.clone();
                        self.lite_key_id = profile.key_id.clone();
                        self.lite_mod_opus = strip_model_1m_suffix(
                            env.default_opus_model.as_deref().unwrap_or_default(),
                        )
                        .to_string();
                        self.lite_mod_sonnet = strip_model_1m_suffix(
                            env.default_sonnet_model.as_deref().unwrap_or_default(),
                        )
                        .to_string();
                        self.lite_mod_haiku = strip_model_1m_suffix(
                            env.default_haiku_model.as_deref().unwrap_or_default(),
                        )
                        .to_string();
                        self.lite_mod_model =
                            strip_model_1m_suffix(env.model.as_deref().unwrap_or_default())
                                .to_string();
                        self.lite_mod_subagent = strip_model_1m_suffix(
                            env.subagent_model.as_deref().unwrap_or_default(),
                        )
                        .to_string();
                        let ends_1m: [&str; 5] = [
                            env.default_opus_model.as_deref().unwrap_or_default(),
                            env.default_sonnet_model.as_deref().unwrap_or_default(),
                            env.default_haiku_model.as_deref().unwrap_or_default(),
                            env.model.as_deref().unwrap_or_default(),
                            env.subagent_model.as_deref().unwrap_or_default(),
                        ];
                        for (i, value) in ends_1m.iter().enumerate() {
                            self.lite_1m[i] = model_has_1m_suffix(value);
                        }
                        self.lite_name = profile.name.clone();
                        self.lite_alias = profile.alias.clone().unwrap_or_default();
                        self.lite_edit_id = profile.id.clone();
                        self.lite_step = 0;
                        self.lite_extras = env.extras.clone();
                        self.lite_launch_args =
                            profile.launch_args.map(|v| v.join(" ")).unwrap_or_default();
                        self.providers_cache = self.manager.list_providers().unwrap_or_default();
                        self.lite_provider_keys = if let Some(ref pid) = self.lite_provider_id {
                            self.providers_cache
                                .iter()
                                .find(|p| p.id == *pid)
                                .map(|prov| {
                                    let mut ks: Vec<_> = prov.keys.values().cloned().collect();
                                    ks.sort_by(|a, b| a.name.cmp(&b.name));
                                    ks
                                })
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        let token = self.lite_token.clone();
                        let base_url = self.lite_url.clone();
                        self.mode = Mode::LiteFetching;
                        lite::set_lite_models_from_result(self, fetch_models(&base_url, &token));
                        self.mode = Mode::LiteEdit {
                            profile_id: profile.id.clone(),
                        };
                    } else {
                        self.mode = Mode::Message("No env config found.".into(), true);
                    }
                } else {
                    self.lite_name = profile.name.clone();
                    self.lite_alias = profile.alias.clone().unwrap_or_default();
                    self.lite_launch_args =
                        profile.launch_args.map(|v| v.join(" ")).unwrap_or_default();
                    self.lite_provider_id = None;
                    self.lite_key_id = None;
                    self.lite_provider_keys.clear();
                    self.cursor_pos = self.lite_name.len();
                    self.mode = Mode::EditProfile {
                        profile_id: profile.id.clone(),
                        step: 0,
                    };
                }
            }

            KeyCode::Char('m') => {
                if let Some(p) = self.selected_profile()
                    && p.kind == ProfileKind::Lightweight
                {
                    for i in 0..5 {
                        self.lite_1m[i] = !self.lite_1m[i];
                    }
                }
            }

            KeyCode::Char('r') => {
                if let Some(p) = self.selected_profile() {
                    let id = p.id.clone();
                    let name = p.name.clone();
                    match self.manager.refresh_profile(&id) {
                        Ok(_) => {
                            self.refresh()?;
                            self.select_by_id(&id);
                            self.mode =
                                Mode::Message(format!("Profile '{}' refreshed.", name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }

            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_duplicate_profile_name(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let profile_id = match &self.mode {
            Mode::DuplicateProfileName { profile_id } => profile_id.clone(),
            _ => return Ok(()),
        };
        let return_mode = self.mode.clone();

        match code {
            KeyCode::Enter => {
                let name = self.input_buffer.trim().to_string();
                if name.is_empty() {
                    self.show_message(
                        "Profile name cannot be empty.".into(),
                        true,
                        Some(return_mode),
                    );
                    return Ok(());
                }
                self.lite_name = name.clone();
                match self.manager.suggest_duplicate_alias(&profile_id, &name) {
                    Ok(alias) => {
                        self.input_buffer = alias.unwrap_or_default();
                        self.cursor_pos = self.input_buffer.len();
                        self.mode = Mode::DuplicateProfileAlias { profile_id };
                    }
                    Err(e) => self.show_message(e.to_string(), true, Some(return_mode)),
                }
            }
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.input_buffer,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_duplicate_profile_alias(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let profile_id = match &self.mode {
            Mode::DuplicateProfileAlias { profile_id } => profile_id.clone(),
            _ => return Ok(()),
        };
        let return_mode = self.mode.clone();

        match code {
            KeyCode::Enter => {
                let alias = self.input_buffer.trim().to_string();
                let alias_opt = if alias.is_empty() {
                    None
                } else {
                    Some(alias.as_str())
                };
                match self.manager.duplicate_profile_with_alias_override(
                    &profile_id,
                    &self.lite_name,
                    alias_opt,
                    false,
                ) {
                    Ok(profile) => {
                        self.sync_shims();
                        self.refresh()?;
                        self.select_by_id(&profile.id);
                        self.mode =
                            Mode::Message(format!("Profile '{}' duplicated.", profile.name), false);
                    }
                    Err(e) => self.show_message(e.to_string(), true, Some(return_mode)),
                }
            }
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ => {
                if let KeyCode::Char(c) = code {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        emacs_edit(
                            code,
                            modifiers,
                            &mut self.input_buffer,
                            &mut self.cursor_pos,
                            true,
                        );
                    }
                } else {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.input_buffer,
                        &mut self.cursor_pos,
                        false,
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_search_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.search_query.clear();
                self.cursor_pos = 0;
                self.apply_filter();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
            }
            _ if Self::is_prev_filter_list_key(code, modifiers) => self.move_up(),
            _ if Self::is_next_filter_list_key(code, modifiers) => self.move_down(),
            _ => {
                if emacs_edit(
                    code,
                    modifiers,
                    &mut self.search_query,
                    &mut self.cursor_pos,
                    true,
                ) {
                    self.apply_filter();
                }
            }
        }
        Ok(false)
    }

    pub(super) fn handle_confirm_delete(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(p) = self.selected_profile() {
                    let name = p.name.clone();
                    let id = p.id.clone();
                    match self.manager.remove_profile(&id) {
                        Ok(_) => {
                            self.sync_shims();
                            self.refresh()?;
                            self.mode =
                                Mode::Message(format!("Profile '{}' removed.", name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }
            _ => self.mode = Mode::Normal,
        }
        Ok(())
    }

    pub(super) fn handle_add_full_name(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            KeyCode::Enter => {
                let name = self.input_buffer.trim().to_string();
                if name.is_empty() {
                    self.mode = Mode::Normal;
                    return Ok(());
                }
                self.lite_name = name;
                self.input_buffer.clear();
                self.mode = Mode::AddFullAlias;
            }
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.input_buffer,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_add_full_alias(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        match code {
            KeyCode::Enter => {
                let alias = self.input_buffer.trim().to_string();
                let alias_opt = if alias.is_empty() {
                    None
                } else {
                    Some(alias.as_str())
                };
                let name = self.lite_name.clone();
                match self.manager.add_profile(&name, alias_opt) {
                    Ok(p) => {
                        self.sync_shims();
                        self.refresh()?;
                        self.select_by_id(&p.id);
                        self.mode = Mode::Message(format!("Profile '{}' added.", name), false);
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ => {
                if let KeyCode::Char(c) = code {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        emacs_edit(
                            code,
                            modifiers,
                            &mut self.input_buffer,
                            &mut self.cursor_pos,
                            true,
                        );
                    }
                } else {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.input_buffer,
                        &mut self.cursor_pos,
                        false,
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_edit_profile(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let total_steps: usize = 3;
        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let step = match &self.mode {
                    Mode::EditProfile { step, .. } => *step,
                    _ => return Ok(()),
                };
                if step < total_steps - 1 {
                    let next_step = step + 1;
                    self.cursor_pos = match next_step {
                        0 => self.lite_name.len(),
                        1 => self.lite_alias.len(),
                        _ => self.lite_launch_args.len(),
                    };
                    self.mode = match &self.mode {
                        Mode::EditProfile { profile_id, .. } => Mode::EditProfile {
                            profile_id: profile_id.clone(),
                            step: next_step,
                        },
                        _ => return Ok(()),
                    };
                } else {
                    let new_name = self.lite_name.trim().to_string();
                    if new_name.is_empty() {
                        self.mode = Mode::Message("Profile name cannot be empty.".into(), true);
                        return Ok(());
                    }
                    let new_alias = self.lite_alias.trim().to_string();
                    let alias_opt = if new_alias.is_empty() {
                        None
                    } else {
                        Some(new_alias.as_str())
                    };
                    let id = match &self.mode {
                        Mode::EditProfile { profile_id, .. } => profile_id.clone(),
                        _ => return Ok(()),
                    };
                    let launch: Option<Vec<String>> = {
                        let s = self.lite_launch_args.trim();
                        if s.is_empty() {
                            None
                        } else {
                            Some(s.split_whitespace().map(String::from).collect())
                        }
                    };
                    match self.manager.rename_profile(&id, &new_name, alias_opt) {
                        Ok(p) => {
                            let _ = self.manager.set_launch_args(&p.id, launch);
                            self.sync_shims();
                            self.refresh()?;
                            self.select_by_id(&p.id);
                            self.mode =
                                Mode::Message(format!("Profile '{}' updated.", p.name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }
            KeyCode::Tab | KeyCode::Char('n')
                if code == KeyCode::Tab || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let next_step = match &self.mode {
                    Mode::EditProfile { step, .. } => (step + 1) % total_steps,
                    _ => return Ok(()),
                };
                self.cursor_pos = match next_step {
                    0 => self.lite_name.len(),
                    1 => self.lite_alias.len(),
                    _ => self.lite_launch_args.len(),
                };
                self.mode = match &self.mode {
                    Mode::EditProfile { profile_id, step } => Mode::EditProfile {
                        profile_id: profile_id.clone(),
                        step: (step + 1) % total_steps,
                    },
                    _ => return Ok(()),
                };
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                let step = match &self.mode {
                    Mode::EditProfile { step, .. } => *step,
                    _ => 0,
                };
                let next_step = (step + total_steps - 1) % total_steps;
                self.cursor_pos = match next_step {
                    0 => self.lite_name.len(),
                    1 => self.lite_alias.len(),
                    _ => self.lite_launch_args.len(),
                };
                self.mode = match &self.mode {
                    Mode::EditProfile { profile_id, .. } => Mode::EditProfile {
                        profile_id: profile_id.clone(),
                        step: next_step,
                    },
                    _ => return Ok(()),
                };
            }
            KeyCode::Backspace => {
                let step = match &self.mode {
                    Mode::EditProfile { step, .. } => *step,
                    _ => 0,
                };
                let buf = match step {
                    0 => &mut self.lite_name,
                    1 => &mut self.lite_alias,
                    _ => &mut self.lite_launch_args,
                };
                emacs_edit(code, modifiers, buf, &mut self.cursor_pos, false);
            }
            _ => {
                let step = match &self.mode {
                    Mode::EditProfile { step, .. } => *step,
                    _ => 0,
                };
                match step {
                    0 => {
                        emacs_edit(
                            code,
                            modifiers,
                            &mut self.lite_name,
                            &mut self.cursor_pos,
                            true,
                        );
                    }
                    1 => {
                        if let KeyCode::Char(c) = code {
                            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                                emacs_edit(
                                    code,
                                    modifiers,
                                    &mut self.lite_alias,
                                    &mut self.cursor_pos,
                                    true,
                                );
                            }
                        } else {
                            emacs_edit(
                                code,
                                modifiers,
                                &mut self.lite_alias,
                                &mut self.cursor_pos,
                                false,
                            );
                        }
                    }
                    2 => {
                        emacs_edit(
                            code,
                            modifiers,
                            &mut self.lite_launch_args,
                            &mut self.cursor_pos,
                            true,
                        );
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub(super) fn move_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) | None => self.filtered_indices.len() - 1,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
        self.list_scroll = self.list_scroll.position(i);
    }

    pub(super) fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => (i + 1) % self.filtered_indices.len(),
            None => 0,
        };
        self.list_state.select(Some(i));
        self.list_scroll = self.list_scroll.position(i);
    }
}
