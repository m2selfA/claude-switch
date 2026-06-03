use super::*;

pub(super) fn render_lite_provider_select_popup(app: &mut App, f: &mut Frame) {
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

    if app.providers_cache.is_empty() {
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

    let items: Vec<ListItem> = app
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
    f.render_stateful_widget(list, list_area, &mut app.provider_list_state);

    let count = app.providers_cache.len();
    if count > 1 {
        let selected = app.provider_list_state.selected().unwrap_or(0);
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

pub(super) fn render_lite_key_select_popup(app: &App, f: &mut Frame) {
    let key_count = app.provider_keys_cache.len().min(8);
    let height = 8 + key_count as u16;
    let area = centered_rect(70, height, f.area());
    f.render_widget(Clear, area);

    let provider_name = app
        .lite_provider_id
        .as_ref()
        .and_then(|pid| app.providers_cache.iter().find(|p| p.id == *pid))
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
    if app.provider_keys_cache.is_empty() {
        lines.push(Line::from(Span::styled(
            "  This provider has no keys.",
            Style::default().fg(DIM),
        )));
    } else {
        let visible = 8usize;
        let selected = app
            .provider_key_selected
            .min(app.provider_keys_cache.len().saturating_sub(1));
        let start = selected.saturating_sub(visible.saturating_sub(1));
        for (i, key) in app
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
        if app.provider_keys_cache.len() > visible {
            lines.push(Line::from(Span::styled(
                format!(
                    "  showing {}-{} of {}",
                    start + 1,
                    (start + visible).min(app.provider_keys_cache.len()),
                    app.provider_keys_cache.len()
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

pub(super) fn render_lite_fetching_popup(_app: &App, f: &mut Frame) {
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

impl App {
    pub(super) fn render_lite_model_select_popup(&self, f: &mut Frame) {
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
            "CLAUDE_SWITCH_TINYFISH",
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
        lines.push(Line::from(Span::styled(
            "  cswitch-only control: CLAUDE_SWITCH_TINYFISH=off disables TinyFish injection for this profile",
            Style::default().fg(Color::Rgb(95, 105, 115)),
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
}
