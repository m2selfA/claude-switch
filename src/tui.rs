use anyhow::{Result, bail};
use arboard::Clipboard;
#[cfg(test)]
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::profile::{
    LightweightEnv, McpServer, ModelDiscoverySuccess, Profile, ProfileKind, ProfileManager,
    Provider, ProviderKey, RuntimeSessionInfo, discover_models, discover_models_with_timeout,
    fetch_models, test_anthropic_message, test_anthropic_message_with_timeout,
};
pub(super) use crate::profile::{McpServerInput, McpServerUpdate};
mod chrome;
mod editing;
mod helpers;
mod lite;
mod lite_actions;
mod lite_rendering;
mod lite_utils;
mod mcp_actions;
mod mcp_rendering;
mod process_switch;
mod process_switch_rendering;
mod profile_actions;
mod profile_rendering;
mod provider_import;
mod provider_key_actions;
mod provider_key_rendering;
mod provider_management;
mod provider_rendering;
mod provider_test;
mod provider_test_rendering;
mod public_site_model;
mod public_site_rendering;
mod public_site_runtime;
mod rendering;
mod runtime;
mod smart_paste;
mod state;
use editing::{
    display_with_cursor, emacs_edit, insert_filtered_str_at_cursor, insert_str_at_cursor,
    is_alias_char, provider_add_cursor_pos, provider_edit_cursor_pos, provider_key_cursor_pos,
    visible_window,
};
use helpers::*;
use lite_utils::*;
use provider_test::*;
use public_site_model::*;
pub(super) use ratatui::layout::Rect;
use smart_paste::{SmartProviderPaste, inferred_provider_name, parse_provider_smart_paste};
pub use state::App;
use state::*;
pub(super) use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

#[cfg(test)]
mod tests;
