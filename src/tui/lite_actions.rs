use super::*;

pub(super) fn handle_lite_provider_select(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<()> {
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(());
    }

    match code {
        _ if App::is_cancel_key(code, modifiers) => app.mode = Mode::Normal,
        _ if App::is_prev_list_key(code, modifiers) => app.move_provider_up(),
        _ if App::is_next_list_key(code, modifiers) => app.move_provider_down(),
        KeyCode::Enter => {
            let provider = app
                .provider_list_state
                .selected()
                .and_then(|i| app.providers_cache.get(i))
                .cloned();
            let Some(provider) = provider else {
                return Ok(());
            };

            app.provider_keys_cache = app.manager.list_keys(&provider.id)?;
            if app.provider_keys_cache.is_empty() {
                app.mode = Mode::Message(
                    format!(
                        "Provider '{}' has no keys. Add a key in Provider Manager first.",
                        provider.name
                    ),
                    true,
                );
                return Ok(());
            }

            app.lite_provider_id = Some(provider.id.clone());
            app.lite_url = provider.base_url;
            app.lite_provider_keys = app.provider_keys_cache.clone();
            app.provider_key_selected = 0;
            if let Some(key) = app.provider_keys_cache.first() {
                app.lite_key_id = Some(key.id.clone());
                app.lite_token = key.api_key.clone();
            }
            app.mode = Mode::LiteKeySelect {
                provider_id: provider.id,
            };
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_lite_key_select(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<()> {
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(());
    }

    match code {
        _ if App::is_cancel_key(code, modifiers) => app.mode = Mode::LiteProviderSelect,
        _ if App::is_prev_list_key(code, modifiers) => app.move_provider_key_up(),
        _ if App::is_next_list_key(code, modifiers) => app.move_provider_key_down(),
        KeyCode::Enter => {
            let Some(key) = app.selected_provider_key().cloned() else {
                return Ok(());
            };
            app.lite_key_id = Some(key.id.clone());
            app.lite_token = key.api_key;
            app.lite_provider_keys = app.provider_keys_cache.clone();
            open_lite_model_builder(app);
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn handle_lite_model_select(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Result<()> {
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(());
    }
    let models_per_page: usize = 8;
    let is_edit = matches!(app.mode, Mode::LiteEdit { .. });
    let total_steps: usize = 11;

    match code {
        _ if App::is_cancel_key(code, modifiers) => app.mode = Mode::Normal,
        _ if App::is_next_field_key(code, modifiers) => {
            app.lite_step = (app.lite_step + 1) % total_steps;
            app.cursor_pos = lite_cursor_pos_for_step(app, app.lite_step);
        }
        _ if App::is_prev_field_key(code, modifiers) => {
            app.lite_step = if app.lite_step == 0 {
                total_steps - 1
            } else {
                app.lite_step - 1
            };
            app.cursor_pos = lite_cursor_pos_for_step(app, app.lite_step);
        }
        KeyCode::Tab => {
            if app.lite_step >= 2 && app.lite_step <= 6 && !app.lite_models.is_empty() {
                let current = current_slot_value(app);
                if let Some(pos) = app.lite_models.iter().position(|m| m == &current) {
                    let next = (pos + 1) % app.lite_models.len();
                    set_slot_value(app, app.lite_models[next].clone());
                } else if !current.is_empty()
                    && let Some(m) = app.lite_models.iter().find(|m| m.contains(&current))
                {
                    set_slot_value(app, m.clone());
                }
            } else if app.lite_step == 7 {
                let prefix = app.input_buffer.split('=').next().unwrap_or("");
                let mut vars: Vec<&str> = crate::env_vars::all_var_names().to_vec();
                vars.push("CLAUDE_SWITCH_TINYFISH");
                vars.sort();
                vars.dedup();
                if let Some(pos) = vars.iter().position(|v| v == &prefix) {
                    let next = (pos + 1) % vars.len();
                    app.input_buffer = format!("{}=", vars[next]);
                    app.cursor_pos = app.input_buffer.len();
                } else if !prefix.is_empty()
                    && let Some(v) = vars.iter().find(|v| v.starts_with(prefix))
                {
                    app.input_buffer = format!("{}=", v);
                    app.cursor_pos = app.input_buffer.len();
                }
            } else if app.lite_step == 8 {
                let flags = crate::cli_args::all_flag_names();
                if !flags.is_empty() {
                    let last_word = app.lite_launch_args.split_whitespace().last().unwrap_or("");
                    if let Some(pos) = flags.iter().position(|f| f == &last_word) {
                        let next = (pos + 1) % flags.len();
                        app.lite_launch_args =
                            replace_last_word(&app.lite_launch_args, flags[next]);
                        app.cursor_pos = app.lite_launch_args.len();
                    } else if !last_word.is_empty()
                        && let Some(f) = flags.iter().find(|f| f.starts_with(last_word))
                    {
                        app.lite_launch_args = replace_last_word(&app.lite_launch_args, f);
                        app.cursor_pos = app.lite_launch_args.len();
                    }
                }
            } else if app.lite_step == 9 {
                if !app.providers_cache.is_empty() {
                    let current = app.lite_provider_id.clone().unwrap_or_default();
                    let pos = app
                        .providers_cache
                        .iter()
                        .position(|p| p.id == current)
                        .map(|p| (p + 1) % app.providers_cache.len())
                        .unwrap_or(0);
                    let prov = &app.providers_cache[pos];
                    app.lite_provider_id = Some(prov.id.clone());
                    app.lite_provider_keys = {
                        let mut ks: Vec<_> = prov.keys.values().cloned().collect();
                        ks.sort_by(|a, b| a.name.cmp(&b.name));
                        ks
                    };
                    app.lite_key_id = app.lite_provider_keys.first().map(|k| k.id.clone());
                    app.lite_token = app
                        .lite_provider_keys
                        .first()
                        .map(|k| k.api_key.clone())
                        .unwrap_or_default();
                    app.lite_url = prov.base_url.clone();
                }
            } else if app.lite_step == 10 {
                if !app.lite_provider_keys.is_empty() {
                    let current = app.lite_key_id.as_deref().unwrap_or("");
                    let pos = app
                        .lite_provider_keys
                        .iter()
                        .position(|k| k.id == current)
                        .map(|p| (p + 1) % app.lite_provider_keys.len())
                        .unwrap_or(0);
                    app.lite_key_id = Some(app.lite_provider_keys[pos].id.clone());
                    app.lite_token = app.lite_provider_keys[pos].api_key.clone();
                }
            } else {
                app.lite_step = (app.lite_step + 1) % total_steps;
                app.cursor_pos = lite_cursor_pos_for_step(app, app.lite_step);
            }
        }
        KeyCode::Char('m')
            if modifiers.contains(KeyModifiers::CONTROL)
                && app.lite_step >= 2
                && app.lite_step <= 6 =>
        {
            let idx = app.lite_step - 2;
            app.lite_1m[idx] = !app.lite_1m[idx];
            let normalized =
                apply_model_1m_flag(&current_slot_value(app), app.lite_1m[idx]).to_string();
            set_slot_value(app, normalized);
        }
        KeyCode::Char('p')
            if modifiers.contains(KeyModifiers::ALT)
                && app.lite_step >= 2
                && app.lite_step <= 6
                && !app.lite_models.is_empty() =>
        {
            let current = current_slot_value(app);
            if let Some(pos) = app.lite_models.iter().position(|m| m == &current) {
                let prev = if pos == 0 {
                    app.lite_models.len() - 1
                } else {
                    pos - 1
                };
                set_slot_value(app, app.lite_models[prev].clone());
            } else if !current.is_empty()
                && let Some(m) = app.lite_models.iter().find(|m| m.contains(&current))
            {
                set_slot_value(app, m.clone());
            }
        }
        KeyCode::Char('n')
            if modifiers.contains(KeyModifiers::ALT)
                && app.lite_step >= 2
                && app.lite_step <= 6
                && !app.lite_models.is_empty() =>
        {
            let current = current_slot_value(app);
            if let Some(pos) = app.lite_models.iter().position(|m| m == &current) {
                let next = (pos + 1) % app.lite_models.len();
                set_slot_value(app, app.lite_models[next].clone());
            } else if !current.is_empty()
                && let Some(m) = app.lite_models.iter().find(|m| m.contains(&current))
            {
                set_slot_value(app, m.clone());
            }
        }
        KeyCode::PageDown => {
            let total = app.lite_models.len();
            if app.lite_model_page + models_per_page < total {
                app.lite_model_page += models_per_page;
            }
        }
        KeyCode::PageUp => {
            app.lite_model_page = app.lite_model_page.saturating_sub(models_per_page);
        }
        KeyCode::Enter => {
            if app.lite_step == 7 {
                let val = app.input_buffer.trim().to_string();
                if !val.is_empty() && val.contains('=') {
                    app.lite_extras.push(val);
                }
                app.input_buffer.clear();
                return Ok(());
            }

            let name = app.lite_name.trim().to_string();
            if name.is_empty() {
                app.mode = Mode::Message("Enter a profile name first".to_string(), false);
                return Ok(());
            }
            let alias = app.lite_alias.trim().to_string();
            let alias_opt = if alias.is_empty() {
                None
            } else {
                Some(alias.as_str())
            };

            let apply = |s: &str, idx: usize| -> Option<String> {
                if s.is_empty() {
                    None
                } else {
                    Some(apply_model_1m_flag(s, app.lite_1m[idx]).to_string())
                }
            };
            let env = LightweightEnv {
                auth_token: Some(app.lite_token.clone()),
                base_url: Some(app.lite_url.clone()),
                default_opus_model: apply(&app.lite_mod_opus, 0),
                default_sonnet_model: apply(&app.lite_mod_sonnet, 1),
                default_haiku_model: apply(&app.lite_mod_haiku, 2),
                model: apply(&app.lite_mod_model, 3),
                subagent_model: apply(&app.lite_mod_subagent, 4),
                extras: app.lite_extras.clone(),
            };

            if is_edit {
                let id = app.lite_edit_id.clone();
                match app.manager.update_lightweight(&id, &name, alias_opt, env) {
                    Ok(p) => {
                        let _ = app
                            .manager
                            .set_launch_args(&p.id, launch_args_from_str(&app.lite_launch_args));
                        if let Some(ref pid) = app.lite_provider_id {
                            if let Some(ref kid) = app.lite_key_id {
                                if let Err(e) = app.manager.set_provider(&p.id, pid, kid) {
                                    app.mode = Mode::Message(e.to_string(), true);
                                    return Ok(());
                                }
                            } else {
                                app.mode =
                                    Mode::Message("Select a provider key first.".into(), true);
                                return Ok(());
                            }
                        } else if let Err(e) = app.manager.unset_provider(&p.id) {
                            app.mode = Mode::Message(e.to_string(), true);
                            return Ok(());
                        }
                        app.sync_shims();
                        app.refresh()?;
                        app.select_by_id(&p.id);
                        app.mode = Mode::Message(format!("Profile '{}' updated.", p.name), false);
                    }
                    Err(e) => app.mode = Mode::Message(e.to_string(), true),
                }
            } else {
                match app
                    .manager
                    .create_lightweight_profile(&name, alias_opt, env)
                {
                    Ok(p) => {
                        let _ = app
                            .manager
                            .set_launch_args(&p.id, launch_args_from_str(&app.lite_launch_args));
                        if let Some(ref pid) = app.lite_provider_id {
                            if let Some(ref kid) = app.lite_key_id {
                                if let Err(e) = app.manager.set_provider(&p.id, pid, kid) {
                                    app.mode = Mode::Message(e.to_string(), true);
                                    return Ok(());
                                }
                            } else {
                                app.mode =
                                    Mode::Message("Select a provider key first.".into(), true);
                                return Ok(());
                            }
                        } else if let Err(e) = app.manager.unset_provider(&p.id) {
                            app.mode = Mode::Message(e.to_string(), true);
                            return Ok(());
                        }
                        app.sync_shims();
                        app.refresh()?;
                        app.select_by_id(&p.id);
                        app.mode = Mode::Message(format!("Profile '{}' created.", p.name), false);
                    }
                    Err(e) => app.mode = Mode::Message(e.to_string(), true),
                }
            }
        }
        KeyCode::Backspace => match app.lite_step {
            0 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_name,
                    &mut app.cursor_pos,
                    true,
                );
            }
            1 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_alias,
                    &mut app.cursor_pos,
                    false,
                );
            }
            2 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_opus,
                    &mut app.cursor_pos,
                    true,
                );
            }
            3 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_sonnet,
                    &mut app.cursor_pos,
                    true,
                );
            }
            4 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_haiku,
                    &mut app.cursor_pos,
                    true,
                );
            }
            5 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_model,
                    &mut app.cursor_pos,
                    true,
                );
            }
            6 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_subagent,
                    &mut app.cursor_pos,
                    true,
                );
            }
            7 => {
                if !app.input_buffer.is_empty() {
                    let _ = emacs_edit(
                        code,
                        modifiers,
                        &mut app.input_buffer,
                        &mut app.cursor_pos,
                        true,
                    );
                } else if !app.lite_extras.is_empty() {
                    app.lite_extras.pop();
                }
            }
            8 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_launch_args,
                    &mut app.cursor_pos,
                    true,
                );
            }
            9 => {
                app.lite_provider_id = None;
                app.lite_key_id = None;
                app.lite_provider_keys.clear();
            }
            _ => {}
        },
        _ if app.lite_step <= 8 => match app.lite_step {
            0 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_name,
                    &mut app.cursor_pos,
                    true,
                );
            }
            1 => {
                if let KeyCode::Char(c) = code {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        let _ = emacs_edit(
                            code,
                            modifiers,
                            &mut app.lite_alias,
                            &mut app.cursor_pos,
                            true,
                        );
                    }
                } else {
                    let _ = emacs_edit(
                        code,
                        modifiers,
                        &mut app.lite_alias,
                        &mut app.cursor_pos,
                        false,
                    );
                }
            }
            2 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_opus,
                    &mut app.cursor_pos,
                    true,
                );
            }
            3 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_sonnet,
                    &mut app.cursor_pos,
                    true,
                );
            }
            4 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_haiku,
                    &mut app.cursor_pos,
                    true,
                );
            }
            5 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_model,
                    &mut app.cursor_pos,
                    true,
                );
            }
            6 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_mod_subagent,
                    &mut app.cursor_pos,
                    true,
                );
            }
            7 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.input_buffer,
                    &mut app.cursor_pos,
                    true,
                );
            }
            8 => {
                let _ = emacs_edit(
                    code,
                    modifiers,
                    &mut app.lite_launch_args,
                    &mut app.cursor_pos,
                    true,
                );
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}
