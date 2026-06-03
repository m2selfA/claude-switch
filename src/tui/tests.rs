use super::*;
use crate::profile::ProfileManager;
use std::env;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use tempfile::TempDir;

struct TestApp {
    app: App,
    _tmp: TempDir,
    _guard: MutexGuard<'static, ()>,
}

impl Deref for TestApp {
    type Target = App;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.app
    }
}

fn make_test_app() -> TestApp {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let old_home = env::var_os("USERPROFILE");
    let old_home_unix = env::var_os("HOME");
    unsafe {
        env::set_var("USERPROFILE", tmp.path());
        env::set_var("HOME", tmp.path());
        env::set_var("CSWITCH_TEST_DISABLE_SHIM_SYNC", "1");
    }
    let manager = ProfileManager::new_for_test(&tmp.path().join(".claude-switch")).unwrap();
    let app = App::new(manager).unwrap();
    unsafe {
        match old_home {
            Some(value) => env::set_var("USERPROFILE", value),
            None => env::remove_var("USERPROFILE"),
        }
        match old_home_unix {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }
    TestApp {
        app,
        _tmp: tmp,
        _guard: guard,
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut header_end = None;
    let mut body_len = 0usize;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if header_end.is_none()
            && let Some(pos) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            header_end = Some(pos + 4);
            let headers = String::from_utf8_lossy(&request[..pos + 4]);
            body_len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
        }
        if let Some(end) = header_end
            && request.len() >= end + body_len
        {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn spawn_model_discovery_server(models: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /v1/models "), "{request}");
        let response_body = serde_json::json!({
            "data": models
                .into_iter()
                .map(|id| serde_json::json!({ "id": id }))
                .collect::<Vec<_>>()
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        )
        .unwrap();
    });
    (format!("http://{}", addr), handle)
}

#[test]
fn smart_paste_parses_newapi_json() {
    let base_url = "https://generated-provider.invalid";
    let api_key = "sk-test-generated-key-000000000000000000000000";
    let parsed = parse_provider_smart_paste(&format!(
        r#"{{"_type":"newapi_channel_conn","key":"{}","url":"{}"}}"#,
        api_key, base_url
    ))
    .unwrap();

    assert_eq!(parsed.name, "generated-provider.invalid");
    assert_eq!(parsed.base_url, base_url);
    assert_eq!(parsed.api_key, api_key);
    assert_eq!(parsed.key_name, "Default");
}

#[test]
fn smart_paste_parses_cherrystudio_url() {
    let base_url = "https://generated-provider.invalid";
    let api_key = "sk-test-generated-key-111111111111111111111111";
    let data = URL_SAFE_NO_PAD.encode(
        serde_json::json!({
            "id": "generated-api",
            "baseUrl": base_url,
            "apiKey": api_key,
        })
        .to_string(),
    );
    let parsed = parse_provider_smart_paste(&format!(
        "cherrystudio://providers/api-keys?v=1&data={data}"
    ))
    .unwrap();

    assert_eq!(parsed.name, "generated-provider.invalid");
    assert_eq!(parsed.key_name, "generated-api");
    assert_eq!(parsed.base_url, base_url);
    assert_eq!(parsed.api_key, api_key);
}

#[test]
fn smart_paste_parses_nextchat_url() {
    let base_url = "https://generated-provider.invalid";
    let api_key = "sk-test-generated-key-555555555555555555555555";
    let parsed = parse_provider_smart_paste(&format!(
            "https://app.nextchat.dev/#/?settings={{%22key%22:%22{api_key}%22,%22url%22:%22https%3A%2F%2Fgenerated-provider.invalid%22}}"
        ))
        .unwrap();

    assert_eq!(parsed.name, "generated-provider.invalid");
    assert_eq!(parsed.key_name, "Default");
    assert_eq!(parsed.base_url, base_url);
    assert_eq!(parsed.api_key, api_key);
}

#[test]
fn smart_paste_parses_opencat_url() {
    let base_url = "https://generated-provider.invalid";
    let api_key = "sk-test-generated-key-666666666666666666666666";
    let parsed = parse_provider_smart_paste(&format!(
        "opencat://team/join?domain=https%3A%2F%2Fgenerated-provider.invalid&token={api_key}"
    ))
    .unwrap();

    assert_eq!(parsed.name, "generated-provider.invalid");
    assert_eq!(parsed.key_name, "Default");
    assert_eq!(parsed.base_url, base_url);
    assert_eq!(parsed.api_key, api_key);
}

#[test]
fn visible_window_returns_correct_range() {
    assert_eq!(visible_window(0, 10, 6), (0, 6));
    assert_eq!(visible_window(5, 10, 6), (0, 6));
    assert_eq!(visible_window(6, 10, 6), (6, 10));
    assert_eq!(visible_window(9, 10, 6), (6, 10));
    assert_eq!(visible_window(0, 0, 6), (0, 0));
    assert_eq!(visible_window(0, 3, 6), (0, 3));
}

#[test]
fn display_with_cursor_inserts_at_start_middle_end() {
    assert_eq!(display_with_cursor("abc", 0), "█abc");
    assert_eq!(display_with_cursor("abc", 1), "a█bc");
    assert_eq!(display_with_cursor("abc", 3), "abc█");
}

#[test]
fn display_with_cursor_handles_empty_string() {
    assert_eq!(display_with_cursor("", 0), "█");
}

#[test]
fn display_with_cursor_clamps_to_utf8_boundary() {
    assert_eq!(display_with_cursor("白日", "白".len()), "白█日");
    assert_eq!(display_with_cursor("白日", 1), "█白日");
    assert_eq!(display_with_cursor("白日", usize::MAX), "白日█");
}

#[test]
fn insert_str_at_cursor_inserts_at_utf8_boundary() {
    let mut buf = "白日".to_string();
    let mut cursor_pos = 1usize;

    insert_str_at_cursor(&mut buf, &mut cursor_pos, "X");

    assert_eq!(buf, "X白日");
    assert_eq!(cursor_pos, "X".len());
}

#[test]
fn insert_filtered_str_at_cursor_filters_alias_chars() {
    let mut buf = "ab".to_string();
    let mut cursor_pos = 1usize;

    insert_filtered_str_at_cursor(&mut buf, &mut cursor_pos, "c.d_1-", is_alias_char);

    assert_eq!(buf, "acd_1-b");
    assert_eq!(cursor_pos, "acd_1-".len());
}

#[test]
fn emacs_edit_handles_multiple_chinese_characters() {
    let mut buf = String::new();
    let mut pos = 0usize;

    assert!(emacs_edit(
        KeyCode::Char('白'),
        KeyModifiers::empty(),
        &mut buf,
        &mut pos,
        true
    ));
    assert_eq!(buf, "白");

    assert!(emacs_edit(
        KeyCode::Char('日'),
        KeyModifiers::empty(),
        &mut buf,
        &mut pos,
        true
    ));
    assert_eq!(buf, "白日");
    assert_eq!(pos, buf.len());
}

#[test]
fn emacs_edit_treats_ctrl_h_as_backspace() {
    let mut buf = "Provider".to_string();
    let mut pos = buf.len();

    assert!(emacs_edit(
        KeyCode::Char('h'),
        KeyModifiers::CONTROL,
        &mut buf,
        &mut pos,
        true
    ));

    assert_eq!(buf, "Provide");
    assert_eq!(pos, buf.len());
}

#[test]
fn emacs_edit_treats_backspace_control_chars_as_backspace() {
    for code in [KeyCode::Char('\u{8}'), KeyCode::Char('\u{7f}')] {
        let mut buf = "Provider".to_string();
        let mut pos = buf.len();

        assert!(emacs_edit(
            code,
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true
        ));

        assert_eq!(buf, "Provide");
        assert_eq!(pos, buf.len());
    }
}

#[test]
fn emacs_edit_backspace_handles_utf8_in_small_add_dialog_buffers() {
    let mut buf = "白日".to_string();
    let mut pos = buf.len();

    assert!(emacs_edit(
        KeyCode::Backspace,
        KeyModifiers::empty(),
        &mut buf,
        &mut pos,
        true
    ));

    assert_eq!(buf, "白");
    assert_eq!(pos, buf.len());
}

#[test]
fn alias_input_still_filters_invalid_chars() {
    let mut buf = String::new();
    let mut pos = 0usize;

    if 'x'.is_ascii_alphanumeric() || 'x' == '-' || 'x' == '_' {
        emacs_edit(
            KeyCode::Char('x'),
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true,
        );
    }
    if '.'.is_ascii_alphanumeric() || '.' == '-' || '.' == '_' {
        emacs_edit(
            KeyCode::Char('.'),
            KeyModifiers::empty(),
            &mut buf,
            &mut pos,
            true,
        );
    }

    assert_eq!(buf, "x");
}

#[test]
fn provider_test_key_selection_detects_empty_single_and_multiple() {
    assert_eq!(
        provider_test_key_selection(&[]),
        ProviderTestKeySelection::NoKeys
    );

    let only = ProviderKey {
        id: "key_one".into(),
        name: "Default".into(),
        api_key: "sk-test-generated-key-222222222222222222222222".into(),
    };
    assert_eq!(
        provider_test_key_selection(std::slice::from_ref(&only)),
        ProviderTestKeySelection::Single(only)
    );

    let many = vec![
        ProviderKey {
            id: "key_one".into(),
            name: "A".into(),
            api_key: "sk-test-generated-key-333333333333333333333333".into(),
        },
        ProviderKey {
            id: "key_two".into(),
            name: "B".into(),
            api_key: "sk-test-generated-key-444444444444444444444444".into(),
        },
    ];
    assert_eq!(
        provider_test_key_selection(&many),
        ProviderTestKeySelection::Multiple
    );
}

#[test]
fn collect_public_site_targets_include_shared_provider_key_profiles() {
    let mut app = make_test_app();
    let official = app
        .manager
        .add_provider_with_key_name(
            "Official",
            "https://api.anthropic.com",
            "Default",
            "sk-test-generated-key-official-1111111111111",
        )
        .unwrap();
    let official_key = official.keys.values().next().unwrap().clone();
    let relay = app
        .manager
        .add_provider_with_key_name(
            "Relay",
            "https://relay.example/api",
            "Default",
            "sk-test-generated-key-relay-2222222222222222",
        )
        .unwrap();
    let relay_key = relay.keys.values().next().unwrap().clone();

    let relay_default = app
        .manager
        .create_lightweight_profile(
            "relay-default",
            Some("relay-default"),
            LightweightEnv {
                default_sonnet_model: Some("claude-default-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&relay_default.id, &relay.id, &relay_key.id)
        .unwrap();

    let relay_explicit = app
        .manager
        .create_lightweight_profile(
            "relay-explicit",
            Some("relay-explicit"),
            LightweightEnv {
                model: Some("relay-explicit-model".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&relay_explicit.id, &relay.id, &relay_key.id)
        .unwrap();

    let official_profile = app
        .manager
        .create_lightweight_profile(
            "official-profile",
            Some("official-profile"),
            LightweightEnv {
                model: Some("official-model".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&official_profile.id, &official.id, &official_key.id)
        .unwrap();

    app.refresh().unwrap();

    let targets = app.collect_public_site_targets().unwrap();
    assert_eq!(targets.len(), 2);
    let relay_targets: Vec<&PublicSiteTarget> = targets
        .iter()
        .filter(|target| target.provider_id == relay.id && target.key_id == relay_key.id)
        .collect();
    assert_eq!(relay_targets.len(), 2);
    assert!(
        relay_targets
            .iter()
            .any(|target| target.profile_name == "relay-default")
    );
    assert!(
        relay_targets
            .iter()
            .any(|target| target.profile_name == "relay-explicit")
    );
    assert!(
        relay_targets
            .iter()
            .any(|target| target.configured_model.as_deref() == Some("relay-explicit-model"))
    );
}

#[test]
fn collect_public_site_targets_excludes_deepseek_and_includes_inline_profiles() {
    let mut app = make_test_app();
    let deepseek = app
        .manager
        .add_provider_with_key_name(
            "DeepSeek",
            "https://api.deepseek.com/anthropic",
            "Default",
            "sk-test-generated-key-deepseek-111111111111111",
        )
        .unwrap();
    let deepseek_key = deepseek.keys.values().next().unwrap().clone();
    let deepseek_profile = app
        .manager
        .create_lightweight_profile(
            "deepseek-profile",
            Some("deepseek-profile"),
            LightweightEnv {
                model: Some("deepseek-chat".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&deepseek_profile.id, &deepseek.id, &deepseek_key.id)
        .unwrap();

    app.manager
        .create_lightweight_profile(
            "inline-relay",
            Some("inline-relay"),
            LightweightEnv {
                auth_token: Some("sk-inline-generated-222222222222222".into()),
                base_url: Some("https://relay.inline.example/api".into()),
                model: Some("inline-model".into()),
                ..Default::default()
            },
        )
        .unwrap();

    app.refresh().unwrap();

    let targets = app.collect_public_site_targets().unwrap();
    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(target.profile_name, "inline-relay");
    assert_eq!(target.provider_name, "Inline");
    assert_eq!(target.key_name, "Inline");
    assert_eq!(target.configured_model.as_deref(), Some("inline-model"));
}

#[test]
fn collect_public_site_targets_rechecks_provider_fallback_url_for_stale_links() {
    let mut app = make_test_app();
    let official = app
        .manager
        .add_provider_with_key_name(
            "Official",
            "https://api.anthropic.com",
            "Default",
            "sk-test-generated-key-official-1111111111111",
        )
        .unwrap();
    let official_key_id = official.keys.keys().next().unwrap().clone();
    let relay = app
        .manager
        .add_provider_with_key_name(
            "Relay",
            "https://relay.example/api",
            "Default",
            "sk-test-generated-key-relay-2222222222222222",
        )
        .unwrap();
    let relay_key_id = relay.keys.keys().next().unwrap().clone();

    let official_profile = app
        .manager
        .create_lightweight_profile(
            "official-stale",
            Some("official-stale"),
            LightweightEnv::default(),
        )
        .unwrap();
    app.manager
        .set_provider(&official_profile.id, &official.id, &official_key_id)
        .unwrap();
    let relay_profile = app
        .manager
        .create_lightweight_profile(
            "relay-stale",
            Some("relay-stale"),
            LightweightEnv::default(),
        )
        .unwrap();
    app.manager
        .set_provider(&relay_profile.id, &relay.id, &relay_key_id)
        .unwrap();

    app.refresh().unwrap();
    for (profile_id, missing_key_id) in [
        (official_profile.id.as_str(), "missing-official-key"),
        (relay_profile.id.as_str(), "missing-relay-key"),
    ] {
        let profile = app
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .unwrap();
        profile.key_id = Some(missing_key_id.into());
    }

    let targets = app.collect_public_site_targets().unwrap();
    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(target.profile_name, "relay-stale");
    assert_eq!(target.provider_id, relay.id);
    assert_eq!(target.base_url, "https://relay.example/api");
    assert!(
        target
            .preflight_error
            .as_deref()
            .is_some_and(|error| { error.contains("references missing key 'missing-relay-key'") })
    );
}

#[test]
fn build_public_site_request_plans_reuses_identical_provider_key_model_prompt() {
    let targets = vec![
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_shared".into(),
            key_name: "Default".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-a".into(),
            profile_name: "profile-a".into(),
            api_key: "sk-shared".into(),
            preflight_error: None,
            configured_model: Some("model-a".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_shared".into(),
            key_name: "Default".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-b".into(),
            profile_name: "profile-b".into(),
            api_key: "sk-shared".into(),
            preflight_error: None,
            configured_model: Some("model-a".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
    ];

    let plans = build_public_site_request_plans(&targets, "Hello");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].consumers.len(), 2);
}

#[test]
fn build_public_site_request_plans_split_on_model_and_key_identity() {
    let same_key_diff_model = vec![
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_shared".into(),
            key_name: "Default".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-a".into(),
            profile_name: "profile-a".into(),
            api_key: "sk-shared".into(),
            preflight_error: None,
            configured_model: Some("model-a".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_shared".into(),
            key_name: "Default".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-b".into(),
            profile_name: "profile-b".into(),
            api_key: "sk-shared".into(),
            preflight_error: None,
            configured_model: Some("model-b".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
    ];
    assert_eq!(
        build_public_site_request_plans(&same_key_diff_model, "Hello").len(),
        2
    );

    let same_base_diff_key = vec![
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_one".into(),
            key_name: "Key One".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-a".into(),
            profile_name: "profile-a".into(),
            api_key: "sk-one".into(),
            preflight_error: None,
            configured_model: Some("model-a".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
        PublicSiteTarget {
            provider_id: "prov_shared".into(),
            provider_name: "Relay".into(),
            key_id: "key_two".into(),
            key_name: "Key Two".into(),
            base_url: "https://relay.example/api".into(),
            profile_id: "profile-b".into(),
            profile_name: "profile-b".into(),
            api_key: "sk-two".into(),
            preflight_error: None,
            configured_model: Some("model-a".into()),
            model_source: PublicSiteModelSource::ExplicitModel,
        },
    ];
    assert_eq!(
        build_public_site_request_plans(&same_base_diff_key, "Hello").len(),
        2
    );
}

#[test]
fn public_site_model_from_profile_prefers_haiku_and_strips_1m_suffix() {
    let profile = Profile {
        id: "profile-generated".into(),
        name: "profile-generated".into(),
        alias: Some("profile-generated".into()),
        added: chrono::Utc::now(),
        last_used: None,
        kind: ProfileKind::Lightweight,
        env: Some(LightweightEnv {
            model: Some("claude-3-7-sonnet[1m]".into()),
            default_sonnet_model: Some("claude-default-sonnet[1m]".into()),
            default_haiku_model: Some("claude-default-haiku[1m]".into()),
            ..Default::default()
        }),
        launch_args: None,
        provider_id: None,
        key_id: None,
        mcp_server_ids: Vec::new(),
    };

    let (source, model) = public_site_model_from_profile(&profile);
    assert_eq!(source, PublicSiteModelSource::DefaultHaiku);
    assert_eq!(model.as_deref(), Some("claude-default-haiku"));
}

#[test]
fn public_site_model_from_profile_falls_back_to_explicit_model_when_haiku_missing() {
    let profile = Profile {
        id: "profile-generated".into(),
        name: "profile-generated".into(),
        alias: Some("profile-generated".into()),
        added: chrono::Utc::now(),
        last_used: None,
        kind: ProfileKind::Lightweight,
        env: Some(LightweightEnv {
            model: Some("claude-3-7-sonnet[1m]".into()),
            default_sonnet_model: Some("claude-default-sonnet[1m]".into()),
            ..Default::default()
        }),
        launch_args: None,
        provider_id: None,
        key_id: None,
        mcp_server_ids: Vec::new(),
    };

    let (source, model) = public_site_model_from_profile(&profile);
    assert_eq!(source, PublicSiteModelSource::ExplicitModel);
    assert_eq!(model.as_deref(), Some("claude-3-7-sonnet"));
}

#[test]
fn public_site_provider_test_slot_keys_map_to_model_slots() {
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('h'), KeyModifiers::empty()),
        Some(PublicSiteProviderTestSlot::Haiku)
    );
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('s'), KeyModifiers::empty()),
        Some(PublicSiteProviderTestSlot::Sonnet)
    );
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('o'), KeyModifiers::empty()),
        Some(PublicSiteProviderTestSlot::Opus)
    );
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('m'), KeyModifiers::empty()),
        Some(PublicSiteProviderTestSlot::Model)
    );
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('a'), KeyModifiers::empty()),
        Some(PublicSiteProviderTestSlot::Subagent)
    );
    assert_eq!(
        public_site_provider_test_slot_from_key(KeyCode::Char('h'), KeyModifiers::CONTROL),
        None
    );
}

#[test]
fn public_site_provider_test_model_from_profile_reads_each_lite_slot() {
    let profile = Profile {
        id: "profile-generated".into(),
        name: "profile-generated".into(),
        alias: Some("profile-generated".into()),
        added: chrono::Utc::now(),
        last_used: None,
        kind: ProfileKind::Lightweight,
        env: Some(LightweightEnv {
            default_opus_model: Some("claude-opus[1m]".into()),
            default_sonnet_model: Some("claude-sonnet[1m]".into()),
            default_haiku_model: Some("claude-haiku[1m]".into()),
            model: Some("claude-model[1m]".into()),
            subagent_model: Some("claude-subagent[1m]".into()),
            ..Default::default()
        }),
        launch_args: None,
        provider_id: None,
        key_id: None,
        mcp_server_ids: Vec::new(),
    };

    assert_eq!(
        public_site_provider_test_model_from_profile(&profile, PublicSiteProviderTestSlot::Haiku)
            .as_deref(),
        Some("claude-haiku")
    );
    assert_eq!(
        public_site_provider_test_model_from_profile(&profile, PublicSiteProviderTestSlot::Sonnet)
            .as_deref(),
        Some("claude-sonnet")
    );
    assert_eq!(
        public_site_provider_test_model_from_profile(&profile, PublicSiteProviderTestSlot::Opus)
            .as_deref(),
        Some("claude-opus")
    );
    assert_eq!(
        public_site_provider_test_model_from_profile(&profile, PublicSiteProviderTestSlot::Model)
            .as_deref(),
        Some("claude-model")
    );
    assert_eq!(
        public_site_provider_test_model_from_profile(
            &profile,
            PublicSiteProviderTestSlot::Subagent
        )
        .as_deref(),
        Some("claude-subagent")
    );
}

#[test]
fn public_site_result_detail_lines_split_multiline_fields() {
    let result = PublicSiteTestResult {
        provider_name: "Inline".into(),
        key_name: "Inline".into(),
        base_url: "https://relay.inline.example/api".into(),
        profile_id: "profile-inline-relay".into(),
        profile_name: "inline-relay".into(),
        model: "inline-model".into(),
        first_char: Some("H".into()),
        response_preview: Some("Hello world".into()),
        endpoint_used: Some("https://relay.inline.example/v1/messages".into()),
        latency_ms: Some(4321),
        input_tokens: Some(4),
        output_tokens: Some(8),
        is_success: false,
        error: Some("generated multiline error body".into()),
    };

    let lines = public_site_result_detail_lines(&result);
    assert!(lines.len() > 3, "{lines:?}");
    assert!(lines.iter().any(|line| line.contains("Provider: Inline")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Error: generated multiline error body"))
    );
}

#[test]
fn public_site_result_detail_lines_prioritize_error_for_failures() {
    let result = PublicSiteTestResult {
        provider_name: "SZC".into(),
        key_name: "Default".into(),
        base_url: "https://api.szc.asia".into(),
        profile_id: "profile-szc-gpt".into(),
        profile_name: "szc-gpt".into(),
        model: "gpt-5.5".into(),
        first_char: None,
        response_preview: None,
        endpoint_used: None,
        latency_ms: Some(466),
        input_tokens: None,
        output_tokens: None,
        is_success: false,
        error: Some("generated upstream unauthorized body".into()),
    };

    let lines = public_site_result_detail_lines(&result);
    assert_eq!(lines[0], "Error: generated upstream unauthorized body");
    assert_eq!(lines[1], "");
    let provider_pos = lines
        .iter()
        .position(|line| line.starts_with("Provider: "))
        .unwrap();
    assert!(provider_pos > 1, "{lines:?}");
}

#[test]
fn public_site_detail_scroll_limit_counts_wrapped_error_lines() {
    let lines = vec![
            "Error: generated upstream unauthorized body with a very long detail string that should wrap across multiple visual rows in the popup body.".to_string(),
            String::new(),
            "Provider: SZC".to_string(),
        ];

    let limit = public_site_detail_scroll_limit(&lines, 24, 4);
    assert!(
        limit > 0,
        "expected wrapped lines to require scrolling, got {limit}"
    );
}

#[test]
fn public_site_results_sort_success_then_latency_then_first_char() {
    let mut results = vec![
        PublicSiteTestResult {
            provider_name: "relay-c".into(),
            key_name: "key-c".into(),
            base_url: "https://relay-c.example".into(),
            profile_id: "profile-c".into(),
            profile_name: "profile-c".into(),
            model: "model-c".into(),
            first_char: Some("B".into()),
            response_preview: Some("Bravo".into()),
            endpoint_used: Some("https://relay-c.example/v1/messages".into()),
            latency_ms: Some(320),
            input_tokens: Some(4),
            output_tokens: Some(8),
            is_success: true,
            error: None,
        },
        PublicSiteTestResult {
            provider_name: "relay-a".into(),
            key_name: "key-a".into(),
            base_url: "https://relay-a.example".into(),
            profile_id: "profile-a".into(),
            profile_name: "profile-a".into(),
            model: "model-a".into(),
            first_char: Some("A".into()),
            response_preview: Some("Alpha".into()),
            endpoint_used: Some("https://relay-a.example/v1/messages".into()),
            latency_ms: Some(320),
            input_tokens: Some(4),
            output_tokens: Some(8),
            is_success: true,
            error: None,
        },
        PublicSiteTestResult {
            provider_name: "relay-z".into(),
            key_name: "key-z".into(),
            base_url: "https://relay-z.example".into(),
            profile_id: "profile-z".into(),
            profile_name: "profile-z".into(),
            model: "model-z".into(),
            first_char: None,
            response_preview: None,
            endpoint_used: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            is_success: false,
            error: Some("timeout".into()),
        },
        PublicSiteTestResult {
            provider_name: "relay-b".into(),
            key_name: "key-b".into(),
            base_url: "https://relay-b.example".into(),
            profile_id: "profile-b".into(),
            profile_name: "profile-b".into(),
            model: "model-b".into(),
            first_char: Some("Z".into()),
            response_preview: Some("Zulu".into()),
            endpoint_used: Some("https://relay-b.example/v1/messages".into()),
            latency_ms: Some(810),
            input_tokens: Some(4),
            output_tokens: Some(8),
            is_success: true,
            error: None,
        },
    ];

    sort_public_site_results(&mut results);

    let ordered: Vec<&str> = results
        .iter()
        .map(|result| result.provider_name.as_str())
        .collect();
    assert_eq!(ordered, vec!["relay-a", "relay-c", "relay-b", "relay-z"]);
}

#[test]
fn public_site_request_timeout_is_16_seconds() {
    assert_eq!(public_site_request_timeout(), Duration::from_secs(16));
}

#[test]
fn public_site_batch_with_only_preflight_errors_finishes_immediately() {
    let mut app = make_test_app();
    app.public_site_prompt_buf = "Hello".into();
    app.public_site_targets = vec![PublicSiteTarget {
        provider_id: "prov".into(),
        provider_name: "Relay".into(),
        key_id: "key".into(),
        key_name: "Default".into(),
        base_url: "https://relay.example/api".into(),
        profile_id: "profile".into(),
        profile_name: "profile".into(),
        api_key: String::new(),
        preflight_error: Some("No resolved auth token/key for this profile.".into()),
        configured_model: Some("claude-test".into()),
        model_source: PublicSiteModelSource::ExplicitModel,
    }];

    app.start_public_site_batch_test();

    assert_eq!(app.mode, Mode::PublicSiteResults);
    assert_eq!(app.public_site_total, 1);
    assert_eq!(app.public_site_completed, 1);
    assert_eq!(app.public_site_results.len(), 1);
    assert!(app.public_site_event_rx.is_none());
    assert_eq!(app.public_site_status, "Finished 1 provider-key tests");
}

#[test]
fn public_site_result_detail_preserves_full_error_text() {
    let result = PublicSiteTestResult {
            provider_name: "relay-z".into(),
            key_name: "key-z".into(),
            base_url: "https://relay-z.example".into(),
            profile_id: "profile-z".into(),
            profile_name: "profile-z".into(),
            model: "model-z".into(),
            first_char: None,
            response_preview: None,
            endpoint_used: None,
            latency_ms: Some(1234),
            input_tokens: None,
            output_tokens: None,
            is_success: false,
            error: Some(
                "Anthropic test failed with HTTP 401 at https://relay-z.example/v1/messages: generated unauthorized body with extra detail"
                    .into(),
            ),
        };

    let detail = public_site_result_detail(&result);
    assert!(detail.contains("HTTP 401"));
    assert!(detail.contains("generated unauthorized body with extra detail"));
    assert!(!detail.contains("..."));
}

#[test]
fn public_site_results_ctrl_d_scrolls_detail_and_selection_change_resets_scroll() {
    let mut app = make_test_app();
    app.mode = Mode::PublicSiteResults;
    app.public_site_results = vec![
            PublicSiteTestResult {
                provider_name: "SZC".into(),
                key_name: "Default".into(),
                base_url: "https://api.szc.asia".into(),
                profile_id: "profile-szc-gpt".into(),
                profile_name: "szc-gpt".into(),
                model: "gpt-5.5".into(),
                first_char: None,
                response_preview: None,
                endpoint_used: None,
                latency_ms: Some(466),
                input_tokens: None,
                output_tokens: None,
                is_success: false,
                error: Some(
                    "generated upstream unauthorized body with a long detail string that needs to scroll in the popup body"
                        .into(),
                ),
            },
            PublicSiteTestResult {
                provider_name: "ABRDNS".into(),
                key_name: "Custom".into(),
                base_url: "https://new-api.abrdns.com".into(),
                profile_id: "profile-abrdns-gpt".into(),
                profile_name: "abrdns-gpt".into(),
                model: "gpt-5.5".into(),
                first_char: None,
                response_preview: None,
                endpoint_used: None,
                latency_ms: Some(395),
                input_tokens: None,
                output_tokens: None,
                is_success: false,
                error: Some("generated rate-limit detail".into()),
            },
        ];

    app.handle_public_site_results(KeyCode::Char('d'), KeyModifiers::CONTROL)
        .unwrap();
    assert!(app.public_site_detail_scroll > 0);

    app.handle_public_site_results(KeyCode::Down, KeyModifiers::empty())
        .unwrap();
    assert_eq!(app.public_site_result_selected, 1);
    assert_eq!(app.public_site_detail_scroll, 0);
}

#[test]
fn execute_public_site_target_with_timeout_respects_custom_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _request = read_http_request(&mut stream);
        thread::sleep(Duration::from_millis(150));
        let response_body = serde_json::json!({
            "content": [
                { "type": "text", "text": "Hello from generated test server" }
            ],
            "usage": {
                "input_tokens": 7,
                "output_tokens": 11
            }
        })
        .to_string();
        write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
    });

    let target = PublicSiteTarget {
        provider_id: "prov_generated".into(),
        provider_name: "Relay".into(),
        key_id: "key_generated".into(),
        key_name: "Default".into(),
        base_url: format!("http://{}", addr),
        profile_id: "profile_generated".into(),
        profile_name: "relay-profile".into(),
        api_key: "sk-test-generated-key-timeout-111111111111111".into(),
        preflight_error: None,
        configured_model: Some("claude-test-generated-model".into()),
        model_source: PublicSiteModelSource::ExplicitModel,
    };

    let result = execute_public_site_target_with_timeout(&target, "Hello", Duration::from_secs(1));
    handle.join().unwrap();

    assert!(result.is_success, "{result:?}");
    assert_eq!(
        result.response_preview.as_deref(),
        Some("Hello from generated test server")
    );
}

#[test]
fn execute_public_site_target_returns_preflight_error_without_network() {
    let target = PublicSiteTarget {
        provider_id: String::new(),
        provider_name: "Inline".into(),
        key_id: String::new(),
        key_name: "Inline".into(),
        base_url: String::new(),
        profile_id: "profile_generated".into(),
        profile_name: "inline-profile".into(),
        api_key: String::new(),
        preflight_error: Some("No resolved auth token/key for this profile.".into()),
        configured_model: Some("inline-model".into()),
        model_source: PublicSiteModelSource::ExplicitModel,
    };

    let result = execute_public_site_target_with_timeout(&target, "Hello", Duration::from_secs(1));

    assert!(!result.is_success);
    assert_eq!(
        result.error.as_deref(),
        Some("No resolved auth token/key for this profile.")
    );
    assert_eq!(result.latency_ms, None);
}

#[test]
fn profile_manager_public_site_shortcut_opens_prompt() {
    let mut app = make_test_app();
    let relay = app
        .manager
        .add_provider_with_key_name(
            "Relay",
            "https://relay.example/api",
            "Default",
            "sk-test-generated-key-relay-3333333333333333",
        )
        .unwrap();
    let relay_key = relay.keys.values().next().unwrap().clone();
    let relay_profile = app
        .manager
        .create_lightweight_profile(
            "relay-explicit",
            Some("relay-explicit"),
            LightweightEnv {
                model: Some("relay-explicit-model".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&relay_profile.id, &relay.id, &relay_key.id)
        .unwrap();
    app.refresh().unwrap();

    app.handle_profile_page_key(KeyCode::Char('T'), KeyModifiers::SHIFT)
        .unwrap();

    assert_eq!(app.mode, Mode::PublicSitePrompt);
    assert_eq!(app.public_site_prompt_buf, "Hello");
    assert_eq!(app.public_site_targets.len(), 1);
    assert_eq!(app.cursor_pos, app.public_site_prompt_buf.len());
}

#[test]
fn public_site_slot_key_opens_provider_test_with_profile_model() {
    let (base_url, handle) = spawn_model_discovery_server(vec!["fetched-model"]);
    let mut app = make_test_app();
    let relay = app
        .manager
        .add_provider_with_key_name(
            "Relay",
            &base_url,
            "Default",
            "sk-test-generated-key-relay-4444444444444444",
        )
        .unwrap();
    let relay_key = relay.keys.values().next().unwrap().clone();
    let relay_profile = app
        .manager
        .create_lightweight_profile(
            "relay-slots",
            Some("relay-slots"),
            LightweightEnv {
                default_haiku_model: Some("configured-haiku[1m]".into()),
                default_sonnet_model: Some("configured-sonnet".into()),
                default_opus_model: Some("configured-opus".into()),
                model: Some("configured-model".into()),
                subagent_model: Some("configured-subagent".into()),
                ..Default::default()
            },
        )
        .unwrap();
    app.manager
        .set_provider(&relay_profile.id, &relay.id, &relay_key.id)
        .unwrap();
    app.refresh().unwrap();
    app.mode = Mode::PublicSiteResults;
    app.public_site_results = vec![PublicSiteTestResult {
        provider_name: "Relay".into(),
        key_name: "Default".into(),
        base_url: base_url.clone(),
        profile_id: relay_profile.id.clone(),
        profile_name: "relay-slots".into(),
        model: "configured-haiku".into(),
        first_char: Some("H".into()),
        response_preview: Some("Hello".into()),
        endpoint_used: Some(format!("{base_url}/v1/messages")),
        latency_ms: Some(120),
        input_tokens: Some(4),
        output_tokens: Some(8),
        is_success: true,
        error: None,
    }];

    app.handle_public_site_results(KeyCode::Char('h'), KeyModifiers::empty())
        .unwrap();
    handle.join().unwrap();

    assert_eq!(app.provider_test_model_buf, "configured-haiku");
    assert_eq!(app.provider_test_prompt_buf, "Hello");
    assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
    assert_eq!(app.provider_test_models, vec!["fetched-model".to_string()]);
    assert_eq!(
        app.mode,
        Mode::ProviderAnthropicTest {
            provider_id: relay.id,
            key_id: relay_key.id,
            source: ProviderTestSource::PublicSite,
            field: 0,
        }
    );
}

#[test]
fn trim_model_context_suffix_removes_1m_suffix() {
    assert_eq!(
        trim_model_context_suffix("claude-3-7-sonnet[1m]"),
        "claude-3-7-sonnet"
    );
    assert_eq!(
        trim_model_context_suffix("claude-3-7-sonnet"),
        "claude-3-7-sonnet"
    );
}

#[test]
fn apply_model_1m_flag_normalizes_suffix() {
    assert_eq!(
        apply_model_1m_flag("claude-3-7-sonnet[1m]", false),
        "claude-3-7-sonnet"
    );
    assert_eq!(
        apply_model_1m_flag("claude-3-7-sonnet", true),
        "claude-3-7-sonnet[1m]"
    );
    assert_eq!(
        apply_model_1m_flag("claude-3-7-sonnet[1m]", true),
        "claude-3-7-sonnet[1m]"
    );
}

#[test]
fn complete_provider_test_model_prefers_exact_then_fuzzy_match() {
    let models = vec![
        "LongCat-2.0-Preview".to_string(),
        "deepseek-ai/deepseek-v4-flash".to_string(),
        "claude-3-7-sonnet".to_string(),
    ];

    assert_eq!(
        complete_provider_test_model(&models, ""),
        Some("LongCat-2.0-Preview".to_string())
    );
    assert_eq!(
        complete_provider_test_model(&models, "claude-3-7-sonnet"),
        Some("claude-3-7-sonnet".to_string())
    );
    assert_eq!(
        complete_provider_test_model(&models, "deepseek-v4"),
        Some("deepseek-ai/deepseek-v4-flash".to_string())
    );
}

#[test]
fn provider_test_q_is_text_input() {
    let mut app = make_test_app();
    app.provider_test_prompt_buf = "quic".into();
    app.cursor_pos = app.provider_test_prompt_buf.len();
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 1,
    };

    app.handle_provider_anthropic_test(KeyCode::Char('q'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.provider_test_prompt_buf, "quicq");
    assert_eq!(app.cursor_pos, app.provider_test_prompt_buf.len());
    assert_eq!(
        app.mode,
        Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 1,
        }
    );
}

#[test]
fn provider_test_ctrl_n_moves_to_prompt_field() {
    let mut app = make_test_app();
    app.provider_test_models = vec!["model-a".into(), "model-b".into()];
    app.provider_test_model_buf = "model-a".into();
    app.provider_test_prompt_buf = "Hello".into();
    app.provider_test_model_selected = 0;
    app.cursor_pos = app.provider_test_model_buf.len();
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 0,
    };

    app.handle_provider_anthropic_test(KeyCode::Char('n'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(app.provider_test_model_selected, 0);
    assert_eq!(app.provider_test_model_buf, "model-a");
    assert_eq!(app.cursor_pos, app.provider_test_prompt_buf.len());
    assert_eq!(
        app.mode,
        Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 1,
        }
    );
}

#[test]
fn provider_test_model_field_accepts_bare_j_and_k() {
    let mut app = make_test_app();
    app.provider_test_models = vec!["model-a".into(), "model-b".into()];
    app.provider_test_model_buf.clear();
    app.provider_test_model_selected = 0;
    app.cursor_pos = 0;
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 0,
    };

    app.handle_provider_anthropic_test(KeyCode::Char('j'), KeyModifiers::empty())
        .unwrap();
    app.handle_provider_anthropic_test(KeyCode::Char('k'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.provider_test_model_buf, "jk");
    assert_eq!(app.provider_test_model_selected, 0);
}

#[test]
fn provider_test_model_field_down_still_navigates_models() {
    let mut app = make_test_app();
    app.provider_test_models = vec!["model-a".into(), "model-b".into()];
    app.provider_test_model_buf = "model-a".into();
    app.provider_test_model_selected = 0;
    app.cursor_pos = app.provider_test_model_buf.len();
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 0,
    };

    app.handle_provider_anthropic_test(KeyCode::Down, KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.provider_test_model_buf, "model-b");
    assert_eq!(app.provider_test_model_selected, 1);
    assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
}

#[test]
fn provider_test_manual_model_match_syncs_selection() {
    let mut app = make_test_app();
    app.provider_test_models = vec!["model-a".into(), "model-b".into(), "model-c".into()];
    app.provider_test_model_buf = "model-a".into();
    app.provider_test_model_selected = 0;
    app.cursor_pos = app.provider_test_model_buf.len();
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 0,
    };

    app.handle_provider_anthropic_test(KeyCode::Char('u'), KeyModifiers::CONTROL)
        .unwrap();
    for ch in "model-c".chars() {
        app.handle_provider_anthropic_test(KeyCode::Char(ch), KeyModifiers::empty())
            .unwrap();
    }

    assert_eq!(app.provider_test_model_buf, "model-c");
    assert_eq!(app.provider_test_model_selected, 2);
}

#[test]
fn lite_set_slot_value_moves_cursor_to_end() {
    let mut app = make_test_app();
    app.lite_step = 5;
    app.cursor_pos = 0;

    super::lite::set_slot_value(&mut app, "claude-sonnet".into());

    assert_eq!(app.lite_mod_model, "claude-sonnet");
    assert_eq!(app.cursor_pos, app.lite_mod_model.len());
}

#[test]
fn lite_fetching_ctrl_g_cancels() {
    let mut app = make_test_app();
    app.mode = Mode::LiteFetching;

    match app.mode.clone() {
        Mode::LiteFetching => {
            if App::is_cancel_key(KeyCode::Char('g'), KeyModifiers::CONTROL) {
                app.mode = Mode::Normal;
            }
        }
        _ => unreachable!(),
    }

    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn lite_creation_selects_provider_key_and_persists_shared_reference() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://provider.example.invalid",
            "First",
            "sk-first",
        )
        .unwrap();
    let second_key = app
        .manager
        .add_key(&provider.id, "Second", "sk-second")
        .unwrap();

    super::lite::start_lite_profile_creation(&mut app).unwrap();
    assert_eq!(app.mode, Mode::LiteProviderSelect);
    super::lite::handle_lite_provider_select(&mut app, KeyCode::Enter, KeyModifiers::empty())
        .unwrap();
    assert_eq!(
        app.mode,
        Mode::LiteKeySelect {
            provider_id: provider.id.clone(),
        }
    );
    app.move_provider_key_down();
    assert_eq!(
        app.selected_provider_key().map(|key| key.id.as_str()),
        Some(second_key.id.as_str())
    );
    super::lite::handle_lite_key_select(&mut app, KeyCode::Enter, KeyModifiers::empty()).unwrap();
    assert!(matches!(app.mode, Mode::LiteModelSelect { .. }));

    app.lite_name = "shared-lite".into();
    app.lite_alias = "sl".into();
    app.lite_mod_model = "claude-sonnet".into();
    super::lite::handle_lite_model_select(&mut app, KeyCode::Enter, KeyModifiers::empty()).unwrap();

    let created = app.manager.get_profile("sl").unwrap();
    assert_eq!(created.provider_id.as_deref(), Some(provider.id.as_str()));
    assert_eq!(created.key_id.as_deref(), Some(second_key.id.as_str()));
    let (token, url) = app.manager.resolve_credentials(&created).unwrap();
    assert_eq!(token.as_deref(), Some("sk-second"));
    assert_eq!(url.as_deref(), Some("https://provider.example.invalid"));
}

