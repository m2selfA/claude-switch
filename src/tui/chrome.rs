use super::*;

impl App {
    pub(super) fn handle_first_run_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool> {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            _ => self.mode = Mode::Normal,
        }
        Ok(false)
    }

    pub(super) fn render_first_run(&self, f: &mut Frame, area: Rect) {
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
                "  Welcome! Press any non-quit key to open the profile manager, then use t or a to create a profile.",
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
                Span::styled("quit  ", Style::default().fg(DIM)),
                Span::styled(" q ", Style::default().fg(ACCENT).bold()),
                Span::styled("quit", Style::default().fg(DIM)),
            ]))
            .block(footer_block),
            layout[2],
        );
    }

    pub(super) fn render_header(&self, f: &mut Frame, area: Rect) {
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
                        Page::Profile => "  profile manager",
                        Page::Provider => "  provider manager",
                        Page::Mcp => "  mcp manager",
                        Page::Plugin => "  plugin manager",
                        Page::Settings => "  settings",
                    },
                    Style::default().fg(DIM),
                ),
            ]))
            .block(block),
            area,
        );

        let (count, total) = match self.page {
            Page::Profile => (self.filtered_indices.len(), self.profiles.len()),
            Page::Provider => (self.providers_cache.len(), self.providers_cache.len()),
            Page::Mcp => (self.mcps_cache.len(), self.mcps_cache.len()),
            Page::Plugin => (self.plugins_cache.len(), self.plugins_cache.len()),
            Page::Settings => (1, 1),
        };
        let item_name = match self.page {
            Page::Profile => "profile",
            Page::Provider => "provider",
            Page::Mcp => "mcp",
            Page::Plugin => "plugin",
            Page::Settings => "setting",
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

    pub(super) fn render_help(&self, f: &mut Frame) {
        let area = centered_rect(68, 27, f.area());
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
            ("T", "Batch test non-official provider keys"),
            ("M", "Select MCP servers for a lightweight profile"),
            ("P", "Select hosted plugins for a profile"),
            ("c", "Duplicate selected profile"),
            ("Ctrl+Y", "Smart input provider from clipboard"),
            ("MCP: Ctrl+Y", "Import MCP JSON from clipboard"),
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
            ("Shift+Tab", "Cycle managers from manager pages"),
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

    pub(super) fn render_message(&self, f: &mut Frame, msg: &str, is_err: bool) {
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
}
