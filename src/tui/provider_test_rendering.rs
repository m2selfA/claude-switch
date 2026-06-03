use super::*;

impl App {
    pub(super) fn render_provider_test_key_list_popup(&mut self, f: &mut Frame) {
        let area = centered_rect(62, 14, f.area());
        f.render_widget(Clear, area);
        let pid = match &self.mode {
            Mode::ProviderTestKeyList { provider_id } => provider_id.clone(),
            _ => return,
        };
        let prov_name = self
            .manager
            .get_provider(&pid)
            .map(|p| p.name)
            .unwrap_or_default();

        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Test Key — {} ", prov_name),
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
                    "  This provider has no keys.",
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
        lines.push(Line::from(Span::styled(
            "  Select a key for provider testing.",
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(""));
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
                format!("  Page {}/{}", current_page, total_pages),
                Style::default().fg(DIM),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ctrl+P/N nav  t=models  T=anthropic  Esc/Ctrl+G=back",
            Style::default().fg(DIM),
        )));
        f.render_widget(Paragraph::new(Text::from(lines)), block.inner(area));
    }

    pub(super) fn render_provider_anthropic_test_popup(&self, f: &mut Frame) {
        let (provider_id, key_id, field) = match &self.mode {
            Mode::ProviderAnthropicTest {
                provider_id,
                key_id,
                field,
                ..
            } => (provider_id, key_id, *field),
            _ => return,
        };
        let provider_name = self
            .providers_cache
            .iter()
            .find(|provider| &provider.id == provider_id)
            .map(|provider| provider.name.as_str())
            .unwrap_or("Provider");
        let key_name = self
            .provider_keys_cache
            .iter()
            .find(|key| &key.id == key_id)
            .map(|key| key.name.as_str())
            .unwrap_or("Key");

        let area = centered_rect(78, 22, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" Provider Test — {} / {} ", provider_name, key_name),
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
                Constraint::Length(9),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
            ])
            .split(inner);

        let model_active = field == 0;
        let prompt_active = field == 1;
        let model_value = if model_active {
            display_with_cursor(&self.provider_test_model_buf, self.cursor_pos)
        } else if self.provider_test_model_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_test_model_buf.clone()
        };
        let prompt_value = if prompt_active {
            display_with_cursor(&self.provider_test_prompt_buf, self.cursor_pos)
        } else if self.provider_test_prompt_buf.is_empty() {
            "(empty)".to_string()
        } else {
            self.provider_test_prompt_buf.clone()
        };

        let list_block = Block::default()
            .title(Line::from(Span::styled(
                " Models ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(BG));
        let mut list_lines = vec![Line::from("")];
        let page_size = 5usize;
        let total = self.provider_test_models.len();
        let (page_start, page_end) =
            visible_window(self.provider_test_model_selected, total, page_size);
        if total == 0 {
            let msg = match &self.provider_test_model_fetch_state {
                ModelFetchState::Loaded | ModelFetchState::Empty => {
                    "  No models returned from provider.".to_string()
                }
                ModelFetchState::Unavailable(reason) => format!("  {}", reason),
            };
            list_lines.push(Line::from(Span::styled(msg, Style::default().fg(DIM))));
        } else {
            for (offset, model) in self.provider_test_models[page_start..page_end]
                .iter()
                .enumerate()
            {
                let index = page_start + offset;
                let selected = index == self.provider_test_model_selected;
                let prefix = if selected { "▶ " } else { "  " };
                let style = if selected {
                    Style::default().fg(ACCENT).bold()
                } else {
                    Style::default().fg(TEXT)
                };
                list_lines.push(Line::from(vec![Span::styled(
                    format!("  {}{}", prefix, model),
                    style,
                )]));
            }
            let total_pages = total.div_ceil(page_size);
            let current_page = page_start / page_size + 1;
            list_lines.push(Line::from(""));
            list_lines.push(Line::from(Span::styled(
                format!(
                    "  Page {}/{}  PgUp/PgDn to scroll",
                    current_page, total_pages
                ),
                Style::default().fg(DIM),
            )));
        }
        f.render_widget(
            Paragraph::new(Text::from(list_lines))
                .block(list_block)
                .wrap(Wrap { trim: false }),
            sections[0],
        );

        let model_lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("  {}Model   ", if model_active { "▶ " } else { "  " }),
                    Style::default()
                        .fg(if model_active { ACCENT } else { DIM })
                        .bold(),
                ),
                Span::styled(model_value, Style::default().fg(TEXT)),
            ]),
            Line::from(Span::styled(
                "  Tab completes from fetched models; you can also type manually.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(Paragraph::new(Text::from(model_lines)), sections[2]);

        let prompt_lines = vec![Line::from(vec![
            Span::styled(
                format!("  {}Prompt  ", if prompt_active { "▶ " } else { "  " }),
                Style::default()
                    .fg(if prompt_active { ACCENT } else { DIM })
                    .bold(),
            ),
            Span::styled(prompt_value, Style::default().fg(TEXT)),
        ])];
        f.render_widget(Paragraph::new(Text::from(prompt_lines)), sections[3]);

        let footer_lines = vec![
            Line::from(Span::styled(
                "  Ctrl+P/N switches fields. Up/Down browse fetched models while Model is focused.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Enter sends one non-streaming /v1/messages request. Esc/Ctrl+G exits.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(
            Paragraph::new(Text::from(footer_lines)).wrap(Wrap { trim: false }),
            sections[4],
        );
    }

    pub(super) fn render_provider_anthropic_outcome_popup(&self, f: &mut Frame) {
        let (model, endpoint_used, input_tokens, output_tokens, body, is_error) = match &self.mode {
            Mode::ProviderAnthropicOutcome {
                model,
                endpoint_used,
                input_tokens,
                output_tokens,
                body,
                is_error,
                ..
            } => (
                model,
                endpoint_used.as_deref(),
                *input_tokens,
                *output_tokens,
                body,
                *is_error,
            ),
            _ => return,
        };

        let area = centered_rect(78, 18, f.area());
        f.render_widget(Clear, area);
        let accent = if is_error { DANGER } else { SUCCESS };
        let block = Block::default()
            .title(Line::from(Span::styled(
                if is_error {
                    " Anthropic Test Error "
                } else {
                    " Anthropic Test Result "
                },
                Style::default().fg(accent).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(PANEL));
        f.render_widget(block.clone(), area);

        let inner = block.inner(area);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Length(1),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        let usage = match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => format!("input {}   output {}", input, output),
            (Some(input), None) => format!("input {}", input),
            (None, Some(output)) => format!("output {}", output),
            (None, None) => "(no usage returned)".to_string(),
        };

        let mut meta_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Model   ", Style::default().fg(DIM)),
                Span::styled(model, Style::default().fg(TEXT).bold()),
            ]),
        ];
        if let Some(endpoint) = endpoint_used {
            meta_lines.push(Line::from(vec![
                Span::styled("  Endpoint ", Style::default().fg(DIM)),
                Span::styled(endpoint, Style::default().fg(TEXT)),
            ]));
        }
        meta_lines.push(Line::from(vec![
            Span::styled("  Usage   ", Style::default().fg(DIM)),
            Span::styled(
                if is_error {
                    "(request failed)".to_string()
                } else {
                    usage
                },
                Style::default().fg(TEXT),
            ),
        ]));
        f.render_widget(Paragraph::new(meta_lines), sections[0]);

        let reply_block = Block::default()
            .title(Line::from(Span::styled(
                if is_error { " Error " } else { " Reply " },
                Style::default()
                    .fg(if is_error { DANGER } else { ACCENT })
                    .bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(BG));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(format!("  {}", body)),
            ]))
            .block(reply_block)
            .wrap(Wrap { trim: false }),
            sections[2],
        );

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Enter returns to test. Esc/Ctrl+G or q exits.",
                Style::default().fg(DIM),
            ))),
            sections[3],
        );
    }
}