#[test]
fn lite_creation_without_provider_shows_error_message() {
    let mut app = make_test_app();

    super::lite::start_lite_profile_creation(&mut app).unwrap();

    assert_eq!(
        app.mode,
        Mode::Message(
            "No providers found. Add one in Provider Manager first.".to_string(),
            true,
        )
    );
}

#[test]
fn lite_creation_provider_without_keys_shows_error_message() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider(
            "Empty Provider",
            "https://empty.example.invalid",
            "sk-empty",
        )
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap().clone();
    app.manager.remove_key(&provider.id, &key_id).unwrap();

    super::lite::start_lite_profile_creation(&mut app).unwrap();
    super::lite::handle_lite_provider_select(&mut app, KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::Message(
            "Provider 'Empty Provider' has no keys. Add a key in Provider Manager first."
                .to_string(),
            true,
        )
    );
}

#[test]
fn provider_test_outcome_non_q_returns_to_same_form() {
    assert_eq!(
        provider_test_outcome_next_mode(
            KeyCode::Enter,
            KeyModifiers::empty(),
            "prov_generated",
            "key_generated",
            ProviderTestSource::KeyList,
            1
        ),
        Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::KeyList,
            field: 1,
        }
    );
}

#[test]
fn provider_test_outcome_q_exits_to_parent_mode() {
    assert_eq!(
        provider_test_outcome_next_mode(
            KeyCode::Char('q'),
            KeyModifiers::empty(),
            "prov_generated",
            "key_generated",
            ProviderTestSource::KeyList,
            0
        ),
        Mode::ProviderKeyList {
            provider_id: "prov_generated".into(),
        }
    );
    assert_eq!(
        provider_test_outcome_next_mode(
            KeyCode::Char('q'),
            KeyModifiers::empty(),
            "prov_generated",
            "key_generated",
            ProviderTestSource::TestKeyList,
            0
        ),
        Mode::ProviderTestKeyList {
            provider_id: "prov_generated".into(),
        }
    );
    assert_eq!(
        provider_test_outcome_next_mode(
            KeyCode::Char('q'),
            KeyModifiers::empty(),
            "prov_generated",
            "key_generated",
            ProviderTestSource::Page,
            0
        ),
        Mode::Normal
    );
}

