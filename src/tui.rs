use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};

use crate::profile::{fetch_models, LightweightEnv, Profile, ProfileKind, ProfileManager};

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
    /// Lightweight creation: token input
    LiteToken,
    /// Lightweight creation: URL input
    LiteUrl,
    /// Fetching models spinner
    LiteFetching,
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
}

// ── App ───────────────────────────────────────────────────────────────────────
pub struct App {
    manager: ProfileManager,
    profiles: Vec<Profile>,
    list_state: ListState,
    mode: Mode,
    input_buffer: String,
    search_query: String,
    filtered_indices: Vec<usize>,
    /// Per-slot [1m] suffix flags (opus, sonnet, haiku, model, subagent)
    lite_1m: [bool; 5],
    /// Current slot index in LiteModelSelect/LiteEdit:
    /// 0=name, 1=alias, 2=opus, 3=sonnet, 4=haiku, 5=model, 6=subagent, 7=extras
    lite_step: usize,
    /// Fetched models
    lite_models: Vec<String>,
    /// Model list pagination offset
    lite_model_page: usize,
    /// Profile id being edited (for LiteEdit)
    lite_edit_id: String,
    /// Collected values
    lite_name: String,
    lite_alias: String,
    lite_token: String,
    lite_url: String,
    lite_mod_opus: String,
    lite_mod_sonnet: String,
    lite_mod_haiku: String,
    lite_mod_model: String,
    lite_mod_subagent: String,
    lite_extras: Vec<String>,
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
            mode,
            input_buffer,
            search_query: String::new(),
            filtered_indices,
            lite_1m: [false; 5],
            lite_step: 0,
            lite_models: Vec::new(),
            lite_model_page: 0,
            lite_name: String::new(),
            lite_alias: String::new(),
            lite_token: String::new(),
            lite_url: "https://api.anthropic.com".to_string(),
            lite_edit_id: String::new(),
            lite_mod_opus: String::new(),
            lite_mod_sonnet: String::new(),
            lite_mod_haiku: String::new(),
            lite_mod_model: String::new(),
            lite_mod_subagent: String::new(),
            lite_extras: Vec::new(),
        })
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn refresh(&mut self) -> Result<()> {
        self.profiles = self.manager.list_profiles()?;
        self.apply_filter();
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            let idx = self.list_state.selected().unwrap_or(0);
            self.list_state
                .select(Some(idx.min(self.filtered_indices.len() - 1)));
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
                        || p.alias.as_deref().map(|a| a.to_lowercase().contains(&q)).unwrap_or(false)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            let sel = self.list_state.selected().unwrap_or(0);
            self.list_state
                .select(Some(sel.min(self.filtered_indices.len() - 1)));
        }
    }

    fn select_by_id(&mut self, id: &str) {
        if let Some(fi) = self
            .filtered_indices
            .iter()
            .position(|&i| self.profiles[i].id == id)
        {
            self.list_state.select(Some(fi));
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

    fn set_slot_value(&mut self, val: String) {
        match self.lite_step {
            2 => self.lite_mod_opus = val,
            3 => self.lite_mod_sonnet = val,
            4 => self.lite_mod_haiku = val,
            5 => self.lite_mod_model = val,
            6 => self.lite_mod_subagent = val,
            _ => {}
        }
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

            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
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
                        self.handle_add_full_name(key.code)?;
                    }
                    Mode::AddFullAlias => {
                        self.handle_add_full_alias(key.code)?;
                    }
                    Mode::LiteToken => {
                        self.handle_lite_token(key.code, key.modifiers)?;
                    }
                    Mode::LiteUrl => {
                        self.handle_lite_url(key.code, key.modifiers)?;
                    }
                    Mode::LiteFetching => {
                        if key.code == KeyCode::Esc {
                            self.mode = Mode::Normal;
                        }
                    }
                    Mode::LiteModelSelect { .. } => {
                        self.handle_lite_model_select(key.code, key.modifiers)?;
                    }
                    Mode::LiteEdit { .. } => {
                        self.handle_lite_model_select(key.code, key.modifiers)?;
                    }
                    Mode::EditProfile { .. } => {
                        self.handle_edit_profile(key.code)?;
                    }
                    Mode::Message(_, _) => {
                        self.mode = Mode::Normal;
                    }
                }
            }
        }
    }

    // ── Key handlers ──────────────────────────────────────────────────────────

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
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),

            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),

            KeyCode::Char('/') => {
                self.search_query.clear();
                self.apply_filter();
                self.mode = Mode::Search;
            }

            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }

            KeyCode::Enter => {
                if let Some(p) = self.selected_profile() {
                    let name = p.name.clone();
                    ratatui::restore();
                    println!("Launching Claude with profile '{}'…", name);
                    self.manager.launch_claude(&p.id, &[])?;
                }
            }

            KeyCode::Char('t') => {
                self.lite_token.clear();
                self.lite_url = "https://api.anthropic.com".to_string();
                self.lite_name.clear();
                self.lite_alias.clear();
                self.lite_step = 0;
                self.lite_models.clear();
                self.lite_model_page = 0;
                self.lite_1m = [false; 5];
                self.lite_extras.clear();
                self.lite_mod_opus.clear();
                self.lite_mod_sonnet.clear();
                self.lite_mod_haiku.clear();
                self.lite_mod_model.clear();
                self.lite_mod_subagent.clear();
                self.mode = Mode::LiteToken;
                self.input_buffer.clear();
            }

            KeyCode::Char('a') => {
                self.mode = Mode::AddFullName;
                self.input_buffer.clear();
            }

            KeyCode::Char('d') | KeyCode::Delete => {
                if self.selected_profile().is_some() {
                    self.mode = Mode::ConfirmDelete;
                }
            }

            KeyCode::Char('e') => {
                let profile = match self.selected_profile() {
                    Some(p) => p.clone(),
                    None => return Ok(false),
                };
                self.input_buffer = profile.name.clone();
                self.lite_name = profile.name.clone();
                self.lite_alias = profile.alias.clone().unwrap_or_default();
                self.mode = Mode::EditProfile {
                    profile_id: profile.id.clone(),
                    step: 0,
                };
            }

            KeyCode::Char('E') => {
                // Edit lightweight env vars (existing 'e' behavior for lite profiles)
                let profile = match self.selected_profile() {
                    Some(p) if p.kind == ProfileKind::Lightweight => p.clone(),
                    _ => return Ok(false),
                };
                if let Some(ref env) = profile.env {
                    self.lite_token = env.auth_token.clone().unwrap_or_default();
                    self.lite_url = env.base_url.clone().unwrap_or_else(|| "https://api.anthropic.com".to_string());
                    self.lite_mod_opus = env.default_opus_model.clone().unwrap_or_default();
                    self.lite_mod_sonnet = env.default_sonnet_model.clone().unwrap_or_default();
                    self.lite_mod_haiku = env.default_haiku_model.clone().unwrap_or_default();
                    self.lite_mod_model = env.model.clone().unwrap_or_default();
                    self.lite_mod_subagent = env.subagent_model.clone().unwrap_or_default();
                    let ends_1m: [&str; 5] = [&self.lite_mod_opus, &self.lite_mod_sonnet, &self.lite_mod_haiku, &self.lite_mod_model, &self.lite_mod_subagent];
                    for i in 0..5 { self.lite_1m[i] = ends_1m[i].ends_with("[1m]"); }
                    self.lite_name = profile.name.clone();
                    self.lite_alias = profile.alias.clone().unwrap_or_default();
                    self.lite_edit_id = profile.id.clone();
                    self.lite_step = 0;
                    self.lite_extras = env.extras.clone();
                    let token = self.lite_token.clone();
                    let base_url = self.lite_url.clone();
                    self.mode = Mode::LiteFetching;
                    match fetch_models(&base_url, &token) {
                        Ok(models) => {
                            self.lite_models = models;
                            self.mode = Mode::LiteEdit { profile_id: profile.id.clone() };
                        }
                        Err(_) => {
                            self.lite_models = Vec::new();
                            self.mode = Mode::LiteEdit { profile_id: profile.id.clone() };
                        }
                    }
                }
            }

            KeyCode::Char('m') => {
                if let Some(p) = self.selected_profile() {
                    if p.kind == ProfileKind::Lightweight {
                        for i in 0..5 { self.lite_1m[i] = !self.lite_1m[i]; }
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
                            self.mode = Mode::Message(format!("Profile '{}' refreshed.", name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
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
            KeyCode::Esc => {
                self.search_query.clear();
                self.apply_filter();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.apply_filter();
            }
            KeyCode::Up | KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_up();
            }
            KeyCode::Down | KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_down();
            }
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.apply_filter();
            }
            _ => {}
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
                            self.refresh()?;
                            self.mode = Mode::Message(format!("Profile '{}' removed.", name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }
            _ => self.mode = Mode::Normal,
        }
        Ok(())
    }

    fn handle_add_full_name(&mut self, code: KeyCode) -> Result<()> {
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
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_add_full_alias(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Enter => {
                let alias = self.input_buffer.trim().to_string();
                let alias_opt = if alias.is_empty() { None } else { Some(alias.as_str()) };
                let name = self.lite_name.clone();
                match self.manager.add_profile(&name, alias_opt) {
                    Ok(p) => {
                        self.refresh()?;
                        self.select_by_id(&p.id);
                        self.mode = Mode::Message(format!("Profile '{}' added.", name), false);
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    // ── Edit Profile (name/alias) ────────────────────────────────────────────

    fn handle_edit_profile(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let new_name = self.lite_name.trim().to_string();
                if new_name.is_empty() {
                    self.mode = Mode::Message("Profile name cannot be empty.".into(), true);
                    return Ok(());
                }
                let new_alias = self.lite_alias.trim().to_string();
                let alias_opt = if new_alias.is_empty() { None } else { Some(new_alias.as_str()) };
                let id = match &self.mode {
                    Mode::EditProfile { profile_id, .. } => profile_id.clone(),
                    _ => return Ok(()),
                };
                match self.manager.rename_profile(&id, &new_name, alias_opt) {
                    Ok(p) => {
                        self.refresh()?;
                        self.select_by_id(&p.id);
                        self.mode = Mode::Message(format!("Profile '{}' updated.", p.name), false);
                    }
                    Err(e) => self.mode = Mode::Message(e.to_string(), true),
                }
            }
            KeyCode::Backspace => {
                match self.mode {
                    Mode::EditProfile { step: 0, .. } => { self.lite_name.pop(); }
                    Mode::EditProfile { step: 1, .. } => { self.lite_alias.pop(); }
                    _ => {}
                }
            }
            KeyCode::Up | KeyCode::Char('p') if code == KeyCode::Char('p') => {
                // Ctrl+p navigation
                self.mode = match &self.mode {
                    Mode::EditProfile { profile_id, step } => Mode::EditProfile {
                        profile_id: profile_id.clone(),
                        step: if *step == 0 { 1 } else { 0 },
                    },
                    _ => return Ok(()),
                };
            }
            KeyCode::Down | KeyCode::Char('n') if code == KeyCode::Char('n') => {
                self.mode = match &self.mode {
                    Mode::EditProfile { profile_id, step } => Mode::EditProfile {
                        profile_id: profile_id.clone(),
                        step: (step + 1) % 2,
                    },
                    _ => return Ok(()),
                };
            }
            KeyCode::Tab => {
                // Toggle between name and alias
                self.mode = match &self.mode {
                    Mode::EditProfile { profile_id, step } => Mode::EditProfile {
                        profile_id: profile_id.clone(),
                        step: (step + 1) % 2,
                    },
                    _ => return Ok(()),
                };
            }
            KeyCode::Char(c) => {
                match &self.mode {
                    Mode::EditProfile { step: 0, .. } => { self.lite_name.push(c); }
                    Mode::EditProfile { step: 1, .. } => {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            self.lite_alias.push(c);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ── Lightweight profile key handlers ─────────────────────────────────────

    fn handle_lite_token(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let token = self.input_buffer.trim().to_string();
                if token.is_empty() {
                    return Ok(());
                }
                self.lite_token = token;
                self.input_buffer.clear();
                self.mode = Mode::LiteUrl;
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_lite_url(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let url = self.input_buffer.trim().to_string();
                self.lite_url = if url.is_empty() {
                    "https://api.anthropic.com".to_string()
                } else {
                    url
                };
                self.input_buffer.clear();
                self.mode = Mode::LiteFetching;
                match fetch_models(&self.lite_url, &self.lite_token) {
                    Ok(models) => {
                        self.lite_models = models;
                        self.lite_step = 0;
                        self.mode = Mode::LiteModelSelect {
                            profile_name: String::new(),
                            token: self.lite_token.clone(),
                            base_url: self.lite_url.clone(),
                            models: self.lite_models.clone(),
                        };
                    }
                    Err(_) => {
                        self.lite_models = Vec::new();
                        self.mode = Mode::LiteModelSelect {
                            profile_name: String::new(),
                            token: self.lite_token.clone(),
                            base_url: self.lite_url.clone(),
                            models: Vec::new(),
                        };
                    }
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_lite_model_select(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        let models_per_page: usize = 15;
        let is_edit = matches!(self.mode, Mode::LiteEdit { .. });
        // 8 steps: 0=name, 1=alias, 2-6=models, 7=extras
        let total_steps: usize = 8;

        match code {
            KeyCode::Esc => self.mode = Mode::Normal,

            // Slot navigation
            KeyCode::Down | KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.lite_step = (self.lite_step + 1) % total_steps;
            }
            KeyCode::Up | KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.lite_step = if self.lite_step == 0 { total_steps - 1 } else { self.lite_step - 1 };
            }
            KeyCode::Tab => {
                self.lite_step = (self.lite_step + 1) % total_steps;
            }

            // [1m] toggle (steps 2-6 are model slots)
            KeyCode::Char('m') if modifiers.contains(KeyModifiers::CONTROL) => {
                if self.lite_step >= 2 && self.lite_step <= 6 {
                    let idx = self.lite_step - 2;
                    self.lite_1m[idx] = !self.lite_1m[idx];
                }
            }

            // Alt+p/n: cycle through model candidates (steps 2-6)
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::ALT) => {
                if self.lite_step >= 2 && self.lite_step <= 6 && !self.lite_models.is_empty() {
                    let old = self.lite_step;
                    let current = self.current_slot_value();
                    if let Some(pos) = self.lite_models.iter().position(|m| m == &current) {
                        let prev = if pos == 0 { self.lite_models.len() - 1 } else { pos - 1 };
                        self.lite_step = old;
                        self.set_slot_value(self.lite_models[prev].clone());
                    } else if !current.is_empty() {
                        if let Some(m) = self.lite_models.iter().find(|m| m.contains(&current)) {
                            self.set_slot_value(m.clone());
                        }
                    }
                }
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::ALT) => {
                if self.lite_step >= 2 && self.lite_step <= 6 && !self.lite_models.is_empty() {
                    let old = self.lite_step;
                    let current = self.current_slot_value();
                    if let Some(pos) = self.lite_models.iter().position(|m| m == &current) {
                        let next = (pos + 1) % self.lite_models.len();
                        self.lite_step = old;
                        self.set_slot_value(self.lite_models[next].clone());
                    } else if !current.is_empty() {
                        if let Some(m) = self.lite_models.iter().find(|m| m.contains(&current)) {
                            self.set_slot_value(m.clone());
                        }
                    }
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
                let alias_opt = if alias.is_empty() { None } else { Some(alias.as_str()) };

                let apply = |s: &str, idx: usize| -> Option<String> {
                    if s.is_empty() {
                        None
                    } else {
                        let v = s.to_string();
                        if self.lite_1m[idx] && !v.ends_with("[1m]") {
                            Some(format!("{}[1m]", v))
                        } else {
                            Some(v)
                        }
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
                            self.refresh()?;
                            self.select_by_id(&p.id);
                            self.mode = Mode::Message(format!("Profile '{}' updated.", p.name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                } else {
                    match self.manager.create_lightweight_profile(&name, alias_opt, env) {
                        Ok(p) => {
                            self.refresh()?;
                            self.select_by_id(&p.id);
                            self.mode = Mode::Message(format!("Profile '{}' created.", p.name), false);
                        }
                        Err(e) => self.mode = Mode::Message(e.to_string(), true),
                    }
                }
            }

            // Backspace
            KeyCode::Backspace => {
                match self.lite_step {
                    0 => { self.lite_name.pop(); }
                    1 => { self.lite_alias.pop(); }
                    2 => { self.lite_mod_opus.pop(); }
                    3 => { self.lite_mod_sonnet.pop(); }
                    4 => { self.lite_mod_haiku.pop(); }
                    5 => { self.lite_mod_model.pop(); }
                    6 => { self.lite_mod_subagent.pop(); }
                    7 => {
                        if !self.input_buffer.is_empty() {
                            self.input_buffer.pop();
                        } else if !self.lite_extras.is_empty() {
                            self.lite_extras.pop();
                        }
                    }
                    _ => {}
                }
            }

            // Typing
            KeyCode::Char(c) => {
                match self.lite_step {
                    0 => { self.lite_name.push(c); }
                    1 => {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            self.lite_alias.push(c);
                        }
                    }
                    2 => { self.lite_mod_opus.push(c); }
                    3 => { self.lite_mod_sonnet.push(c); }
                    4 => { self.lite_mod_haiku.push(c); }
                    5 => { self.lite_mod_model.push(c); }
                    6 => { self.lite_mod_subagent.push(c); }
                    7 => { self.input_buffer.push(c); }
                    _ => {}
                }
            }
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

        self.render_profile_list(f, cols[0]);
        self.render_detail_panel(f, cols[1]);
        self.render_footer(f, layout[2]);

        // Overlays
        match &self.mode.clone() {
            Mode::Help => self.render_help(f),
            Mode::ConfirmDelete => self.render_confirm_delete_popup(f),
            Mode::AddFullName => self.render_add_name_popup(f),
            Mode::AddFullAlias => self.render_add_alias_popup(f),
            Mode::EditProfile { step, .. } => self.render_edit_profile_popup(f, *step),
            Mode::LiteToken => self.render_lite_token_popup(f),
            Mode::LiteUrl => self.render_lite_url_popup(f),
            Mode::LiteFetching => self.render_lite_fetching_popup(f),
            Mode::LiteModelSelect { .. } | Mode::LiteEdit { .. } => {
                self.render_lite_model_select_popup(f)
            }
            Mode::Message(msg, is_err) => self.render_message(f, msg, *is_err),
            _ => {}
        }
    }

    // ── First-run screen ──────────────────────────────────────────────────────

    fn render_first_run(&self, f: &mut Frame, area: Rect) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
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
                "  Welcome! Press 't' for lightweight (env vars) or 'a' for full (directory) profile.",
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
                Span::styled("  profile manager", Style::default().fg(DIM)),
            ]))
            .block(block),
            area,
        );

        let count = self.filtered_indices.len();
        let total = self.profiles.len();
        let label = if count == total {
            format!(" {} profile{} ", total, if total == 1 { "" } else { "s" })
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
                Span::styled(self.search_query.clone(), Style::default().fg(SEARCH_HL).bold()),
                Span::styled("█ ", Style::default().fg(SEARCH_HL)),
            ])
        } else if !self.search_query.is_empty() {
            Line::from(vec![
                Span::styled(" Search: ", Style::default().fg(DIM)),
                Span::styled(self.search_query.clone(), Style::default().fg(SEARCH_HL)),
            ])
        } else {
            Line::from(Span::styled(" Profiles ", Style::default().fg(ACCENT).bold()))
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
    }

    fn render_detail_panel(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(" Details ", Style::default().fg(ACCENT).bold())))
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
                Span::styled(profile_dir.display().to_string(), Style::default().fg(MUTED)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ─────────────────────────────────────────",
                Style::default().fg(BORDER),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Launch command", Style::default().fg(DIM))));
            lines.push(Line::from(Span::styled(
                if cfg!(target_os = "windows") {
                    format!("  $env:CLAUDE_CONFIG_DIR='{}'; claude", profile_dir.display())
                } else {
                    format!("  CLAUDE_CONFIG_DIR='{}' claude", profile_dir.display())
                },
                Style::default().fg(Color::Rgb(140, 200, 140)),
            )));
        } else if let Some(ref env) = profile.env {
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
            if let Some(ref m) = env.default_opus_model { lines.push(Line::from(vec![Span::styled("  Opus model   ", Style::default().fg(DIM)), Span::styled(m, Style::default().fg(TEXT))])); }
            if let Some(ref m) = env.default_sonnet_model { lines.push(Line::from(vec![Span::styled("  Sonnet model ", Style::default().fg(DIM)), Span::styled(m, Style::default().fg(TEXT))])); }
            if let Some(ref m) = env.default_haiku_model { lines.push(Line::from(vec![Span::styled("  Haiku model  ", Style::default().fg(DIM)), Span::styled(m, Style::default().fg(TEXT))])); }
            if let Some(ref m) = env.model { lines.push(Line::from(vec![Span::styled("  Model        ", Style::default().fg(DIM)), Span::styled(m, Style::default().fg(TEXT))])); }
            if let Some(ref m) = env.subagent_model { lines.push(Line::from(vec![Span::styled("  Subagent     ", Style::default().fg(DIM)), Span::styled(m, Style::default().fg(TEXT))])); }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ─── Options ─────────────────────────────",
                Style::default().fg(BORDER),
            )));
            lines.push(Line::from(""));
            let any_mark = if self.lite_1m.iter().any(|&x| x) { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", any_mark), Style::default().fg(ACCENT).bold()),
                Span::styled("Press 'm' to toggle [1m] on/off for all slots", Style::default().fg(TEXT)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Launch", Style::default().fg(DIM))));
        }

        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let keys: Vec<(&str, &str)> = if self.mode == Mode::Search {
            vec![
                ("↑/↓", "navigate"),
                ("enter", "confirm"),
                ("esc", "clear"),
            ]
        } else {
            vec![
                ("↑↓/jk", "nav"),
                ("enter", "launch"),
                ("/", "search"),
                ("t", "lite"),
                ("a", "add"),
                ("e/E", "edit"),
                ("m", "[1m]"),
                ("r", "refresh"),
                ("d", "delete"),
                ("?", "help"),
                ("q", "quit"),
            ]
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

        f.render_widget(
            Paragraph::new(Line::from(spans)).block(block),
            area,
        );
    }

    // ── Overlay popups ────────────────────────────────────────────────────────

    fn render_help(&self, f: &mut Frame) {
        let area = centered_rect(65, 25, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(" Help — Keybindings ", Style::default().fg(ACCENT).bold())))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let help_entries: Vec<(&str, &str)> = vec![
            ("↑/↓  j/k", "Navigate profiles"),
            ("Enter", "Launch Claude with selected profile"),
            ("/", "Search profiles by name or alias"),
            ("t", "Add lightweight (env-var based) profile"),
            ("a", "Add full (directory-isolated) profile"),
            ("e", "Edit profile name / alias"),
            ("E", "Edit lightweight env vars (lite only)"),
            ("m", "Toggle [1m] suffix (lightweight profiles)"),
            ("r", "Refresh — re-copy ~/.claude into selected"),
            ("d / Del", "Delete selected profile"),
            ("?", "Toggle this help dialog"),
            ("q / Esc", "Quit"),
        ];

        let mut lines: Vec<Line> = vec![Line::from("")];

        for (key, desc) in &help_entries {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", key), Style::default().fg(ACCENT).bold()),
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
            .title(Line::from(Span::styled(" Confirm Delete ", Style::default().fg(DANGER).bold())))
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
                    Span::styled(self.input_buffer.clone(), Style::default().fg(TEXT).bold()),
                    Span::styled("█", Style::default().fg(ACCENT)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  Any characters allowed. Enter to continue, Esc to cancel.",
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
                        if self.input_buffer.is_empty() { "(none)".to_string() } else { format!("{}█", self.input_buffer) },
                        Style::default().fg(TEXT).bold(),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  Short CLI-friendly name (a-z, 0-9, -, _). Enter to skip.",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    fn render_edit_profile_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(60, 9, f.area());
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

        let name_cursor = if step == 0 { "█" } else { "" };
        let alias_cursor = if step == 1 { "█" } else { "" };
        let alias_display = if self.lite_alias.is_empty() && step != 1 { "(none)" } else { &self.lite_alias };

        let name_prefix = if step == 0 { "▶ " } else { "  " };
        let alias_prefix = if step == 1 { "▶ " } else { "  " };

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(name_prefix, Style::default().fg(ACCENT).bold()),
                    Span::styled("Name:  ", Style::default().fg(DIM)),
                    Span::styled(format!("{}{}", self.lite_name, name_cursor), Style::default().fg(TEXT).bold()),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(alias_prefix, Style::default().fg(ACCENT).bold()),
                    Span::styled("Alias: ", Style::default().fg(DIM)),
                    Span::styled(format!("{}{}", alias_display, alias_cursor), Style::default().fg(Color::Rgb(140, 200, 140))),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "  ↑/↓ to switch fields, Enter to save, Esc to cancel",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    // ── Lightweight profile popups ────────────────────────────────────────────

    fn render_lite_token_popup(&self, f: &mut Frame) {
        let area = centered_rect(60, 8, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(" Lightweight Profile — Token ", Style::default().fg(ACCENT).bold())))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let display = if self.input_buffer.is_empty() { "█".to_string() } else { format!("{}█", self.input_buffer) };
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![Span::styled("  Token: ", Style::default().fg(DIM)), Span::styled(display, Style::default().fg(TEXT).bold())]),
                Line::from(""),
                Line::from(Span::styled("  Enter your ANTHROPIC_AUTH_TOKEN value.", Style::default().fg(DIM))),
            ])).block(block),
            area,
        );
    }

    fn render_lite_url_popup(&self, f: &mut Frame) {
        let area = centered_rect(60, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(" Lightweight Profile — Base URL ", Style::default().fg(ACCENT).bold())))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let val = if self.input_buffer.is_empty() { "https://api.anthropic.com".to_string() } else { format!("{}█", self.input_buffer) };
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![Span::styled("  URL: ", Style::default().fg(DIM)), Span::styled(val, Style::default().fg(TEXT).bold())]),
                Line::from(""),
                Line::from(Span::styled("  Enter ANTHROPIC_BASE_URL (or Enter for default).", Style::default().fg(DIM))),
            ])).block(block),
            area,
        );
    }

    fn render_lite_fetching_popup(&self, f: &mut Frame) {
        let area = centered_rect(50, 6, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(" Fetching Models ", Style::default().fg(ACCENT).bold())))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled("  Connecting to /v1/models...", Style::default().fg(TEXT))),
                Line::from(Span::styled("  Press Esc to skip", Style::default().fg(DIM))),
            ])).block(block),
            area,
        );
    }

    fn render_lite_model_select_popup(&self, f: &mut Frame) {
        let area = centered_rect(90, 37, f.area());
        f.render_widget(Clear, area);
        let is_edit = matches!(self.mode, Mode::LiteEdit { .. });
        let title = if is_edit { " Edit Profile — Model Selection " } else { " Lite Profile — Model Selection " };
        let block = Block::default()
            .title(Line::from(Span::styled(title, Style::default().fg(ACCENT).bold())))
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
            let page_models: Vec<&str> = self.lite_models.iter().skip(page_start).take(models_per_page).map(|s| s.as_str()).collect();
            let page_end = page_start + page_models.len();
            let page_info = if total > models_per_page {
                format!("  Models ({}-{} of {}):", page_start + 1, page_end, total)
            } else {
                "  Available models:".to_string()
            };
            lines.push(Line::from(Span::styled(page_info, Style::default().fg(DIM))));
            for (i, m) in page_models.iter().enumerate() {
                let idx = page_start + i + 1;
                lines.push(Line::from(Span::styled(format!("{:>4}. {}", idx, m), Style::default().fg(Color::Rgb(140, 200, 140)))));
            }
            if total > models_per_page {
                lines.push(Line::from(Span::styled("     PgUp/PgDn scroll", Style::default().fg(Color::Rgb(80, 120, 80)))));
            }
        } else {
            lines.push(Line::from(Span::styled("  No models (type manually or use Alt+p/Alt+n to cycle)", Style::default().fg(DIM))));
        }
        lines.push(Line::from(Span::styled("  ───────────────────────────────────────────────────────────────────", Style::default().fg(BORDER))));

        // Step 0: Profile name (any characters)
        let nf = if self.lite_step == 0 { "▶ " } else { "  " };
        let nd = if self.lite_step == 0 { format!("{}█", self.lite_name) } else { self.lite_name.clone() };
        lines.push(Line::from(vec![
            Span::styled(nf, Style::default().fg(ACCENT).bold()),
            Span::styled("Name      ", Style::default().fg(DIM)),
            Span::styled(nd, Style::default().fg(Color::Rgb(200, 200, 120)).bold()),
        ]));

        // Step 1: Alias (alphanumeric only)
        let af = if self.lite_step == 1 { "▶ " } else { "  " };
        let ad = if self.lite_step == 1 { format!("{}█", self.lite_alias) } else { self.lite_alias.clone() };
        let ad_display = if ad.is_empty() && self.lite_step != 1 { "(none)".to_string() } else { ad };
        lines.push(Line::from(vec![
            Span::styled(af, Style::default().fg(ACCENT).bold()),
            Span::styled("Alias     ", Style::default().fg(DIM)),
            Span::styled(ad_display, Style::default().fg(Color::Rgb(140, 200, 140))),
        ]));

        // Model slots (steps 2-6)
        let slots = [
            ("Opus", 0, 2), ("Sonnet", 1, 3), ("Haiku", 2, 4), ("Model", 3, 5), ("Subagent", 4, 6)
        ];
        for (label, idx1m, step) in slots.iter() {
            let prefix = if *step == self.lite_step { "▶ " } else { "  " };
            let cursor = if *step == self.lite_step { "█" } else { "" };
            let val = match *step {
                2 => &self.lite_mod_opus,
                3 => &self.lite_mod_sonnet,
                4 => &self.lite_mod_haiku,
                5 => &self.lite_mod_model,
                6 => &self.lite_mod_subagent,
                _ => unreachable!(),
            };
            let display = format!("{}{}", val, cursor);
            let ck = if self.lite_1m[*idx1m] { "1m✓" } else { "1m " };
            let hint = if !val.is_empty() && !self.lite_models.is_empty() {
                if let Some(m) = self.lite_models.iter().find(|m| m.contains(val.as_str())) {
                    if m != val { format!(" ↩{}", m) } else { String::new() }
                } else { String::new() }
            } else { String::new() };

            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(ACCENT).bold()),
                Span::styled(format!("{:<10}", label), Style::default().fg(DIM)),
                Span::styled(format!("{:<36}", display), Style::default().fg(TEXT).bold()),
                Span::styled(ck, Style::default().fg(ACCENT).bold()),
                Span::styled(hint, Style::default().fg(Color::Rgb(100, 130, 100))),
            ]));
        }

        // Extras section (step 7)
        lines.push(Line::from(Span::styled("  ───────────────────────────────────────────────────────────────────", Style::default().fg(BORDER))));
        let extras_focus = self.lite_step == 7;
        let ex_prefix = if extras_focus { "▶" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", ex_prefix), Style::default().fg(ACCENT).bold()),
            Span::styled("Extras", Style::default().fg(DIM)),
            Span::styled(" (enter KEY=VALUE per line)", Style::default().fg(Color::Rgb(120, 120, 130))),
        ]));
        // Show a curated subset of commonly used env vars as hints
        let hint_vars = [
            "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY",
            "ANTHROPIC_MODEL", "CLAUDE_CODE_SUBAGENT_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_BETAS", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX",
            "API_TIMEOUT_MS", "MAX_THINKING_TOKENS", "CLAUDE_CONFIG_DIR",
        ];
        let total_known = crate::env_vars::all_var_names().len();
        lines.push(Line::from(Span::styled(
            format!("  Known env vars ({} total; see https://code.claude.com/docs/en/env-vars):", total_known),
            Style::default().fg(Color::Rgb(80, 100, 110)),
        )));
        lines.push(Line::from(Span::styled(
            format!("  {}", hint_vars.join("  ")),
            Style::default().fg(Color::Rgb(70, 80, 90)),
        )));

        for extra in &self.lite_extras {
            lines.push(Line::from(Span::styled(format!("  {}", extra), Style::default().fg(Color::Rgb(160, 200, 160)))));
        }
        if extras_focus {
            let buf = if self.input_buffer.is_empty() { "█".to_string() } else { format!("{}█", self.input_buffer) };
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(buf, Style::default().fg(TEXT).bold()),
            ]));
            lines.push(Line::from(Span::styled("  Enter to add, Backspace to remove last entry", Style::default().fg(DIM))));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ↑↓/", Style::default().fg(DIM)),
            Span::styled("Cp/Cn", Style::default().fg(ACCENT).bold()),
            Span::styled(" nav  ", Style::default().fg(DIM)),
            Span::styled("Tab", Style::default().fg(ACCENT).bold()),
            Span::styled(" next  ", Style::default().fg(DIM)),
            Span::styled("Cm", Style::default().fg(ACCENT).bold()),
            Span::styled(" 1m  ", Style::default().fg(DIM)),
            Span::styled("Ap/An", Style::default().fg(ACCENT).bold()),
            Span::styled(" cycle  ", Style::default().fg(DIM)),
            Span::styled("Enter", Style::default().fg(ACCENT).bold()),
            Span::styled(" save", Style::default().fg(DIM)),
        ]));

        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_message(&self, f: &mut Frame, msg: &str, is_err: bool) {
        let area = centered_rect(60, 6, f.area());
        f.render_widget(Clear, area);

        let color = if is_err { DANGER } else { SUCCESS };
        let title = if is_err { " Error " } else { " Done " };

        let block = Block::default()
            .title(Line::from(Span::styled(title, Style::default().fg(color).bold())))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled(format!("  {}", msg), Style::default().fg(TEXT))),
                Line::from(""),
                Line::from(Span::styled("  Press any key to continue", Style::default().fg(DIM))),
            ]))
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * percent_x / 100;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width: w,
        height: height.min(area.height),
    }
}