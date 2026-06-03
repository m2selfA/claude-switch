use super::*;

pub(super) fn current_slot_value(app: &App) -> String {
    match app.lite_step {
        2 => app.lite_mod_opus.clone(),
        3 => app.lite_mod_sonnet.clone(),
        4 => app.lite_mod_haiku.clone(),
        5 => app.lite_mod_model.clone(),
        6 => app.lite_mod_subagent.clone(),
        _ => String::new(),
    }
}

pub(super) fn lite_cursor_pos_for_step(app: &App, step: usize) -> usize {
    match step {
        0 => app.lite_name.len(),
        1 => app.lite_alias.len(),
        2 => app.lite_mod_opus.len(),
        3 => app.lite_mod_sonnet.len(),
        4 => app.lite_mod_haiku.len(),
        5 => app.lite_mod_model.len(),
        6 => app.lite_mod_subagent.len(),
        7 => app.input_buffer.len(),
        8 => app.lite_launch_args.len(),
        _ => 0,
    }
}

pub(super) fn set_slot_value(app: &mut App, val: String) {
    match app.lite_step {
        2 => app.lite_mod_opus = val,
        3 => app.lite_mod_sonnet = val,
        4 => app.lite_mod_haiku = val,
        5 => app.lite_mod_model = val,
        6 => app.lite_mod_subagent = val,
        _ => {}
    }
    app.cursor_pos = lite_cursor_pos_for_step(app, app.lite_step);
}

pub(super) fn reset_lite_builder(app: &mut App) {
    app.lite_token.clear();
    app.lite_url = "https://api.anthropic.com".to_string();
    app.lite_provider_id = None;
    app.lite_key_id = None;
    app.lite_provider_keys.clear();
    app.provider_keys_cache.clear();
    app.provider_key_selected = 0;
    app.lite_name.clear();
    app.lite_alias.clear();
    app.lite_step = 0;
    app.lite_models.clear();
    app.lite_model_fetch_state = ModelFetchState::Loaded;
    app.lite_model_page = 0;
    app.lite_1m = [false; 5];
    app.lite_extras.clear();
    app.lite_launch_args = "--dangerously-skip-permissions".to_string();
    app.lite_mod_opus.clear();
    app.lite_mod_sonnet.clear();
    app.lite_mod_haiku.clear();
    app.lite_mod_model.clear();
    app.lite_mod_subagent.clear();
    app.input_buffer.clear();
    app.cursor_pos = 0;
}

pub(super) fn set_lite_models_from_result(app: &mut App, fetched: Result<Vec<String>>) {
    match fetched {
        Ok(models) => {
            app.lite_models = models
                .into_iter()
                .map(|model| trim_model_context_suffix(&model).to_string())
                .collect();
            app.lite_models.sort();
            app.lite_models.dedup();
            app.lite_model_fetch_state = model_fetch_state_for_models(&app.lite_models);
        }
        Err(e) => {
            app.lite_models.clear();
            app.lite_model_fetch_state =
                ModelFetchState::Unavailable(model_fetch_unavailable_message(&e.to_string()));
        }
    }
}

pub(super) fn start_lite_profile_creation(app: &mut App) -> Result<()> {
    reset_lite_builder(app);
    app.providers_cache = app.manager.list_providers()?;
    app.provider_list_state = ListState::default();
    app.provider_list_scroll = ScrollbarState::default();

    if app.providers_cache.is_empty() {
        app.mode = Mode::Message(
            "No providers found. Add one in Provider Manager first.".to_string(),
            true,
        );
        return Ok(());
    }

    app.provider_list_state.select(Some(0));
    app.mode = Mode::LiteProviderSelect;
    Ok(())
}

pub(super) fn open_lite_model_builder(app: &mut App) {
    app.mode = Mode::LiteFetching;
    set_lite_models_from_result(app, fetch_models(&app.lite_url, &app.lite_token));
    app.lite_step = 0;
    app.lite_model_page = 0;
    app.mode = Mode::LiteModelSelect {
        profile_name: String::new(),
        token: app.lite_token.clone(),
        base_url: app.lite_url.clone(),
        models: app.lite_models.clone(),
    };
}
