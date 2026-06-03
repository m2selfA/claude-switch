use super::*;

impl App {
    pub(super) fn poll_public_site_worker_events(&mut self) {
        let mut disconnected = false;
        let mut pending = Vec::new();
        if let Some(rx) = self.public_site_event_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(event) => pending.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        for event in pending {
            match event {
                PublicSiteWorkerEvent::Result(result) => {
                    self.public_site_completed += 1;
                    self.public_site_status = format!(
                        "Completed {}/{} provider keys",
                        self.public_site_completed, self.public_site_total
                    );
                    self.public_site_results.push(result);
                    sort_public_site_results(&mut self.public_site_results);
                    if self.public_site_result_selected >= self.public_site_results.len()
                        && !self.public_site_results.is_empty()
                    {
                        self.public_site_result_selected = self.public_site_results.len() - 1;
                    }
                }
            }
        }

        if (disconnected || self.public_site_completed >= self.public_site_total)
            && self.public_site_event_rx.is_some()
        {
            self.public_site_event_rx = None;
            if matches!(self.mode, Mode::PublicSiteTesting) {
                self.mode = Mode::PublicSiteResults;
            }
            if self.public_site_total == 0 {
                self.public_site_status = "No eligible non-official provider keys found.".into();
            } else {
                self.public_site_status =
                    format!("Finished {} provider-key tests", self.public_site_total);
            }
        }
    }

    fn resolve_public_site_credentials(
        &self,
        profile: &Profile,
    ) -> (String, String, Option<String>) {
        match self.manager.resolve_credentials(profile) {
            Ok((token, url)) => {
                let token = token.unwrap_or_default().trim().to_string();
                let url = url
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches('/')
                    .to_string();
                let preflight_error = if token.is_empty() {
                    Some("No resolved auth token/key for this profile.".to_string())
                } else if url.is_empty() {
                    Some("No resolved base URL for this profile.".to_string())
                } else {
                    None
                };
                (token, url, preflight_error)
            }
            Err(error) => (String::new(), String::new(), Some(error.to_string())),
        }
    }

    pub(super) fn collect_public_site_targets(&self) -> Result<Vec<PublicSiteTarget>> {
        let providers = self.manager.list_providers()?;
        let provider_map: HashMap<String, Provider> = providers
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect();
        let mut targets = Vec::new();

        for profile in &self.profiles {
            if profile.kind != ProfileKind::Lightweight {
                continue;
            }
            let (token, resolved_url, preflight_error) =
                self.resolve_public_site_credentials(profile);
            let (model_source, configured_model) = public_site_model_from_profile(profile);

            if let (Some(provider_id), Some(key_id)) = (&profile.provider_id, &profile.key_id) {
                let provider = provider_map.get(provider_id);
                let fallback_base_url = provider
                    .map(|p| p.base_url.trim().trim_end_matches('/').to_string())
                    .unwrap_or_default();
                let target_base_url = if resolved_url.is_empty() {
                    fallback_base_url
                } else {
                    resolved_url
                };
                if !target_base_url.is_empty() && is_public_test_excluded_base_url(&target_base_url)
                {
                    continue;
                }
                let key = provider.and_then(|p| p.keys.get(key_id));
                targets.push(PublicSiteTarget {
                    provider_id: provider_id.clone(),
                    provider_name: provider
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| provider_id.clone()),
                    key_id: key_id.clone(),
                    key_name: key
                        .map(|k| k.name.clone())
                        .unwrap_or_else(|| key_id.clone()),
                    base_url: target_base_url,
                    profile_id: profile.id.clone(),
                    profile_name: profile.name.clone(),
                    api_key: token,
                    preflight_error,
                    configured_model,
                    model_source,
                });
            } else {
                if !resolved_url.is_empty() && is_public_test_excluded_base_url(&resolved_url) {
                    continue;
                }
                targets.push(PublicSiteTarget {
                    provider_id: String::new(),
                    provider_name: "Inline".into(),
                    key_id: String::new(),
                    key_name: "Inline".into(),
                    base_url: resolved_url,
                    profile_id: profile.id.clone(),
                    profile_name: profile.name.clone(),
                    api_key: token,
                    preflight_error,
                    configured_model,
                    model_source,
                });
            }
        }