#[test]
fn model_fetch_unavailable_message_marks_manual_entry_possible() {
    let message = model_fetch_unavailable_message("403 forbidden");

    assert!(message.contains("/v1/models unavailable"));
    assert!(message.contains("Manual model entry still works"));
}

#[test]
fn model_fetch_state_for_empty_models_is_empty() {
    assert_eq!(model_fetch_state_for_models(&[]), ModelFetchState::Empty);
}

#[test]
fn model_fetch_state_for_non_empty_models_is_loaded() {
    assert_eq!(
        model_fetch_state_for_models(&["claude-3-7-sonnet".to_string()]),
        ModelFetchState::Loaded
    );
}

#[test]
fn provider_edit_cursor_pos_tracks_active_field() {
    assert_eq!(
        provider_edit_cursor_pos(0, "Provider Name", "https://example.invalid"),
        "Provider Name".len()
    );
    assert_eq!(
        provider_edit_cursor_pos(1, "Provider Name", "https://example.invalid"),
        "https://example.invalid".len()
    );
    assert_eq!(
        provider_edit_cursor_pos(2, "Provider Name", "https://example.invalid"),
        0
    );
}

#[test]
fn shift_tab_switches_manager_in_allowed_modes() {
    let mut app = make_test_app();
    app.mode = Mode::Normal;
    app.page = Page::Profile;

    assert!(
        app.handle_manager_switch_key(KeyCode::BackTab, KeyModifiers::empty())
            .unwrap()
    );
    assert_eq!(app.page, Page::Provider);
    assert_eq!(app.mode, Mode::Normal);

    app.mode = Mode::Search;
    assert!(
        app.handle_manager_switch_key(KeyCode::Tab, KeyModifiers::SHIFT)
            .unwrap()
    );
    assert_eq!(app.page, Page::Mcp);
    assert_eq!(app.mode, Mode::Normal);

    assert!(
        app.handle_manager_switch_key(KeyCode::BackTab, KeyModifiers::empty())
            .unwrap()
    );
    assert_eq!(app.page, Page::Profile);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn shift_tab_does_not_switch_manager_in_edit_mode() {
    let mut app = make_test_app();
    app.mode = Mode::EditProfile {
        profile_id: "profile-generated".into(),
        step: 1,
    };
    app.page = Page::Profile;

    assert!(
        !app.handle_manager_switch_key(KeyCode::BackTab, KeyModifiers::empty())
            .unwrap()
    );
    assert_eq!(app.page, Page::Profile);
    assert_eq!(
        app.mode,
        Mode::EditProfile {
            profile_id: "profile-generated".into(),
            step: 1,
        }
    );
}

#[test]
fn search_input_accepts_bare_j_and_k() {
    let mut app = make_test_app();
    app.mode = Mode::Search;

    app.handle_search_key(KeyCode::Char('j'), KeyModifiers::empty())
        .unwrap();
    app.handle_search_key(KeyCode::Char('k'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.search_query, "jk");
    assert_eq!(app.cursor_pos, app.search_query.len());
}

#[test]
fn mcp_profile_picker_saves_selected_mcps_for_lightweight_profile() {
    let mut app = make_test_app();
    let mcp = app
        .manager
        .add_mcp_server(McpServerInput {
            name: "codex-sessions".into(),
            server_type: "stdio".into(),
            command: Some("codex-sessions-mcp".into()),
            ..Default::default()
        })
        .unwrap();
    let profile = app
        .manager
        .create_lightweight_profile("lite", Some("lite-mcp-tui"), LightweightEnv::default())
        .unwrap();
    app.refresh().unwrap();
    app.select_by_id(&profile.id);

    app.start_selected_profile_mcp_picker().unwrap();
    assert!(matches!(app.mode, Mode::McpProfilePicker { .. }));
    app.handle_mcp_profile_picker(KeyCode::Char(' '), KeyModifiers::empty())
        .unwrap();
    app.handle_mcp_profile_picker(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    let updated = app.manager.get_profile(&profile.id).unwrap();
    assert_eq!(updated.mcp_server_ids, vec![mcp.id]);
}

#[test]
fn mcp_profile_picker_filter_accepts_bare_j_and_k() {
    let mut app = make_test_app();
    app.manager
        .add_mcp_server(McpServerInput {
            name: "json-kernel".into(),
            server_type: "stdio".into(),
            command: Some("json-kernel-mcp".into()),
            ..Default::default()
        })
        .unwrap();
    let profile = app
        .manager
        .create_lightweight_profile("lite", Some("lite-filter"), LightweightEnv::default())
        .unwrap();
    app.refresh().unwrap();
    app.select_by_id(&profile.id);
    app.start_selected_profile_mcp_picker().unwrap();

    app.handle_mcp_profile_picker(KeyCode::Char('j'), KeyModifiers::empty())
        .unwrap();
    app.handle_mcp_profile_picker(KeyCode::Char('k'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.mcp_filter_buf, "jk");
    assert_eq!(app.cursor_pos, app.mcp_filter_buf.len());
}

#[test]
fn mcp_editor_adds_stdio_server() {
    let mut app = make_test_app();
    app.reset_mcp_editor();
    app.mode = Mode::McpAdd { step: 0 };
    app.mcp_name_buf = "codex-sessions".into();
    app.mcp_command_buf = "codex-sessions-mcp".into();

    app.handle_mcp_editor(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    let servers = app.manager.list_mcp_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "codex-sessions");
    assert_eq!(servers[0].command.as_deref(), Some("codex-sessions-mcp"));
}

#[test]
fn provider_edit_tab_and_ctrl_n_advance_fields() {
    let mut app = make_test_app();
    app.page = Page::Provider;
    app.provider_name_buf = "Provider Name".into();
    app.provider_url_buf = "https://example.invalid".into();
    app.mode = Mode::ProviderEdit {
        provider_id: "prov_generated".into(),
        step: 0,
    };

    app.handle_provider_edit(KeyCode::Tab, KeyModifiers::empty())
        .unwrap();
    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: "prov_generated".into(),
            step: 1,
        }
    );
    assert_eq!(app.cursor_pos, "https://example.invalid".len());

    app.mode = Mode::ProviderEdit {
        provider_id: "prov_generated".into(),
        step: 0,
    };
    app.cursor_pos = app.provider_name_buf.len();

    app.handle_provider_edit(KeyCode::Char('n'), KeyModifiers::CONTROL)
        .unwrap();
    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: "prov_generated".into(),
            step: 1,
        }
    );
    assert_eq!(app.cursor_pos, app.provider_url_buf.len());
}

