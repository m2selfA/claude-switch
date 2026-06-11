use super::*;

impl App {
    pub(super) fn render_profile_list(&mut self, f: &mut Frame, area: Rect) {
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

    pub(super) fn render_detail_panel(&self, f: &mut Frame, area: Rect) {
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
        } else {
            if let Some(ref env) = profile.env {
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
            }

            if let Some(mcp_lines) = self.profile_mcp_section_lines(profile) {
                lines.extend(mcp_lines);
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

    pub(super) fn profile_mcp_section_lines(&self, profile: &Profile) -> Option<Vec<Line<'_>>> {
        if profile.kind != ProfileKind::Lightweight || profile.mcp_server_ids.is_empty() {
            return None;
        }

        let mut mcp_names = self
            .manager
            .list_mcp_servers()
            .unwrap_or_default()
            .into_iter()
            .filter(|mcp| profile.mcp_server_ids.iter().any(|id| id == &mcp.id))
            .map(|mcp| mcp.name)
            .collect::<Vec<_>>();
        mcp_names.sort();
        if mcp_names.is_empty() {
            return None;
        }

        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ─── MCP Servers ───────────────────────",
                Style::default().fg(BORDER),
            )),
        ];
        for name in mcp_names {
            lines.push(Line::from(vec![
                Span::styled("  MCP          ", Style::default().fg(DIM)),
                Span::styled(name, Style::default().fg(ACCENT)),
            ]));
        }
        Some(lines)
    }

    pub(super) fn render_add_name_popup(&self, f: &mut Frame) {
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

    pub(super) fn render_add_alias_popup(&self, f: &mut Frame) {
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

    pub(super) fn render_duplicate_name_popup(&self, f: &mut Frame) {
        let area = centered_rect(56, 7, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Duplicate Profile — Name ",
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

    pub(super) fn render_duplicate_alias_popup(&self, f: &mut Frame) {
        let area = centered_rect(56, 8, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Duplicate Profile — Alias ",
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
                    "  Short CLI-friendly name (a-z, 0-9, -, _). Clear it to save without an alias.",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }

    pub(super) fn render_edit_profile_popup(&self, f: &mut Frame, step: usize) {
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
}
