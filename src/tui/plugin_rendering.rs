use super::*;

impl App {
    pub(super) fn render_plugin_list_page(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Hosted Plugins ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        if self.plugins_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No hosted plugins yet. Press 'a' to install from configured marketplaces.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .plugins_cache
            .iter()
            .map(|plugin| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(" ", Style::default()),
                        Span::styled(
                            display_pad(&plugin.plugin_name, 24),
                            Style::default().fg(TEXT).bold(),
                        ),
                        Span::styled(
                            format!(
                                " {}",
                                if plugin.explicit {
                                    "explicit"
                                } else {
                                    "dependency"
                                }
                            ),
                            Style::default().fg(DIM),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("  id: ", Style::default().fg(MUTED)),
                        Span::styled(display_pad(&plugin.id, 28), Style::default().fg(MUTED)),
                        Span::styled(
                            plugin.version.as_deref().unwrap_or("—"),
                            Style::default().fg(DIM),
                        ),
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
        f.render_stateful_widget(list, area, &mut self.plugin_list_state);
    }

    pub(super) fn render_plugin_detail_page(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Plugin Detail ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(plugin) = self.selected_plugin() else {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No hosted plugin selected.",
                    Style::default().fg(DIM),
                ))),
                inner,
            );
            return;
        };

        let mut lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Name         ", Style::default().fg(DIM)),
                Span::styled(
                    plugin.plugin_name.clone(),
                    Style::default().fg(ACCENT).bold(),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ID           ", Style::default().fg(DIM)),
                Span::styled(plugin.id.clone(), Style::default().fg(MUTED)),
            ]),
            Line::from(vec![
                Span::styled("  Marketplace  ", Style::default().fg(DIM)),
                Span::styled(plugin.marketplace_name.clone(), Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Version      ", Style::default().fg(DIM)),
                Span::styled(
                    plugin.version.as_deref().unwrap_or("—"),
                    Style::default().fg(TEXT),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Type         ", Style::default().fg(DIM)),
                Span::styled(
                    if plugin.explicit {
                        "explicit"
                    } else {
                        "dependency"
                    },
                    Style::default().fg(TEXT),
                ),
            ]),
        ];
        if let Some(url) = &plugin.source_url {
            lines.push(Line::from(vec![
                Span::styled("  Source URL   ", Style::default().fg(DIM)),
                Span::styled(display_ellipsize(url, 40), Style::default().fg(TEXT)),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("  Dependencies ", Style::default().fg(DIM)),
            Span::styled(
                plugin.dependencies.len().to_string(),
                Style::default().fg(TEXT),
            ),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Linked profiles",
            Style::default().fg(DIM),
        )));
        if self.plugin_profile_links_cache.is_empty() {
            lines.push(Line::from(Span::styled(
                "    none",
                Style::default().fg(MUTED),
            )));
        } else {
            for profile in self.plugin_profile_links_cache.iter().take(8) {
                lines.push(Line::from(vec![
                    Span::styled("    ", Style::default()),
                    Span::styled(profile.name.clone(), Style::default().fg(TEXT)),
                ]));
            }
        }

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    pub(super) fn render_plugin_install_picker_popup(&self, f: &mut Frame) {
        let area = centered_rect(84, 24, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Install Hosted Plugin ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let filtered = self.filtered_plugin_catalog_indices();
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Filter: ", Style::default().fg(DIM)),
                Span::styled(
                    display_with_cursor(&self.plugin_filter_buf, self.cursor_pos),
                    Style::default().fg(TEXT).bold(),
                ),
            ]),
            Line::from(""),
        ];
        for idx in filtered.into_iter().take(14) {
            let plugin = &self.plugin_catalog_cache[idx];
            let cursor = self.plugin_list_state.selected() == Some(idx);
            let prefix = if cursor { "▶" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", prefix), Style::default().fg(ACCENT)),
                Span::styled(
                    format!(" {}", display_pad(&plugin.id, 32)),
                    Style::default().fg(TEXT),
                ),
                Span::styled(
                    plugin.version.as_deref().unwrap_or(""),
                    Style::default().fg(DIM),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter install  Tab complete filter  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(super) fn render_plugin_profile_picker_popup(&self, f: &mut Frame) {
        let area = centered_rect(84, 24, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Select Hosted Plugins ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let q = self.plugin_filter_buf.to_lowercase();
        let filtered = self
            .plugins_cache
            .iter()
            .enumerate()
            .filter(|(_, plugin)| {
                q.is_empty()
                    || plugin.id.to_lowercase().contains(&q)
                    || plugin.plugin_name.to_lowercase().contains(&q)
            })
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Filter: ", Style::default().fg(DIM)),
                Span::styled(
                    display_with_cursor(&self.plugin_filter_buf, self.cursor_pos),
                    Style::default().fg(TEXT).bold(),
                ),
            ]),
            Line::from(""),
        ];
        for idx in filtered.into_iter().take(14) {
            let plugin = &self.plugins_cache[idx];
            let selected = self.plugin_selected_ids.iter().any(|id| id == &plugin.id);
            let cursor = self.plugin_list_state.selected() == Some(idx);
            let marker = if selected { "[x]" } else { "[ ]" };
            let prefix = if cursor { "▶" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} {}", prefix, marker),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    format!(" {}", display_pad(&plugin.id, 34)),
                    Style::default().fg(TEXT),
                ),
                Span::styled(
                    plugin.version.as_deref().unwrap_or(""),
                    Style::default().fg(DIM),
                ),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Space toggle  Enter save  Tab complete filter  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }
}