#[test]
fn provider_edit_ctrl_p_moves_to_previous_field() {
    let mut app = make_test_app();
    app.page = Page::Provider;
    app.provider_name_buf = "Provider Name".into();
    app.provider_url_buf = "https://example.invalid".into();
    app.mode = Mode::ProviderEdit {
        provider_id: "prov_generated".into(),
        step: 1,
    };

    app.handle_provider_edit(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: "prov_generated".into(),
            step: 0,
        }
    );
    assert_eq!(app.cursor_pos, app.provider_name_buf.len());
}

#[test]
fn provider_edit_bare_n_is_text_input() {
    let mut app = make_test_app();
    app.page = Page::Provider;
    app.provider_name_buf = "Provide".into();
    app.provider_url_buf = "https://example.invalid".into();
    app.cursor_pos = app.provider_name_buf.len();
    app.mode = Mode::ProviderEdit {
        provider_id: "prov_generated".into(),
        step: 0,
    };

    app.handle_provider_edit(KeyCode::Char('n'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(app.provider_name_buf, "Providen");
    assert_eq!(app.cursor_pos, app.provider_name_buf.len());
    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: "prov_generated".into(),
            step: 0,
        }
    );
}

#[test]
fn provider_test_ctrl_p_moves_to_previous_field() {
    let mut app = make_test_app();
    app.provider_test_model_buf = "claude-3-7-sonnet".into();
    app.provider_test_prompt_buf = "Hello".into();
    app.mode = Mode::ProviderAnthropicTest {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        source: ProviderTestSource::Page,
        field: 1,
    };

    app.handle_provider_anthropic_test(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderAnthropicTest {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            source: ProviderTestSource::Page,
            field: 0,
        }
    );
    assert_eq!(app.cursor_pos, app.provider_test_model_buf.len());
}

