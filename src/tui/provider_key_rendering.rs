use super::*;

impl App {
    pub(super) fn render_provider_key_list_popup(&mut self, f: &mut Frame) {
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
            "  Ctrl+P/N nav  a=add  r=rename  e=edit  d=delete  t=test  Esc/Ctrl+G=back",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }

    pub(super) fn render_provider_key_add_popup(&self, f: &mut Frame, step: usize) {
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

    pub(super) fn render_provider_key_edit_popup(&self, f: &mut Frame, step: usize) {
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

    pub(super) fn render_provider_key_rename_popup(&self, f: &mut Frame) {
        let area = centered_rect(56, 7, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Rename Key ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));

        let mut lines = vec![Line::from("")];
        lines.push(Line::from(Span::styled(
            format!(
                "  Name: {}",
                display_with_cursor(&self.provider_key_name_buf, self.cursor_pos)
            ),
            Style::default().fg(TEXT).bold(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter confirm  Esc/Ctrl+G cancel",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    pub(super) fn render_provider_edit_key_input_popup(&self, f: &mut Frame, step: usize) {
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

    pub(super) fn render_confirm_delete_key_popup(&self, f: &mut Frame) {
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

    pub(super) fn render_provider_key_in_use_popup(&self, f: &mut Frame) {
        let name = match &self.mode {
            Mode::ProviderKeyInUse { name, .. } => name.clone(),
            _ => return,
        };
        let area = centered_rect(70, 16, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Key '{}' In Use ", name),
                Style::default().fg(DANGER).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DANGER))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Delete linked profiles one by one, or unlink them all and remove this key.",
                Style::default().fg(TEXT),
            )),
            Line::from(""),
        ];
        if self.provider_key_linked_profiles.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No linked profiles remain.",
                Style::default().fg(MUTED),
            )));
        } else {
            for (idx, profile) in self.provider_key_linked_profiles.iter().enumerate() {
                let selected = idx == self.provider_key_linked_profile_selected;
                let prefix = if selected { "▶" } else { " " };
                let alias = profile
                    .alias
                    .as_deref()
                    .map(|a| format!(" [{}]", a))
                    .unwrap_or_default();
                let style = if selected {
                    Style::default().fg(ACCENT).bold()
                } else {
                    Style::default().fg(TEXT)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", prefix), style),
                    Span::styled(format!("{}{}", profile.name, alias), style),
                ]));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N nav  d=delete profile  y=unlink all and remove key  Esc/Ctrl+G=back",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }
}