        targets.sort_by(|left, right| {
            normalize_base_url_key(&left.base_url)
                .cmp(&normalize_base_url_key(&right.base_url))
                .then_with(|| left.profile_name.cmp(&right.profile_name))
                .then_with(|| left.provider_name.cmp(&right.provider_name))
                .then_with(|| left.key_name.cmp(&right.key_name))
        });
        Ok(targets)
    }

    pub(super) fn start_public_site_prompt(&mut self) -> Result<()> {
        self.refresh()?;
        let targets = self.collect_public_site_targets()?;
        if targets.is_empty() {
            self.show_message(
                "No non-official provider-key linked lightweight profiles found.".into(),
                true,
                None,
            );
            return Ok(());
        }
        self.public_site_targets = targets;
        self.public_site_prompt_buf = PUBLIC_SITE_TEST_DEFAULT_PROMPT.to_string();
        self.public_site_results.clear();
        self.public_site_result_selected = 0;
        self.public_site_detail_scroll = 0;
        self.public_site_completed = 0;
        self.public_site_total = 0;
        self.public_site_status.clear();
        self.public_site_event_rx = None;
        self.cursor_pos = self.public_site_prompt_buf.len();
        self.mode = Mode::PublicSitePrompt;
        Ok(())
    }

    pub(super) fn start_public_site_batch_test(&mut self) {
        let prompt = self.public_site_prompt_buf.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        self.public_site_results.clear();
        self.public_site_result_selected = 0;
        self.public_site_detail_scroll = 0;
        self.public_site_completed = 0;
        self.public_site_total = self.public_site_targets.len();
        let request_plans = build_public_site_request_plans(&self.public_site_targets, &prompt);
        self.public_site_status = format!(
            "Running {} profiles via {} request plans across {} base URLs",
            self.public_site_total,
            request_plans.len(),
            request_plans
                .iter()
                .map(|plan| plan.key.base_url.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );

        for target in &self.public_site_targets {
            if target.preflight_error.is_some() {
                self.public_site_results
                    .push(public_site_target_preflight_result(target));
                self.public_site_completed += 1;
            }
        }
        sort_public_site_results(&mut self.public_site_results);

        if self.public_site_completed >= self.public_site_total {
            self.public_site_event_rx = None;
            self.public_site_status = if self.public_site_total == 0 {
                "No eligible non-official provider keys found.".into()
            } else {
                format!("Finished {} provider-key tests", self.public_site_total)
            };
            self.mode = Mode::PublicSiteResults;
            return;
        }

        let (tx, rx) = mpsc::channel();
        let mut groups: BTreeMap<String, Vec<PublicSiteRequestPlan>> = BTreeMap::new();
        for plan in request_plans {
            groups
                .entry(plan.key.base_url.clone())
                .or_default()
                .push(plan);
        }
        for (_, group) in groups {
            let tx = tx.clone();
            let prompt = prompt.clone();
            thread::spawn(move || {
                let total = group.len();
                for (idx, plan) in group.into_iter().enumerate() {
                    let template = execute_public_site_target(&plan.request_target, &prompt);
                    for target in &plan.consumers {
                        let result = fan_out_public_site_result(&template, target);
                        if tx.send(PublicSiteWorkerEvent::Result(result)).is_err() {
                            return;
                        }
                    }
                    if idx + 1 < total {
                        thread::sleep(Duration::from_millis(PUBLIC_SITE_TEST_GROUP_GAP_MS));
                    }
                }
            });
        }
        drop(tx);

        self.public_site_event_rx = Some(rx);
        self.mode =
            if self.public_site_completed >= self.public_site_total && self.public_site_total > 0 {
                Mode::PublicSiteResults
            } else {
                Mode::PublicSiteTesting
            };
    }

    pub(super) fn start_public_site_provider_test(
        &mut self,
        slot: PublicSiteProviderTestSlot,
    ) -> Result<()> {
        let Some(result) = self
            .public_site_results
            .get(
                self.public_site_result_selected
                    .min(self.public_site_results.len().saturating_sub(1)),
            )
            .cloned()
        else {
            return Ok(());
        };

        let profile = match self.manager.get_profile(&result.profile_id) {
            Ok(profile) => profile,
            Err(_) => {
                self.show_message(
                    format!(
                        "Profile '{}' is no longer available for provider testing.",
                        result.profile_name
                    ),
                    true,
                    Some(Mode::PublicSiteResults),
                );
                return Ok(());
            }
        };
        let Some(model) = public_site_provider_test_model_from_profile(&profile, slot) else {
            self.show_message(
                format!(
                    "Profile '{}' has no {} model configured.",
                    profile.name,
                    slot.label()
                ),
                true,
                Some(Mode::PublicSiteResults),
            );
            return Ok(());
        };
        let (Some(provider_id), Some(key_id)) = (&profile.provider_id, &profile.key_id) else {
            self.show_message(
                format!(
                    "Profile '{}' uses inline credentials; provider test requires a provider/key link.",
                    profile.name
                ),
                true,
                Some(Mode::PublicSiteResults),
            );
            return Ok(());
        };

        let provider = self.manager.get_provider(provider_id)?;
        let key = provider
            .keys
            .get(key_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Key '{}' not found.", key_id))?;
        self.start_provider_test_popup(&provider, &key, ProviderTestSource::PublicSite)?;
        self.provider_test_model_buf = model;
        self.cursor_pos = self.provider_test_model_buf.len();
        self.sync_provider_test_model_selection_from_buffer();
        Ok(())
    }

    pub(super) fn handle_public_site_prompt(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        if Self::is_cancel_key(code, modifiers) {
            self.mode = Mode::Normal;
            return Ok(());
        }

        match code {
            KeyCode::Enter => {
                if !self.public_site_prompt_buf.trim().is_empty() {
                    self.start_public_site_batch_test();
                }
            }
            _ => {
                emacs_edit(
                    code,
                    modifiers,
                    &mut self.public_site_prompt_buf,
                    &mut self.cursor_pos,
                    true,
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_public_site_results(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        if Self::is_cancel_key(code, modifiers)
            || matches!(code, KeyCode::Enter | KeyCode::Char('q'))
        {
            self.public_site_event_rx = None;
            self.public_site_detail_scroll = 0;
            self.mode = Mode::Normal;
            return Ok(());
        }

        if let Some(slot) = public_site_provider_test_slot_from_key(code, modifiers) {
            self.start_public_site_provider_test(slot)?;
            return Ok(());
        }

        let previous_selected = self.public_site_result_selected;
        match code {
            _ if Self::is_prev_list_key(code, modifiers)
                && !self.public_site_results.is_empty() =>
            {
                self.public_site_result_selected =
                    self.public_site_result_selected.saturating_sub(1);
            }
            _ if Self::is_next_list_key(code, modifiers)
                && !self.public_site_results.is_empty() =>
            {
                self.public_site_result_selected = (self.public_site_result_selected + 1)
                    .min(self.public_site_results.len().saturating_sub(1));
            }
            KeyCode::PageUp => {
                self.public_site_result_selected = self
                    .public_site_result_selected
                    .saturating_sub(PUBLIC_SITE_TEST_PAGE_SIZE);
            }
            KeyCode::PageDown if !self.public_site_results.is_empty() => {
                self.public_site_result_selected = (self.public_site_result_selected
                    + PUBLIC_SITE_TEST_PAGE_SIZE)
                    .min(self.public_site_results.len().saturating_sub(1));
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.public_site_detail_scroll = self
                    .public_site_detail_scroll
                    .saturating_sub(PUBLIC_SITE_DETAIL_SCROLL_STEP);
            }
            KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.public_site_detail_scroll = self
                    .public_site_detail_scroll
                    .saturating_add(PUBLIC_SITE_DETAIL_SCROLL_STEP);
            }
            _ => {}
        }
        if self.public_site_result_selected != previous_selected {
            self.public_site_detail_scroll = 0;
        }
        Ok(())
    }
}
