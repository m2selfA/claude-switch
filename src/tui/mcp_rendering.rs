use super::*;

impl App {
    pub(super) fn render_mcp_list_page(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " MCP Servers ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        if self.mcps_cache.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No MCP servers yet. Press 'a' to add or Ctrl+Y to import.",
                    Style::default().fg(DIM),
                )))
                .block(block),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .mcps_cache
            .iter()
            .map(|mcp| {
                let target = mcp.command.as_deref().or(mcp.url.as_deref()).unwrap_or("");
                let disabled = if mcp.disabled.unwrap_or(false) {
                    " disabled"
                } else {
                    ""
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(" ", Style::default()),
                        Span::styled(display_pad(&mcp.name, 24), Style::default().fg(TEXT).bold()),
                        Span::styled(
                            format!(" {}", display_pad(&mcp.server_type, 15)),
                            Style::default().fg(DIM),
                        ),
                        Span::styled(disabled, Style::default().fg(DANGER)),
                    ]),
                    Line::from(vec![
                        Span::styled("  id: ", Style::default().fg(MUTED)),
                        Span::styled(display_pad(&mcp.id, 14), Style::default().fg(MUTED)),
                        Span::styled(display_ellipsize(target, 36), Style::default().fg(DIM)),
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
        f.render_stateful_widget(list, area, &mut self.mcp_list_state);

        let count = self.mcps_cache.len();
        if count > 1 {
            let selected = self.mcp_list_state.selected().unwrap_or(0);
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(ACCENT))
                .track_style(Style::default().fg(BORDER));
            let mut sb = ScrollbarState::new(count).position(selected);
            f.render_stateful_widget(scrollbar, area, &mut sb);
        }
    }

    pub(super) fn render_mcp_detail_page(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(Line::from(Span::styled(
                " MCP Detail ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(mcp) = self.selected_mcp() else {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  No MCP selected.",
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
                Span::styled(mcp.name.clone(), Style::default().fg(ACCENT).bold()),
            ]),
            Line::from(vec![
                Span::styled("  ID           ", Style::default().fg(DIM)),
                Span::styled(mcp.id.clone(), Style::default().fg(MUTED)),
            ]),
            Line::from(vec![
                Span::styled("  Type         ", Style::default().fg(DIM)),
                Span::styled(mcp.server_type.clone(), Style::default().fg(TEXT)),
            ]),
        ];
        if let Some(command) = &mcp.command {
            lines.push(Line::from(vec![
                Span::styled("  Command      ", Style::default().fg(DIM)),
                Span::styled(command.clone(), Style::default().fg(TEXT)),
            ]));
        }
        if let Some(url) = &mcp.url {
            lines.push(Line::from(vec![
                Span::styled("  URL          ", Style::default().fg(DIM)),
                Span::styled(url.clone(), Style::default().fg(TEXT)),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("  Args         ", Style::default().fg(DIM)),
            Span::styled(mcp.args.len().to_string(), Style::default().fg(TEXT)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Env          ", Style::default().fg(DIM)),
            Span::styled(mcp.env.len().to_string(), Style::default().fg(TEXT)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Headers      ", Style::default().fg(DIM)),
            Span::styled(mcp.headers.len().to_string(), Style::default().fg(TEXT)),
        ]));
        if let Some(timeout) = mcp.timeout {
            lines.push(Line::from(vec![
                Span::styled("  Timeout      ", Style::default().fg(DIM)),
                Span::styled(timeout.to_string(), Style::default().fg(TEXT)),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("  Always load  ", Style::default().fg(DIM)),
            Span::styled(
                optional_bool_label(mcp.always_load),
                Style::default().fg(TEXT),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Disabled     ", Style::default().fg(DIM)),
            Span::styled(optional_bool_label(mcp.disabled), Style::default().fg(TEXT)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Linked lightweight profiles",
            Style::default().fg(DIM),
        )));
        for profile in self.mcp_profile_links_cache.iter().take(8) {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(profile.name.clone(), Style::default().fg(TEXT)),
            ]));
        }
        if self.mcp_profile_links_cache.is_empty() {
            lines.push(Line::from(Span::styled(
                "    none",
                Style::default().fg(MUTED),
            )));
        }

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    pub(super) fn render_mcp_editor_popup(&self, f: &mut Frame, step: usize) {
        let area = centered_rect(86, 34, f.area());
        f.render_widget(Clear, area);
        let is_edit = matches!(self.mode, Mode::McpEdit { .. });
        let title = if is_edit {
            " Edit MCP Server "
        } else {
            " Add MCP Server "
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

        let mut lines = vec![Line::from("")];
        let field_line = |current_step: usize, label: &str, value: String| -> Line {
            let prefix = if step == current_step { "▶ " } else { "  " };
            let style = if step == current_step {
                Style::default().fg(TEXT).bold()
            } else {
                Style::default().fg(MUTED)
            };
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(ACCENT).bold()),
                Span::styled(display_pad(label, 15), Style::default().fg(DIM)),
                Span::styled(value, style),
            ])
        };

        lines.push(field_line(
            0,
            "Name",
            if step == 0 {
                display_with_cursor(&self.mcp_name_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_name_buf)
            },
        ));
        lines.push(field_line(
            1,
            "Type",
            MCP_TYPES[self.mcp_type_idx % MCP_TYPES.len()].to_string(),
        ));
        lines.push(field_line(
            2,
            "Command",
            if step == 2 {
                display_with_cursor(&self.mcp_command_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_command_buf)
            },
        ));
        lines.push(field_line(
            3,
            "Args",
            format!("{} entrie(s)", self.mcp_args.len()),
        ));
        if step == 3 {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    display_with_cursor(&self.input_buffer, self.cursor_pos)
                ),
                Style::default().fg(TEXT),
            )));
        }
        lines.push(field_line(
            4,
            "Env",
            format!("{} entrie(s)", self.mcp_env.len()),
        ));
        if step == 4 {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    display_with_cursor(&self.input_buffer, self.cursor_pos)
                ),
                Style::default().fg(TEXT),
            )));
        }
        lines.push(field_line(
            5,
            "Cwd",
            if step == 5 {
                display_with_cursor(&self.mcp_cwd_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_cwd_buf)
            },
        ));
        lines.push(field_line(
            6,
            "Url",
            if step == 6 {
                display_with_cursor(&self.mcp_url_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_url_buf)
            },
        ));
        lines.push(field_line(
            7,
            "Headers",
            format!("{} entrie(s)", self.mcp_headers.len()),
        ));
        if step == 7 {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    display_with_cursor(&self.input_buffer, self.cursor_pos)
                ),
                Style::default().fg(TEXT),
            )));
        }
        lines.push(field_line(
            8,
            "OAuth JSON",
            if step == 8 {
                display_ellipsize(
                    &display_with_cursor(&self.mcp_oauth_buf, self.cursor_pos),
                    56,
                )
            } else {
                empty_label(&display_ellipsize(&self.mcp_oauth_buf, 56))
            },
        ));
        lines.push(field_line(
            9,
            "Headers helper",
            if step == 9 {
                display_with_cursor(&self.mcp_headers_helper_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_headers_helper_buf)
            },
        ));
        lines.push(field_line(
            10,
            "Timeout ms",
            if step == 10 {
                display_with_cursor(&self.mcp_timeout_buf, self.cursor_pos)
            } else {
                empty_label(&self.mcp_timeout_buf)
            },
        ));
        lines.push(field_line(
            11,
            "Always load",
            optional_bool_label(self.mcp_always_load),
        ));
        lines.push(field_line(
            12,
            "Disabled",
            optional_bool_label(self.mcp_disabled),
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N fields  Tab cycle type/bool  Enter add list item or save  Backspace removes last list item",
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  stdio requires Command; http/streamable-http/sse require Url; Env/Headers use KEY=VALUE",
            Style::default().fg(DIM),
        )));

        f.render_widget(
            Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    pub(super) fn render_mcp_profile_picker_popup(&self, f: &mut Frame) {
        let area = centered_rect(78, 24, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Select MCP Servers ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let filtered = self.filtered_mcp_indices();
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Filter: ", Style::default().fg(DIM)),
                Span::styled(
                    display_with_cursor(&self.mcp_filter_buf, self.cursor_pos),
                    Style::default().fg(TEXT).bold(),
                ),
            ]),
            Line::from(""),
        ];
        for idx in filtered.into_iter().take(14) {
            let mcp = &self.mcps_cache[idx];
            let selected = self.mcp_selected_ids.iter().any(|id| id == &mcp.id);
            let cursor = self.mcp_list_state.selected() == Some(idx);
            let marker = if selected { "[x]" } else { "[ ]" };
            let prefix = if cursor { "▶" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} {}", prefix, marker),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(
                    format!(" {}", display_pad(&mcp.name, 24)),
                    Style::default().fg(TEXT),
                ),
                Span::styled(format!(" {}", mcp.server_type), Style::default().fg(DIM)),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Space toggle  Tab complete filter  Enter save  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(super) fn render_mcp_smart_paste_popup(&self, f: &mut Frame) {
        let area = centered_rect(80, 14, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Import MCP JSON ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let preview = display_ellipsize(
            &display_with_cursor(&self.mcp_oauth_buf, self.cursor_pos),
            72,
        );
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Paste a single server object, or an object with mcpServers containing one server.",
                Style::default().fg(DIM),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", preview),
                Style::default().fg(TEXT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Enter import  Esc/Ctrl+G cancel",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(
            Paragraph::new(Text::from(lines))
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    pub(super) fn render_confirm_delete_mcp_popup(&self, f: &mut Frame) {
        let name = match &self.mode {
            Mode::ConfirmDeleteMcp { name, .. } => name.clone(),
            _ => return,
        };
        self.render_confirm_popup(
            f,
            &format!("Delete MCP '{}'?", name),
            "This cannot be undone. Press 'y' to confirm.",
        );
    }
}