#[test]
fn provider_edit_cancel_does_not_save_changes() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Original Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-777777777777777777777777",
        )
        .unwrap();
    app.providers_cache = app.manager.list_providers().unwrap();
    app.page = Page::Provider;
    app.provider_name_buf = "Changed Provider".into();
    app.provider_url_buf = "https://changed.invalid".into();
    app.mode = Mode::ProviderEdit {
        provider_id: provider.id.clone(),
        step: 0,
    };

    app.handle_provider_edit(KeyCode::Esc, KeyModifiers::empty())
        .unwrap();

    let refreshed = app.manager.get_provider(&provider.id).unwrap();
    assert_eq!(refreshed.name, "Original Provider");
    assert_eq!(refreshed.base_url, "https://example.invalid");
}

#[test]
fn edit_profile_down_no_longer_advances_fields() {
    let mut app = make_test_app();
    app.mode = Mode::EditProfile {
        profile_id: "profile-generated".into(),
        step: 0,
    };

    app.handle_edit_profile(KeyCode::Down, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::EditProfile {
            profile_id: "profile-generated".into(),
            step: 0,
        }
    );
}

#[test]
fn provider_key_add_ctrl_n_advances_fields() {
    let mut app = make_test_app();
    app.mode = Mode::ProviderKeyAdd {
        provider_id: "prov_generated".into(),
        step: 0,
    };

    app.handle_provider_key_add(KeyCode::Char('n'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyAdd {
            provider_id: "prov_generated".into(),
            step: 1,
        }
    );
}

#[test]
fn provider_key_edit_ctrl_p_moves_to_previous_field() {
    let mut app = make_test_app();
    app.mode = Mode::ProviderKeyEdit {
        provider_id: "prov_generated".into(),
        key_id: "key_generated".into(),
        step: 1,
        source: KeyEditSource::ProviderKeyList,
    };

    app.handle_provider_key_edit(KeyCode::Char('p'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyEdit {
            provider_id: "prov_generated".into(),
            key_id: "key_generated".into(),
            step: 0,
            source: KeyEditSource::ProviderKeyList,
        }
    );
}

#[test]
fn provider_key_list_r_opens_rename_for_selected_key() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-rename-111111111111111111111",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.provider_key_selected = 0;
    app.mode = Mode::ProviderKeyList {
        provider_id: provider.id.clone(),
    };

    app.handle_provider_key_list(KeyCode::Char('r'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyRename {
            provider_id: provider.id,
            key_id: key.id,
            source: KeyEditSource::ProviderKeyList,
        }
    );
    assert_eq!(app.provider_key_name_buf, "Default");
    assert_eq!(app.cursor_pos, "Default".len());
}

#[test]
fn provider_edit_keys_r_opens_rename_for_selected_key() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-rename-444444444444444444444",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.provider_key_selected = 0;
    app.provider_name_buf = provider.name.clone();
    app.provider_url_buf = provider.base_url.clone();
    app.mode = Mode::ProviderEdit {
        provider_id: provider.id.clone(),
        step: 2,
    };

    app.handle_provider_edit(KeyCode::Char('r'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyRename {
            provider_id: provider.id,
            key_id: key.id,
            source: KeyEditSource::ProviderEdit,
        }
    );
    assert_eq!(app.provider_key_name_buf, "Default");
    assert_eq!(app.cursor_pos, "Default".len());
}

#[test]
fn provider_key_rename_saves_name_without_changing_token() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-rename-222222222222222222222",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    app.provider_key_name_buf = "Team A".into();
    app.cursor_pos = app.provider_key_name_buf.len();
    app.mode = Mode::ProviderKeyRename {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        source: KeyEditSource::ProviderKeyList,
    };

    app.handle_provider_key_rename(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }
    );
    let renamed = app
        .manager
        .list_keys(&provider.id)
        .unwrap()
        .into_iter()
        .find(|stored| stored.id == key.id)
        .unwrap();
    assert_eq!(renamed.name, "Team A");
    assert_eq!(renamed.api_key, key.api_key);
}

