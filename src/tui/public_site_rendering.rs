use super::*;

impl App {
    pub(super) fn render_public_site_prompt_popup(&self, f: &mut Frame) {
        let area = centered_rect(80, 10, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                " Public Site Quick Test ",
                Style::default().fg(ACCENT).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let prompt_value = if self.public_site_prompt_buf.is_empty() {
            "█".to_string()
        } else {
            display_with_cursor(&self.public_site_prompt_buf, self.cursor_pos)
        };
        let base_url_count = self
            .public_site_targets
            .iter()
            .map(|target| normalize_base_url_key(&target.base_url))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "  Non-official provider keys: {} across {} base URLs",
                    self.public_site_targets.len(),
                    base_url_count
                ),
                Style::default().fg(TEXT),
            )),
            Line::from(Span::styled(
                "  One worker runs per base URL; keys on the same base URL are spaced out.",
                Style::default().fg(DIM),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Prompt  ", Style::default().fg(ACCENT).bold()),
                Span::styled(prompt_value, Style::default().fg(TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Enter starts the batch test. Esc/Ctrl+G exits.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }

    pub(super) fn render_public_site_results_popup(&self, f: &mut Frame) {
        let area = centered_rect(90, 24, f.area());
        f.render_widget(Clear, area);
        let running = matches!(self.mode, Mode::PublicSiteTesting);
        let title = if running {
            " Public Site Quick Test — Running "
        } else {
            " Public Site Quick Test — Results "
        };
        let accent = if running { ACCENT } else { SUCCESS };
        let block = Block::default()
            .title(Line::from(Span::styled(
                title,
                Style::default().fg(accent).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(accent))
            .style(Style::default().bg(PANEL));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(8),
                Constraint::Length(4),
            ])
            .split(inner);

        let summary = vec![
            Line::from(Span::styled(
                format!(
                    "  Progress: {}/{}  Successful: {}",
                    self.public_site_completed,
                    self.public_site_total,
                    self.public_site_results
                        .iter()
                        .filter(|result| result.is_success)
                        .count()
                ),
                Style::default().fg(TEXT),
            )),
            Line::from(Span::styled(
                format!("  {}", self.public_site_status),
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(
            Paragraph::new(summary).wrap(Wrap { trim: false }),
            sections[0],
        );

        let total = self.public_site_results.len();
        let selected = self
            .public_site_result_selected
            .min(total.saturating_sub(1));
        let (page_start, page_end) = visible_window(selected, total, PUBLIC_SITE_TEST_PAGE_SIZE);
        let mut lines = Vec::new();
        if total == 0 {
            lines.push(Line::from(Span::styled(
                if running {
                    "  Waiting for the first result..."
                } else {
                    "  No results."
                },
                Style::default().fg(DIM),
            )));
        } else {
            for (offset, result) in self.public_site_results[page_start..page_end]
                .iter()
                .enumerate()
            {
                let index = page_start + offset;
                let selected_row = index == selected;
                let row_color = if result.is_success { SUCCESS } else { DANGER };
                let status = if result.is_success { "OK " } else { "ERR" };
                let latency = result
                    .latency_ms
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "n/a".into());
                let first = result.first_char.as_deref().unwrap_or("—");
                let location = ellipsize(&result.base_url, 24);
                lines.push(Line::from(vec![
                    Span::styled(
                        if selected_row { "▶ " } else { "  " },
                        Style::default().fg(ACCENT).bold(),
                    ),
                    Span::styled(
                        format!("[{status}] "),
                        Style::default().fg(row_color).bold(),
                    ),
                    Span::styled(
                        format!(
                            "{} / {} / {}  {}  first:{}  {}",
                            ellipsize(&result.provider_name, 14),
                            ellipsize(&result.key_name, 10),
                            ellipsize(&result.profile_name, 16),
                            latency,
                            first,
                            ellipsize(&location, 20)
                        ),
                        Style::default().fg(if selected_row { ACCENT } else { TEXT }),
                    ),
                ]));
            }
        }
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            sections[1],
        );

        let detail_sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(sections[2]);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "  Selected Result",
                    Style::default().fg(ACCENT).bold(),
                )),
                Line::from(""),
            ]),
            detail_sections[0],
        );
        let detail_lines = if let Some(result) = self.public_site_results.get(selected) {
            public_site_result_detail_lines(result)
        } else {
            vec!["No selected result.".to_string()]
        };
        let detail_scroll = self
            .public_site_detail_scroll
            .min(public_site_detail_scroll_limit(
                &detail_lines,
                detail_sections[1].width,
                detail_sections[1].height,
            ));
        let detail_body = detail_lines
            .into_iter()
            .map(|line| {
                Line::from(Span::styled(
                    if line.is_empty() {
                        String::new()
                    } else {
                        format!("  {line}")
                    },
                    Style::default().fg(TEXT),
                ))
            })
            .collect::<Vec<_>>();
        f.render_widget(
            Paragraph::new(detail_body)
                .wrap(Wrap { trim: false })
                .scroll((detail_scroll, 0)),
            detail_sections[1],
        );

        let footer = vec![
            Line::from(Span::styled(
                "  Failures show Error first. Ctrl+U/Ctrl+D scrolls detail.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  h/s/o/m/a opens Provider Test with haiku/sonnet/opus/model/subagent.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Shift+S opens process switch for the selected result's provider/key/model.",
                Style::default().fg(DIM),
            )),
            Line::from(Span::styled(
                "  Ctrl+P/N moves results. PgUp/PgDn jumps pages. Enter/q/Esc/Ctrl+G exits.",
                Style::default().fg(DIM),
            )),
        ];
        f.render_widget(
            Paragraph::new(footer).wrap(Wrap { trim: false }),
            sections[3],
        );
    }
}
