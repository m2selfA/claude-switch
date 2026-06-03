use super::*;

impl App {
    pub(super) fn render_provider_list_page(&mut self, f: &mut Frame, area: Rect) {
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

    pub(super) fn render_provider_detail_page(&self, f: &mut Frame, area: Rect) {
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

    pub(super) fn render_provider_list_popup(&mut self, f: &mut Frame) {
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

    pub(super) fn render_provider_add_popup(&self, f: &mut Frame, step: usize) {
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

    pub(super) fn render_provider_smart_paste_popup(&self, f: &mut Frame) {
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

    pub(super) fn render_provider_edit_popup(&mut self, f: &mut Frame) {
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
                "  r=rename  e=edit token  a=add  d=delete  Ctrl+P/N=nav  Enter=save provider",
                Style::default().fg(DIM),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  Ctrl+P/N fields  Tab next  Enter next/save  Tab to Keys for key rename",
                Style::default().fg(DIM),
            )));
        }
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    pub(super) fn render_confirm_delete_provider_popup(&self, f: &mut Frame) {
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
}