#[test]
fn provider_key_rename_keeps_renamed_key_selected_after_resort() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Alpha",
            "sk-test-generated-key-rename-sort-111111111111111",
        )
        .unwrap();
    let key = app
        .manager
        .add_key(
            &provider.id,
            "Bravo",
            "sk-test-generated-key-rename-sort-222222222222222",
        )
        .unwrap();
    app.manager
        .add_key(
            &provider.id,
            "Charlie",
            "sk-test-generated-key-rename-sort-333333333333333",
        )
        .unwrap();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.provider_key_selected = app
        .provider_keys_cache
        .iter()
        .position(|stored| stored.id == key.id)
        .unwrap();
    app.provider_key_name_buf = "Zulu".into();
    app.cursor_pos = app.provider_key_name_buf.len();
    app.mode = Mode::ProviderKeyRename {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        source: KeyEditSource::ProviderKeyList,
    };

    app.handle_provider_key_rename(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }
    );
    assert_eq!(
        app.selected_provider_key().map(|stored| stored.id.as_str()),
        Some(key.id.as_str())
    );
    assert_eq!(
        app.provider_keys_cache
            .iter()
            .map(|stored| stored.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Charlie", "Zulu"]
    );
}

#[test]
fn provider_key_rename_from_provider_edit_returns_to_keys_row() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-rename-555555555555555555555",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    app.provider_key_name_buf = "Team A".into();
    app.cursor_pos = app.provider_key_name_buf.len();
    app.mode = Mode::ProviderKeyRename {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        source: KeyEditSource::ProviderEdit,
    };

    app.handle_provider_key_rename(KeyCode::Enter, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: provider.id.clone(),
            step: 2,
        }
    );
    let renamed = app
        .manager
        .list_keys(&provider.id)
        .unwrap()
        .into_iter()
        .find(|stored| stored.id == key.id)
        .unwrap();
    assert_eq!(renamed.name, "Team A");
    assert_eq!(renamed.api_key, key.api_key);
}

