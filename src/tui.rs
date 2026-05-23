use anyhow::{Result, bail};
use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::cli_args::all_flag_names;
use crate::env_vars::all_var_names;
use crate::profile::{
    LightweightEnv, ModelDiscoverySuccess, Profile, ProfileKind, ProfileManager, Provider,
    ProviderKey, discover_models, fetch_models, test_anthropic_message,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ── Palette ───────────────────────────────────────────────────────────────────
const ACCENT: Color = Color::Rgb(255, 149, 0);
const DIM: Color = Color::Rgb(100, 100, 110);
const SUCCESS: Color = Color::Rgb(80, 200, 120);
const DANGER: Color = Color::Rgb(220, 80, 80);
const BG: Color = Color::Rgb(14, 14, 18);
const PANEL: Color = Color::Rgb(22, 22, 28);
const BORDER: Color = Color::Rgb(50, 50, 60);
const TEXT: Color = Color::Rgb(220, 220, 230);
const MUTED: Color = Color::Rgb(140, 140, 155);
const SEARCH_HL: Color = Color::Rgb(255, 230, 140);

// ── Mode ──────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum Mode {
    FirstRun,
    Normal,
    Search,
    Help,
    ConfirmDelete,
    AddFullName,
    AddFullAlias,
    Message(String, bool),
    /// Lightweight creation: choose provider
    LiteProviderSelect,
    /// Lightweight creation: choose provider key
    LiteKeySelect {
        provider_id: String,
    },
    /// Fetching models spinner
    LiteFetching,
    /// Anthropic provider test: choose a model, then edit prompt and send.
    ProviderAnthropicTest {
        provider_id: String,
        key_id: String,
        source: ProviderTestSource,
        field: usize,
    },
    ProviderAnthropicOutcome {
        provider_id: String,
        key_id: String,
        source: ProviderTestSource,
        field: usize,
        model: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        body: String,
        is_error: bool,
    },
    /// Lightweight profile being built / edited
    /// Steps: 0=name, 1=alias, 2=opus, 3=sonnet, 4=haiku, 5=model, 6=subagent, 7=extras
    LiteModelSelect {
        profile_name: String,
        token: String,
        base_url: String,
        models: Vec<String>,
    },
    /// Editing an existing lightweight profile
    LiteEdit {
        profile_id: String,
    },
    /// Edit name/alias for any profile type
    EditProfile {
        profile_id: String,
        step: usize, // 0=name, 1=alias
    },
    /// Provider list browsing
    ProviderList,
    /// Adding a new provider (step: 0=name, 1=url, 2=token)
    ProviderAdd {
        step: usize,
    },
    /// Smart input raw provider data, then continue provider add.
    ProviderSmartPaste,
    /// Editing provider (step: 0=name, 1=url, 2=keys)
    ProviderEdit {
        provider_id: String,
        step: usize,
    },
    /// Adding a key from within provider edit
    ProviderEditKeyInput {
        provider_id: String,
        step: usize,
    },
    /// Key list for a specific provider
    ProviderKeyList {
        provider_id: String,
    },
    /// Key picker used only for provider tests on multi-key providers
    ProviderTestKeyList {
        provider_id: String,
    },
    /// Add a key to a provider (step: 0=name, 1=token)
    ProviderKeyAdd {
        provider_id: String,
        step: usize,
    },
    /// Edit an existing key (step: 0=name, 1=token)
    ProviderKeyEdit {
        provider_id: String,
        key_id: String,
        step: usize,
        source: KeyEditSource,
    },
    /// Confirm delete provider
    ConfirmDeleteProvider {
        provider_id: String,
        name: String,
    },
    /// Confirm delete key
    ConfirmDeleteKey {
        provider_id: String,
        key_id: String,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    ProfileManager,
    ProviderManager,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum KeyEditSource {
    ProviderKeyList,
    ProviderEdit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProviderTestSource {
    Page,
    KeyList,
    TestKeyList,
}

#[derive(Debug, Clone, PartialEq)]
enum ProviderTestKeySelection {
    NoKeys,
    Single(ProviderKey),
    Multiple,
}

#[derive(Debug, Clone, PartialEq)]
enum ModelFetchState {
    Loaded,
    Empty,
    Unavailable(String),
}

/// Emacs-style text editing on a String buffer with cursor position.
/// Returns true if the key was consumed.
fn emacs_edit(
    code: KeyCode,
    modifiers: KeyModifiers,
    buf: &mut String,
    pos: &mut usize,
    accept_char: bool,
) -> bool {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    // Clamp cursor to a UTF-8 character boundary.
    *pos = nearest_prev_char_boundary(buf, *pos);
    if *pos > buf.len() {
        *pos = buf.len();
    }
    match code {
        KeyCode::Char('a') if ctrl => {
            *pos = 0;
            true
        }
        KeyCode::Char('e') if ctrl => {
            *pos = buf.len();
            true
        }
        KeyCode::Char('b') if ctrl => {
            *pos = prev_char_boundary(buf, *pos);
            true
        }
        KeyCode::Char('f') if ctrl => {
            *pos = next_char_boundary(buf, *pos);
            true
        }
        KeyCode::Char('d') if ctrl => {
            if *pos < buf.len() {
                let next = next_char_boundary(buf, *pos);
                if next > *pos {
                    buf.drain(*pos..next);
                }
            }
            true
        }
        KeyCode::Char('k') if ctrl => {
            buf.truncate(*pos);
            true
        }
        KeyCode::Char('u') if ctrl => {
            if *pos > 0 {
                buf.drain(..*pos);
                *pos = 0;
            }
            true
        }
        KeyCode::Char('w') if ctrl => {
            let mut new_pos = None;
            for (idx, ch) in buf[..*pos].char_indices().rev() {
                if !(ch.is_alphanumeric() || ch == '_' || ch == '-') {
                    new_pos = Some(idx + ch.len_utf8());
                    break;
                }
            }
            if let Some(new_pos) = new_pos {
                if new_pos < *pos {
                    buf.drain(new_pos..*pos);
                    *pos = new_pos;
                }
            } else if *pos > 0 {
                buf.drain(..*pos);
                *pos = 0;
            }
            true
        }
        KeyCode::Backspace => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Char('h') if ctrl => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Char('\u{8}') | KeyCode::Char('\u{7f}') => {
            let prev = prev_char_boundary(buf, *pos);
            if prev < *pos {
                buf.drain(prev..*pos);
                *pos = prev;
            }
            true
        }
        KeyCode::Left => {
            *pos = prev_char_boundary(buf, *pos);
            true
        }
        KeyCode::Right => {
            *pos = next_char_boundary(buf, *pos);
            true
        }
        KeyCode::Home => {
            *pos = 0;
            true
        }
        KeyCode::End => {
            *pos = buf.len();
            true
        }
        KeyCode::Delete => {
            if *pos < buf.len() {
                let next = next_char_boundary(buf, *pos);
                if next > *pos {
                    buf.drain(*pos..next);
                }
            }
            true
        }
        KeyCode::Char(c) if accept_char => {
            buf.insert(*pos, c);
            *pos = next_char_boundary(buf, *pos);
            true
        }
        _ => false,
    }
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut pos = nearest_prev_char_boundary(s, pos);
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn nearest_prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut pos = pos.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let pos = pos.min(s.len());
    if pos >= s.len() {
        return s.len();
    }
    let mut next = pos + 1;
    while next < s.len() && !s.is_char_boundary(next) {
        next += 1;
    }
    next
}

fn display_with_cursor(value: &str, cursor_pos: usize) -> String {
    let pos = nearest_prev_char_boundary(value, cursor_pos);
    format!("{}█{}", &value[..pos], &value[pos..])
}

fn insert_str_at_cursor(buf: &mut String, cursor_pos: &mut usize, text: &str) {
    *cursor_pos = nearest_prev_char_boundary(buf, *cursor_pos);
    buf.insert_str(*cursor_pos, text);
    *cursor_pos += text.len();
}

fn insert_filtered_str_at_cursor(
    buf: &mut String,
    cursor_pos: &mut usize,
    text: &str,
    keep: impl Fn(char) -> bool,
) {
    let filtered: String = text.chars().filter(|ch| keep(*ch)).collect();
    insert_str_at_cursor(buf, cursor_pos, &filtered);
}

fn is_alias_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn provider_key_cursor_pos(step: usize, key_name: &str, key: &str) -> usize {
    match step {
        0 => key_name.len(),
        _ => key.len(),
    }
}

fn provider_add_cursor_pos(
    step: usize,
    existing_id: Option<&str>,
    name: &str,
    url: &str,
    key_name: &str,
    key: &str,
) -> usize {
    match step {
        0 if existing_id.is_some() => key_name.len(),
        0 => name.len(),
        1 => url.len(),
        2 => key_name.len(),
        _ => key.len(),
    }
}

fn visible_window(selected: usize, total: usize, page_size: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let page_size = page_size.max(1);
    let start = (selected / page_size) * page_size;
    let end = (start + page_size).min(total);
    (start, end)
}

fn provider_edit_cursor_pos(step: usize, name: &str, url: &str) -> usize {
    match step {
        0 => name.len(),
        1 => url.len(),
        _ => 0,
    }
}

// ── App ───────────────────────────────────────────────────────────────────────
pub struct App {
    manager: ProfileManager,
    profiles: Vec<Profile>,
    list_state: ListState,
    list_scroll: ScrollbarState,
    mode: Mode,
    page: Page,
    input_buffer: String,
    cursor_pos: usize,
    search_query: String,
    filtered_indices: Vec<usize>,
    /// Per-slot [1m] suffix flags (opus, sonnet, haiku, model, subagent)
    lite_1m: [bool; 5],
    /// Current slot index in LiteModelSelect/LiteEdit:
    /// 0=name, 1=alias, 2=opus, 3=sonnet, 4=haiku, 5=model, 6=subagent, 7=extras
    lite_step: usize,
    /// Fetched models
    lite_models: Vec<String>,
    lite_model_fetch_state: ModelFetchState,
    /// Model list pagination offset
    lite_model_page: usize,
    /// Profile id being edited (for LiteEdit)
    lite_edit_id: String,
    /// Collected values
    lite_name: String,
    lite_alias: String,
    lite_token: String,
    lite_url: String,
    lite_provider_id: Option<String>,
    lite_key_id: Option<String>,
    lite_provider_keys: Vec<ProviderKey>,
    provider_list_state: ListState,
    provider_list_scroll: ScrollbarState,
    provider_name_buf: String,
    provider_url_buf: String,
    provider_key_buf: String,
    provider_key_name_buf: String,
    provider_add_existing_id: Option<String>,
    provider_smart_paste_buf: String,
    provider_smart_paste_error: Option<String>,
    provider_test_prompt_buf: String,
    provider_test_model_buf: String,
    provider_test_models: Vec<String>,
    provider_test_model_fetch_state: ModelFetchState,
    provider_test_model_selected: usize,
    message_return_mode: Option<Mode>,
    providers_cache: Vec<Provider>,
    provider_keys_cache: Vec<ProviderKey>,
    provider_key_selected: usize,
    lite_mod_opus: String,
    lite_mod_sonnet: String,
    lite_mod_haiku: String,
    lite_mod_model: String,
    lite_mod_subagent: String,
    lite_extras: Vec<String>,
    lite_launch_args: String,
}

impl App {
    pub fn new(manager: ProfileManager) -> Result<Self> {
        let profiles = manager.list_profiles()?;
        let filtered_indices: Vec<usize> = (0..profiles.len()).collect();
        let mut list_state = ListState::default();
        if !profiles.is_empty() {
            list_state.select(Some(0));
        }

        let (mode, input_buffer) = if profiles.is_empty() {
            (Mode::FirstRun, "default".to_string())
        } else {
            (Mode::Normal, String::new())
        };

        Ok(Self {
            manager,
            profiles,
            list_state,
            list_scroll: ScrollbarState::default(),
            mode,
            page: Page::ProfileManager,
            input_buffer,
            cursor_pos: 0,
            search_query: String::new(),
            filtered_indices,
            lite_1m: [false; 5],
            lite_step: 0,
            lite_models: Vec::new(),
            lite_model_fetch_state: ModelFetchState::Loaded,
            lite_model_page: 0,
            lite_name: String::new(),
            lite_alias: String::new(),
            lite_token: String::new(),
            lite_url: "https://api.anthropic.com".to_string(),
            lite_provider_id: None,
            lite_key_id: None,
            lite_provider_keys: Vec::new(),
            provider_list_state: ListState::default(),
            provider_list_scroll: ScrollbarState::default(),
            provider_name_buf: String::new(),
            provider_url_buf: String::new(),
            provider_key_buf: String::new(),
            provider_key_name_buf: String::new(),
            provider_add_existing_id: None,
            provider_smart_paste_buf: String::new(),
            provider_smart_paste_error: None,
            provider_test_prompt_buf: "Hello".to_string(),
            provider_test_model_buf: String::new(),
            provider_test_models: Vec::new(),
            provider_test_model_fetch_state: ModelFetchState::Loaded,
            provider_test_model_selected: 0,
            message_return_mode: None,
            providers_cache: Vec::new(),
            provider_keys_cache: Vec::new(),
            provider_key_selected: 0,
            lite_edit_id: String::new(),
            lite_mod_opus: String::new(),
            lite_mod_sonnet: String::new(),
            lite_mod_haiku: String::new(),
            lite_mod_model: String::new(),
            lite_mod_subagent: String::new(),
            lite_extras: Vec::new(),
            lite_launch_args: "--dangerously-skip-permissions".to_string(),
        })
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn refresh(&mut self) -> Result<()> {
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

    fn apply_filter(&mut self) {
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

    fn is_manager_switch_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::BackTab)
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT))
    }

    fn mode_allows_manager_switch(&self) -> bool {
        matches!(self.mode, Mode::Normal | Mode::Search | Mode::ProviderList)
    }

    fn switch_manager_page(&mut self) -> Result<()> {
        self.page = match self.page {
            Page::ProfileManager => {
                self.providers_cache = self.manager.list_providers().unwrap_or_default();
                self.provider_list_state = ListState::default();
                if !self.providers_cache.is_empty() {
                    self.provider_list_state.select(Some(0));
                }
                Page::ProviderManager
            }
            Page::ProviderManager => {
                self.refresh()?;
                Page::ProfileManager
            }
        };
        self.mode = Mode::Normal;
        Ok(())
    }

    fn handle_manager_switch_key(
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

    fn is_cancel_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Esc)
            || (code == KeyCode::Char('g') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_prev_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up | KeyCode::Char('k'))
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_next_list_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down | KeyCode::Char('j'))
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_prev_selection_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up)
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_next_selection_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down)
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_prev_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Up)
            || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn is_next_field_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
        matches!(code, KeyCode::Down)
            || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL))
    }

    fn select_by_id(&mut self, id: &str) {
        if let Some(fi) = self
            .filtered_indices
            .iter()
            .position(|&i| self.profiles[i].id == id)
        {
            self.list_state.select(Some(fi));
            self.list_scroll = self.list_scroll.position(fi);
        }
    }

    fn selected_profile(&self) -> Option<&Profile> {
        self.list_state
            .selected()
            .and_then(|fi| self.filtered_indices.get(fi))
            .and_then(|&i| self.profiles.get(i))
    }

    fn move_up(&mut self) {
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

    fn move_down(&mut self) {
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

    fn current_slot_value(&self) -> String {
        match self.lite_step {
            2 => self.lite_mod_opus.clone(),
            3 => self.lite_mod_sonnet.clone(),
            4 => self.lite_mod_haiku.clone(),
            5 => self.lite_mod_model.clone(),
            6 => self.lite_mod_subagent.clone(),
            _ => String::new(),
        }
    }

    fn lite_cursor_pos_for_step(&self, step: usize) -> usize {
        match step {
            0 => self.lite_name.len(),
            1 => self.lite_alias.len(),
            2 => self.lite_mod_opus.len(),
            3 => self.lite_mod_sonnet.len(),
            4 => self.lite_mod_haiku.len(),
            5 => self.lite_mod_model.len(),
            6 => self.lite_mod_subagent.len(),
            7 => self.input_buffer.len(),
            8 => self.lite_launch_args.len(),
            _ => 0,
        }
    }

    fn set_slot_value(&mut self, val: String) {
        match self.lite_step {
            2 => self.lite_mod_opus = val,
            3 => self.lite_mod_sonnet = val,
            4 => self.lite_mod_haiku = val,
            5 => self.lite_mod_model = val,
            6 => self.lite_mod_subagent = val,
            _ => {}
        }
        self.cursor_pos = self.lite_cursor_pos_for_step(self.lite_step);
    }

    fn reset_lite_builder(&mut self) {
        self.lite_token.clear();
        self.lite_url = "https://api.anthropic.com".to_string();
        self.lite_provider_id = None;
        self.lite_key_id = None;
        self.lite_provider_keys.clear();
        self.provider_keys_cache.clear();
        self.provider_key_selected = 0;
        self.lite_name.clear();
        self.lite_alias.clear();
        self.lite_step = 0;
        self.lite_models.clear();
        self.lite_model_fetch_state = ModelFetchState::Loaded;
        self.lite_model_page = 0;
        self.lite_1m = [false; 5];
        self.lite_extras.clear();
        self.lite_launch_args = "--dangerously-skip-permissions".to_string();
        self.lite_mod_opus.clear();
        self.lite_mod_sonnet.clear();
        self.lite_mod_haiku.clear();
        self.lite_mod_model.clear();
        self.lite_mod_subagent.clear();
        self.input_buffer.clear();
        self.cursor_pos = 0;
    }

    fn set_lite_models_from_result(&mut self, fetched: Result<Vec<String>>) {
        match fetched {
            Ok(models) => {
                self.lite_models = models
                    .into_iter()
                    .map(|model| trim_model_context_suffix(&model).to_string())
                    .collect();
                self.lite_models.sort();
                self.lite_models.dedup();
                self.lite_model_fetch_state = model_fetch_state_for_models(&self.lite_models);
            }
            Err(e) => {
                self.lite_models.clear();
                self.lite_model_fetch_state =
                    ModelFetchState::Unavailable(model_fetch_unavailable_message(&e.to_string()));
            }
        }
    }

    fn set_provider_test_models_from_result(
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

    fn sync_provider_test_model_selection_from_buffer(&mut self) {
        if let Some(index) = self
            .provider_test_models
            .iter()
            .position(|model| model == &self.provider_test_model_buf)
        {
            self.provider_test_model_selected = index;
        }
    }

    fn start_lite_profile_creation(&mut self) -> Result<()> {
        self.reset_lite_builder();
        self.providers_cache = self.manager.list_providers()?;
        self.provider_list_state = ListState::default();
        self.provider_list_scroll = ScrollbarState::default();

        if self.providers_cache.is_empty() {
            self.mode = Mode::Message(
                "No providers found. Add one in Provider Manager first.".to_string(),
                true,
            );
            return Ok(());
        }

        self.provider_list_state.select(Some(0));
        self.mode = Mode::LiteProviderSelect;
        Ok(())
    }

    fn open_lite_model_builder(&mut self) {
        self.mode = Mode::LiteFetching;
        self.set_lite_models_from_result(fetch_models(&self.lite_url, &self.lite_token));
        self.lite_step = 0;
        self.lite_model_page = 0;
        self.mode = Mode::LiteModelSelect {
            profile_name: String::new(),
            token: self.lite_token.clone(),
            base_url: self.lite_url.clone(),
            models: self.lite_models.clone(),
        };
    }

    // ── Run ───────────────────────────────────────────────────────────────────

    pub fn run(mut self) -> Result<()> {
        let mut terminal = ratatui::init();
        terminal.clear()?;
        let result = self.event_loop(&mut terminal);
        ratatui::restore();
        result
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

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
                        Mode::LiteProviderSelect => {
                            self.handle_lite_provider_select(key.code, key.modifiers)?;
                        }
                        Mode::LiteKeySelect { .. } => {
                            self.handle_lite_key_select(key.code, key.modifiers)?;
                        }
                        Mode::LiteFetching => {
                            if Self::is_cancel_key(key.code, key.modifiers) {
                                self.mode = Mode::Normal;
                            }
                        }
                        Mode::LiteModelSelect { .. } => {
                            self.handle_lite_model_select(key.code, key.modifiers)?;
                        }
                        Mode::LiteEdit { .. } => {
                            self.handle_lite_model_select(key.code, key.modifiers)?;
                        }
                        Mode::ProviderAnthropicTest { .. } => {
                            self.handle_provider_anthropic_test(key.code, key.modifiers)?;
                        }
                        Mode::ProviderAnthropicOutcome { .. } => {
                            self.handle_provider_anthropic_outcome(key.code, key.modifiers)?;
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
                        Mode::ConfirmDeleteProvider { .. } => {
                            self.handle_confirm_delete_provider(key.code)?;
                        }
                        Mode::ConfirmDeleteKey { .. } => {
                            self.handle_confirm_delete_key(key.code)?;
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

    // ── Key handlers ──────────────────────────────────────────────────────────

    fn handle_paste(&mut self, text: &str) -> Result<()> {
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
            _ => {}
        }
        Ok(())
    }

    fn handle_first_run_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<bool> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            _ => self.mode = Mode::Normal,
        }
        Ok(false)
    }

    fn handle_normal_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<bool> {
        // Global keys (work on both pages)
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            _ => {}
        }

        match self.page {
            Page::ProfileManager => self.handle_profile_page_key(code, modifiers),
            Page::ProviderManager => self.handle_provider_page_normal_key(code, modifiers),
        }
    }

    fn handle_profile_page_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<bool> {
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
                // Shift+Enter: launch WITHOUT stored launch_args
                if let Some(p) = self.selected_profile() {
                    ratatui::restore();
                    println!(
                        "Launching Claude with profile '{}' (without extra args)...",
                        p.name
                    );
                    self.manager.launch_claude(&p.id, &[], false)?;
                }
            }

            KeyCode::Enter => {
                if let Some(p) = self.selected_profile() {
                    let name = p.name.clone();
                    ratatui::restore();
                    println!(
                        "Launching Claude with profile '{}' (with extra args)…",
                        name
                    );
                    self.manager.launch_claude(&p.id, &[], true)?;
                }
            }

            KeyCode::Char('t') => {
                self.start_lite_profile_creation()?;
            }

            KeyCode::Char('a') => {
                self.mode = Mode::AddFullName;
                self.input_buffer.clear();
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
                    // Lightweight: full edit with models + extras
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
                        self.set_lite_models_from_result(fetch_models(&base_url, &token));
                        self.mode = Mode::LiteEdit {
                            profile_id: profile.id.clone(),
                        };
                    } else {
                        self.mode = Mode::Message("No env config found.".into(), true);
                    }
                } else {
                    // Full profile: edit name/alias + launch args
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

    fn handle_provider_page_normal_key(
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

    fn handle_search_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<bool> {
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
            _ if Self::is_prev_list_key(code, modifiers) => self.move_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_down(),
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

    fn handle_confirm_delete(&mut self, code: KeyCode) -> Result<()> {
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

    fn handle_add_full_name(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    fn handle_add_full_alias(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    // ── Shim sync ─────────────────────────────────────────────────────────────

    fn sync_shims(&self) {
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

    // ── Edit Profile (name / alias / launch args) ──────────────────────────────

    fn handle_edit_profile(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        // 3 steps: 0=name, 1=alias, 2=launch_args
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
                    // Save on Enter at last step
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
                // Backward cycle
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
            // Typing
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

    // ── Lightweight profile key handlers ─────────────────────────────────────

    fn handle_lite_provider_select(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }

        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,
            _ if Self::is_prev_list_key(code, modifiers) => self.move_provider_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_provider_down(),
            KeyCode::Enter => {
                let provider = self
                    .provider_list_state
                    .selected()
                    .and_then(|i| self.providers_cache.get(i))
                    .cloned();
                let Some(provider) = provider else {
                    return Ok(());
                };

                self.provider_keys_cache = self.manager.list_keys(&provider.id)?;
                if self.provider_keys_cache.is_empty() {
                    self.mode = Mode::Message(
                        format!(
                            "Provider '{}' has no keys. Add a key in Provider Manager first.",
                            provider.name
                        ),
                        true,
                    );
                    return Ok(());
                }

                self.lite_provider_id = Some(provider.id.clone());
                self.lite_url = provider.base_url;
                self.lite_provider_keys = self.provider_keys_cache.clone();
                self.provider_key_selected = 0;
                if let Some(key) = self.provider_keys_cache.first() {
                    self.lite_key_id = Some(key.id.clone());
                    self.lite_token = key.api_key.clone();
                }
                self.mode = Mode::LiteKeySelect {
                    provider_id: provider.id,
                };
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_lite_key_select(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }

        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::LiteProviderSelect,
            _ if Self::is_prev_list_key(code, modifiers) => self.move_provider_key_up(),
            _ if Self::is_next_list_key(code, modifiers) => self.move_provider_key_down(),
            KeyCode::Enter => {
                let Some(key) = self.selected_provider_key().cloned() else {
                    return Ok(());
                };
                self.lite_key_id = Some(key.id.clone());
                self.lite_token = key.api_key;
                self.lite_provider_keys = self.provider_keys_cache.clone();
                self.open_lite_model_builder();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_lite_model_select(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        let models_per_page: usize = 8;
        let is_edit = matches!(self.mode, Mode::LiteEdit { .. });
        // 11 steps: 0=name, 1=alias, 2-6=models, 7=extras, 8=launch_args, 9=provider, 10=key
        let total_steps: usize = 11;

        match code {
            _ if Self::is_cancel_key(code, modifiers) => self.mode = Mode::Normal,

            // Slot navigation
            _ if Self::is_next_field_key(code, modifiers) => {
                self.lite_step = (self.lite_step + 1) % total_steps;
                self.cursor_pos = self.lite_cursor_pos_for_step(self.lite_step);
            }
            _ if Self::is_prev_field_key(code, modifiers) => {
                self.lite_step = if self.lite_step == 0 {
                    total_steps - 1
                } else {
                    self.lite_step - 1
                };
                self.cursor_pos = self.lite_cursor_pos_for_step(self.lite_step);
            }
            KeyCode::Tab => {
                // Tab = completion on model slots, extras, launch_args, and provider/key.
                // On name/alias steps it advances to the next field.
                if self.lite_step >= 2 && self.lite_step <= 6 && !self.lite_models.is_empty() {
                    // Model slots: cycle through fetched model IDs
                    let current = self.current_slot_value();
                    if let Some(pos) = self.lite_models.iter().position(|m| m == &current) {
                        let next = (pos + 1) % self.lite_models.len();
                        self.set_slot_value(self.lite_models[next].clone());
                    } else if !current.is_empty()
                        && let Some(m) = self.lite_models.iter().find(|m| m.contains(&current))
                    {
                        self.set_slot_value(m.clone());
                    }
                } else if self.lite_step == 7 {
                    // Extras: cycle through known Claude Code env var names
                    let prefix = self.input_buffer.split('=').next().unwrap_or("");
                    let vars = all_var_names();
                    if let Some(pos) = vars.iter().position(|v| v == &prefix) {
                        let next = (pos + 1) % vars.len();
                        self.input_buffer = format!("{}=", vars[next]);
                        self.cursor_pos = self.input_buffer.len();
                    } else if !prefix.is_empty()
                        && let Some(v) = vars.iter().find(|v| v.starts_with(prefix))
                    {
                        self.input_buffer = format!("{}=", v);
                        self.cursor_pos = self.input_buffer.len();
                    }
                } else if self.lite_step == 8 {
                    // Launch args: cycle through known CLI flags (replace the last word)
                    let flags = all_flag_names();
                    if !flags.is_empty() {
                        let last_word = self
                            .lite_launch_args
                            .split_whitespace()
                            .last()
                            .unwrap_or("");
                        if let Some(pos) = flags.iter().position(|f| f == &last_word) {
                            let next = (pos + 1) % flags.len();
                            self.lite_launch_args =
                                replace_last_word(&self.lite_launch_args, flags[next]);
                            self.cursor_pos = self.lite_launch_args.len();
                        } else if !last_word.is_empty()
                            && let Some(f) = flags.iter().find(|f| f.starts_with(last_word))
                        {
                            self.lite_launch_args = replace_last_word(&self.lite_launch_args, f);
                            self.cursor_pos = self.lite_launch_args.len();
                        }
                    }
                } else if self.lite_step == 9 {
                    // Provider: cycle through available providers
                    if !self.providers_cache.is_empty() {
                        let current = self.lite_provider_id.clone().unwrap_or_default();
                        let pos = self
                            .providers_cache
                            .iter()
                            .position(|p| p.id == current)
                            .map(|p| (p + 1) % self.providers_cache.len())
                            .unwrap_or(0);
                        let prov = &self.providers_cache[pos];
                        self.lite_provider_id = Some(prov.id.clone());
                        self.lite_provider_keys = {
                            let mut ks: Vec<_> = prov.keys.values().cloned().collect();
                            ks.sort_by(|a, b| a.name.cmp(&b.name));
                            ks
                        };
                        self.lite_key_id = self.lite_provider_keys.first().map(|k| k.id.clone());
                        self.lite_token = self
                            .lite_provider_keys
                            .first()
                            .map(|k| k.api_key.clone())
                            .unwrap_or_default();
                        self.lite_url = prov.base_url.clone();
                    }
                } else if self.lite_step == 10 {
                    // Key: cycle through provider keys
                    if !self.lite_provider_keys.is_empty() {
                        let current = self.lite_key_id.as_deref().unwrap_or("");
                        let pos = self
                            .lite_provider_keys
                            .iter()
                            .position(|k| k.id == current)
                            .map(|p| (p + 1) % self.lite_provider_keys.len())
                            .unwrap_or(0);
                        self.lite_key_id = Some(self.lite_provider_keys[pos].id.clone());
                        self.lite_token = self.lite_provider_keys[pos].api_key.clone();
                    }
                } else {
                    self.lite_step = (self.lite_step + 1) % total_steps;
                    self.cursor_pos = self.lite_cursor_pos_for_step(self.lite_step);
                }
            }

            // [1m] toggle (steps 2-6 are model slots)
            KeyCode::Char('m')
                if modifiers.contains(KeyModifiers::CONTROL)
                    && self.lite_step >= 2
                    && self.lite_step <= 6 =>
            {
                let idx = self.lite_step - 2;
                self.lite_1m[idx] = !self.lite_1m[idx];
                let normalized =
                    apply_model_1m_flag(&self.current_slot_value(), self.lite_1m[idx]).to_string();
                self.set_slot_value(normalized);
            }

            // Alt+p/n: cycle through model candidates (steps 2-6)
            KeyCode::Char('p')
                if modifiers.contains(KeyModifiers::ALT)
                    && self.lite_step >= 2
                    && self.lite_step <= 6
                    && !self.lite_models.is_empty() =>
            {
                let old = self.lite_step;
                let current = self.current_slot_value();
                if let Some(pos) = self.lite_models.iter().position(|m| m == &current) {
                    let prev = if pos == 0 {
                        self.lite_models.len() - 1
                    } else {
                        pos - 1
                    };
                    self.lite_step = old;
                    self.set_slot_value(self.lite_models[prev].clone());
                } else if !current.is_empty()
                    && let Some(m) = self.lite_models.iter().find(|m| m.contains(&current))
                {
                    self.set_slot_value(m.clone());
                }
            }
            KeyCode::Char('n')
                if modifiers.contains(KeyModifiers::ALT)
                    && self.lite_step >= 2
                    && self.lite_step <= 6
                    && !self.lite_models.is_empty() =>
            {
                let old = self.lite_step;
                let current = self.current_slot_value();
                if let Some(pos) = self.lite_models.iter().position(|m| m == &current) {
                    let next = (pos + 1) % self.lite_models.len();
                    self.lite_step = old;
                    self.set_slot_value(self.lite_models[next].clone());
                } else if !current.is_empty()
                    && let Some(m) = self.lite_models.iter().find(|m| m.contains(&current))
                {
                    self.set_slot_value(m.clone());
                }
            }

            // Model list paging
            KeyCode::PageDown => {
                let total = self.lite_models.len();
                if self.lite_model_page + models_per_page < total {
                    self.lite_model_page += models_per_page;
                }
            }
            KeyCode::PageUp => {
                self.lite_model_page = self.lite_model_page.saturating_sub(models_per_page);
            }

            // Enter: add extras (step 7), otherwise save
            KeyCode::Enter => {
                if self.lite_step == 7 {
                    // Add extras
                    let val = self.input_buffer.trim().to_string();
                    if !val.is_empty() && val.contains('=') {
                        self.lite_extras.push(val);
                    }
                    self.input_buffer.clear();
                    return Ok(());
                }

                let name = self.lite_name.trim().to_string();
                if name.is_empty() {
                    self.mode = Mode::Message("Enter a profile name first".to_string(), false);
                    return Ok(());
                }
                let alias = self.lite_alias.trim().to_string();
                let alias_opt = if alias.is_empty() {
                    None
                } else {
                    Some(alias.as_str())
                };

                let apply = |s: &str, idx: usize| -> Option<String> {
                    if s.is_empty() {
                        None
                    } else {
                        Some(apply_model_1m_flag(s, self.lite_1m[idx]).to_string())
                    }
                };
                let env = LightweightEnv {
                    auth_token: Some(self.lite_token.clone()),
                    base_url: Some(self.lite_url.clone()),
                    default_opus_model: apply(&self.lite_mod_opus, 0),
                    default_sonnet_model: apply(&self.lite_mod_sonnet, 1),
                    default_haiku_model: apply(&self.lite_mod_haiku, 2),
                    model: apply(&self.lite_mod_model, 3),
                    subagent_model: apply(&self.lite_mod_subagent, 4),
                    extras: self.lite_extras.clone(),
                };

                if is_edit {
                    let id = self.lite_edit_id.clone();
                    match self.manager.update_lightweight(&id, &name, alias_opt, env) {
                        Ok(p) => {
                            let _ = self.manager.set_launch_args(
                                &p.id,
                                launch_args_from_str(&self.lite_launch_args),
                            );
                            if let Some(ref pid) = self.lite_provider_id {
                                if let Some(ref kid) = self.lite_key_id {
                                    if let Err(e) = self.manager.set_provider(&p.id, pid, kid) {
                                        self.mode = Mode::Message(e.to_string(), true);
                                        return Ok(());
                                    }
                                } else {
                                    self.mode =
                                        Mode::Message("Select a provider key first.".into(), true);
                                    return Ok(());
                                }
                            } else {
                                if let Err(e) = self.manager.unset_provider(&p.id) {
                                    self.mode = Mode::Message(e.to_string(), true);
                                    return Ok(());
                                }
                            }
                            self.sync_shims();
                            self.refresh()?;
                            self.select_by_id(&p.id);
                            self.mode =
                                Mode::Message(format!("Profile '{}' updated.", p.name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                } else {
                    match self
                        .manager
                        .create_lightweight_profile(&name, alias_opt, env)
                    {
                        Ok(p) => {
                            let _ = self.manager.set_launch_args(
                                &p.id,
                                launch_args_from_str(&self.lite_launch_args),
                            );
                            if let Some(ref pid) = self.lite_provider_id {
                                if let Some(ref kid) = self.lite_key_id {
                                    if let Err(e) = self.manager.set_provider(&p.id, pid, kid) {
                                        self.mode = Mode::Message(e.to_string(), true);
                                        return Ok(());
                                    }
                                } else {
                                    self.mode =
                                        Mode::Message("Select a provider key first.".into(), true);
                                    return Ok(());
                                }
                            } else {
                                if let Err(e) = self.manager.unset_provider(&p.id) {
                                    self.mode = Mode::Message(e.to_string(), true);
                                    return Ok(());
                                }
                            }
                            self.sync_shims();
                            self.refresh()?;
                            self.select_by_id(&p.id);
                            self.mode =
                                Mode::Message(format!("Profile '{}' created.", p.name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }

            // Backspace
            KeyCode::Backspace => match self.lite_step {
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
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_alias,
                        &mut self.cursor_pos,
                        false,
                    );
                }
                2 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_opus,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                3 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_sonnet,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                4 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_haiku,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                5 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_model,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                6 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_subagent,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                7 => {
                    if !self.input_buffer.is_empty() {
                        emacs_edit(
                            code,
                            modifiers,
                            &mut self.input_buffer,
                            &mut self.cursor_pos,
                            true,
                        );
                    } else if !self.lite_extras.is_empty() {
                        self.lite_extras.pop();
                    }
                }
                8 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_launch_args,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                9 => {
                    self.lite_provider_id = None;
                    self.lite_key_id = None;
                    self.lite_provider_keys.clear();
                }
                _ => {}
            },

            // Typing
            _ if self.lite_step <= 8 => match self.lite_step {
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
                        &mut self.lite_mod_opus,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                3 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_sonnet,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                4 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_haiku,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                5 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_model,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                6 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_mod_subagent,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                7 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.input_buffer,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                8 => {
                    emacs_edit(
                        code,
                        modifiers,
                        &mut self.lite_launch_args,
                        &mut self.cursor_pos,
                        true,
                    );
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    // ══════════════════════════════════════════════════════════════════════════
    // Rendering
    // ══════════════════════════════════════════════════════════════════════════

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::default().style(Style::default().bg(BG)), area);

        if self.mode == Mode::FirstRun {
            self.render_first_run(f, area);
            return;
        }

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        self.render_header(f, layout[0]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(layout[1]);

        match self.page {
            Page::ProfileManager => {
                self.render_profile_list(f, cols[0]);
                self.render_detail_panel(f, cols[1]);
            }
            Page::ProviderManager => {
                self.render_provider_list_page(f, cols[0]);
                self.render_provider_detail_page(f, cols[1]);
            }
        }
        self.render_footer(f, layout[2]);

        // Overlays
        match &self.mode.clone() {
            Mode::Help => self.render_help(f),
            Mode::ConfirmDelete => self.render_confirm_delete_popup(f),
            Mode::AddFullName => self.render_add_name_popup(f),
            Mode::AddFullAlias => self.render_add_alias_popup(f),
            Mode::EditProfile { step, .. } => self.render_edit_profile_popup(f, *step),
            Mode::LiteProviderSelect => self.render_lite_provider_select_popup(f),
            Mode::LiteKeySelect { .. } => self.render_lite_key_select_popup(f),
            Mode::LiteFetching => self.render_lite_fetching_popup(f),
            Mode::ProviderAnthropicTest { .. } => self.render_provider_anthropic_test_popup(f),
            Mode::ProviderAnthropicOutcome { .. } => {
                self.render_provider_anthropic_outcome_popup(f)
            }
            Mode::LiteModelSelect { .. } | Mode::LiteEdit { .. } => {
                self.render_lite_model_select_popup(f)
            }
            Mode::Message(msg, is_err) => self.render_message(f, msg, *is_err),
            Mode::ProviderList => self.render_provider_list_popup(f),
            Mode::ProviderAdd { step } => self.render_provider_add_popup(f, *step),
            Mode::ProviderSmartPaste => self.render_provider_smart_paste_popup(f),
            Mode::ProviderEdit { .. } => self.render_provider_edit_popup(f),
            Mode::ProviderEditKeyInput { step, .. } => {
                self.render_provider_edit_key_input_popup(f, *step)
            }
            Mode::ProviderKeyList { .. } => self.render_provider_key_list_popup(f),
            Mode::ProviderTestKeyList { .. } => self.render_provider_test_key_list_popup(f),
            Mode::ProviderKeyAdd { step, .. } => self.render_provider_key_add_popup(f, *step),
            Mode::ProviderKeyEdit { step, .. } => self.render_provider_key_edit_popup(f, *step),
            Mode::ConfirmDeleteProvider { .. } => self.render_confirm_delete_provider_popup(f),
            Mode::ConfirmDeleteKey { .. } => self.render_confirm_delete_key_popup(f),
            _ => {}
        }
    }

    // ── First-run screen ──────────────────────────────────────────────────────

    fn render_first_run(&self, f: &mut Frame, area: Rect) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        let header_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ◆ ", Style::default().fg(ACCENT).bold()),
                Span::styled("claude-switch", Style::default().fg(TEXT).bold()),
                Span::styled("  first run setup", Style::default().fg(DIM)),
            ]))
            .block(header_block),
            layout[0],
        );

        let body_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let inner = body_block.inner(layout[1]);
        f.render_widget(body_block, layout[1]);

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Welcome! Press 't' for lightweight (provider/key) or 'a' for full (directory) profile.",
                Style::default().fg(DIM),
            )))
            .wrap(Wrap { trim: false }),
            inner,
        );

        let footer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" esc ", Style::default().fg(ACCENT).bold()),
                Span::styled("continue  ", Style::default().fg(DIM)),
                Span::styled(" q ", Style::default().fg(ACCENT).bold()),
                Span::styled("quit", Style::default().fg(DIM)),
            ]))
            .block(footer_block),
            layout[2],
        );
    }

    // ── Normal view widgets ───────────────────────────────────────────────────

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ◆ ", Style::default().fg(ACCENT).bold()),
                Span::styled("claude-switch", Style::default().fg(TEXT).bold()),
                Span::styled(
                    match self.page {
                        Page::ProfileManager => "  profile manager",
                        Page::ProviderManager => "  provider manager",
                    },
                    Style::default().fg(DIM),
                ),
            ]))
            .block(block),
            area,
        );

        let (count, total) = match self.page {
            Page::ProfileManager => (self.filtered_indices.len(), self.profiles.len()),
            Page::ProviderManager => (self.providers_cache.len(), self.providers_cache.len()),
        };
        let item_name = match self.page {
            Page::ProfileManager => "profile",
            Page::ProviderManager => "provider",
        };
        let label = if count == total {
            format!(
                " {} {}{} ",
                total,
                item_name,
                if total == 1 { "" } else { "s" }
            )
        } else {
            format!(" {}/{} ", count, total)
        };

        let count_area = Rect {
            x: area.x + area.width.saturating_sub(label.len() as u16 + 2),
            y: area.y + 1,
            width: label.len() as u16 + 1,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(label, Style::default().fg(DIM)))
                .alignment(Alignment::Right),
            count_area,
        );
    }

    fn render_profile_list(&mut self, f: &mut Frame, area: Rect) {
        let title_line: Line = if self.mode == Mode::Search {
            Line::from(vec![
                Span::styled(" /", Style::default().fg(SEARCH_HL).bold()),
                Span::styled(
                    self.search_query.clone(),
                    Style::default().fg(SEARCH_HL).bold(),
                ),
                Span::styled("█ ", Style::default().fg(SEARCH_HL)),
            ])
        } else if !self.search_query.is_empty() {
            Line::from(vec![
                Span::styled(" Search: ", Style::default().fg(DIM)),
                Span::styled(self.search_query.clone(), Style::default().fg(SEARCH_HL)),
            ])
        } else {
            Line::from(Span::styled(
                " Profiles ",
                Style::default().fg(ACCENT).bold(),
            ))
        };

        let block = Block::default()
            .title(title_line)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(if self.mode == Mode::Search {
                Style::default().fg(SEARCH_HL)
            } else {
                Style::default().fg(BORDER)
            })
            .style(Style::default().bg(PANEL));

        let items: Vec<ListItem> = self
            .filtered_indices
            .iter()
            .map(|&i| {
                let p = &self.profiles[i];
                let kind = match p.kind {
                    ProfileKind::Lightweight => "lite",
                    ProfileKind::Full => "full",
                };
                let alias = p.alias.as_deref().unwrap_or("");
                let alias_str = if alias.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", alias)
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(" ", Style::default()),
                        Span::styled(p.name.clone(), Style::default().fg(TEXT).bold()),
                        Span::styled(alias_str, Style::default().fg(MUTED)),
                    ]),
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(kind, Style::default().fg(DIM)),
                    ]),
                ])
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(35, 35, 45))
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, area, &mut self.list_state);

        // Scrollbar
        let count = self.filtered_indices.len();
        let selected = self.list_state.selected().unwrap_or(0);
        if count > 1 {
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ACCENT))
                .track_style(Style::default().fg(BORDER));
            let mut scrollbar_state = ScrollbarState::new(count).position(selected);
            f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
        }
    }

    fn render_detail_panel(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Details ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(profile) = self.selected_profile() else {
            let hint = if self.search_query.is_empty() {
                "  No profiles yet. Press 'a' to add (full), 't' for lightweight."
            } else {
                "  No profiles match your search."
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(DIM)))),
                inner,
            );
            return;
        };

        let mut lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Name         ", Style::default().fg(DIM)),
                Span::styled(profile.name.clone(), Style::default().fg(ACCENT).bold()),
            ]),
        ];

        if let Some(ref a) = profile.alias {
            lines.push(Line::from(vec![
                Span::styled("  Alias        ", Style::default().fg(DIM)),
                Span::styled(a.clone(), Style::default().fg(Color::Rgb(140, 200, 140))),
            ]));
        }

        lines.push(Line::from(vec![
            Span::styled("  ID           ", Style::default().fg(DIM)),
            Span::styled(&profile.id[..8], Style::default().fg(MUTED)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Kind         ", Style::default().fg(DIM)),
            Span::styled(
                match profile.kind {
                    ProfileKind::Lightweight => "lightweight (env vars)",
                    ProfileKind::Full => "full (directory)",
                },
                Style::default().fg(TEXT),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Added        ", Style::default().fg(DIM)),
            Span::styled(
                profile.added.format("%Y-%m-%d %H:%M UTC").to_string(),
                Style::default().fg(TEXT),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Last used    ", Style::default().fg(DIM)),
            Span::styled(
                profile
                    .last_used
                    .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or("never".into()),
                Style::default().fg(TEXT),
            ),
        ]));

        if profile.kind == ProfileKind::Full {
            let profile_dir = self.manager.profile_dir(profile);
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Config dir   ", Style::default().fg(DIM)),
                Span::styled(
                    profile_dir.display().to_string(),
                    Style::default().fg(MUTED),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ─────────────────────────────────────────",
                Style::default().fg(BORDER),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Launch command",
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(Span::styled(
                if cfg!(target_os = "windows") {
                    format!(
                        "  $env:CLAUDE_CONFIG_DIR='{}'; claude",
                        profile_dir.display()
                    )
                } else {
                    format!("  CLAUDE_CONFIG_DIR='{}' claude", profile_dir.display())
                },
                Style::default().fg(Color::Rgb(140, 200, 140)),
            )));
        } else if let Some(ref env) = profile.env {
            // Show provider info if referenced
            if let Some(ref pid) = profile.provider_id
                && let Ok(provider) = self.manager.get_provider(pid)
            {
                let prov_label = format!("{} ({})", provider.name, provider.id);
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  ─── Provider ─────────────────────────────",
                    Style::default().fg(BORDER),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  Provider     ", Style::default().fg(DIM)),
                    Span::styled(prov_label, Style::default().fg(ACCENT)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Base URL     ", Style::default().fg(DIM)),
                    Span::styled(
                        provider.base_url,
                        Style::default().fg(Color::Rgb(140, 200, 140)),
                    ),
                ]));
                // Show key info
                if let Some(ref kid) = profile.key_id
                    && let Some(k) = provider.keys.get(kid)
                {
                    lines.push(Line::from(vec![
                        Span::styled("  Key          ", Style::default().fg(DIM)),
                        Span::styled(
                            format!("{} ({})", k.name, k.id),
                            Style::default().fg(ACCENT),
                        ),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ─── Env Vars ───────────────────────────",
                Style::default().fg(BORDER),
            )));
            lines.push(Line::from(""));
            if let Some(ref u) = env.base_url {
                lines.push(Line::from(vec![
                    Span::styled("  Base URL     ", Style::default().fg(DIM)),
                    Span::styled(u, Style::default().fg(Color::Rgb(140, 200, 140))),
                ]));
            }
            if let Some(ref m) = env.default_opus_model {
                lines.push(Line::from(vec![
                    Span::styled("  Opus model   ", Style::default().fg(DIM)),
                    Span::styled(m, Style::default().fg(TEXT)),
                ]));
            }
            if let Some(ref m) = env.default_sonnet_model {
                lines.push(Line::from(vec![
                    Span::styled("  Sonnet model ", Style::default().fg(DIM)),
                    Span::styled(m, Style::default().fg(TEXT)),
                ]));
            }
            if let Some(ref m) = env.default_haiku_model {
                lines.push(Line::from(vec![
                    Span::styled("  Haiku model  ", Style::default().fg(DIM)),
                    Span::styled(m, Style::default().fg(TEXT)),
                ]));
            }
            if let Some(ref m) = env.model {
                lines.push(Line::from(vec![
                    Span::styled("  Model        ", Style::default().fg(DIM)),
                    Span::styled(m, Style::default().fg(TEXT)),
                ]));
            }
            if let Some(ref m) = env.subagent_model {
                lines.push(Line::from(vec![
                    Span::styled("  Subagent     ", Style::default().fg(DIM)),
                    Span::styled(m, Style::default().fg(TEXT)),
                ]));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ─── Options ─────────────────────────────",
                Style::default().fg(BORDER),
            )));
            lines.push(Line::from(""));
            let any_mark = if self.lite_1m.iter().any(|&x| x) {
                "[x]"
            } else {
                "[ ]"
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", any_mark),
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::styled(
                    "Press 'm' to toggle [1m] on/off for all slots",
                    Style::default().fg(TEXT),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Launch",
                Style::default().fg(DIM),
            )));
        }

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    // ── Provider page widgets ──────────────────────────────────────────────────

    fn render_provider_list_page(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Providers ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        if self.providers_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No providers yet. Press 'a' to add.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .providers_cache
            .iter()
            .map(|p| {
                let url_short = display_ellipsize(&p.base_url, 35);
                let key_count = format!("keys:{}", p.keys.len());
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(" ", Style::default()),
                        Span::styled(display_pad(&p.name, 24), Style::default().fg(TEXT).bold()),
                        Span::styled(format!("  {:<8}", key_count), Style::default().fg(DIM)),
                    ]),
                    Line::from(vec![
                        Span::styled("  id: ", Style::default().fg(MUTED)),
                        Span::styled(
                            display_pad(&display_ellipsize(&p.id, 12), 12),
                            Style::default().fg(MUTED),
                        ),
                        Span::styled("  ", Style::default()),
                        Span::styled(url_short, Style::default().fg(DIM)),
                    ]),
                ])
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(35, 35, 45))
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, area, &mut self.provider_list_state);

        let count = self.providers_cache.len();
        if count > 1 {
            let sel = self.provider_list_state.selected().unwrap_or(0);
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ACCENT))
                .track_style(Style::default().fg(BORDER));
            let mut sb = ScrollbarState::new(count).position(sel);
            f.render_stateful_widget(scrollbar, area, &mut sb);
        }
    }

    fn render_provider_detail_page(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Provider Detail ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let idx = match self.provider_list_state.selected() {
            Some(i) if i < self.providers_cache.len() => i,
            _ => {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "  No provider selected.",
                        Style::default().fg(DIM),
                    ))),
                    inner,
                );
                return;
            }
        };

        let p = &self.providers_cache[idx];
        let mut lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Name         ", Style::default().fg(DIM)),
                Span::styled(p.name.clone(), Style::default().fg(ACCENT).bold()),
            ]),
            Line::from(vec![
                Span::styled("  ID           ", Style::default().fg(DIM)),
                Span::styled(&p.id[..p.id.len().min(12)], Style::default().fg(MUTED)),
            ]),
            Line::from(vec![
                Span::styled("  Base URL     ", Style::default().fg(DIM)),
                Span::styled(p.base_url.clone(), Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Keys         ", Style::default().fg(DIM)),
                Span::styled(format!("{}", p.keys.len()), Style::default().fg(TEXT)),
            ]),
            Line::from(""),
        ];

        // List keys
        let mut keys: Vec<&crate::profile::ProviderKey> = p.keys.values().collect();
        keys.sort_by(|a, b| a.name.cmp(&b.name));
        for k in keys.iter().take(10) {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(display_pad(&k.name, 20), Style::default().fg(TEXT)),
                Span::styled("  ", Style::default()),
                Span::styled(mask_api_key(&k.api_key), Style::default().fg(MUTED)),
            ]));
        }
        if keys.len() > 10 {
            lines.push(Line::from(Span::styled(
                format!("    ... and {} more", keys.len() - 10),
                Style::default().fg(DIM),
            )));
        }

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let keys: Vec<(&str, &str)> = if self.mode == Mode::Search {
            vec![
                ("Ctrl+P/N", "navigate"),
                ("enter", "confirm"),
                ("esc/Ctrl+G", "clear"),
            ]
        } else if matches!(self.mode, Mode::ProviderAnthropicOutcome { .. }) {
            vec![("any key", "back"), ("q", "quit")]
        } else if matches!(self.mode, Mode::ProviderAnthropicTest { .. }) {
            vec![
                ("Ctrl+N/P", "field"),
                ("Tab", "complete"),
                ("PgUp/PgDn", "page"),
                ("enter", "send"),
                ("esc/Ctrl+G", "back"),
                ("q", "quit"),
            ]
        } else if let Mode::ProviderKeyList { .. } = &self.mode {
            vec![
                ("Ctrl+P/N", "nav"),
                ("a", "add key"),
                ("e", "edit key"),
                ("d", "delete key"),
                ("t", "test"),
                ("esc/Ctrl+G", "back"),
            ]
        } else {
            match self.page {
                Page::ProfileManager => vec![
                    ("Ctrl+P/N", "nav"),
                    ("enter", "launch"),
                    ("Shift+Enter", "w/o args"),
                    ("/", "search"),
                    ("t", "lite"),
                    ("a", "add"),
                    ("e", "edit"),
                    ("m", "[1m]"),
                    ("r", "refresh"),
                    ("d", "delete"),
                    ("?", "help"),
                    ("Shift+Tab", "providers"),
                    ("q", "quit"),
                ],
                Page::ProviderManager => vec![
                    ("Ctrl+P/N", "nav"),
                    ("enter", "keys"),
                    ("a", "add"),
                    ("t", "test"),
                    ("Ctrl+Y", "smart input"),
                    ("e", "edit"),
                    ("d", "delete"),
                    ("?", "help"),
                    ("Shift+Tab", "profiles"),
                    ("q", "quit"),
                ],
            }
        };

        let spans: Vec<Span> = keys
            .iter()
            .flat_map(|(k, v)| {
                vec![
                    Span::styled(format!(" {} ", k), Style::default().fg(ACCENT).bold()),
                    Span::styled(*v, Style::default().fg(DIM)),
                    Span::styled(" ", Style::default()),
                ]
            })
            .collect();

        f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    }

    // ── Overlay popups ────────────────────────────────────────────────────────

    fn render_help(&self, f: &mut Frame) {
        let area = centered_rect(65, 25, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Help — Keybindings ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let help_entries: Vec<(&str, &str)> = vec![
            ("Ctrl+P/N", "Navigate lists and selections"),
            ("↑/↓", "Compatibility navigation keys"),
            ("Enter", "Launch with stored flags (default)"),
            ("Shift+Enter", "Launch without stored flags"),
            ("/", "Search profiles by name or alias"),
            ("t", "Add lightweight profile from provider/key"),
            ("Ctrl+Y", "Smart input provider from clipboard"),
            (
                "Provider: t",
                "Discover models from compatible endpoints; manual entry still works on failure",
            ),
            ("a", "Add full (directory-isolated) profile"),
            ("e", "Edit profile (name/alias, or models for lite)"),
            ("m", "Toggle [1m] suffix (lightweight profiles)"),
            ("Ctrl+A/E/B/F", "Move cursor in text fields"),
            ("Ctrl+H/D/K/U/W", "Edit text in Emacs style"),
            ("Ctrl+G / Esc", "Cancel or go back"),
            ("Shift+Tab", "Switch profile/provider manager"),
            ("r", "Refresh — re-copy ~/.claude into selected"),
            ("d / Del", "Delete selected profile"),
            ("?", "Toggle this help dialog"),
            ("q / Esc", "Quit"),
        ];

        let mut lines: Vec<Line> = vec![Line::from("")];

        for (key, desc) in &help_entries {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}", display_pad(key, 14)),
                    Style::default().fg(ACCENT).bold(),
                ),
                Span::styled(*desc, Style::default().fg(TEXT)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ───────────────────────────────────────",
            Style::default().fg(BORDER),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  CLI:  cswitch --help  for command-line usage",
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Press any key to close",
            Style::default().fg(DIM),
        )));

        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_confirm_delete_popup(&self, f: &mut Frame) {
        let name = self
            .selected_profile()
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        let area = centered_rect(50, 7, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Confirm Delete ",
                Style::default().fg(DANGER).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DANGER))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Delete profile ", Style::default().fg(TEXT)),
                    Span::styled(name.to_string(), Style::default().fg(DANGER).bold()),
                    Span::styled("? This cannot be undone.", Style::default().fg(TEXT)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("y", Style::default().fg(DANGER).bold()),
                    Span::styled(" confirm   ", Style::default().fg(DIM)),
                    Span::styled("any other key", Style::default().fg(ACCENT).bold()),
                    Span::styled(" cancel", Style::default().fg(DIM)),
                ]),
            ]))
            .block(block),
            area,
        );
    }

    fn render_add_name_popup(&self, f: &mut Frame) {
        let area = centered_rect(50, 7, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Add Full Profile — Name ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Name: ", Style::default().fg(DIM)),
                    Span::styled(
                        display_with_cursor(&self.input_buffer, self.cursor_pos),
                        Style::default().fg(TEXT).bold(),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  Any characters allowed. Enter to continue, Esc/Ctrl+G to cancel.",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    fn render_add_alias_popup(&self, f: &mut Frame) {
        let area = centered_rect(50, 8, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Add Full Profile — Alias ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Alias: ", Style::default().fg(DIM)),
                    Span::styled(
                        if self.input_buffer.is_empty() {
                            "█".to_string()
                        } else {
                            display_with_cursor(&self.input_buffer, self.cursor_pos)
                        },
                        Style::default().fg(TEXT).bold(),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  Short CLI-friendly name (a-z, 0-9, -, _). Enter to skip. Esc/Ctrl+G cancels.",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    fn render_edit_profile_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(70, 12, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Edit Profile ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        macro_rules! field {
            ($step:expr, $label:expr, $value:expr, $color:expr) => {{
                let prefix = if step == $step { "▶ " } else { "  " };
                let display = if step == $step {
                    display_with_cursor($value, self.cursor_pos)
                } else if $value.is_empty() {
                    "(none)".to_string()
                } else {
                    $value.to_string()
                };
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(ACCENT).bold()),
                    Span::styled($label, Style::default().fg(DIM)),
                    Span::styled(display, $color),
                ])
            }};
        }

        let name_disp = self.lite_name.to_string();
        let alias_disp = self.lite_alias.to_string();
        let args_disp = self.lite_launch_args.to_string();

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                field!(
                    0,
                    "Name:     ",
                    &name_disp,
                    Style::default().fg(TEXT).bold()
                ),
                Line::from(""),
                field!(
                    1,
                    "Alias:    ",
                    &alias_disp,
                    Style::default().fg(Color::Rgb(140, 200, 140))
                ),
                Line::from(""),
                field!(
                    2,
                    "Flags:    ",
                    &args_disp,
                    Style::default().fg(Color::Rgb(200, 160, 100))
                ),
                Line::from(""),
                Line::from(Span::styled(
                    "  Ctrl+P/N fields  Tab next  Enter save  Esc/Ctrl+G cancel",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    // ── Lightweight profile popups ────────────────────────────────────────────

    fn render_lite_provider_select_popup(&mut self, f: &mut Frame) {
        let area = centered_rect(78, 17, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Lightweight Profile — Provider ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.providers_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No providers yet. Add one in Provider Manager first.",
                    Style::default().fg(DIM),
                ))),
                inner,
            );
            return;
        }

        let list_height = inner.height.saturating_sub(2);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_height,
        };
        let hint_area = Rect {
            x: inner.x,
            y: inner.y + list_height,
            width: inner.width,
            height: inner.height.saturating_sub(list_height),
        };

        let items: Vec<ListItem> = self
            .providers_cache
            .iter()
            .map(|p| {
                let key_count = format!("keys:{}", p.keys.len());
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {}", display_pad(&p.name, 22)),
                        Style::default().fg(TEXT).bold(),
                    ),
                    Span::styled(
                        format!(" {} ", display_pad(&key_count, 8)),
                        Style::default().fg(DIM),
                    ),
                    Span::styled(
                        display_ellipsize(&p.base_url, 36),
                        Style::default().fg(MUTED),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(35, 35, 45))
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶");
        f.render_stateful_widget(list, list_area, &mut self.provider_list_state);

        let count = self.providers_cache.len();
        if count > 1 {
            let selected = self.provider_list_state.selected().unwrap_or(0);
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ACCENT))
                .track_style(Style::default().fg(BORDER));
            let mut sb = ScrollbarState::new(count).position(selected);
            f.render_stateful_widget(scrollbar, list_area, &mut sb);
        }

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Enter", Style::default().fg(ACCENT).bold()),
                Span::styled(" select provider  ", Style::default().fg(DIM)),
                Span::styled("Esc/Ctrl+G", Style::default().fg(ACCENT).bold()),
                Span::styled(" cancel", Style::default().fg(DIM)),
            ])),
            hint_area,
        );
    }

    fn render_lite_key_select_popup(&self, f: &mut Frame) {
        let key_count = self.provider_keys_cache.len().min(8);
        let height = 8 + key_count as u16;
        let area = centered_rect(70, height, f.area());
        f.render_widget(Clear, area);

        let provider_name = self
            .lite_provider_id
            .as_ref()
            .and_then(|pid| self.providers_cache.iter().find(|p| p.id == *pid))
            .map(|p| p.name.as_str())
            .unwrap_or("Provider");
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Lightweight Profile — Key: {} ", provider_name),
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let mut lines = vec![Line::from("")];
        if self.provider_keys_cache.is_empty() {
            lines.push(Line::from(Span::styled(
                "  This provider has no keys.",
                Style::default().fg(DIM),
            )));
        } else {
            let visible = 8usize;
            let selected = self
                .provider_key_selected
                .min(self.provider_keys_cache.len().saturating_sub(1));
            let start = selected.saturating_sub(visible.saturating_sub(1));
            for (i, key) in self
                .provider_keys_cache
                .iter()
                .enumerate()
                .skip(start)
                .take(visible)
            {
                let is_selected = i == selected;
                let style = if is_selected {
                    Style::default().fg(ACCENT).bold()
                } else {
                    Style::default().fg(TEXT)
                };
                let prefix = if is_selected { "▶" } else { " " };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", prefix), style),
                    Span::styled(display_pad(&key.name, 22), style),
                    Span::styled("  ", Style::default()),
                    Span::styled(mask_api_key(&key.api_key), Style::default().fg(DIM)),
                ]));
            }
            if self.provider_keys_cache.len() > visible {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  showing {}-{} of {}",
                        start + 1,
                        (start + visible).min(self.provider_keys_cache.len()),
                        self.provider_keys_cache.len()
                    ),
                    Style::default().fg(DIM),
                )));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Enter", Style::default().fg(ACCENT).bold()),
            Span::styled(" continue  ", Style::default().fg(DIM)),
            Span::styled("Esc/Ctrl+G", Style::default().fg(ACCENT).bold()),
            Span::styled(" back to providers", Style::default().fg(DIM)),
        ]));

        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_lite_fetching_popup(&self, f: &mut Frame) {
        let area = centered_rect(50, 6, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Fetching Models ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Connecting to /v1/models...",
                    Style::default().fg(TEXT),
                )),
                Line::from(Span::styled(
                    "  Press Esc/Ctrl+G to skip",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    fn render_lite_model_select_popup(&self, f: &mut Frame) {
        let area = centered_rect(90, 41, f.area());
        f.render_widget(Clear, area);
        let is_edit = matches!(self.mode, Mode::LiteEdit { .. });
        let title = if is_edit {
            " Edit Profile — Model Selection "
        } else {
            " Lite Profile — Model Selection "
        };
        let block = Block::default()
            .title(Line::from(Span::styled(
                title,
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let mut lines: Vec<Line> = vec![Line::from("")];

        // Available models
        let models_per_page: usize = 8;
        let total = self.lite_models.len();
        if !self.lite_models.is_empty() {
            let page_start = self.lite_model_page.min(total.saturating_sub(1));
            let page_end = (page_start + models_per_page).min(total);
            let current_page = page_start / models_per_page + 1;
            let total_pages = total.div_ceil(models_per_page);
            let page_info = if total > models_per_page {
                format!(
                    "  Models ({}-{} of {}, page {}/{}):",
                    page_start + 1,
                    page_end,
                    total,
                    current_page,
                    total_pages
                )
            } else {
                "  Available models:".to_string()
            };
            lines.push(Line::from(Span::styled(
                page_info,
                Style::default().fg(DIM),
            )));
            let page_models: Vec<&str> = self
                .lite_models
                .iter()
                .skip(page_start)
                .take(models_per_page)
                .map(|s| s.as_str())
                .collect();
            for (i, m) in page_models.iter().enumerate() {
                let idx = page_start + i + 1;
                lines.push(Line::from(Span::styled(
                    format!("{:>4}. {}", idx, m),
                    Style::default().fg(Color::Rgb(140, 200, 140)),
                )));
            }
            if total > models_per_page {
                lines.push(Line::from(Span::styled(
                    "     PgUp/PgDn scroll",
                    Style::default().fg(Color::Rgb(80, 120, 80)),
                )));
                // Visual page indicator bar
                let bar_width = 30usize;
                let filled = (current_page as f64 / total_pages as f64 * bar_width as f64)
                    .round()
                    .max(1.0)
                    .min(bar_width as f64) as usize;
                let bar = format!(
                    "     [{}{}]",
                    "█".repeat(filled),
                    "░".repeat(bar_width - filled)
                );
                lines.push(Line::from(Span::styled(bar, Style::default().fg(ACCENT))));
            }
        } else {
            let msg = match &self.lite_model_fetch_state {
                ModelFetchState::Loaded | ModelFetchState::Empty => {
                    "  No models (type manually or use Alt+p/Alt+n to cycle)".to_string()
                }
                ModelFetchState::Unavailable(reason) => format!("  {}", reason),
            };
            lines.push(Line::from(Span::styled(msg, Style::default().fg(DIM))));
        }
        lines.push(Line::from(Span::styled(
            "  ───────────────────────────────────────────────────────────────────",
            Style::default().fg(BORDER),
        )));

        // Step 0: Profile name (any characters)
        let nf = if self.lite_step == 0 { "▶ " } else { "  " };
        let nd = if self.lite_step == 0 {
            display_with_cursor(&self.lite_name, self.cursor_pos)
        } else {
            self.lite_name.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(nf, Style::default().fg(ACCENT).bold()),
            Span::styled("Name      ", Style::default().fg(DIM)),
            Span::styled(nd, Style::default().fg(Color::Rgb(200, 200, 120)).bold()),
        ]));

        // Step 1: Alias (alphanumeric only)
        let af = if self.lite_step == 1 { "▶ " } else { "  " };
        let ad = if self.lite_step == 1 {
            display_with_cursor(&self.lite_alias, self.cursor_pos)
        } else {
            self.lite_alias.clone()
        };
        let ad_display = if ad.is_empty() && self.lite_step != 1 {
            "(none)".to_string()
        } else {
            ad
        };
        lines.push(Line::from(vec![
            Span::styled(af, Style::default().fg(ACCENT).bold()),
            Span::styled("Alias     ", Style::default().fg(DIM)),
            Span::styled(ad_display, Style::default().fg(Color::Rgb(140, 200, 140))),
        ]));

        // Model slots (steps 2-6)
        let slots = [
            ("Opus", 0, 2),
            ("Sonnet", 1, 3),
            ("Haiku", 2, 4),
            ("Model", 3, 5),
            ("Subagent", 4, 6),
        ];
        for (label, idx1m, step) in slots.iter() {
            let prefix = if *step == self.lite_step {
                "▶ "
            } else {
                "  "
            };
            let val = match *step {
                2 => &self.lite_mod_opus,
                3 => &self.lite_mod_sonnet,
                4 => &self.lite_mod_haiku,
                5 => &self.lite_mod_model,
                6 => &self.lite_mod_subagent,
                _ => unreachable!(),
            };
            let display = if *step == self.lite_step {
                display_with_cursor(val, self.cursor_pos)
            } else {
                val.clone()
            };
            let ck = if self.lite_1m[*idx1m] { "1m✓" } else { "1m " };
            let hint = if !val.is_empty() && !self.lite_models.is_empty() {
                if let Some(m) = self.lite_models.iter().find(|m| m.contains(val.as_str())) {
                    if m != val {
                        format!(" ↩{}", m)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(ACCENT).bold()),
                Span::styled(display_pad(label, 10), Style::default().fg(DIM)),
                Span::styled(display_pad(&display, 36), Style::default().fg(TEXT).bold()),
                Span::styled(ck, Style::default().fg(ACCENT).bold()),
                Span::styled(hint, Style::default().fg(Color::Rgb(100, 130, 100))),
            ]));
        }

        // Extras section (step 7)
        lines.push(Line::from(Span::styled(
            "  ───────────────────────────────────────────────────────────────────",
            Style::default().fg(BORDER),
        )));
        let extras_focus = self.lite_step == 7;
        let ex_prefix = if extras_focus { "▶" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", ex_prefix),
                Style::default().fg(ACCENT).bold(),
            ),
            Span::styled("Extras", Style::default().fg(DIM)),
            Span::styled(
                " (enter KEY=VALUE per line)",
                Style::default().fg(Color::Rgb(120, 120, 130)),
            ),
        ]));
        // Show a curated subset of commonly used env vars as hints
        let hint_vars = [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_BETAS",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "API_TIMEOUT_MS",
            "MAX_THINKING_TOKENS",
            "CLAUDE_CONFIG_DIR",
        ];
        let total_known = crate::env_vars::all_var_names().len();
        lines.push(Line::from(Span::styled(
            format!(
                "  Known env vars ({} total; see https://code.claude.com/docs/en/env-vars):",
                total_known
            ),
            Style::default().fg(Color::Rgb(80, 100, 110)),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", hint_vars.join("  ")),
            Style::default().fg(Color::Rgb(70, 80, 90)),
        )));

        for extra in &self.lite_extras {
            lines.push(Line::from(Span::styled(
                format!("  {}", extra),
                Style::default().fg(Color::Rgb(160, 200, 160)),
            )));
        }
        if extras_focus {
            let buf = display_with_cursor(&self.input_buffer, self.cursor_pos);
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(buf, Style::default().fg(TEXT).bold()),
            ]));
            lines.push(Line::from(Span::styled(
                "  Enter to add, Backspace to remove last entry",
                Style::default().fg(DIM),
            )));
        }

        // Launch args (step 8)
        lines.push(Line::from(Span::styled(
            "  ───────────────────────────────────────────────────────────────────",
            Style::default().fg(BORDER),
        )));
        let la_focus = self.lite_step == 8;
        let la_prefix = if la_focus { "▶ " } else { "  " };
        let la_display = if la_focus {
            display_with_cursor(&self.lite_launch_args, self.cursor_pos)
        } else if self.lite_launch_args.is_empty() {
            "(none)".to_string()
        } else {
            self.lite_launch_args.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(la_prefix, Style::default().fg(ACCENT).bold()),
            Span::styled("L. args  ", Style::default().fg(DIM)),
            Span::styled(la_display, Style::default().fg(Color::Rgb(200, 160, 100))),
        ]));
        lines.push(Line::from(Span::styled(
            "  CLI flags to pass to claude on launch (space-separated, e.g. --dangerously-skip-permissions)",
            Style::default().fg(DIM),
        )));

        // Provider (step 9)
        lines.push(Line::from(Span::styled(
            "  ───────────────────────────────────────────────────────────────────",
            Style::default().fg(BORDER),
        )));
        let prov_focus = self.lite_step == 9;
        let prov_prefix = if prov_focus { "▶ " } else { "  " };
        let prov_name = self
            .lite_provider_id
            .as_ref()
            .and_then(|pid| self.providers_cache.iter().find(|p| p.id == *pid))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "(none — Tab to cycle)".to_string());
        lines.push(Line::from(vec![
            Span::styled(prov_prefix, Style::default().fg(ACCENT).bold()),
            Span::styled("Provider  ", Style::default().fg(DIM)),
            Span::styled(
                prov_name,
                Style::default().fg(Color::Rgb(200, 160, 100)).bold(),
            ),
        ]));
        if prov_focus {
            lines.push(Line::from(Span::styled(
                "  Tab=cycle provider  Backspace=clear  Ctrl+P/N=move fields",
                Style::default().fg(DIM),
            )));
        }

        // Key (step 10)
        let key_focus = self.lite_step == 10;
        let key_prefix = if key_focus { "▶ " } else { "  " };
        let key_name = self
            .lite_key_id
            .as_ref()
            .and_then(|kid| self.lite_provider_keys.iter().find(|k| k.id == *kid))
            .map(|k| k.name.clone())
            .unwrap_or_else(|| {
                if self.lite_provider_id.is_none() {
                    "(select provider first)".to_string()
                } else {
                    "(none — Tab to cycle)".to_string()
                }
            });
        let key_color = if self.lite_provider_id.is_some() {
            Color::Rgb(160, 180, 210)
        } else {
            DIM
        };
        lines.push(Line::from(vec![
            Span::styled(key_prefix, Style::default().fg(ACCENT).bold()),
            Span::styled("Key       ", Style::default().fg(DIM)),
            Span::styled(key_name, Style::default().fg(key_color)),
        ]));
        if key_focus && self.lite_provider_id.is_some() {
            lines.push(Line::from(Span::styled(
                "  Tab=cycle key  Ctrl+P/N=move fields",
                Style::default().fg(DIM),
            )));
        } else if key_focus {
            lines.push(Line::from(Span::styled(
                "  Select a provider first (step 9), then Tab here to cycle keys",
                Style::default().fg(DIM),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Ctrl+P/N", Style::default().fg(ACCENT).bold()),
            Span::styled(" fields  ", Style::default().fg(DIM)),
            Span::styled("Tab", Style::default().fg(ACCENT).bold()),
            Span::styled(" complete  ", Style::default().fg(DIM)),
            Span::styled("Cm", Style::default().fg(ACCENT).bold()),
            Span::styled(" 1m  ", Style::default().fg(DIM)),
            Span::styled("Enter", Style::default().fg(ACCENT).bold()),
            Span::styled(" save  ", Style::default().fg(DIM)),
            Span::styled("Esc/Ctrl+G", Style::default().fg(ACCENT).bold()),
            Span::styled(" cancel", Style::default().fg(DIM)),
        ]));

        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_message(&self, f: &mut Frame, msg: &str, is_err: bool) {
        let area = centered_rect(60, 6, f.area());
        f.render_widget(Clear, area);

        let color = if is_err { DANGER } else { SUCCESS };
        let title = if is_err { " Error " } else { " Done " };

        let block = Block::default()
            .title(Line::from(Span::styled(
                title,
                Style::default().fg(color).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {}", msg),
                    Style::default().fg(TEXT),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press any key to continue",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
    }

    // ── Provider management ──────────────────────────────────────────────────

    fn handle_provider_list(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    fn handle_provider_add(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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
                if self.page == Page::ProviderManager {
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
                            if self.page == Page::ProviderManager {
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

    fn handle_provider_smart_paste(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.reset_provider_smart_input();
            self.mode = if self.page == Page::ProviderManager {
                Mode::Normal
            } else {
                Mode::ProviderList
            };
            return Ok(());
        }

        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.reset_provider_smart_input();
                self.mode = if self.page == Page::ProviderManager {
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

    fn reset_provider_smart_input(&mut self) {
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

    fn start_provider_smart_input(&mut self) -> Result<()> {
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

    fn show_message(&mut self, msg: String, is_err: bool, return_mode: Option<Mode>) {
        self.message_return_mode = return_mode;
        self.mode = Mode::Message(msg, is_err);
    }

    fn start_selected_provider_test(&mut self) -> Result<()> {
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

    fn start_provider_key_test(&mut self) -> Result<()> {
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

    fn start_provider_test_popup(
        &mut self,
        provider: &Provider,
        key: &ProviderKey,
        source: ProviderTestSource,
    ) -> Result<()> {
        let fetched_models = discover_models(&provider.base_url, &key.api_key).map_err(|failure| {
            format!(
                "Provider '{}' key '{}' could not discover models: {}",
                provider.name, key.name, failure.message
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

    fn apply_provider_smart_paste(&mut self, parsed: SmartProviderPaste) -> Result<()> {
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

    fn handle_provider_anthropic_test(
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
            _ if field == 0 && Self::is_prev_selection_key(code, modifiers) => {
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
            _ if field == 0 && Self::is_next_selection_key(code, modifiers) => {
                if !self.provider_test_models.is_empty() {
                    self.provider_test_model_selected =
                        (self.provider_test_model_selected + 1) % self.provider_test_models.len();
                    self.provider_test_model_buf =
                        self.provider_test_models[self.provider_test_model_selected].clone();
                    self.cursor_pos = self.provider_test_model_buf.len();
                }
            }
            _ if Self::is_next_field_key(code, modifiers) => {
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
            _ if Self::is_prev_field_key(code, modifiers) => {
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

    fn handle_provider_anthropic_outcome(
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

    fn handle_provider_edit(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        let (pid, step) = match &self.mode {
            Mode::ProviderEdit { provider_id, step } => (provider_id.clone(), *step),
            _ => return Ok(()),
        };
        let total_steps: usize = 3; // 0=name, 1=url, 2=keys

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
                if self.page == Page::ProviderManager {
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
                    if self.page == Page::ProviderManager {
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
                // Add key from within edit
                self.provider_key_name_buf.clear();
                self.provider_key_buf.clear();
                self.cursor_pos = 0;
                self.mode = Mode::ProviderEditKeyInput {
                    provider_id: pid,
                    step: 0,
                };
            }
            KeyCode::Char('d') if step == 2 => {
                // Delete selected key
                if let Some(k) = self.selected_provider_key() {
                    let _ = self.manager.remove_key(&pid, &k.id);
                    self.sync_shims();
                    self.provider_keys_cache = self.manager.list_keys(&pid).unwrap_or_default();
                    if self.provider_key_selected >= self.provider_keys_cache.len() {
                        self.provider_key_selected = self.provider_key_selected.saturating_sub(1);
                    }
                }
            }
            KeyCode::Char('e') if step == 2 => {
                // Edit selected key
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

    fn handle_provider_edit_key_input(
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

    fn handle_provider_key_list(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        match code {
            _ if Self::is_cancel_key(code, modifiers) => {
                self.providers_cache = self.manager.list_providers().unwrap_or_default();
                if self.page == Page::ProviderManager {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
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

    fn handle_provider_key_add(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    fn handle_provider_test_key_list(
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
                        self.show_message(
                            format!(
                                "Provider '{}' key '{}' could not discover models: {}. The provider may still work with a manually entered model name.",
                                provider.name, key.name, failure.message
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

    fn handle_provider_key_edit(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
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

    fn handle_confirm_delete_provider(&mut self, code: KeyCode) -> Result<()> {
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
                        if self.page == Page::ProviderManager {
                            self.mode = Mode::Normal;
                        } else {
                            self.mode = Mode::ProviderList;
                        }
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            _ => {
                if self.page == Page::ProviderManager {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::ProviderList;
                }
            }
        }
        Ok(())
    }

    fn handle_confirm_delete_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let (pid, kid) = match &self.mode {
                    Mode::ConfirmDeleteKey {
                        provider_id,
                        key_id,
                        ..
                    } => (provider_id.clone(), key_id.clone()),
                    _ => return Ok(()),
                };
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

    // ── Provider helpers ─────────────────────────────────────────────────────

    fn move_provider_up(&mut self) {
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

    fn move_provider_down(&mut self) {
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

    fn move_provider_key_up(&mut self) {
        if self.provider_keys_cache.is_empty() {
            return;
        }
        if self.provider_key_selected > 0 {
            self.provider_key_selected -= 1;
        } else {
            self.provider_key_selected = self.provider_keys_cache.len() - 1;
        }
    }

    fn move_provider_key_down(&mut self) {
        if self.provider_keys_cache.is_empty() {
            return;
        }
        if self.provider_key_selected + 1 < self.provider_keys_cache.len() {
            self.provider_key_selected += 1;
        } else {
            self.provider_key_selected = 0;
        }
    }

    fn selected_provider_key(&self) -> Option<&ProviderKey> {
        self.provider_keys_cache
            .get(self.provider_key_selected)
            .or_else(|| self.provider_keys_cache.first())
    }

    // ── Provider renderers ───────────────────────────────────────────────────

    fn render_provider_list_popup(&mut self, f: &mut Frame) {
        let area = centered_rect(70, 16, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Providers ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        if self.providers_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No providers yet.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        for (i, p) in self.providers_cache.iter().enumerate() {
            let selected = self.provider_list_state.selected() == Some(i);
            let style = if selected {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(TEXT)
            };
            let prefix = if selected { "▶" } else { " " };
            let id_short = display_ellipsize(&p.id, 12);
            let url_short = display_ellipsize(&p.base_url, 35);
            let key_count = format!("keys:{}", p.keys.len());
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", prefix), style),
                Span::styled(
                    format!("{} ", display_pad(&id_short, 12)),
                    Style::default().fg(MUTED),
                ),
                Span::styled(format!("{} ", display_pad(&p.name, 18)), style),
                Span::styled(
                    format!("{} ", display_pad(&key_count, 8)),
                    Style::default().fg(DIM),
                ),
                Span::styled(url_short, Style::default().fg(DIM)),
            ]));
        }

        let scrollbar =
            Scrollbar::new(ScrollbarOrientation::VerticalRight).style(Style::default().fg(BORDER));
        self.provider_list_scroll = self
            .provider_list_scroll
            .content_length(self.providers_cache.len());
        f.render_stateful_widget(scrollbar, block.inner(area), &mut self.provider_list_scroll);

        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }

    fn render_provider_add_popup(&self, f: &mut Frame, step: usize) {
        let existing = self.provider_add_existing_id.is_some();
        let area = centered_rect(64, if existing { 10 } else { 12 }, f.area());
        f.render_widget(Clear, area);
        let title = if existing {
            " Add Key To Existing Provider "
        } else {
            " Add Provider "
        };
        let block = Block::default()
            .title(Line::from(Span::styled(
                title,
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let fields: Vec<(&str, &String, bool)> = if existing {
            vec![
                ("Provider", &self.provider_name_buf, false),
                ("Base URL", &self.provider_url_buf, false),
                ("Key name", &self.provider_key_name_buf, true),
                ("API Key", &self.provider_key_buf, false),
            ]
        } else {
            vec![
                ("Name", &self.provider_name_buf, true),
                ("Base URL", &self.provider_url_buf, true),
                ("Key name", &self.provider_key_name_buf, true),
                ("API Key", &self.provider_key_buf, true),
            ]
        };
        let mut lines = vec![Line::from("")];
        let mut editable_index = 0usize;
        for (label, value, editable) in fields {
            let active = editable && step == editable_index;
            let prefix = if active { "▶ " } else { "  " };
            let display = if active {
                display_with_cursor(value, self.cursor_pos)
            } else if value.is_empty() {
                "(empty)".to_string()
            } else {
                value.clone()
            };
            let style = if active {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {}{}: {}", prefix, display_pad(label, 9), display),
                style,
            )]));
            if editable {
                editable_index += 1;
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Enter", Style::default().fg(ACCENT).bold()),
            Span::styled(" next/save  ", Style::default().fg(DIM)),
            Span::styled("Esc/Ctrl+G", Style::default().fg(ACCENT).bold()),
            Span::styled(" cancel", Style::default().fg(DIM)),
        ]));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_provider_smart_paste_popup(&self, f: &mut Frame) {
        let area = centered_rect(76, 12, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Smart Input Provider ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let display = if self.provider_smart_paste_buf.is_empty() {
            "█".to_string()
        } else {
            display_with_cursor(&self.provider_smart_paste_buf, self.cursor_pos)
        };
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Ctrl+Y reads clipboard; paste JSON or cherrystudio://providers/api-keys below:",
                Style::default().fg(DIM),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", display),
                Style::default().fg(TEXT).bold(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Enter", Style::default().fg(ACCENT).bold()),
                Span::styled(" parse  ", Style::default().fg(DIM)),
                Span::styled("Esc/Ctrl+G", Style::default().fg(ACCENT).bold()),
                Span::styled(" back", Style::default().fg(DIM)),
            ]),
        ];
        if let Some(err) = &self.provider_smart_paste_error {
            lines.insert(
                4,
                Line::from(Span::styled(
                    format!("  {}", err),
                    Style::default().fg(DANGER),
                )),
            );
        }

        f.render_widget(
            Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_provider_edit_popup(&mut self, f: &mut Frame) {
        let (_pid, step) = match &self.mode {
            Mode::ProviderEdit { provider_id, step } => (provider_id.clone(), *step),
            _ => return,
        };

        let key_page_size = 6usize;
        let key_count = if step == 2 {
            self.provider_keys_cache.len().min(key_page_size)
        } else {
            0
        };
        let height = if step == 2 {
            12u16 + key_count as u16
        } else {
            10
        };
        let area = centered_rect(60, height, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Edit Provider ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let nf = if step == 0 { "▶ " } else { "  " };
        let nv = if step == 0 {
            display_with_cursor(&self.provider_name_buf, self.cursor_pos)
        } else if self.provider_name_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_name_buf.clone()
        };
        let ns = if step == 0 {
            Style::default().fg(TEXT).bold()
        } else {
            Style::default().fg(MUTED)
        };

        let uf = if step == 1 { "▶ " } else { "  " };
        let uv = if step == 1 {
            display_with_cursor(&self.provider_url_buf, self.cursor_pos)
        } else if self.provider_url_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_url_buf.clone()
        };
        let us = if step == 1 {
            Style::default().fg(TEXT).bold()
        } else {
            Style::default().fg(MUTED)
        };

        let kf = if step == 2 { "▶ " } else { "  " };
        let ks = if step == 2 {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default().fg(MUTED)
        };

        let mut lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(format!("  {}Name:  {}", nf, nv), ns)]),
            Line::from(vec![Span::styled(format!("  {}URL:   {}", uf, uv), us)]),
            Line::from(vec![Span::styled(
                format!("  {}Keys: {} keys", kf, self.provider_keys_cache.len()),
                ks,
            )]),
            Line::from(""),
        ];

        if step == 2 {
            if self.provider_keys_cache.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (no keys — press 'a' to add)",
                    Style::default().fg(DIM),
                )));
            } else {
                let total = self.provider_keys_cache.len();
                let (start, end) = visible_window(self.provider_key_selected, total, key_page_size);
                for (i, k) in self.provider_keys_cache[start..end].iter().enumerate() {
                    let index = start + i;
                    let selected = index == self.provider_key_selected;
                    let style = if selected {
                        Style::default().fg(ACCENT).bold()
                    } else {
                        Style::default().fg(TEXT)
                    };
                    let prefix = if selected { "▶" } else { " " };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", prefix), style),
                        Span::styled(format!("{} ", display_pad(&k.name, 20)), style),
                        Span::styled(mask_api_key(&k.api_key), Style::default().fg(DIM)),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  a=add  d=delete  Ctrl+P/N=nav  Enter=open/edit  Esc/Ctrl+G=cancel",
                Style::default().fg(DIM),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  Ctrl+P/N fields  Tab next  Enter save/open  Esc/Ctrl+G cancel",
                Style::default().fg(DIM),
            )));
        }
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_provider_key_list_popup(&mut self, f: &mut Frame) {
        let area = centered_rect(60, 14, f.area());
        f.render_widget(Clear, area);
        let pid = match &self.mode {
            Mode::ProviderKeyList { provider_id } => provider_id.clone(),
            _ => return,
        };
        let prov_name = self
            .manager
            .get_provider(&pid)
            .map(|p| p.name)
            .unwrap_or_default();

        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Keys — {} ", prov_name),
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        if self.provider_keys_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No keys yet. Press 'a' to add.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let sel = self.provider_key_selected;
        let total = self.provider_keys_cache.len();
        let page_size = 6usize;
        let (start, end) = visible_window(sel, total, page_size);
        let mut lines: Vec<Line> = Vec::new();
        for (i, k) in self.provider_keys_cache[start..end].iter().enumerate() {
            let index = start + i;
            let selected = index == sel;
            let style = if selected {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(TEXT)
            };
            let prefix = if selected { "▶" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", prefix), style),
                Span::styled(format!("{} ", display_pad(&k.name, 20)), style),
                Span::styled(mask_api_key(&k.api_key), Style::default().fg(DIM)),
            ]));
        }
        if total > page_size {
            let current_page = start / page_size + 1;
            let total_pages = total.div_ceil(page_size);
            lines.push(Line::from(Span::styled(
                format!(
                    "  Page {}/{}  Ctrl+P/N to scroll",
                    current_page, total_pages
                ),
                Style::default().fg(DIM),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N nav  a=add  e=edit  d=delete  t=models  T=anthropic  Esc/Ctrl+G=back",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }

    fn render_provider_test_key_list_popup(&mut self, f: &mut Frame) {
        let area = centered_rect(62, 14, f.area());
        f.render_widget(Clear, area);
        let pid = match &self.mode {
            Mode::ProviderTestKeyList { provider_id } => provider_id.clone(),
            _ => return,
        };
        let prov_name = self
            .manager
            .get_provider(&pid)
            .map(|p| p.name)
            .unwrap_or_default();

        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Test Key — {} ", prov_name),
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        if self.provider_keys_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  This provider has no keys.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let sel = self.provider_key_selected;
        let total = self.provider_keys_cache.len();
        let page_size = 6usize;
        let (start, end) = visible_window(sel, total, page_size);
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            "  Select a key for provider testing.",
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(""));
        for (i, k) in self.provider_keys_cache[start..end].iter().enumerate() {
            let index = start + i;
            let selected = index == sel;
            let style = if selected {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(TEXT)
            };
            let prefix = if selected { "▶" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", prefix), style),
                Span::styled(format!("{} ", display_pad(&k.name, 20)), style),
                Span::styled(mask_api_key(&k.api_key), Style::default().fg(DIM)),
            ]));
        }
        if total > page_size {
            let current_page = start / page_size + 1;
            let total_pages = total.div_ceil(page_size);
            lines.push(Line::from(Span::styled(
                format!("  Page {}/{}", current_page, total_pages),
                Style::default().fg(DIM),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N nav  t=models  T=anthropic  Esc/Ctrl+G=back",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }

    fn render_provider_key_add_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(60, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Add Key ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let labels = ["Name", "API Key"];
        let values = [&self.provider_key_name_buf, &self.provider_key_buf];
        let mut lines = vec![Line::from("")];
        for i in 0..2 {
            let active = step == i;
            let prefix = if active { "▶ " } else { "  " };
            let display = if active {
                display_with_cursor(values[i], self.cursor_pos)
            } else if values[i].is_empty() {
                "(empty)".to_string()
            } else {
                values[i].clone()
            };
            let style = if active {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {}{}: {}", prefix, labels[i], display),
                style,
            )]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N fields  Tab next  Enter confirm  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_provider_key_edit_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(60, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Edit Key ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let labels = ["Name", "API Key"];
        let values = [&self.provider_key_name_buf, &self.provider_key_buf];
        let mut lines = vec![Line::from("")];
        for i in 0..2 {
            let active = step == i;
            let prefix = if active { "▶ " } else { "  " };
            let display = if active {
                display_with_cursor(values[i], self.cursor_pos)
            } else if values[i].is_empty() {
                "(empty)".to_string()
            } else {
                values[i].clone()
            };
            let style = if active {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {}{}: {}", prefix, labels[i], display),
                style,
            )]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N fields  Tab next  Enter confirm  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_provider_anthropic_test_popup(&self, f: &mut Frame) {
        let (provider_id, key_id, field) = match &self.mode {
            Mode::ProviderAnthropicTest {
                provider_id,
                key_id,
                field,
                ..
            } => (provider_id, key_id, *field),
            _ => return,
        };
        let provider_name = self
            .providers_cache
            .iter()
            .find(|provider| &provider.id == provider_id)
            .map(|provider| provider.name.as_str())
            .unwrap_or("Provider");
        let key_name = self
            .provider_keys_cache
            .iter()
            .find(|key| &key.id == key_id)
            .map(|key| key.name.as_str())
            .unwrap_or("Key");

        let area = centered_rect(78, 22, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Provider Test — {} / {} ", provider_name, key_name),
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        let inner = block.inner(area);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(9),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(inner);

        let model_active = field == 0;
        let prompt_active = field == 1;
        let model_value = if model_active {
            display_with_cursor(&self.provider_test_model_buf, self.cursor_pos)
        } else if self.provider_test_model_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_test_model_buf.clone()
        };
        let prompt_value = if prompt_active {
            display_with_cursor(&self.provider_test_prompt_buf, self.cursor_pos)
        } else if self.provider_test_prompt_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_test_prompt_buf.clone()
        };

        let list_block = Block::default()
            .title(Line::from(Span::styled(
                " Models ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(BG));
        let mut list_lines = vec![Line::from("")];
        let page_size = 5usize;
        let total = self.provider_test_models.len();
        let (page_start, page_end) =
            visible_window(self.provider_test_model_selected, total, page_size);
        if total == 0 {
            let msg = match &self.provider_test_model_fetch_state {
                ModelFetchState::Loaded | ModelFetchState::Empty => {
                    "  No models returned from provider.".to_string()
                }
                ModelFetchState::Unavailable(reason) => format!("  {}", reason),
            };
            list_lines.push(Line::from(Span::styled(msg, Style::default().fg(DIM))));
        } else {
            for (offset, model) in self.provider_test_models[page_start..page_end]
                .iter()
                .enumerate()
            {
                let index = page_start + offset;
                let selected = index == self.provider_test_model_selected;
                let prefix = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(ACCENT).bold()
                } else {
                    Style::default().fg(TEXT)
                };
                list_lines.push(Line::from(vec![Span::styled(
                    format!("  {}{}", prefix, model),
                    style,
                )]));
            }
            let total_pages = total.div_ceil(page_size);
            let current_page = page_start / page_size + 1;
            list_lines.push(Line::from(""));
            list_lines.push(Line::from(Span::styled(
                format!(
                    "  Page {}/{}  PgUp/PgDn to scroll",
                    current_page, total_pages
                ),
                Style::default().fg(DIM),
            )));
        }
        f.render_widget(
            Paragraph::new(Text::from(list_lines))
                .block(list_block)
                .wrap(Wrap { trim: false }),
            sections[0],
        );

        let model_lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("  {}Model   ", if model_active { "▶ " } else { "  " }),
                    Style::default()
                        .fg(if model_active { ACCENT } else { DIM })
                        .bold(),
                ),
                Span::styled(model_value, Style::default().fg(TEXT)),
            ]),
            Line::from(Span::styled(
                "  Tab completes from fetched models; you can also type manually.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(Paragraph::new(Text::from(model_lines)), sections[2]);

        let prompt_lines = vec![Line::from(vec![
            Span::styled(
                format!("  {}Prompt  ", if prompt_active { "▶ " } else { "  " }),
                Style::default()
                    .fg(if prompt_active { ACCENT } else { DIM })
                    .bold(),
            ),
            Span::styled(prompt_value, Style::default().fg(TEXT)),
        ])];
        f.render_widget(Paragraph::new(Text::from(prompt_lines)), sections[3]);

        let footer_lines = vec![
            Line::from(Span::styled(
                "  Ctrl+P/N switches fields. When Model is focused, the same keys browse fetched models.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Enter sends one non-streaming /v1/messages request. Esc/Ctrl+G exits.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(
            Paragraph::new(Text::from(footer_lines)).wrap(Wrap { trim: false }),
            sections[4],
        );
    }

    fn render_provider_anthropic_outcome_popup(&self, f: &mut Frame) {
        let (model, input_tokens, output_tokens, body, is_error) = match &self.mode {
            Mode::ProviderAnthropicOutcome {
                model,
                input_tokens,
                output_tokens,
                body,
                is_error,
                ..
            } => (model, *input_tokens, *output_tokens, body, *is_error),
            _ => return,
        };

        let area = centered_rect(78, 16, f.area());
        f.render_widget(Clear, area);
        let accent = if is_error { DANGER } else { SUCCESS };
        let block = Block::default()
            .title(Line::from(Span::styled(
                if is_error {
                    " Anthropic Test Error "
                } else {
                    " Anthropic Test Result "
                },
                Style::default().fg(accent).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        let inner = block.inner(area);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Length(1),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        let usage = match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => format!("input {}   output {}", input, output),
            (Some(input), None) => format!("input {}", input),
            (None, Some(output)) => format!("output {}", output),
            (None, None) => "(no usage returned)".to_string(),
        };

        let meta_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Model   ", Style::default().fg(DIM)),
                Span::styled(model, Style::default().fg(TEXT).bold()),
            ]),
            Line::from(vec![
                Span::styled("  Usage   ", Style::default().fg(DIM)),
                Span::styled(
                    if is_error {
                        "(request failed)".to_string()
                    } else {
                        usage
                    },
                    Style::default().fg(TEXT),
                ),
            ]),
        ];
        f.render_widget(Paragraph::new(meta_lines), sections[0]);

        let reply_block = Block::default()
            .title(Line::from(Span::styled(
                if is_error { " Error " } else { " Reply " },
                Style::default()
                    .fg(if is_error { DANGER } else { ACCENT })
                    .bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(BG));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(format!("  {}", body)),
            ]))
            .block(reply_block)
            .wrap(Wrap { trim: false }),
            sections[2],
        );

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Enter returns to test. Esc/Ctrl+G or q exits.",
                Style::default().fg(DIM),
            ))),
            sections[3],
        );
    }

    fn render_confirm_delete_provider_popup(&self, f: &mut Frame) {
        let name = match &self.mode {
            Mode::ConfirmDeleteProvider { name, .. } => name.clone(),
            _ => return,
        };
        self.render_confirm_popup(
            f,
            &format!("Delete provider '{}'?", name),
            "This cannot be undone. Press 'y' to confirm.",
        );
    }

    fn render_provider_edit_key_input_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(60, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Add Key ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let labels = ["Name", "API Key"];
        let values = [&self.provider_key_name_buf, &self.provider_key_buf];
        let mut lines = vec![Line::from("")];
        for i in 0..2 {
            let active = step == i;
            let prefix = if active { "▶ " } else { "  " };
            let display = if active {
                display_with_cursor(values[i], self.cursor_pos)
            } else if values[i].is_empty() {
                "(empty)".to_string()
            } else {
                values[i].clone()
            };
            let style = if active {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("  {}{}: {}", prefix, labels[i], display),
                style,
            )]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter to confirm, Esc/Ctrl+G to cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn render_confirm_delete_key_popup(&self, f: &mut Frame) {
        let name = match &self.mode {
            Mode::ConfirmDeleteKey { name, .. } => name.clone(),
            _ => return,
        };
        self.render_confirm_popup(
            f,
            &format!("Delete key '{}'?", name),
            "This cannot be undone. Press 'y' to confirm.",
        );
    }

    fn render_confirm_popup(&self, f: &mut Frame, title: &str, hint: &str) {
        let area = centered_rect(50, 7, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" {} ", title),
                Style::default().fg(DANGER).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DANGER))
            .style(Style::default().bg(PANEL));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {}", hint),
                    Style::default().fg(TEXT),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  [y] yes    [other] cancel",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn launch_args_from_str(s: &str) -> Option<Vec<String>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.split_whitespace().map(String::from).collect())
    }
}

/// Replace the last whitespace-delimited word in `s` with `replacement`.
fn replace_last_word(s: &str, replacement: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return replacement.to_string();
    }
    if let Some(last_space) = trimmed.rfind(char::is_whitespace) {
        format!("{} {}", &trimmed[..last_space], replacement)
    } else {
        replacement.to_string()
    }
}

fn provider_test_key_selection(keys: &[ProviderKey]) -> ProviderTestKeySelection {
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
    }
}

fn provider_test_outcome_next_mode(
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

fn model_fetch_state_for_models(models: &[String]) -> ModelFetchState {
    if models.is_empty() {
        ModelFetchState::Empty
    } else {
        ModelFetchState::Loaded
    }
}

fn model_fetch_unavailable_message(error: &str) -> String {
    format!(
        "/v1/models unavailable: {}. Manual model entry still works.",
        error
    )
}

fn trim_model_context_suffix(model: &str) -> &str {
    strip_model_1m_suffix(model)
}

fn complete_provider_test_model(models: &[String], current: &str) -> Option<String> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SmartProviderPaste {
    name: String,
    base_url: String,
    key_name: String,
    api_key: String,
}

fn parse_provider_smart_paste(raw: &str) -> Result<SmartProviderPaste> {
    let input = raw.trim();
    if input.starts_with("https://app.nextchat.dev/#/?settings=") {
        return parse_nextchat_settings_url(input);
    }
    if input.starts_with("opencat://team/join?") {
        return parse_opencat_join_url(input);
    }
    if input.starts_with("cherrystudio://providers/api-keys") {
        return parse_cherrystudio_provider_url(input);
    }
    parse_newapi_provider_json(input)
}

fn parse_newapi_provider_json(input: &str) -> Result<SmartProviderPaste> {
    let value: serde_json::Value = serde_json::from_str(input)?;
    let base_url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("Smart paste needs JSON fields 'url' and 'key'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn parse_cherrystudio_provider_url(input: &str) -> Result<SmartProviderPaste> {
    let data = input
        .split_once('?')
        .map(|(_, query)| query)
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                (key == "data").then_some(value)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Cherry Studio URL is missing data=."))?;
    let decoded_param = percent_decode(data)?;
    let decoded = URL_SAFE_NO_PAD.decode(decoded_param.as_bytes())?;
    let value: serde_json::Value = serde_json::from_slice(&decoded)?;

    let base_url = value
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("apiKey")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("Cherry Studio data needs 'baseUrl' and 'apiKey'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Default")
            .to_string(),
        api_key,
    })
}

fn parse_nextchat_settings_url(input: &str) -> Result<SmartProviderPaste> {
    let encoded = input
        .split_once("#/?settings=")
        .map(|(_, value)| value)
        .ok_or_else(|| anyhow::anyhow!("NextChat URL is missing settings=."))?;
    let decoded = percent_decode(encoded)?;
    let value: serde_json::Value = serde_json::from_str(&decoded)?;
    let base_url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let api_key = value
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        bail!("NextChat settings need 'url' and 'key'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn parse_opencat_join_url(input: &str) -> Result<SmartProviderPaste> {
    let query = input
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| anyhow::anyhow!("OpenCat URL is missing query params."))?;
    let mut base_url = String::new();
    let mut api_key = String::new();
    for part in query.split('&') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(value)?;
        match key {
            "domain" => base_url = decoded.trim().to_string(),
            "token" => api_key = decoded.trim().to_string(),
            _ => {}
        }
    }
    if base_url.is_empty() || api_key.is_empty() {
        bail!("OpenCat join URL needs 'domain' and 'token'.");
    }

    Ok(SmartProviderPaste {
        name: inferred_provider_name(&base_url),
        base_url,
        key_name: "Default".to_string(),
        api_key,
    })
}

fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
                let value = u8::from_str_radix(hex, 16)?;
                out.push(value);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    Ok(String::from_utf8(out)?)
}

fn inferred_provider_name(base_url: &str) -> String {
    base_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Provider")
        .to_string()
}

fn model_has_1m_suffix(model: &str) -> bool {
    model.trim_end().ends_with("[1m]")
}

fn strip_model_1m_suffix(model: &str) -> &str {
    let trimmed = model.trim_end();
    trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim_end()
}

fn apply_model_1m_flag(model: &str, enabled: bool) -> String {
    let base = strip_model_1m_suffix(model).trim_end().to_string();
    if base.is_empty() {
        return base;
    }
    if enabled {
        format!("{}[1m]", base)
    } else {
        base
    }
}

fn display_ellipsize(s: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }

    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let target_width = max_width - 3;
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > target_width {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out.push_str("...");
    out
}

fn display_pad(s: &str, width: usize) -> String {
    let value = display_ellipsize(s, width);
    let value_width = UnicodeWidthStr::width(value.as_str());
    if value_width >= width {
        value
    } else {
        format!("{}{}", value, " ".repeat(width - value_width))
    }
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.chars().count() <= 12 {
        return api_key.to_string();
    }

    let prefix: String = api_key.chars().take(6).collect();
    let suffix: String = api_key
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width: w,
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileManager;
    use std::env;
    use tempfile::TempDir;

    fn make_test_app() -> App {
        let tmp = TempDir::new().unwrap();
        let old_home = env::var_os("USERPROFILE");
        let old_home_unix = env::var_os("HOME");
        unsafe {
            env::set_var("USERPROFILE", tmp.path());
            env::set_var("HOME", tmp.path());
        }
        let manager = ProfileManager::new().unwrap();
        let app = App::new(manager).unwrap();
        unsafe {
            match old_home {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }
            match old_home_unix {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
        }
        app
    }

    #[test]
    fn smart_paste_parses_newapi_json() {
        let base_url = "https://generated-provider.invalid";
        let api_key = "sk-test-generated-key-000000000000000000000000";
        let parsed = parse_provider_smart_paste(&format!(
            r#"{{"_type":"newapi_channel_conn","key":"{}","url":"{}"}}"#,
            api_key, base_url
        ))
        .unwrap();

        assert_eq!(parsed.name, "generated-provider.invalid");
        assert_eq!(parsed.base_url, base_url);
        assert_eq!(parsed.api_key, api_key);
        assert_eq!(parsed.key_name, "Default");
    }

    #[test]
    fn smart_paste_parses_cherrystudio_url() {
        let base_url = "https://generated-provider.invalid";
        let api_key = "sk-test-generated-key-111111111111111111111111";
        let data = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "id": "generated-api",
                "baseUrl": base_url,
                "apiKey": api_key,
            })
            .to_string(),
        );
        let parsed = parse_provider_smart_paste(&format!(
            "cherrystudio://providers/api-keys?v=1&data={data}"
        ))
        .unwrap();

        assert_eq!(parsed.name, "generated-provider.invalid");
        assert_eq!(parsed.key_name, "generated-api");
        assert_eq!(parsed.base_url, base_url);
        assert_eq!(parsed.api_key, api_key);
    }

    #[test]
    fn smart_paste_parses_nextchat_url() {
        let base_url = "https://generated-provider.invalid";
        let api_key = "sk-test-generated-key-555555555555555555555555";
        let parsed = parse_provider_smart_paste(&format!(
            "https://app.nextchat.dev/#/?settings={{%22key%22:%22{api_key}%22,%22url%22:%22https%3A%2F%2Fgenerated-provider.invalid%22}}"
        ))
        .unwrap();

        assert_eq!(parsed.name, "generated-provider.invalid");
        assert_eq!(parsed.key_name, "Default");
        assert_eq!(parsed.base_url, base_url);
        assert_eq!(parsed.api_key, api_key);
    }

    #[test]
    fn smart_paste_parses_opencat_url() {
        let base_url = "https://generated-provider.invalid";
        let api_key = "sk-test-generated-key-666666666666666666666666";
        let parsed = parse_provider_smart_paste(&format!(
            "opencat://team/join?domain=https%3A%2F%2Fgenerated-provider.invalid&token={api_key}"
        ))
        .unwrap();

        assert_eq!(parsed.name, "generated-provider.invalid");
        assert_eq!(parsed.key_name, "Default");
        assert_eq!(parsed.base_url, base_url);
        assert_eq!(parsed.api_key, api_key);
    }

    #[test]
    fn visible_window_returns_correct_range() {
        assert_eq!(visible_window(0, 10, 6), (0, 6));
        assert_eq!(visible_window(5, 10, 6), (0, 6));
        assert_eq!(visible_window(6, 10, 6), (6, 10));
        assert_eq!(visible_window(9, 10, 6), (6, 10));
        assert_eq!(visible_window(0, 0, 6), (0, 0));
        assert_eq!(visible_window(0, 3, 6), (0, 3));
    }

    #[test]
    fn display_with_cursor_inserts_at_start_middle_end() {
        assert_eq!(display_with_cursor("abc", 0), "█abc");
        assert_eq!(display_with_cursor("abc", 1), "a█bc");
        assert_eq!(display_with_cursor("abc", 3), "abc█");
    }

    #[test]
    fn display_with_cursor_handles_empty_string() {
        assert_eq!(display_with_cursor("", 0), "█");
    }

    #[test]
    fn display_with_cursor_clamps_to_utf8_boundary() {
        assert_eq!(display_with_cursor("白日", "白".len()), "白█日");
        assert_eq!(display_with_cursor("白日", 1), "█白日");
        assert_eq!(display_with_cursor("白日", usize::MAX), "白日█");
    }

    #[test]
    fn insert_str_at_cursor_inserts_at_utf8_boundary() {
        let mut buf = "白日".to_string();
        let mut cursor_pos = 1usize;

        insert_str_at_cursor(&mut buf, &mut cursor_pos, "X");

        assert_eq!(buf, "X白日");
        assert_eq!(cursor_pos, "X".len());
    }

    #[test]
    fn insert_filtered_str_at_cursor_filters_alias_chars() {
        let mut buf = "ab".to_string();
        let mut cursor_pos = 1usize;

        insert_filtered_str_at_cursor(&mut buf, &mut cursor_pos, "c.d_1-", is_alias_char);

        assert_eq!(buf, "acd_1-b");
        assert_eq!(cursor_pos, "acd_1-".len());
    }

    #[test]
    fn emacs_edit_handles_multiple_chinese_characters() {
        let mut buf = String::new();
        let mut pos = 0usize;

        assert!(emacs_edit(
            KeyCode::Char('白'),
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true
        ));
        assert_eq!(buf, "白");

        assert!(emacs_edit(
            KeyCode::Char('日'),
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true
        ));
        assert_eq!(buf, "白日");
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn emacs_edit_treats_ctrl_h_as_backspace() {
        let mut buf = "Provider".to_string();
        let mut pos = buf.len();

        assert!(emacs_edit(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL,
            &mut buf,
            &mut pos,
            true
        ));

        assert_eq!(buf, "Provide");
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn emacs_edit_treats_backspace_control_chars_as_backspace() {
        for code in [KeyCode::Char('\u{8}'), KeyCode::Char('\u{7f}')] {
            let mut buf = "Provider".to_string();
            let mut pos = buf.len();

            assert!(emacs_edit(
                code,
                KeyModifiers::empty(),
                &mut buf,
                &mut pos,
                true
            ));

            assert_eq!(buf, "Provide");
            assert_eq!(pos, buf.len());
        }
    }

    #[test]
    fn emacs_edit_backspace_handles_utf8_in_small_add_dialog_buffers() {
        let mut buf = "白日".to_string();
        let mut pos = buf.len();

        assert!(emacs_edit(
            KeyCode::Backspace,
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true
        ));

        assert_eq!(buf, "白");
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn alias_input_still_filters_invalid_chars() {
        let mut buf = String::new();
        let mut pos = 0usize;

        if 'x'.is_ascii_alphanumeric() || 'x' == '-' || 'x' == '_' {
            emacs_edit(
                KeyCode::Char('x'),
                KeyModifiers::empty(),
                &mut buf,
                &mut pos,
                true,
            );
        }
        if '.'.is_ascii_alphanumeric() || '.' == '-' || '.' == '_' {
            emacs_edit(
                KeyCode::Char('.'),
                KeyModifiers::empty(),
                &mut buf,
                &mut pos,
                true,
            );
        }

        assert_eq!(buf, "x");
    }

    #[test]
    fn provider_test_key_selection_detects_empty_single_and_multiple() {
        assert_eq!(
            provider_test_key_selection(&[]),
            ProviderTestKeySelection::NoKeys
        );

        let only = ProviderKey {
            id: "key_one".into(),
            name: "Default".into(),
            api_key: "sk-test-generated-key-222222222222222222222222".into(),
        };
        assert_eq!(
            provider_test_key_selection(std::slice::from_ref(&only)),
            ProviderTestKeySelection::Single(only)
        );

        let many = vec![
            ProviderKey {
                id: "key_one".into(),
                name: "A".into(),
                api_key: "sk-test-generated-key-333333333333333333333333".into(),
            },
            ProviderKey {
                id: "key_two".into(),
                name: "B".into(),
                api_key: "sk-test-generated-key-444444444444444444444444".into(),
            },
        ];
        assert_eq!(
            provider_test_key_selection(&many),
            ProviderTestKeySelection::Multiple
        );
    }

    #[test]
    fn trim_model_context_suffix_removes_1m_suffix() {
        assert_eq!(
            trim_model_context_suffix("claude-3-7-sonnet[1m]"),
            "claude-3-7-sonnet"
        );
        assert_eq!(
            trim_model_context_suffix("claude-3-7-sonnet"),
            "claude-3-7-sonnet"
        );
    }

    #[test]
    fn apply_model_1m_flag_normalizes_suffix() {
        assert_eq!(
            apply_model_1m_flag("claude-3-7-sonnet[1m]", false),
            "claude-3-7-sonnet"
        );
        assert_eq!(
            apply_model_1m_flag("claude-3-7-sonnet", true),
            "claude-3-7-sonnet[1m]"
        );
        assert_eq!(
            apply_model_1m_flag("claude-3-7-sonnet[1m]", true),
            "claude-3-7-sonnet[1m]"
        );
    }

    #[test]
    fn complete_provider_test_model_prefers_exact_then_fuzzy_match() {
        let models = vec![
            "LongCat-2.0-Preview".to_string(),
            "deepseek-ai/deepseek-v4-flash".to_string(),
            "claude-3-7-sonnet".to_string(),
        ];

        assert_eq!(
            complete_provider_test_model(&models, ""),
            Some("LongCat-2.0-Preview".to_string())
        );
        assert_eq!(
            complete_provider_test_model(&models, "claude-3-7-sonnet"),
            Some("claude-3-7-sonnet".to_string())
        );
        assert_eq!(
            complete_provider_test_model(&models, "deepseek-v4"),
            Some("deepseek-ai/deepseek-v4-flash".to_string())
        );
    }

    #[test]
    fn provider_test_q_is_text_input() {
        let mut app = make_test_app();
        app.provider_test_prompt_buf = "quic".into();
        app.cursor_pos = app.provider_test_prompt_buf.len();
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 1,
        };

        app.handle_provider_anthropic_test(KeyCode::Char('q'), KeyModifiers::empty())
            .unwrap();

        assert_eq!(app.provider_test_prompt_buf, "quicq");
        assert_eq!(app.cursor_pos, app.provider_test_prompt_buf.len());
        assert_eq!(
            app.mode,
            Mode::ProviderAnthropicTest {
                provider_id: "prov_generated".into(),
                key_id: "key_generated".into(),
                source: ProviderTestSource::Page,
                field: 1,
            }
        );
    }

    #[test]
    fn provider_test_model_navigation_takes_priority_when_model_field_is_active() {
        let mut app = make_test_app();
        app.provider_test_models = vec!["model-a".into(), "model-b".into()];
        app.provider_test_model_buf = "model-a".into();
        app.provider_test_prompt_buf = "Hello".into();
        app.provider_test_model_selected = 0;
        app.cursor_pos = app.provider_test_model_buf.len();
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 0,
        };

        app.handle_provider_anthropic_test(KeyCode::Char('n'), KeyModifiers::CONTROL)
            .unwrap();

        assert_eq!(app.provider_test_model_selected, 1);
        assert_eq!(app.provider_test_model_buf, "model-b");
        assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
        assert_eq!(
            app.mode,
            Mode::ProviderAnthropicTest {
                provider_id: "prov_generated".into(),
                key_id: "key_generated".into(),
                source: ProviderTestSource::Page,
                field: 0,
            }
        );
    }

    #[test]
    fn provider_test_model_field_accepts_bare_j_and_k() {
        let mut app = make_test_app();
        app.provider_test_models = vec!["model-a".into(), "model-b".into()];
        app.provider_test_model_buf.clear();
        app.provider_test_model_selected = 0;
        app.cursor_pos = 0;
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 0,
        };

        app.handle_provider_anthropic_test(KeyCode::Char('j'), KeyModifiers::empty())
            .unwrap();
        app.handle_provider_anthropic_test(KeyCode::Char('k'), KeyModifiers::empty())
            .unwrap();

        assert_eq!(app.provider_test_model_buf, "jk");
        assert_eq!(app.provider_test_model_selected, 0);
    }

    #[test]
    fn provider_test_model_field_down_still_navigates_models() {
        let mut app = make_test_app();
        app.provider_test_models = vec!["model-a".into(), "model-b".into()];
        app.provider_test_model_buf = "model-a".into();
        app.provider_test_model_selected = 0;
        app.cursor_pos = app.provider_test_model_buf.len();
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 0,
        };

        app.handle_provider_anthropic_test(KeyCode::Down, KeyModifiers::empty())
            .unwrap();

        assert_eq!(app.provider_test_model_buf, "model-b");
        assert_eq!(app.provider_test_model_selected, 1);
        assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
    }

    #[test]
    fn provider_test_manual_model_match_syncs_selection() {
        let mut app = make_test_app();
        app.provider_test_models = vec!["model-a".into(), "model-b".into(), "model-c".into()];
        app.provider_test_model_buf = "model-a".into();
        app.provider_test_model_selected = 0;
        app.cursor_pos = app.provider_test_model_buf.len();
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 0,
        };

        app.handle_provider_anthropic_test(KeyCode::Char('u'), KeyModifiers::CONTROL)
            .unwrap();
        for ch in "model-c".chars() {
            app.handle_provider_anthropic_test(KeyCode::Char(ch), KeyModifiers::empty())
                .unwrap();
        }

        assert_eq!(app.provider_test_model_buf, "model-c");
        assert_eq!(app.provider_test_model_selected, 2);
    }

    #[test]
    fn lite_set_slot_value_moves_cursor_to_end() {
        let mut app = make_test_app();
        app.lite_step = 5;
        app.cursor_pos = 0;

        app.set_slot_value("claude-sonnet".into());

        assert_eq!(app.lite_mod_model, "claude-sonnet");
        assert_eq!(app.cursor_pos, app.lite_mod_model.len());
    }

    #[test]
    fn lite_fetching_ctrl_g_cancels() {
        let mut app = make_test_app();
        app.mode = Mode::LiteFetching;

        match app.mode.clone() {
            Mode::LiteFetching => {
                if App::is_cancel_key(KeyCode::Char('g'), KeyModifiers::CONTROL) {
                    app.mode = Mode::Normal;
                }
            }
            _ => unreachable!(),
        }

        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn provider_test_outcome_non_q_returns_to_same_form() {
        assert_eq!(
            provider_test_outcome_next_mode(
                KeyCode::Enter,
                KeyModifiers::empty(),
                "prov_generated",
                "key_generated",
                ProviderTestSource::KeyList,
                1
            ),
            Mode::ProviderAnthropicTest {
                provider_id: "prov_generated".into(),
                key_id: "key_generated".into(),
                source: ProviderTestSource::KeyList,
                field: 1,
            }
        );
    }

    #[test]
    fn provider_test_outcome_q_exits_to_parent_mode() {
        assert_eq!(
            provider_test_outcome_next_mode(
                KeyCode::Char('q'),
                KeyModifiers::empty(),
                "prov_generated",
                "key_generated",
                ProviderTestSource::KeyList,
                0
            ),
            Mode::ProviderKeyList {
                provider_id: "prov_generated".into(),
            }
        );
        assert_eq!(
            provider_test_outcome_next_mode(
                KeyCode::Char('q'),
                KeyModifiers::empty(),
                "prov_generated",
                "key_generated",
                ProviderTestSource::TestKeyList,
                0
            ),
            Mode::ProviderTestKeyList {
                provider_id: "prov_generated".into(),
            }
        );
        assert_eq!(
            provider_test_outcome_next_mode(
                KeyCode::Char('q'),
                KeyModifiers::empty(),
                "prov_generated",
                "key_generated",
                ProviderTestSource::Page,
                0
            ),
            Mode::Normal
        );
    }

    #[test]
    fn model_fetch_unavailable_message_marks_manual_entry_possible() {
        let message = model_fetch_unavailable_message("403 forbidden");

        assert!(message.contains("/v1/models unavailable"));
        assert!(message.contains("Manual model entry still works"));
    }

    #[test]
    fn model_fetch_state_for_empty_models_is_empty() {
        assert_eq!(model_fetch_state_for_models(&[]), ModelFetchState::Empty);
    }

    #[test]
    fn model_fetch_state_for_non_empty_models_is_loaded() {
        assert_eq!(
            model_fetch_state_for_models(&["claude-3-7-sonnet".to_string()]),
            ModelFetchState::Loaded
        );
    }

    #[test]
    fn provider_edit_cursor_pos_tracks_active_field() {
        assert_eq!(
            provider_edit_cursor_pos(0, "Provider Name", "https://example.invalid"),
            "Provider Name".len()
        );
        assert_eq!(
            provider_edit_cursor_pos(1, "Provider Name", "https://example.invalid"),
            "https://example.invalid".len()
        );
        assert_eq!(
            provider_edit_cursor_pos(2, "Provider Name", "https://example.invalid"),
            0
        );
    }

    #[test]
    fn shift_tab_switches_manager_in_allowed_modes() {
        let mut app = make_test_app();
        app.mode = Mode::Normal;
        app.page = Page::ProfileManager;

        assert!(
            app.handle_manager_switch_key(KeyCode::BackTab, KeyModifiers::empty())
                .unwrap()
        );
        assert_eq!(app.page, Page::ProviderManager);
        assert_eq!(app.mode, Mode::Normal);

        app.mode = Mode::Search;
        assert!(
            app.handle_manager_switch_key(KeyCode::Tab, KeyModifiers::SHIFT)
                .unwrap()
        );
        assert_eq!(app.page, Page::ProfileManager);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn shift_tab_does_not_switch_manager_in_edit_mode() {
        let mut app = make_test_app();
        app.mode = Mode::EditProfile {
            profile_id: "profile-generated".into(),
            step: 1,
        };
        app.page = Page::ProfileManager;

        assert!(
            !app.handle_manager_switch_key(KeyCode::BackTab, KeyModifiers::empty())
                .unwrap()
        );
        assert_eq!(app.page, Page::ProfileManager);
        assert_eq!(
            app.mode,
            Mode::EditProfile {
                profile_id: "profile-generated".into(),
                step: 1,
            }
        );
    }

    #[test]
    fn provider_edit_tab_only_advances_fields() {
        let mut app = make_test_app();
        app.page = Page::ProviderManager;
        app.provider_name_buf = "Provider Name".into();
        app.provider_url_buf = "https://example.invalid".into();
        app.mode = Mode::ProviderEdit {
            provider_id: "prov_generated".into(),
            step: 0,
        };

        app.handle_provider_edit(KeyCode::Tab, KeyModifiers::empty())
            .unwrap();
        assert_eq!(
            app.mode,
            Mode::ProviderEdit {
                provider_id: "prov_generated".into(),
                step: 1,
            }
        );
        assert_eq!(app.cursor_pos, "https://example.invalid".len());
    }

    #[test]
    fn provider_test_ctrl_p_moves_to_previous_field() {
        let mut app = make_test_app();
        app.provider_test_model_buf = "claude-3-7-sonnet".into();
        app.provider_test_prompt_buf = "Hello".into();
        app.mode = Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 1,
        };

        app.handle_provider_anthropic_test(KeyCode::Char('p'), KeyModifiers::CONTROL)
            .unwrap();

        assert_eq!(
            app.mode,
            Mode::ProviderAnthropicTest {
                provider_id: "prov_generated".into(),
                key_id: "key_generated".into(),
                source: ProviderTestSource::Page,
                field: 0,
            }
        );
        assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
    }

    #[test]
    fn provider_edit_cancel_does_not_save_changes() {
        let mut app = make_test_app();
        let provider = app
            .manager
            .add_provider_with_key_name(
                "Original Provider",
                "https://example.invalid",
                "Default",
                "sk-test-generated-key-777777777777777777777777",
            )
            .unwrap();
        app.providers_cache = app.manager.list_providers().unwrap();
        app.page = Page::ProviderManager;
        app.provider_name_buf = "Changed Provider".into();
        app.provider_url_buf = "https://changed.invalid".into();
        app.mode = Mode::ProviderEdit {
            provider_id: provider.id.clone(),
            step: 0,
        };

        app.handle_provider_edit(KeyCode::Esc, KeyModifiers::empty())
            .unwrap();

        let refreshed = app.manager.get_provider(&provider.id).unwrap();
        assert_eq!(refreshed.name, "Original Provider");
        assert_eq!(refreshed.base_url, "https://example.invalid");
    }

    #[test]
    fn edit_profile_down_no_longer_advances_fields() {
        let mut app = make_test_app();
        app.mode = Mode::EditProfile {
            profile_id: "profile-generated".into(),
            step: 0,
        };

        app.handle_edit_profile(KeyCode::Down, KeyModifiers::empty())
            .unwrap();

        assert_eq!(
            app.mode,
            Mode::EditProfile {
                profile_id: "profile-generated".into(),
                step: 0,
            }
        );
    }

    #[test]
    fn provider_key_add_ctrl_n_advances_fields() {
        let mut app = make_test_app();
        app.mode = Mode::ProviderKeyAdd {
            provider_id: "prov_generated".into(),
            step: 0,
        };

        app.handle_provider_key_add(KeyCode::Char('n'), KeyModifiers::CONTROL)
            .unwrap();

        assert_eq!(
            app.mode,
            Mode::ProviderKeyAdd {
                provider_id: "prov_generated".into(),
                step: 1,
            }
        );
    }

    #[test]
    fn provider_key_edit_ctrl_p_moves_to_previous_field() {
        let mut app = make_test_app();
        app.mode = Mode::ProviderKeyEdit {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            step: 1,
            source: KeyEditSource::ProviderKeyList,
        };

        app.handle_provider_key_edit(KeyCode::Char('p'), KeyModifiers::CONTROL)
            .unwrap();

        assert_eq!(
            app.mode,
            Mode::ProviderKeyEdit {
                provider_id: "prov_generated".into(),
                key_id: "key_generated".into(),
                step: 0,
                source: KeyEditSource::ProviderKeyList,
            }
        );
    }

    #[test]
    fn add_full_name_ctrl_g_cancels() {
        let mut app = make_test_app();
        app.mode = Mode::AddFullName;
        app.input_buffer = "demo".into();

        app.handle_add_full_name(KeyCode::Char('g'), KeyModifiers::CONTROL)
            .unwrap();

        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn provider_test_outcome_ctrl_g_exits_to_parent_mode() {
        assert_eq!(
            provider_test_outcome_next_mode(
                KeyCode::Char('g'),
                KeyModifiers::CONTROL,
                "prov_generated",
                "key_generated",
                ProviderTestSource::KeyList,
                0
            ),
            Mode::ProviderKeyList {
                provider_id: "prov_generated".into(),
            }
        );
    }
}
