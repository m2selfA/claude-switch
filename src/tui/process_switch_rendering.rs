use super::*;
use chrono::Utc;

impl App {
    pub(super) fn render_process_switch_picker_popup(&self, f: &mut Frame) {
        let (provider_id, key_id) = match &self.mode {
            Mode::ProcessSwitchPicker {
                provider_id,
                key_id,
                ..
            } => (provider_id, key_id),
            _ => return,
        };
        let area = centered_rect(88, 22, f.area());
        f.render_widget(Clear, area);
        let provider_name = self
            .manager
            .get_provider(provider_id)
            .ok()
            .map(|provider| provider.name)
            .unwrap_or_else(|| provider_id.clone());
        let key_name = self
            .manager
            .get_provider(provider_id)
            .ok()
            .and_then(|provider| provider.keys.get(key_id).cloned())
            .map(|key| key.name)
            .unwrap_or_else(|| key_id.clone());
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(
                    " Switch Running Process — {} / {} ",
                    provider_name, key_name
                ),
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
                Constraint::Length(11),
                Constraint::Length(1),
                Constraint::Length(6),
                Constraint::Length(2),
            ])
            .split(inner);

        let mut lines = vec![
            Line::from(Span::styled(
                "  Select the running Claude process to retarget.",
                Style::default().fg(DIM),
            )),
            Line::from(""),
        ];
        let total = self.runtime_sessions_cache.len();
        let (start, end) = visible_window(self.runtime_session_selected, total, 7);
        for (offset, session) in self.runtime_sessions_cache[start..end].iter().enumerate() {
            let index = start + offset;
            let selected = index == self.runtime_session_selected;
            let style = if selected {
                Style::default().fg(ACCENT).bold()
            } else {
                Style::default().fg(TEXT)
            };
            let pid = session
                .state
                .pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let profile = session
                .state
                .profile_alias
                .as_ref()
                .map(|alias| format!("{} ({alias})", session.state.profile_name))
                .unwrap_or_else(|| session.state.profile_name.clone());
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", if selected { "▶" } else { " " }), style),
                Span::styled(
                    format!(
                        "{}  pid={}  profile={}",
                        session.state.session_id, pid, profile
                    ),
                    style,
                ),
            ]));
        }
        f.render_widget(Paragraph::new(Text::from(lines)), sections[0]);

        if let Some(session) = self.selected_runtime_session() {
            let provider_key = format!(
                "{}/{}",
                session.state.provider_name.as_deref().unwrap_or("inline"),
                session.state.key_name.as_deref().unwrap_or("no-key")
            );
            let model = session
                .state
                .model
                .clone()
                .unwrap_or_else(|| "—".to_string());
            let cwd = session
                .state
                .cwd
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "—".to_string());
            let age = Utc::now()
                .signed_duration_since(session.state.created_at)
                .num_minutes();
            let detail_lines = vec![
                Line::from(vec![
                    Span::styled("  Session  ", Style::default().fg(DIM)),
                    Span::styled(&session.state.session_id, Style::default().fg(TEXT).bold()),
                ]),
                Line::from(vec![
                    Span::styled("  PID      ", Style::default().fg(DIM)),
                    Span::styled(
                        session
                            .state
                            .pid
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        Style::default().fg(TEXT),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("  Provider ", Style::default().fg(DIM)),
                    Span::styled(provider_key, Style::default().fg(TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Model    ", Style::default().fg(DIM)),
                    Span::styled(model, Style::default().fg(TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  CWD      ", Style::default().fg(DIM)),
                    Span::styled(cwd, Style::default().fg(TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Age      ", Style::default().fg(DIM)),
                    Span::styled(format!("{} min", age.max(0)), Style::default().fg(TEXT)),
                ]),
            ];
            f.render_widget(Paragraph::new(Text::from(detail_lines)), sections[2]);
        }

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(Span::styled(
                    "  Ctrl+P/N selects process. Enter continues to model selection.",
                    Style::default().fg(DIM),
                )),
                Line::from(Span::styled(
                    "  Esc/Ctrl+G returns to the Provider Test result.",
                    Style::default().fg(DIM),
                )),
            ])),
            sections[3],
        );
    }

    pub(super) fn render_process_switch_model_popup(&self, f: &mut Frame) {
        let session_id = match &self.mode {
            Mode::ProcessSwitchModelConfirm { session_id, .. } => session_id,
            _ => return,
        };
        let area = centered_rect(78, 12, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Confirm Model For Switch ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        let selected = self
            .runtime_sessions_cache
            .iter()
            .find(|session| session.state.session_id == *session_id);
        let profile = selected
            .map(|session| {
                session
                    .state
                    .profile_alias
                    .as_ref()
                    .map(|alias| format!("{} ({alias})", session.state.profile_name))
                    .unwrap_or_else(|| session.state.profile_name.clone())
            })
            .unwrap_or_else(|| "unknown".to_string());
        let cwd = selected
            .and_then(|session| session.state.cwd.as_ref())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "—".to_string());
        let model_value = display_with_cursor(&self.runtime_switch_model_buf, self.cursor_pos);
        let lines = vec![
            Line::from(vec![
                Span::styled("  Session  ", Style::default().fg(DIM)),
                Span::styled(session_id, Style::default().fg(TEXT).bold()),
            ]),
            Line::from(vec![
                Span::styled("  Profile  ", Style::default().fg(DIM)),
                Span::styled(profile, Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  CWD      ", Style::default().fg(DIM)),
                Span::styled(cwd, Style::default().fg(TEXT)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Model    ", Style::default().fg(ACCENT).bold()),
                Span::styled(model_value, Style::default().fg(TEXT)),
            ]),
            Line::from(Span::styled(
                "  Enter confirms. Edit the tested model directly or paste a new model id.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Esc/Ctrl+G returns to the process picker.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }
}