#[test]
fn provider_key_rename_cancel_does_not_save() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-rename-333333333333333333333",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    app.provider_key_name_buf = "Team A".into();
    app.cursor_pos = app.provider_key_name_buf.len();
    app.mode = Mode::ProviderKeyRename {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        source: KeyEditSource::ProviderKeyList,
    };

    app.handle_provider_key_rename(KeyCode::Esc, KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }
    );
    let unchanged = app
        .manager
        .list_keys(&provider.id)
        .unwrap()
        .into_iter()
        .find(|stored| stored.id == key.id)
        .unwrap();
    assert_eq!(unchanged.name, "Default");
    assert_eq!(unchanged.api_key, key.api_key);
}

#[test]
fn confirm_delete_key_opens_linked_profile_popup_when_key_is_in_use() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-888888888888888888888888",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    let profile = app
        .manager
        .create_lightweight_profile("linked-profile", Some("linked"), LightweightEnv::default())
        .unwrap();
    app.manager
        .set_provider(&profile.id, &provider.id, &key.id)
        .unwrap();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.page = Page::Provider;
    app.mode = Mode::ConfirmDeleteKey {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        name: key.name.clone(),
    };

    app.handle_confirm_delete_key(KeyCode::Char('y')).unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyInUse {
            provider_id: provider.id.clone(),
            key_id: key.id.clone(),
            name: key.name.clone(),
            return_mode: Box::new(Mode::ProviderKeyList {
                provider_id: provider.id.clone(),
            }),
        }
    );
    assert_eq!(app.provider_key_linked_profiles.len(), 1);
    assert_eq!(app.provider_key_linked_profiles[0].name, "linked-profile");
}

