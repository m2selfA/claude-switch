use super::*;

impl App {
    pub(super) fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::default().style(Style::default().bg(BG)), area);

        if self.mode == Mode::FirstRun {
            self.render_first_run(f, area);
            return;
        }

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        self.render_header(f, layout[0]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(layout[1]);

        match self.page {
            Page::Profile => {
                self.render_profile_list(f, cols[0]);
                self.render_detail_panel(f, cols[1]);
            }
            Page::Provider => {
                self.render_provider_list_page(f, cols[0]);
                self.render_provider_detail_page(f, cols[1]);
            }
            Page::Mcp => {
                self.render_mcp_list_page(f, cols[0]);
                self.render_mcp_detail_page(f, cols[1]);
            }
        }
        self.render_footer(f, layout[2]);

        match &self.mode.clone() {
            Mode::Help => self.render_help(f),
            Mode::ConfirmDelete => self.render_confirm_delete_popup(f),
            Mode::AddFullName => self.render_add_name_popup(f),
            Mode::AddFullAlias => self.render_add_alias_popup(f),
            Mode::EditProfile { step, .. } => self.render_edit_profile_popup(f, *step),
            Mode::LiteProviderSelect => lite::render_lite_provider_select_popup(self, f),
            Mode::LiteKeySelect { .. } => lite::render_lite_key_select_popup(self, f),
            Mode::LiteFetching => lite::render_lite_fetching_popup(self, f),
            Mode::ProviderAnthropicTest { .. } => self.render_provider_anthropic_test_popup(f),
            Mode::ProviderAnthropicOutcome { .. } => {
                self.render_provider_anthropic_outcome_popup(f)
            }
            Mode::LiteModelSelect { .. } | Mode::LiteEdit { .. } => {
                self.render_lite_model_select_popup(f)
            }
            Mode::Message(msg, is_err) => self.render_message(f, msg, *is_err),
            Mode::ProviderList => self.render_provider_list_popup(f),
            Mode::ProviderAdd { step } => self.render_provider_add_popup(f, *step),
            Mode::ProviderSmartPaste => self.render_provider_smart_paste_popup(f),
            Mode::ProviderEdit { .. } => self.render_provider_edit_popup(f),
            Mode::ProviderEditKeyInput { step, .. } => {
                self.render_provider_edit_key_input_popup(f, *step)
            }
            Mode::ProviderKeyList { .. } => self.render_provider_key_list_popup(f),
            Mode::ProviderTestKeyList { .. } => self.render_provider_test_key_list_popup(f),
            Mode::ProviderKeyAdd { step, .. } => self.render_provider_key_add_popup(f, *step),
            Mode::ProviderKeyEdit { step, .. } => self.render_provider_key_edit_popup(f, *step),
            Mode::ProviderKeyRename { .. } => self.render_provider_key_rename_popup(f),
            Mode::ConfirmDeleteProvider { .. } => self.render_confirm_delete_provider_popup(f),
            Mode::ConfirmDeleteKey { .. } => self.render_confirm_delete_key_popup(f),
            Mode::ProviderKeyInUse { .. } => self.render_provider_key_in_use_popup(f),
            Mode::McpAdd { step } | Mode::McpEdit { step, .. } => {
                self.render_mcp_editor_popup(f, *step)
            }
            Mode::McpProfilePicker { .. } => self.render_mcp_profile_picker_popup(f),
            Mode::McpSmartPaste => self.render_mcp_smart_paste_popup(f),
            Mode::ConfirmDeleteMcp { .. } => self.render_confirm_delete_mcp_popup(f),
            Mode::PublicSitePrompt => self.render_public_site_prompt_popup(f),
            Mode::PublicSiteTesting | Mode::PublicSiteResults => {
                self.render_public_site_results_popup(f)
            }
            _ => {}
        }
    }

    pub(super) fn render_footer(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .style(Style::default().bg(PANEL));

        let keys: Vec<(&str, &str)> = if self.mode == Mode::Search {
            vec![
                ("Ctrl+P/N", "navigate"),
                ("enter", "confirm"),
                ("esc/Ctrl+G", "clear"),
            ]
        } else if matches!(self.mode, Mode::PublicSitePrompt) {
            vec![
                ("type", "prompt"),
                ("enter", "run"),
                ("esc/Ctrl+G", "back"),
                ("q", "text"),
            ]
        } else if matches!(self.mode, Mode::PublicSiteTesting | Mode::PublicSiteResults) {
            vec![
                ("Ctrl+P/N", "results"),
                ("PgUp/PgDn", "page"),
                ("h/s/o/m/a", "slot test"),
                ("enter/q", "close"),
                ("esc/Ctrl+G", "back"),
            ]
        } else if matches!(self.mode, Mode::ProviderAnthropicOutcome { .. }) {
            vec![("any key", "back"), ("q", "quit")]
        } else if matches!(self.mode, Mode::ProviderAnthropicTest { .. }) {
            vec![
                ("Ctrl+N/P", "field"),
                ("Tab", "complete"),
                ("PgUp/PgDn", "page"),
                ("enter", "send"),
                ("esc/Ctrl+G", "back"),
            ]
        } else if let Mode::ProviderKeyList { .. } = &self.mode {
            vec![
                ("Ctrl+P/N", "nav"),
                ("a", "add key"),
                ("r", "rename"),
                ("e", "edit key"),
                ("d", "delete key"),
                ("t", "test"),
                ("esc/Ctrl+G", "back"),
            ]
        } else {
            match self.page {
                Page::Profile => vec![
                    ("Ctrl+P/N", "nav"),
                    ("enter", "launch"),
                    ("Shift+Enter", "w/o args"),
                    ("/", "search"),
                    ("t", "lite"),
                    ("T", "public test"),
                    ("M", "mcps"),
                    ("a", "add"),
                    ("e", "edit"),
                    ("m", "[1m]"),
                    ("r", "refresh"),
                    ("d", "delete"),
                    ("?", "help"),
                    ("Shift+Tab", "providers"),
                    ("q", "quit"),
                ],
                Page::Provider => vec![
                    ("Ctrl+P/N", "nav"),
                    ("enter", "keys"),
                    ("a", "add"),
                    ("t", "test"),
                    ("Ctrl+Y", "smart input"),
                    ("e", "edit"),
                    ("d", "delete"),
                    ("?", "help"),
                    ("Shift+Tab", "mcps"),
                    ("q", "quit"),
                ],
                Page::Mcp => vec![
                    ("Ctrl+P/N", "nav"),
                    ("a", "add"),
                    ("e", "edit"),
                    ("d", "delete"),
                    ("Ctrl+Y", "import"),
                    ("enter", "link count"),
                    ("?", "help"),
                    ("Shift+Tab", "profiles"),
                    ("q", "quit"),
                ],
            }
        };

        let spans: Vec<Span> = keys
            .iter()
            .flat_map(|(k, v)| {
                vec![
                    Span::styled(format!(" {} ", k), Style::default().fg(ACCENT).bold()),
                    Span::styled(*v, Style::default().fg(DIM)),
                    Span::styled(" ", Style::default()),
                ]
            })
            .collect();

        f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    }

    pub(super) fn render_confirm_delete_popup(&self, f: &mut Frame) {
        let name = self
            .selected_profile()
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        let area = centered_rect(50, 7, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Confirm Delete ",
                Style::default().fg(DANGER).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DANGER))
            .style(Style::default().bg(PANEL));

        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Delete profile ", Style::default().fg(TEXT)),
                    Span::styled(name.to_string(), Style::default().fg(DANGER).bold()),
                    Span::styled("? This cannot be undone.", Style::default().fg(TEXT)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled("y", Style::default().fg(DANGER).bold()),
                    Span::styled(" confirm   ", Style::default().fg(DIM)),
                    Span::styled("any other key", Style::default().fg(ACCENT).bold()),
                    Span::styled(" cancel", Style::default().fg(DIM)),
                ]),
            ]))
            .block(block),
            area,
        );
    }

    pub(super) fn render_confirm_popup(&self, f: &mut Frame, title: &str, hint: &str) {
        let area = centered_rect(50, 7, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(Line::from(Span::styled(
                format!(" {} ", title),
                Style::default().fg(DANGER).bold(),
            )))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(DANGER))
            .style(Style::default().bg(PANEL));
        f.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {}", hint),
                    Style::default().fg(TEXT),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  [y] yes    [other] cancel",
                    Style::default().fg(DIM),
                )),
            ]))
            .block(block),
            area,
        );
    }
}