#[test]
fn provider_edit_delete_key_opens_linked_profile_popup_when_key_is_in_use() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-777777777777777777777777",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    let profile = app
        .manager
        .create_lightweight_profile(
            "linked-provider-edit",
            Some("linked-pe"),
            LightweightEnv::default(),
        )
        .unwrap();
    app.manager
        .set_provider(&profile.id, &provider.id, &key.id)
        .unwrap();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.page = Page::Provider;
    app.provider_name_buf = provider.name.clone();
    app.provider_url_buf = provider.base_url.clone();
    app.mode = Mode::ProviderEdit {
        provider_id: provider.id.clone(),
        step: 2,
    };

    app.handle_provider_edit(KeyCode::Char('d'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyInUse {
            provider_id: provider.id.clone(),
            key_id: key.id.clone(),
            name: key.name.clone(),
            return_mode: Box::new(Mode::ProviderEdit {
                provider_id: provider.id.clone(),
                step: 2,
            }),
        }
    );

    app.handle_provider_key_in_use(KeyCode::Char('d'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderEdit {
            provider_id: provider.id.clone(),
            step: 2,
        }
    );
    assert!(app.manager.get_profile(&profile.id).is_err());
    assert!(app.manager.list_keys(&provider.id).unwrap().is_empty());
}

#[test]
fn provider_key_in_use_delete_last_profile_then_removes_key() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-999999999999999999999999",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    let profile = app
        .manager
        .create_lightweight_profile("linked-profile", Some("linked"), LightweightEnv::default())
        .unwrap();
    app.manager
        .set_provider(&profile.id, &provider.id, &key.id)
        .unwrap();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.page = Page::Provider;
    app.mode = Mode::ProviderKeyInUse {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        name: key.name.clone(),
        return_mode: Box::new(Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }),
    };
    app.provider_key_linked_profiles = app
        .manager
        .list_profiles_using_key(&provider.id, &key.id)
        .unwrap();
    app.provider_key_linked_profile_selected = 0;

    app.handle_provider_key_in_use(KeyCode::Char('d'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }
    );
    assert!(app.manager.get_profile(&profile.id).is_err());
    assert!(app.manager.list_keys(&provider.id).unwrap().is_empty());
}

#[test]
fn provider_key_in_use_y_unlinks_profiles_and_removes_key() {
    let mut app = make_test_app();
    let provider = app
        .manager
        .add_provider_with_key_name(
            "Example Provider",
            "https://example.invalid",
            "Default",
            "sk-test-generated-key-yyyyyyyyyyyyyyyyyyyyyyyy",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    let first = app
        .manager
        .create_lightweight_profile("linked-one", Some("linked-one"), LightweightEnv::default())
        .unwrap();
    let second = app
        .manager
        .create_lightweight_profile("linked-two", Some("linked-two"), LightweightEnv::default())
        .unwrap();
    app.manager
        .set_provider(&first.id, &provider.id, &key.id)
        .unwrap();
    app.manager
        .set_provider(&second.id, &provider.id, &key.id)
        .unwrap();
    app.provider_keys_cache = app.manager.list_keys(&provider.id).unwrap();
    app.page = Page::Provider;
    app.mode = Mode::ProviderKeyInUse {
        provider_id: provider.id.clone(),
        key_id: key.id.clone(),
        name: key.name.clone(),
        return_mode: Box::new(Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }),
    };
    app.provider_key_linked_profiles = app
        .manager
        .list_profiles_using_key(&provider.id, &key.id)
        .unwrap();

    app.handle_provider_key_in_use(KeyCode::Char('y'), KeyModifiers::empty())
        .unwrap();

    assert_eq!(
        app.mode,
        Mode::ProviderKeyList {
            provider_id: provider.id.clone(),
        }
    );
    assert!(app.manager.list_keys(&provider.id).unwrap().is_empty());
    for profile_id in [first.id, second.id] {
        let profile = app.manager.get_profile(&profile_id).unwrap();
        assert_eq!(profile.provider_id, None);
        assert_eq!(profile.key_id, None);
    }
    assert!(app.provider_key_linked_profiles.is_empty());
}

#[test]
fn add_full_name_ctrl_g_cancels() {
    let mut app = make_test_app();
    app.mode = Mode::AddFullName;
    app.input_buffer = "demo".into();

    app.handle_add_full_name(KeyCode::Char('g'), KeyModifiers::CONTROL)
        .unwrap();

    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn provider_test_outcome_ctrl_g_exits_to_parent_mode() {
    assert_eq!(
        provider_test_outcome_next_mode(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
            "prov_generated",
            "key_generated",
            ProviderTestSource::KeyList,
            0
        ),
        Mode::ProviderKeyList {
            provider_id: "prov_generated".into(),
        }
    );
}
