use super::api_test::{
    ModelDiscoveryFailureKind, build_anthropic_test_request, build_message_candidates,
    is_anyrouter_url, patch_anyrouter_beta_header, strip_compat_suffix,
};
use super::tinyfish::{
    build_lightweight_settings, tinyfish_command_succeeds_with_timeout, tinyfish_fetch_only_hooks,
    tinyfish_full_hooks, tinyfish_hook_command, tinyfish_mode, tinyfish_mode_for_capabilities,
    tinyfish_plugin_hooks, tinyfish_plugin_manifest, tinyfish_prompt,
};
use super::url_match::{NATIVE_FETCH_URLS, NATIVE_SEARCH_URLS, url_matches};
use super::*;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;
#[cfg(not(windows))]
const TINYFISH_TIMEOUT_TEST_PROGRAM: &str = "sh";
#[cfg(windows)]
const TINYFISH_TIMEOUT_TEST_PROGRAM: &str = "cmd";
use std::thread;
use tempfile::TempDir;

// ── Test helpers ──────────────────────────────────────────────────────────

fn make_manager(tmp: &TempDir) -> ProfileManager {
    let base_dir = tmp.path().join(".claude-switch");
    let profiles_dir = base_dir.join("profiles");
    let registry_path = base_dir.join("registry.json");
    fs::create_dir_all(&profiles_dir).unwrap();
    ProfileManager {
        profiles_dir,
        registry_path,
    }
}

#[test]
fn home_dir_layout_keeps_registry_and_generated_shims_under_same_root() {
    let tmp = TempDir::new().unwrap();
    let mgr = ProfileManager::new_in_home_dir(tmp.path()).unwrap();

    assert_eq!(mgr.base_dir(), tmp.path().join(".claude-switch"));
    assert_eq!(
        mgr.registry_path,
        tmp.path().join(".claude-switch").join("registry.json")
    );
    assert_eq!(
        mgr.profiles_dir,
        tmp.path().join(".claude-switch").join("profiles")
    );
    assert!(mgr.profiles_dir.exists());

    #[cfg(target_os = "windows")]
    assert_eq!(
        ProfileManager::cmd_bin_dir_for_home(tmp.path()),
        tmp.path().join(".local").join("bin")
    );

    #[cfg(not(target_os = "windows"))]
    assert_eq!(
        ProfileManager::sh_bin_dir_for_home(tmp.path()),
        tmp.path().join(".varusers").join("bin")
    );
}

fn make_claude_dir(root: &Path) -> PathBuf {
    let dir = root.to_path_buf();
    fs::create_dir_all(&dir).unwrap();
    let claude_json = serde_json::json!({
        "oauthAccount": {
            "emailAddress": "test@example.com",
            "accountUuid": "uuid-0000-test"
        },
        "someOtherConfig": true
    });
    fs::write(
        dir.join(".claude.json"),
        serde_json::to_string_pretty(&claude_json).unwrap(),
    )
    .unwrap();
    let creds_json = serde_json::json!({
        "claudeAiOauth": {
            "accessToken": "access_tok",
            "refreshToken": "refresh_tok",
            "expiresAt": 9_999_999_999_u64,
            "scopes": ["user:inference"],
            "subscriptionType": "max"
        }
    });
    fs::write(
        dir.join(".credentials.json"),
        serde_json::to_string_pretty(&creds_json).unwrap(),
    )
    .unwrap();
    dir
}

fn unquote_single_quoted_shell_literal(value: &str) -> String {
    let inner = value
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .expect("expected single-quoted shell literal");
    inner.replace("'\\''", "'")
}

fn find_line<'a>(content: &'a str, prefix: &str) -> &'a str {
    content
        .lines()
        .find(|line| line.starts_with(prefix))
        .expect("expected line to exist")
}

fn unescape_generated_cmd_set_value(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'\\') => {
                chars.next();
                out.push('\\');
            }
            '\\' if chars.peek() == Some(&'"') => {
                chars.next();
                out.push('"');
            }
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                out.push('%');
            }
            '^' if chars.peek() == Some(&'^') => {
                chars.next();
                out.push('^');
            }
            _ => out.push(ch),
        }
    }
    out
}

fn cmd_set_value<'a>(content: &'a str, var_name: &str) -> &'a str {
    let prefix = format!("set \"{var_name}=");
    let line = find_line(content, &prefix);
    line.trim_start_matches(&prefix)
        .strip_suffix('"')
        .expect("expected set assignment to end with a quote")
}

#[cfg(not(windows))]
fn tinyfish_timeout_test_args() -> Vec<&'static str> {
    vec!["-c", "sleep 5"]
}

#[cfg(windows)]
fn tinyfish_timeout_test_args() -> Vec<&'static str> {
    vec!["/c", "ping -n 6 127.0.0.1 >nul"]
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut header_end = None;
    let mut body_len = 0usize;

    loop {
        let n = stream.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if header_end.is_none()
            && let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n")
        {
            header_end = Some(pos + 4);
            let headers = String::from_utf8_lossy(&buf[..pos + 4]);
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
            && buf.len() >= end + body_len
        {
            break;
        }
    }

    String::from_utf8(buf).unwrap()
}

fn spawn_model_fetch_server(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut paths = Vec::new();
        for (status_line, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            paths.push(path);
            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        paths
    });
    (format!("http://{}", addr), handle)
}

// ── copy_dir_all ──────────────────────────────────────────────────────────

#[test]
fn copy_dir_all_copies_flat_files() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), "hello").unwrap();
    fs::write(src.join("b.txt"), "world").unwrap();
    let dst = tmp.path().join("dst");
    copy_dir_all(&src, &dst).unwrap();
    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
    assert_eq!(fs::read_to_string(dst.join("b.txt")).unwrap(), "world");
}

#[test]
fn copy_dir_all_copies_nested_directories() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("sub/deep")).unwrap();
    fs::write(src.join("root.txt"), "root").unwrap();
    fs::write(src.join("sub").join("mid.txt"), "mid").unwrap();
    fs::write(src.join("sub/deep").join("leaf.txt"), "leaf").unwrap();
    let dst = tmp.path().join("dst");
    copy_dir_all(&src, &dst).unwrap();
    assert_eq!(fs::read_to_string(dst.join("root.txt")).unwrap(), "root");
    assert_eq!(fs::read_to_string(dst.join("sub/mid.txt")).unwrap(), "mid");
    assert_eq!(
        fs::read_to_string(dst.join("sub/deep/leaf.txt")).unwrap(),
        "leaf"
    );
}

// ── add_profile_from ──────────────────────────────────────────────────────

#[test]
fn load_registry_returns_empty_when_file_absent() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let reg = mgr.load_registry().unwrap();
    assert!(reg.profiles.is_empty());
}

#[test]
fn save_and_load_registry_round_trips() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let id = Uuid::new_v4().to_string();
    let mut reg = Registry::default();
    reg.profiles.insert(
        id.clone(),
        Profile {
            id,
            name: "work".into(),
            alias: None,
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Full,
            env: None,
            launch_args: None,
            provider_id: None,
            key_id: None,
            mcp_server_ids: Vec::new(),
        },
    );
    mgr.save_registry(&reg).unwrap();
    let loaded = mgr.load_registry().unwrap();
    assert_eq!(loaded.profiles.len(), 1);
}

// ── add_profile_from ──────────────────────────────────────────────────────

#[test]
fn add_profile_copies_files_into_profiles_dir() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    mgr.add_profile_from("work", None, &src).unwrap();
    let profile = mgr.get_profile("work").unwrap();
    let dest = mgr.profile_dir(&profile);
    assert!(dest.join(".claude.json").exists(), ".claude.json missing");
    assert!(
        dest.join(".credentials.json").exists(),
        ".credentials.json missing"
    );
}

#[test]
fn add_profile_records_entry_in_registry() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    mgr.add_profile_from("slot", None, &src).unwrap();
    let reg = mgr.load_registry().unwrap();
    let found = reg.profiles.values().any(|p| p.name == "slot");
    assert!(found);
}

#[test]
fn add_profile_errors_on_nonexistent_source() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let err = mgr
        .add_profile_from("bad", None, &tmp.path().join("does-not-exist"))
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn add_profile_errors_on_duplicate_name() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    mgr.add_profile_from("dup", None, &src).unwrap();
    let err = mgr.add_profile_from("dup", None, &src).unwrap_err();
    assert!(err.to_string().contains("already in use"), "{err}");
}

#[test]
fn add_profile_with_alias() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let p = mgr
        .add_profile_from("My Work Profile", Some("work"), &src)
        .unwrap();
    assert_eq!(p.name, "My Work Profile");
    assert_eq!(p.alias.as_deref(), Some("work"));
    // Lookup by alias
    let found = mgr.get_profile("work").unwrap();
    assert_eq!(found.id, p.id);
}

// ── find_profile ─────────────────────────────────────────────────────────

#[test]
fn find_profile_by_id_alias_name() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let p = mgr
        .add_profile_from("Display 名称", Some("short"), &src)
        .unwrap();

    // By id
    let (id, _) = mgr.find_profile(&p.id).unwrap();
    assert_eq!(id, p.id);
    // By alias
    let (id2, _) = mgr.find_profile("short").unwrap();
    assert_eq!(id2, p.id);
    // By name
    let (id3, _) = mgr.find_profile("Display 名称").unwrap();
    assert_eq!(id3, p.id);
    // Not found
    assert!(mgr.find_profile("nope").is_err());
}

#[test]
fn find_profile_errors_on_ambiguous_alias() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let s1 = make_claude_dir(&tmp.path().join("c1"));
    let s2 = make_claude_dir(&tmp.path().join("c2"));
    mgr.add_profile_from("Profile One", Some("p"), &s1).unwrap();
    // Force-add second with same alias should remove the first (force behavior)
    // Actually, add_profile_from checks uniqueness, so second should fail
    let err = mgr
        .add_profile_from("Profile Two", Some("p"), &s2)
        .unwrap_err();
    assert!(err.to_string().contains("already in use"), "{err}");
}

// ── force add ────────────────────────────────────────────────────────────

#[test]
fn force_add_overwrites_existing_profile() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("v1"));
    mgr.add_profile_from("slot", None, &src).unwrap();
    let src2 = make_claude_dir(&tmp.path().join("v2"));
    mgr.add_profile_from_force("slot", None, &src2).unwrap();
    let p = mgr.get_profile("slot").unwrap();
    let dest = mgr.profile_dir(&p);
    assert!(dest.join(".claude.json").exists());
}

#[test]
fn force_add_works_when_profile_does_not_yet_exist() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let p = mgr.add_profile_from_force("brand-new", None, &src).unwrap();
    assert_eq!(p.name, "brand-new");
}

// ── list_profiles ─────────────────────────────────────────────────────────

#[test]
fn list_profiles_returns_sorted_by_name() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    for name in &["zebra", "alpha", "mango"] {
        let src = make_claude_dir(&tmp.path().join(format!("src-{name}")));
        mgr.add_profile_from(name, None, &src).unwrap();
    }
    let profiles = mgr.list_profiles().unwrap();
    let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "mango", "zebra"]);
}

// ── remove_profile ────────────────────────────────────────────────────────

#[test]
fn remove_profile_by_name_deletes_directory_and_entry() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let p = mgr.add_profile_from("to-delete", None, &src).unwrap();
    let dir = mgr.profile_dir(&p);
    mgr.remove_profile("to-delete").unwrap();
    assert!(!dir.exists());
    assert!(mgr.get_profile("to-delete").is_err());
}

#[test]
fn remove_profile_by_alias() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    mgr.add_profile_from("Long Display Name", Some("del"), &src)
        .unwrap();
    mgr.remove_profile("del").unwrap();
    assert!(mgr.get_profile("del").is_err());
}

#[test]
fn remove_profile_errors_when_profile_not_found() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let err = mgr.remove_profile("ghost").unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
}

// ── rename_profile ───────────────────────────────────────────────────────

#[test]
fn rename_profile_changes_name_and_alias() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let p = mgr.add_profile_from("old-name", Some("old"), &src).unwrap();
    let renamed = mgr.rename_profile(&p.id, "new-name", Some("new")).unwrap();
    assert_eq!(renamed.name, "new-name");
    assert_eq!(renamed.alias.as_deref(), Some("new"));
    assert_eq!(renamed.id, p.id); // id preserved
    // Old name no longer works
    assert!(mgr.get_profile("old-name").is_err());
    assert!(mgr.get_profile("old").is_err());
    // New name and alias work
    assert!(mgr.get_profile("new-name").is_ok());
    assert!(mgr.get_profile("new").is_ok());
}

#[test]
fn rename_profile_errors_on_duplicate_name() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let s1 = make_claude_dir(&tmp.path().join("c1"));
    let s2 = make_claude_dir(&tmp.path().join("c2"));
    let p1 = mgr.add_profile_from("Profile A", Some("a"), &s1).unwrap();
    mgr.add_profile_from("Profile B", Some("b"), &s2).unwrap();
    let err = mgr.rename_profile(&p1.id, "Profile B", None).unwrap_err();
    assert!(err.to_string().contains("already in use"), "{err}");
}

// ── lightweight profiles ─────────────────────────────────────────────────

#[test]
fn create_and_launch_lightweight() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let env = LightweightEnv {
        auth_token: Some("tok".into()),
        base_url: Some("https://api.example.com".into()),
        ..Default::default()
    };
    let p = mgr
        .create_lightweight_profile("lite-prof", Some("lp"), env.clone())
        .unwrap();
    assert_eq!(p.name, "lite-prof");
    assert_eq!(p.alias.as_deref(), Some("lp"));
    assert_eq!(p.kind, ProfileKind::Lightweight);
    // Lookup by alias
    let found = mgr.get_profile("lp").unwrap();
    assert_eq!(found.id, p.id);
}

#[test]
fn update_lightweight_preserves_id() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let env = LightweightEnv {
        auth_token: Some("old".into()),
        ..Default::default()
    };
    let p = mgr
        .create_lightweight_profile("test", Some("t"), env)
        .unwrap();
    let original_id = p.id.clone();

    let new_env = LightweightEnv {
        auth_token: Some("new".into()),
        ..Default::default()
    };
    let updated = mgr
        .update_lightweight(&original_id, "test-renamed", Some("tr"), new_env)
        .unwrap();
    assert_eq!(updated.id, original_id);
    assert_eq!(updated.name, "test-renamed");
    assert_eq!(updated.alias.as_deref(), Some("tr"));
    assert_eq!(
        updated.env.as_ref().unwrap().auth_token.as_deref(),
        Some("new")
    );
}

#[test]
fn load_registry_keeps_standalone_inline_credentials() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let env = LightweightEnv {
        auth_token: Some("tok".into()),
        base_url: Some("https://api.example.com".into()),
        ..Default::default()
    };
    let profile = mgr
        .create_lightweight_profile("lite-prof", Some("lp"), env)
        .unwrap();

    let loaded = mgr.load_registry().unwrap();
    let migrated = loaded.profiles.get(&profile.id).unwrap();
    assert_eq!(migrated.provider_id, None);
    assert_eq!(migrated.key_id, None);
    assert!(loaded.providers.is_empty());

    let (token, url) = mgr.resolve_credentials(migrated).unwrap();
    assert_eq!(token.as_deref(), Some("tok"));
    assert_eq!(url.as_deref(), Some("https://api.example.com"));
}

#[test]
fn resolve_credentials_errors_on_stale_provider_reference() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let profile = mgr
        .create_lightweight_profile(
            "lite-prof",
            Some("lp"),
            LightweightEnv {
                auth_token: Some("inline-token".into()),
                base_url: Some("https://inline.example.com".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let mut stale = profile.clone();
    stale.provider_id = Some("missing-provider".into());
    stale.key_id = Some("key_missing".into());

    let err = mgr.resolve_credentials(&stale).unwrap_err();

    assert!(
        err.to_string()
            .contains("references missing provider 'missing-provider'"),
        "{err}"
    );
}

#[test]
fn resolve_credentials_errors_on_missing_key_id() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "provider-token")
        .unwrap();
    let profile = mgr
        .create_lightweight_profile(
            "lite-prof",
            Some("lp"),
            LightweightEnv {
                auth_token: Some("inline-token".into()),
                base_url: Some("https://inline.example.com".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let mut missing_key_id = profile.clone();
    missing_key_id.provider_id = Some(provider.id.clone());
    missing_key_id.key_id = None;

    let err = mgr.resolve_credentials(&missing_key_id).unwrap_err();

    assert!(err.to_string().contains("has no key_id"), "{err}");
}

#[test]
fn resolve_credentials_errors_on_stale_key_reference() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "provider-token")
        .unwrap();
    let profile = mgr
        .create_lightweight_profile(
            "lite-prof",
            Some("lp"),
            LightweightEnv {
                auth_token: Some("inline-token".into()),
                base_url: Some("https://inline.example.com".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let mut stale_key = profile.clone();
    stale_key.provider_id = Some(provider.id.clone());
    stale_key.key_id = Some("missing-key".into());

    let err = mgr.resolve_credentials(&stale_key).unwrap_err();

    assert!(
        err.to_string()
            .contains("references missing key 'missing-key'"),
        "{err}"
    );
}

#[test]
fn unset_provider_persists_and_does_not_relink_on_reload() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let env = LightweightEnv {
        auth_token: Some("tok-inline".into()),
        base_url: Some("https://api.example.com".into()),
        ..Default::default()
    };
    let profile = mgr
        .create_lightweight_profile("lite-prof", Some("lp"), env)
        .unwrap();
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "tok-provider")
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap().clone();

    mgr.set_provider(&profile.id, &provider.id, &key_id)
        .unwrap();
    mgr.unset_provider(&profile.id).unwrap();

    let loaded = mgr.load_registry().unwrap();
    let stored = loaded.profiles.get(&profile.id).unwrap();
    assert_eq!(stored.provider_id, None);
    assert_eq!(stored.key_id, None);
    mgr.remove_provider(&provider.id).unwrap();
}

#[test]
fn load_registry_keeps_distinct_providers_with_same_base_url() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let first = mgr
        .add_provider("First", "https://shared.example.invalid", "tok-one")
        .unwrap();
    let second = mgr
        .add_provider("Second", "https://shared.example.invalid", "tok-two")
        .unwrap();

    let loaded = mgr.load_registry().unwrap();
    assert!(loaded.providers.contains_key(&first.id));
    assert!(loaded.providers.contains_key(&second.id));
    assert_eq!(loaded.providers.len(), 2);
    assert_eq!(
        loaded
            .providers
            .get(&first.id)
            .map(|provider| provider.name.as_str()),
        Some("First")
    );
    assert_eq!(
        loaded
            .providers
            .get(&second.id)
            .map(|provider| provider.name.as_str()),
        Some("Second")
    );
}

#[test]
fn build_model_discovery_candidates_strips_known_compat_suffixes() {
    let candidates =
        build_model_discovery_candidates("https://api.deepseek.com/anthropic").unwrap();
    assert_eq!(
        candidates,
        vec![
            "https://api.deepseek.com/anthropic/v1/models",
            "https://api.deepseek.com/v1/models",
            "https://api.deepseek.com/models",
        ]
    );
}

#[test]
fn build_model_discovery_candidates_prefers_longest_suffix() {
    let candidates = build_model_discovery_candidates("https://api.z.ai/api/anthropic").unwrap();
    assert_eq!(
        candidates,
        vec![
            "https://api.z.ai/api/anthropic/v1/models",
            "https://api.z.ai/v1/models",
            "https://api.z.ai/models",
        ]
    );
}

#[test]
fn discover_models_falls_back_to_root_models_endpoint() {
    let (base_url, handle) = spawn_model_fetch_server(vec![
        ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
        (
            "HTTP/1.1 200 OK",
            "{\"data\":[{\"id\":\"deepseek-chat\"},{\"id\":\"deepseek-reasoner\"}]}",
        ),
    ]);

    let result = discover_models(&format!("{base_url}/anthropic"), "sk-test").unwrap();
    let paths = handle.join().unwrap();

    assert_eq!(
        paths,
        vec!["/anthropic/v1/models".to_string(), "/v1/models".to_string()]
    );
    assert_eq!(result.endpoint_used, format!("{base_url}/v1/models"));
    assert_eq!(
        result.models,
        vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
    );
}

#[test]
fn discover_models_classifies_auth_failure() {
    let (base_url, handle) = spawn_model_fetch_server(vec![(
        "HTTP/1.1 401 Unauthorized",
        "{\"error\":\"bad auth\"}",
    )]);

    let failure = discover_models(&base_url, "sk-test").unwrap_err();
    let paths = handle.join().unwrap();
    let expected_endpoint = format!("{base_url}/v1/models");

    assert_eq!(paths, vec!["/v1/models".to_string()]);
    assert_eq!(failure.kind, ModelDiscoveryFailureKind::Auth);
    assert_eq!(
        failure.last_endpoint.as_deref(),
        Some(expected_endpoint.as_str())
    );
}

#[test]
fn discover_models_classifies_endpoint_not_found_after_candidates() {
    let (base_url, handle) = spawn_model_fetch_server(vec![
        ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
        ("HTTP/1.1 405 Method Not Allowed", "{\"error\":\"blocked\"}"),
        ("HTTP/1.1 404 Not Found", "{\"error\":\"missing\"}"),
    ]);

    let failure = discover_models(&format!("{base_url}/api/anthropic"), "sk-test").unwrap_err();
    let paths = handle.join().unwrap();

    assert_eq!(
        paths,
        vec![
            "/api/anthropic/v1/models".to_string(),
            "/v1/models".to_string(),
            "/models".to_string(),
        ]
    );
    assert_eq!(failure.kind, ModelDiscoveryFailureKind::EndpointNotFound);
}

#[test]
fn discover_models_parses_models_field_fallback() {
    let (base_url, handle) = spawn_model_fetch_server(vec![(
        "HTTP/1.1 200 OK",
        "{\"object\":\"list\",\"models\":[{\"id\":\"llama3\"},{\"id\":\"qwen3-coder\"}]}",
    )]);

    let result = discover_models(&base_url, "sk-ollama").unwrap();
    let paths = handle.join().unwrap();

    assert_eq!(paths, vec!["/v1/models".to_string()]);
    assert_eq!(
        result.models,
        vec!["llama3".to_string(), "qwen3-coder".to_string()]
    );
}

#[test]
fn migration_adds_key_id_for_single_key_provider_links() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider_id = "prov_old".to_string();
    let key_id = "key_old".to_string();
    let mut keys = HashMap::new();
    keys.insert(
        key_id.clone(),
        ProviderKey {
            id: key_id.clone(),
            name: "Default".into(),
            api_key: "tok".into(),
        },
    );
    let mut reg = Registry::default();
    reg.providers.insert(
        provider_id.clone(),
        Provider {
            id: provider_id.clone(),
            name: "Example".into(),
            base_url: "https://api.example.com".into(),
            keys,
            api_key: String::new(),
        },
    );
    reg.profiles.insert(
        "profile-1".into(),
        Profile {
            id: "profile-1".into(),
            name: "lite".into(),
            alias: None,
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Lightweight,
            env: None,
            launch_args: None,
            provider_id: Some(provider_id.clone()),
            key_id: None,
            mcp_server_ids: Vec::new(),
        },
    );
    mgr.save_registry(&reg).unwrap();

    let loaded = mgr.load_registry().unwrap();
    let migrated = loaded.profiles.get("profile-1").unwrap();
    assert_eq!(migrated.key_id.as_deref(), Some(key_id.as_str()));
    let err = mgr.remove_key(&provider_id, &key_id).unwrap_err();
    assert!(err.to_string().contains("used by profiles"), "{err}");
}

#[test]
fn list_profiles_using_key_returns_sorted_linked_profiles() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "tok")
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap().clone();
    let alpha = mgr
        .create_lightweight_profile("alpha", Some("alpha"), LightweightEnv::default())
        .unwrap();
    let beta = mgr
        .create_lightweight_profile("beta", Some("beta"), LightweightEnv::default())
        .unwrap();
    mgr.set_provider(&beta.id, &provider.id, &key_id).unwrap();
    mgr.set_provider(&alpha.id, &provider.id, &key_id).unwrap();

    let linked = mgr.list_profiles_using_key(&provider.id, &key_id).unwrap();

    assert_eq!(linked.len(), 2);
    assert_eq!(linked[0].name, "alpha");
    assert_eq!(linked[1].name, "beta");
}

#[test]
fn rename_key_updates_name_without_changing_token_or_links() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider_with_key_name(
            "Example",
            "https://api.example.com",
            "Default",
            "sk-test-generated-key-rename-111111111111111111111",
        )
        .unwrap();
    let key = provider.keys.values().next().unwrap().clone();
    let profile = mgr
        .create_lightweight_profile("linked", Some("linked"), LightweightEnv::default())
        .unwrap();
    mgr.set_provider(&profile.id, &provider.id, &key.id)
        .unwrap();

    let renamed = mgr.rename_key(&provider.id, &key.id, " Team A ").unwrap();

    assert_eq!(renamed.id, key.id);
    assert_eq!(renamed.name, "Team A");
    assert_eq!(renamed.api_key, key.api_key);
    let linked = mgr.get_profile(&profile.id).unwrap();
    assert_eq!(linked.provider_id.as_deref(), Some(provider.id.as_str()));
    assert_eq!(linked.key_id.as_deref(), Some(key.id.as_str()));
    let (token, url) = mgr.resolve_credentials(&linked).unwrap();
    assert_eq!(token.as_deref(), Some(key.api_key.as_str()));
    assert_eq!(url.as_deref(), Some("https://api.example.com"));
}

#[test]
fn rename_key_rejects_empty_name_and_missing_key() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "tok")
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap().clone();

    let err = mgr.rename_key(&provider.id, &key_id, "   ").unwrap_err();
    assert!(
        err.to_string().contains("Key name cannot be empty"),
        "{err}"
    );

    let err = mgr
        .rename_key(&provider.id, "missing-key", "Team A")
        .unwrap_err();
    assert!(
        err.to_string().contains("Key 'missing-key' not found"),
        "{err}"
    );
}

#[test]
fn load_registry_clears_invalid_provider_key_link() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "tok")
        .unwrap();
    let profile = mgr
        .create_lightweight_profile("lite", None, LightweightEnv::default())
        .unwrap();
    let mut reg = mgr.load_registry().unwrap();
    let stored = reg.profiles.get_mut(&profile.id).unwrap();
    stored.provider_id = Some(provider.id.clone());
    stored.key_id = Some("missing-key".into());
    mgr.save_registry(&reg).unwrap();

    let loaded = mgr.load_registry().unwrap();
    let profile = loaded.profiles.get(&profile.id).unwrap();
    assert_eq!(profile.provider_id, None);
    assert_eq!(profile.key_id, None);
}

#[test]
fn set_provider_rejects_full_profiles() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let full = mgr.add_profile_from("full", None, &src).unwrap();
    let provider = mgr
        .add_provider("Example", "https://api.example.com", "tok")
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap();

    let err = mgr
        .set_provider(&full.id, &provider.id, key_id)
        .unwrap_err();
    assert!(err.to_string().contains("lightweight"), "{err}");
}

#[test]
fn migration_moves_deprecated_api_key_even_when_keys_exist() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let raw = serde_json::json!({
        "profiles": {},
        "providers": {
            "prov_old": {
                "id": "prov_old",
                "name": "Example",
                "base_url": "https://api.example.com",
                "keys": {
                    "key_existing": {
                        "id": "key_existing",
                        "name": "Existing",
                        "api_key": "existing-token"
                    }
                },
                "api_key": "deprecated-token"
            }
        }
    });
    fs::write(
        &mgr.registry_path,
        serde_json::to_string_pretty(&raw).unwrap(),
    )
    .unwrap();

    let loaded = mgr.load_registry().unwrap();
    let provider = loaded.providers.get("prov_old").unwrap();
    assert!(provider.api_key.is_empty());
    assert!(
        provider
            .keys
            .values()
            .any(|key| key.api_key == "deprecated-token")
    );
    assert!(
        provider
            .keys
            .values()
            .any(|key| key.api_key == "existing-token")
    );
}

#[test]
fn provider_serialization_omits_deprecated_api_key_field() {
    let mut keys = HashMap::new();
    keys.insert(
        "key_existing".into(),
        ProviderKey {
            id: "key_existing".into(),
            name: "Existing".into(),
            api_key: "sk-test-generated-key-777777777777777777777777".into(),
        },
    );
    let provider = Provider {
        id: "prov_generated".into(),
        name: "Generated".into(),
        base_url: "https://generated-provider.invalid".into(),
        keys,
        api_key: "deprecated-should-not-serialize".into(),
    };

    let value: serde_json::Value = serde_json::to_value(&provider).unwrap();
    assert!(value.get("api_key").is_none(), "{value}");
    assert_eq!(
        value
            .pointer("/keys/key_existing/api_key")
            .and_then(|entry| entry.as_str()),
        Some("sk-test-generated-key-777777777777777777777777")
    );
}

#[test]
fn anyrouter_test_request_for_non_haiku_injects_thinking_and_supported_betas() {
    let request =
        build_anthropic_test_request("HTTPS://ANYROUTER.TOP:443/", "claude-sonnet-4", "Hello");

    assert!(request.anyrouter_non_haiku);
    assert_eq!(
        request.body.pointer("/max_tokens").and_then(|v| v.as_u64()),
        Some(1200)
    );
    assert_eq!(
        request
            .body
            .pointer("/thinking/type")
            .and_then(|v| v.as_str()),
        Some("enabled")
    );
    assert_eq!(
        request
            .body
            .pointer("/thinking/budget_tokens")
            .and_then(|v| v.as_u64()),
        Some(1024)
    );
    let beta = request.anthropic_beta.as_deref().unwrap_or_default();
    assert!(beta.contains("claude-code-20250219"), "{beta}");
    assert!(beta.contains("interleaved-thinking-2025-05-14"), "{beta}");
    assert!(beta.contains("context-1m-2025-08-07"), "{beta}");
    assert!(beta.contains("redact-thinking-2026-02-12"), "{beta}");
    assert!(beta.contains("prompt-caching-scope-2026-01-05"), "{beta}");
    assert!(beta.contains("advanced-tool-use-2025-11-20"), "{beta}");
    assert!(beta.contains("fast-mode-2026-02-01"), "{beta}");
    assert!(!beta.contains("context-management-2025-06-27"), "{beta}");
    assert!(!beta.contains("effort-2025-11-24"), "{beta}");
}

#[test]
fn anyrouter_beta_patch_keeps_field_backed_flags() {
    let body = serde_json::json!({
        "messages": [],
        "thinking": {
            "type": "enabled",
            "budget_tokens": 1024
        },
        "context_management": {
            "edits": []
        },
        "output_config": {
            "effort": "medium"
        }
    });

    let beta = patch_anyrouter_beta_header(
        &[
            "context-1m-2025-08-07",
            "structured-outputs-2025-12-15",
            "context-management-2025-06-27",
            "effort-2025-11-24",
        ],
        &body,
    )
    .unwrap();

    assert_eq!(
        beta,
        "context-1m-2025-08-07,context-management-2025-06-27,effort-2025-11-24"
    );
}

#[test]
fn is_anyrouter_url_accepts_known_canonical_hosts() {
    assert!(is_anyrouter_url(
        "https://a-ocnfniawgw.cn-shanghai.fcapp.run/v1"
    ));
    assert!(is_anyrouter_url("HTTPS://ANYROUTER.TOP:443/api"));
    assert!(!is_anyrouter_url("https://relay.example.invalid/api"));
}

#[test]
fn anyrouter_test_request_for_haiku_keeps_default_shape() {
    let request =
        build_anthropic_test_request("https://anyrouter.top", "claude-3-5-haiku-latest", "Hello");

    assert!(!request.anyrouter_non_haiku);
    assert_eq!(
        request.body.pointer("/max_tokens").and_then(|v| v.as_u64()),
        Some(64)
    );
    assert!(request.body.get("thinking").is_none(), "{}", request.body);
    assert!(request.anthropic_beta.is_none());
}

#[test]
fn regular_provider_test_request_keeps_default_shape_for_non_haiku() {
    let request = build_anthropic_test_request(
        "https://relay.example.invalid/api",
        "claude-sonnet-4",
        "Hello",
    );

    assert!(!request.anyrouter_non_haiku);
    assert_eq!(
        request.body.pointer("/max_tokens").and_then(|v| v.as_u64()),
        Some(64)
    );
    assert!(request.body.get("thinking").is_none(), "{}", request.body);
    assert!(request.anthropic_beta.is_none());
}

#[test]
fn test_anthropic_message_sends_expected_request_and_parses_response() {
    // For a bare host like http://127.0.0.1:PORT, build_message_candidates
    // produces only one candidate: {host}/v1/messages. So a single-response
    // test server is sufficient.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
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
        request
    });

    let result = test_anthropic_message(
        &format!("http://{}", addr),
        "sk-test-generated-key-555555555555555555555555",
        "claude-test-generated-model",
        "Hello",
    )
    .unwrap();
    let request = handle.join().unwrap();

    assert!(request.starts_with("POST /v1/messages HTTP/1.1\r\n"));
    assert!(request.contains("content-type: application/json; charset=utf-8\r\n"));
    assert!(request.contains("x-api-key: sk-test-generated-key-555555555555555555555555\r\n"));
    assert!(
        request
            .contains("authorization: Bearer sk-test-generated-key-555555555555555555555555\r\n")
    );
    assert!(request.contains("anthropic-version: 2023-06-01\r\n"));
    assert!(request.contains("claude-test-generated-model"));
    assert!(request.contains("\"content\": \"Hello\""));
    assert_eq!(result.text, "Hello from generated test server");
    assert!(result.endpoint_used.ends_with("/v1/messages"));
    assert_eq!(result.input_tokens, Some(7));
    assert_eq!(result.output_tokens, Some(11));
}

#[test]
fn test_anthropic_message_surfaces_http_error_body() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _request = read_http_request(&mut stream);
        let response_body = serde_json::json!({
            "error": {
                "message": "generated unauthorized"
            }
        })
        .to_string();
        write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
    });

    let err = test_anthropic_message(
        &format!("http://{}", addr),
        "sk-test-generated-key-666666666666666666666666",
        "claude-test-generated-model",
        "Hello",
    )
    .unwrap_err();
    handle.join().unwrap();

    let msg = err.to_string();
    assert!(msg.contains("HTTP 401"), "{msg}");
    assert!(msg.contains("generated unauthorized"), "{msg}");
}

// ── generate_aliases ──────────────────────────────────────────────────────

#[test]
fn generate_aliases_uses_alias_when_present() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    mgr.add_profile_from("Long Name", Some("ln"), &src).unwrap();
    let out = mgr.generate_aliases().unwrap();
    // Should use "ln" (alias) not "Long Name"
    assert!(out.contains("claude-ln"), "expected 'claude-ln' in:\n{out}");
}

#[test]
fn generate_aliases_when_empty_returns_hint() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let out = mgr.generate_aliases().unwrap();
    assert!(out.contains("No profiles"), "{out}");
}

#[test]
fn recover_shims_parses_legacy_cmd_and_groups_provider_keys() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let shim_dir = tmp.path().join("shims");
    fs::create_dir_all(&shim_dir).unwrap();
    let shim = r#"@echo off
setlocal
:: Generated by cswitch (claude-switch) — do not edit manually
:: Profile: 芒果-deepseek (lightweight)
set "_LAUNCH_ARGS=--dangerously-skip-permissions"
:launch
if defined _E (claude --settings "{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"sk-mango\",\"ANTHROPIC_BASE_URL\":\"https://aigc-llm.mgtv.com\",\"ANTHROPIC_DEFAULT_HAIKU_MODEL\":\"deepseek-v4-flash[1m]\",\"ANTHROPIC_DEFAULT_OPUS_MODEL\":\"deepseek-v4-pro[1m]\",\"ANTHROPIC_DEFAULT_SONNET_MODEL\":\"deepseek-v4-pro[1m]\",\"ANTHROPIC_MODEL\":\"deepseek-v4-pro[1m]\",\"CLAUDE_CODE_SUBAGENT_MODEL\":\"qwen3.7-max[1m]\",\"EXTRA_FLAG\":\"yes\"}}" %_LAUNCH_ARGS%!_R!)
"#;
    fs::write(shim_dir.join("claude-mg-ds.cmd"), shim).unwrap();

    let plan = mgr.plan_shim_recovery(&shim_dir, false).unwrap();
    assert_eq!(plan.files_scanned, 1);
    assert_eq!(plan.files_recoverable, 1);
    assert_eq!(plan.profiles_added, 1);
    assert_eq!(plan.providers_added, 1);
    assert_eq!(plan.provider_keys_added, 1);

    let summary = mgr.recover_shims(&shim_dir, false).unwrap();
    assert_eq!(summary.plan.profiles_added, 1);
    assert!(summary.backup_path.is_none());

    let registry = mgr.load_registry().unwrap();
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.providers.len(), 1);
    let profile = registry.profiles.values().next().unwrap();
    assert_eq!(profile.name, "芒果-deepseek");
    assert_eq!(profile.alias.as_deref(), Some("mg-ds"));
    assert_eq!(
        profile.launch_args.as_deref(),
        Some(&vec!["--dangerously-skip-permissions".to_string()][..])
    );
    let env = profile.env.as_ref().unwrap();
    assert_eq!(env.auth_token, None);
    assert_eq!(env.base_url, None);
    assert_eq!(env.model.as_deref(), Some("deepseek-v4-pro[1m]"));
    assert_eq!(env.subagent_model.as_deref(), Some("qwen3.7-max[1m]"));
    assert_eq!(env.extras, vec!["EXTRA_FLAG=yes"]);
    let provider = registry
        .providers
        .get(profile.provider_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(provider.base_url, "https://aigc-llm.mgtv.com");
    let key = provider.keys.get(profile.key_id.as_ref().unwrap()).unwrap();
    assert_eq!(key.api_key, "sk-mango");
}

#[test]
fn recover_shims_parses_current_cmd_settings_variable() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let profile = Profile {
        id: Uuid::new_v4().to_string(),
        name: "current".into(),
        alias: Some("cur".into()),
        added: Utc::now(),
        last_used: None,
        kind: ProfileKind::Lightweight,
        env: Some(LightweightEnv {
            auth_token: Some("sk-current".into()),
            base_url: Some("https://current.example.invalid".into()),
            model: Some("current-model".into()),
            ..Default::default()
        }),
        launch_args: Some(vec!["--dangerously-skip-permissions".into()]),
        provider_id: None,
        key_id: None,
        mcp_server_ids: Vec::new(),
    };
    let content = mgr.generate_cmd_content(&profile).unwrap();
    let recovered = ProfileManager::parse_recoverable_shim("claude-cur.cmd", &content).unwrap();
    assert_eq!(recovered.name, "current");
    assert_eq!(recovered.alias, "cur");
    assert_eq!(recovered.token, "sk-current");
    assert_eq!(recovered.base_url, "https://current.example.invalid");
    assert_eq!(recovered.env.model.as_deref(), Some("current-model"));
    assert_eq!(
        recovered.launch_args.as_deref(),
        Some(&vec!["--dangerously-skip-permissions".to_string()][..])
    );
}

#[test]
fn recover_shims_parses_shell_settings_env() {
    let content = r#"#!/usr/bin/env bash
# Generated by cswitch (claude-switch) — do not edit manually
# Profile: shell profile (lightweight)
SETTINGS_ENV='{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-shell","ANTHROPIC_BASE_URL":"https://shell.example.invalid","ANTHROPIC_MODEL":"shell-model"}'
BASE_SETTINGS="${SETTINGS_ENV}"'}'
exec claude "${SETTINGS_ARG[@]}"
"#;
    let recovered = ProfileManager::parse_recoverable_shim("claude-shell-prof", content).unwrap();
    assert_eq!(recovered.name, "shell profile");
    assert_eq!(recovered.alias, "shell-prof");
    assert_eq!(recovered.token, "sk-shell");
    assert_eq!(recovered.base_url, "https://shell.example.invalid");
    assert_eq!(recovered.env.model.as_deref(), Some("shell-model"));
}

#[test]
fn recover_shims_conflicts_until_replace() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    mgr.create_lightweight_profile("existing", Some("ex"), LightweightEnv::default())
        .unwrap();
    let shim_dir = tmp.path().join("shims");
    fs::create_dir_all(&shim_dir).unwrap();
    fs::write(
            shim_dir.join("claude-ex.cmd"),
            r#"@echo off
:: Generated by cswitch (claude-switch) — do not edit manually
:: Profile: existing (lightweight)
claude --settings "{\"env\":{\"ANTHROPIC_AUTH_TOKEN\":\"sk-replace\",\"ANTHROPIC_BASE_URL\":\"https://replace.example.invalid\",\"ANTHROPIC_MODEL\":\"replace-model\"}}"
"#,
        )
        .unwrap();

    let plan = mgr.plan_shim_recovery(&shim_dir, false).unwrap();
    assert_eq!(plan.profiles_conflicted, 1);
    assert!(mgr.recover_shims(&shim_dir, false).is_err());

    let summary = mgr.recover_shims(&shim_dir, true).unwrap();
    assert_eq!(summary.plan.profiles_updated, 1);
    assert!(summary.backup_path.is_some());
    let (_, profile) = mgr.find_profile("ex").unwrap();
    assert_eq!(
        profile.env.as_ref().unwrap().model.as_deref(),
        Some("replace-model")
    );
    assert!(profile.provider_id.is_some());
    assert!(profile.key_id.is_some());
}

#[test]
fn remote_path_join_matches_target_os_separator() {
    assert_eq!(
        ProfileManager::join_remote_path("/home/test/.varusers/bin", RemoteOs::Unix, "claude-dev"),
        "/home/test/.varusers/bin/claude-dev"
    );
    assert_eq!(
        ProfileManager::join_remote_path(
            "C:\\Users\\tester\\.local\\bin",
            RemoteOs::Windows,
            "claude-dev.cmd"
        ),
        "C:\\Users\\tester\\.local\\bin\\claude-dev.cmd"
    );
}

#[test]
fn managed_remote_name_filter_only_matches_generated_prefix() {
    assert!(ProfileManager::is_managed_remote_name(
        RemoteOs::Unix,
        "claude-work"
    ));
    assert!(ProfileManager::is_managed_remote_name(
        RemoteOs::Windows,
        "claude-work.cmd"
    ));
    assert!(!ProfileManager::is_managed_remote_name(
        RemoteOs::Unix,
        "aria2c"
    ));
    assert!(!ProfileManager::is_managed_remote_name(
        RemoteOs::Windows,
        "aria2c.exe"
    ));
    assert!(!ProfileManager::is_managed_remote_name(
        RemoteOs::Unix,
        "xclaude-work"
    ));
}

#[test]
fn remote_shim_file_name_skips_full_profiles() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let src = make_claude_dir(&tmp.path().join("fake-claude"));
    let full = mgr
        .add_profile_from("full", Some("full-alias"), &src)
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-alias"), LightweightEnv::default())
        .unwrap();

    assert_eq!(
        ProfileManager::remote_shim_file_name(&full, RemoteOs::Unix),
        None
    );
    assert_eq!(
        ProfileManager::remote_shim_file_name(&lite, RemoteOs::Unix).as_deref(),
        Some("claude-lite-alias")
    );
    assert_eq!(
        ProfileManager::remote_shim_file_name(&lite, RemoteOs::Windows).as_deref(),
        Some("claude-lite-alias.cmd")
    );
}

#[test]
fn remote_upload_batch_includes_chmod_for_unix() {
    let desired = vec![
        ("claude-work".to_string(), "content".to_string()),
        ("claude-play".to_string(), "content".to_string()),
    ];
    let batch = ProfileManager::build_remote_upload_batch(
        std::path::Path::new("/tmp/cswitch-remote"),
        "/share/home/shark/.varusers/bin",
        RemoteOs::Unix,
        &desired,
        true,
    );
    assert_eq!(batch.matches("put ").count(), 2);
    assert!(batch.contains("chmod 755 \"/share/home/shark/.varusers/bin/claude-work\""));
    assert!(batch.contains("chmod 755 \"/share/home/shark/.varusers/bin/claude-play\""));
}

#[test]
fn remote_upload_batch_skips_chmod_for_sidecars() {
    let desired = vec![
        (
            "tinyfish-full/.claude-plugin/plugin.json".to_string(),
            "{\"name\":\"tinyfish-full\"}".to_string(),
        ),
        (
            "tinyfish-full/hooks/hooks.json".to_string(),
            "{\"hooks\":{}}".to_string(),
        ),
    ];
    let batch = ProfileManager::build_remote_upload_batch(
        std::path::Path::new("/tmp/cswitch-remote"),
        "/share/home/shark/.claude-switch/generated/plugins",
        RemoteOs::Unix,
        &desired,
        false,
    );
    assert_eq!(batch.matches("put ").count(), 2);
    assert!(batch.contains(
            "\"/share/home/shark/.claude-switch/generated/plugins/tinyfish-full/.claude-plugin/plugin.json\""
        ));
    assert!(batch.contains(
        "\"/share/home/shark/.claude-switch/generated/plugins/tinyfish-full/hooks/hooks.json\""
    ));
    assert!(!batch.contains("chmod 755"));
}

#[test]
fn generate_cmd_content_available_for_remote_windows_shims_on_non_windows_hosts() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-alias"), LightweightEnv::default())
        .unwrap();

    let content = mgr.generate_cmd_content(&lite).unwrap();

    assert!(content.contains("@echo off"));
    assert!(content.contains(CMD_MARKER));
    assert!(content.contains("claude"));
}

#[test]
fn generate_cmd_content_escapes_settings_for_remote_windows_shims() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "lite",
            Some("lite-alias"),
            LightweightEnv {
                model: Some("claude-sonnet-4".into()),
                extras: vec![
                    "PERCENT=value%with%percent".into(),
                    "BANG=value!with!bang".into(),
                ],
                ..Default::default()
            },
        )
        .unwrap();

    let content = mgr.generate_cmd_content(&lite).unwrap();

    assert!(content.contains("--settings "));
    assert!(content.contains("PERCENT"));
    assert!(content.contains("%%with%%percent"));
    assert!(content.contains("!with!bang"));
    assert!(!content.contains("^!with^!bang"));
    let json = unescape_generated_cmd_set_value(cmd_set_value(&content, "_SETTINGS"));
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        parsed["env"]["PERCENT"].as_str(),
        Some("value%with%percent")
    );
    assert_eq!(parsed["env"]["BANG"].as_str(), Some("value!with!bang"));
}

#[test]
fn strip_compat_suffix_strips_new_entries() {
    let cases = vec![
        (
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope.aliyuncs.com",
        ),
        ("https://api.openrouter.ai/api", "https://api.openrouter.ai"),
        (
            "https://proxy.example.com/api/v1",
            "https://proxy.example.com",
        ),
        ("https://example.com/v1", "https://example.com"),
        ("https://example.com/v1/messages", "https://example.com"),
        ("https://example.com/messages", "https://example.com"),
    ];
    for (url, expected_root) in cases {
        let stripped = strip_compat_suffix(url.trim_end_matches('/'));
        assert_eq!(stripped, Some(expected_root), "strip_compat_suffix({url})");
    }
}

#[test]
fn build_message_candidates_basic() {
    let candidates = build_message_candidates("https://api.anthropic.com").unwrap();
    assert_eq!(candidates, vec!["https://api.anthropic.com/v1/messages"]);
}

#[test]
fn build_message_candidates_with_compat_suffix() {
    let candidates = build_message_candidates("https://proxy.example.com/api").unwrap();
    assert_eq!(
        candidates,
        vec![
            "https://proxy.example.com/api/v1/messages",
            "https://proxy.example.com/v1/messages",
            "https://proxy.example.com/messages",
        ]
    );
}

#[test]
fn build_message_candidates_v1_suffix() {
    let candidates = build_message_candidates("https://example.com/v1").unwrap();
    // /v1 is a KNOWN_COMPAT_SUFFIX, so strip yields root https://example.com
    assert_eq!(
        candidates,
        vec![
            "https://example.com/v1/messages",
            "https://example.com/messages",
        ]
    );
}

#[test]
fn build_message_candidates_messages_suffix() {
    let candidates = build_message_candidates("https://example.com/v1/messages").unwrap();
    // /v1/messages is stripped, root is https://example.com
    assert_eq!(
        candidates,
        vec![
            "https://example.com/v1/messages/v1/messages",
            "https://example.com/v1/messages",
            "https://example.com/messages",
        ]
    );
}

#[test]
fn build_message_candidates_empty_url() {
    assert!(build_message_candidates("").is_err());
    assert!(build_message_candidates("   ").is_err());
}

#[test]
fn build_model_discovery_candidates_new_suffixes() {
    let cases = vec![
        (
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            vec![
                "https://dashscope.aliyuncs.com/compatible-mode/v1/models",
                "https://dashscope.aliyuncs.com/v1/models",
                "https://dashscope.aliyuncs.com/models",
            ],
        ),
        (
            "https://api.openrouter.ai/api",
            vec![
                "https://api.openrouter.ai/api/v1/models",
                "https://api.openrouter.ai/v1/models",
                "https://api.openrouter.ai/models",
            ],
        ),
        (
            "https://proxy.example.com/api/v1",
            vec![
                "https://proxy.example.com/api/v1/models",
                "https://proxy.example.com/v1/models",
                "https://proxy.example.com/models",
            ],
        ),
        (
            "http://localhost:1234/v1",
            vec![
                "http://localhost:1234/v1/models",
                "http://localhost:1234/models",
            ],
        ),
    ];
    for (url, expected) in cases {
        let candidates = build_model_discovery_candidates(url).unwrap();
        assert_eq!(
            candidates, expected,
            "build_model_discovery_candidates({url})"
        );
    }
}

#[test]
fn url_matches_exact() {
    assert!(url_matches(
        "https://api.deepseek.com/anthropic",
        NATIVE_SEARCH_URLS
    ));
    assert!(!url_matches(
        "https://new-api.example.com",
        NATIVE_SEARCH_URLS
    ));
}

#[test]
fn url_matches_trailing_slash() {
    assert!(url_matches(
        "https://api.deepseek.com/anthropic/",
        NATIVE_SEARCH_URLS
    ));
}

#[test]
fn url_matches_canonical_scheme_host_and_default_https_port() {
    assert!(url_matches("HTTPS://API.ANTHROPIC.COM/", NATIVE_FETCH_URLS));
    assert!(url_matches(
        "https://api.anthropic.com:443",
        NATIVE_FETCH_URLS
    ));
    assert!(url_matches(
        "https://API.DEEPSEEK.COM:443/anthropic/v1/messages",
        NATIVE_SEARCH_URLS
    ));
    assert!(!url_matches(
        "https://api.anthropic.com:444",
        NATIVE_FETCH_URLS
    ));
}

#[test]
fn url_matches_no() {
    assert!(!url_matches(
        "https://api.openrouter.ai/api",
        NATIVE_SEARCH_URLS
    ));
    assert!(!url_matches("http://localhost:11434", NATIVE_SEARCH_URLS));
}

#[test]
fn deepseek_has_search_but_not_fetch() {
    let base = "https://api.deepseek.com/anthropic";
    assert!(url_matches(base, NATIVE_SEARCH_URLS));
    assert!(!url_matches(base, NATIVE_FETCH_URLS));
}

#[test]
fn anyrouter_has_both() {
    assert!(url_matches("https://anyrouter.top", NATIVE_SEARCH_URLS));
    assert!(url_matches("https://anyrouter.top", NATIVE_FETCH_URLS));
}

#[test]
fn proxy_has_neither() {
    let base = "https://new-api.example.com";
    assert!(!url_matches(base, NATIVE_SEARCH_URLS));
    assert!(!url_matches(base, NATIVE_FETCH_URLS));
}

#[test]
fn empty_base_url_uses_native_provider_defaults() {
    assert_eq!(tinyfish_mode(""), TinyfishMode::None);
    assert_eq!(tinyfish_mode("   "), TinyfishMode::None);
}

#[test]
fn tinyfish_mode_accepts_canonical_native_urls() {
    assert_eq!(
        tinyfish_mode("HTTPS://API.ANTHROPIC.COM:443/"),
        TinyfishMode::None
    );
    assert_eq!(
        tinyfish_mode("https://API.DEEPSEEK.COM:443/anthropic/"),
        TinyfishMode::FetchOnly
    );
}

#[test]
fn tinyfish_mode_uses_search_only_for_fetch_native_only() {
    assert_eq!(
        tinyfish_mode_for_capabilities(false, true),
        TinyfishMode::SearchOnly
    );
    let hooks =
        tinyfish_plugin_hooks(TinyfishMode::SearchOnly, TinyfishToolShell::PowerShell).unwrap();
    assert!(hooks.contains("WebSearch"));
    assert!(!hooks.contains("WebFetch"));
    let manifest = tinyfish_plugin_manifest(TinyfishMode::SearchOnly).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["name"].as_str(), Some("tinyfish-search-only"));
}

#[test]
fn tinyfish_mode_can_be_disabled_via_reserved_extra() {
    let env = LightweightEnv {
        base_url: Some("https://new-api.example.com".into()),
        extras: vec!["CLAUDE_SWITCH_TINYFISH=off".into()],
        ..Default::default()
    };
    let artifacts = build_lightweight_runtime_artifacts(
        &env,
        Some("sk-test"),
        env.base_url.as_deref(),
        TinyfishToolShell::PowerShell,
    )
    .unwrap();
    assert_eq!(artifacts.tinyfish_mode, TinyfishMode::None);
    assert!(artifacts.tinyfish_plugin_hooks_json.is_none());
    assert!(artifacts.tinyfish_plugin_manifest_json.is_none());
}

#[test]
fn tinyfish_mode_disable_extra_is_case_insensitive() {
    let env = LightweightEnv {
        base_url: Some("https://new-api.example.com".into()),
        extras: vec!["CLAUDE_SWITCH_TINYFISH=FALSE".into()],
        ..Default::default()
    };
    let artifacts = build_lightweight_runtime_artifacts(
        &env,
        Some("sk-test"),
        env.base_url.as_deref(),
        TinyfishToolShell::PowerShell,
    )
    .unwrap();
    assert_eq!(artifacts.tinyfish_mode, TinyfishMode::None);
}

#[test]
fn reserved_tinyfish_extra_is_not_forwarded_to_env() {
    let env = LightweightEnv {
        extras: vec!["CLAUDE_SWITCH_TINYFISH=off".into(), "FOO=bar".into()],
        ..Default::default()
    };
    let settings = build_lightweight_settings(
        &env,
        Some("sk-test"),
        Some("https://new-api.example.com"),
        TinyfishMode::Full,
        TinyfishToolShell::PowerShell,
    );
    let env_map = settings["env"].as_object().unwrap();
    assert!(!env_map.contains_key("CLAUDE_SWITCH_TINYFISH"));
    assert_eq!(env_map["FOO"].as_str(), Some("bar"));
}

#[test]
fn tinyfish_full_hooks_use_requested_tool_shell() {
    let hooks = tinyfish_full_hooks(TinyfishToolShell::PowerShell);
    let pre_tool = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre_tool.len(), 2);
    let matchers: Vec<&str> = pre_tool
        .iter()
        .map(|h| h["matcher"].as_str().unwrap())
        .collect();
    assert!(matchers.contains(&"WebSearch"));
    assert!(matchers.contains(&"WebFetch"));
    let search_hook = pre_tool
        .iter()
        .find(|h| h["matcher"].as_str() == Some("WebSearch"))
        .unwrap();
    assert!(
        search_hook["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("which search provider to use")
    );
    assert!(search_hook["hooks"][0]["shell"].as_str().unwrap() == "powershell");
    let fetch_hook = pre_tool
        .iter()
        .find(|h| h["matcher"].as_str() == Some("WebFetch"))
        .unwrap();
    assert!(
        fetch_hook["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("which fetch provider to use")
    );
    assert!(fetch_hook["hooks"][0]["shell"].as_str().unwrap() == "powershell");
    let subagent = hooks["hooks"]["SubagentStart"].as_array().unwrap();
    assert_eq!(subagent.len(), 1);
    let subagent_cmd = subagent[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(subagent_cmd.contains("tinyfish search query \\\"<QUERY>\\\""));
    assert!(subagent_cmd.contains("tinyfish fetch content get \\\"<URL>\\\""));
    assert!(!subagent_cmd.contains("tinyfish search query QUERY"));
    assert!(!subagent_cmd.contains("tinyfish fetch content get URL"));
    assert!(subagent_cmd.contains("PowerShell tool"));
}

#[test]
fn tinyfish_fetch_only_hooks_use_requested_tool_shell() {
    let hooks = tinyfish_fetch_only_hooks(TinyfishToolShell::PowerShell);
    let pre_tool = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre_tool.len(), 1);
    assert_eq!(pre_tool[0]["matcher"].as_str().unwrap(), "WebFetch");
    assert!(
        pre_tool[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("which fetch provider to use")
    );
    assert!(pre_tool[0]["hooks"][0]["shell"].as_str().unwrap() == "powershell");
    let subagent = hooks["hooks"]["SubagentStart"].as_array().unwrap();
    assert_eq!(subagent.len(), 1);
    let subagent_cmd = subagent[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(subagent_cmd.contains("tinyfish fetch content get \\\"<URL>\\\""));
    assert!(!subagent_cmd.contains("tinyfish search query \\\"<QUERY>\\\""));
    assert!(!subagent_cmd.contains("tinyfish fetch content get URL"));
    assert!(!subagent_cmd.contains("tinyfish search query QUERY"));
    assert!(subagent_cmd.contains("PowerShell tool"));
}

#[test]
fn tinyfish_bash_hook_command_escapes_apostrophes() {
    let command = tinyfish_hook_command(
        TinyfishToolShell::Bash,
        "PreToolUse",
        Some("allow"),
        "don't break",
    );
    assert!(command.starts_with("printf '%s\\n' '"));
    assert!(command.contains("don'\\''t break"));
}

#[test]
fn tinyfish_powershell_hook_command_escapes_apostrophes() {
    let command = tinyfish_hook_command(
        TinyfishToolShell::PowerShell,
        "PreToolUse",
        Some("allow"),
        "don't break",
    );
    assert!(command.starts_with("Write-Output '"));
    assert!(command.contains("don''t break"));
}

#[test]
fn tinyfish_available_probe_times_out() {
    let started = std::time::Instant::now();
    let ok = tinyfish_command_succeeds_with_timeout(
        TINYFISH_TIMEOUT_TEST_PROGRAM,
        &tinyfish_timeout_test_args(),
        Duration::from_millis(100),
    );
    assert!(!ok);
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn tinyfish_prompt_variants_are_platform_specific() {
    let bash_prompt = tinyfish_prompt(TinyfishMode::Full, TinyfishToolShell::Bash).unwrap();
    let powershell_prompt =
        tinyfish_prompt(TinyfishMode::Full, TinyfishToolShell::PowerShell).unwrap();
    assert!(bash_prompt.contains("run via the Bash tool"));
    assert!(!bash_prompt.contains("PowerShell"));
    assert!(powershell_prompt.contains("run via the PowerShell tool"));
    assert!(!powershell_prompt.contains("run via Bash"));
}

#[test]
fn tinyfish_prompt_file_names_are_shared_by_mode_and_shell() {
    assert_eq!(
        ProfileManager::tinyfish_prompt_file_name(
            TinyfishMode::Full,
            TinyfishToolShell::PowerShell
        )
        .as_deref(),
        Some("tinyfish-full.powershell.txt")
    );
    assert_eq!(
        ProfileManager::tinyfish_prompt_file_name(TinyfishMode::FetchOnly, TinyfishToolShell::Bash)
            .as_deref(),
        Some("tinyfish-fetch-only.bash.txt")
    );
    assert_eq!(
        ProfileManager::tinyfish_prompt_file_name(
            TinyfishMode::SearchOnly,
            TinyfishToolShell::PowerShell
        )
        .as_deref(),
        Some("tinyfish-search-only.powershell.txt")
    );
    assert_eq!(
        ProfileManager::tinyfish_prompt_file_name(
            TinyfishMode::None,
            TinyfishToolShell::PowerShell
        ),
        None
    );
    assert!(ProfileManager::is_managed_generated_prompt_name(
        "tinyfish-full.powershell.txt"
    ));
    assert!(ProfileManager::is_managed_generated_prompt_name(
        "tinyfish-fetch-only.bash.txt"
    ));
    assert!(ProfileManager::is_managed_generated_prompt_name(
        "tinyfish-search-only.powershell.txt"
    ));
    assert!(!ProfileManager::is_managed_generated_prompt_name(
        "notes.tinyfish.txt"
    ));
    assert!(!ProfileManager::is_managed_generated_prompt_name(
        "tinyfish-full.json"
    ));
}

#[test]
fn build_lightweight_settings_windows_tinyfish_allows_bash_and_powershell() {
    let settings = build_lightweight_settings(
        &LightweightEnv::default(),
        Some("sk-test"),
        Some("https://new-api.example.com"),
        TinyfishMode::Full,
        TinyfishToolShell::PowerShell,
    );
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    let allow_values: Vec<&str> = allow.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(allow_values.contains(&"Bash(tinyfish:*)"));
    assert!(allow_values.contains(&"PowerShell(tinyfish:*)"));
    assert!(settings.get("hooks").is_none());
}

#[test]
fn build_lightweight_settings_unix_tinyfish_allows_only_bash() {
    let settings = build_lightweight_settings(
        &LightweightEnv::default(),
        Some("sk-test"),
        Some("https://new-api.example.com"),
        TinyfishMode::Full,
        TinyfishToolShell::Bash,
    );
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    let allow_values: Vec<&str> = allow.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(allow_values.contains(&"Bash(tinyfish:*)"));
    assert!(!allow_values.contains(&"PowerShell(tinyfish:*)"));
    assert!(settings.get("hooks").is_none());
}

#[test]
fn build_lightweight_settings_native_provider_omits_tinyfish_permissions() {
    let settings = build_lightweight_settings(
        &LightweightEnv::default(),
        Some("sk-test"),
        Some("https://anyrouter.top"),
        TinyfishMode::None,
        TinyfishToolShell::PowerShell,
    );
    assert!(settings.get("permissions").is_none());
}

#[test]
fn sync_local_tinyfish_artifacts_writes_shared_plugins_and_prompt() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    mgr.create_lightweight_profile(
        "proxy-one",
        Some("proxy-one"),
        LightweightEnv {
            auth_token: Some("sk-one".into()),
            base_url: Some("https://new-api.example.com".into()),
            model: Some("claude-sonnet".into()),
            ..Default::default()
        },
    )
    .unwrap();
    mgr.create_lightweight_profile(
        "proxy-two",
        Some("proxy-two"),
        LightweightEnv {
            auth_token: Some("sk-two".into()),
            base_url: Some("https://new-api.example.com".into()),
            model: Some("claude-opus".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let profiles = mgr.list_profiles().unwrap();

    mgr.sync_local_tinyfish_artifacts(&profiles).unwrap();

    let prompt_path =
        mgr.local_tinyfish_prompt_path(TinyfishMode::Full, native_tinyfish_tool_shell());
    assert!(prompt_path.exists());
    let plugin_path = mgr.local_tinyfish_plugin_root(TinyfishMode::Full);
    assert!(plugin_path.exists());
    assert!(
        plugin_path
            .join(".claude-plugin")
            .join("plugin.json")
            .exists()
    );
    assert!(plugin_path.join("hooks").join("hooks.json").exists());
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(plugin_path.join(".claude-plugin").join("plugin.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["name"].as_str(), Some("tinyfish-full"));
    assert_eq!(manifest["displayName"].as_str(), Some("TinyFish Full"));
    let prompt_files: Vec<_> = fs::read_dir(mgr.generated_prompts_dir())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(prompt_files.len(), 1);
    assert_eq!(
        prompt_files[0],
        ProfileManager::tinyfish_prompt_file_name(TinyfishMode::Full, native_tinyfish_tool_shell())
            .unwrap()
    );
}

#[test]
fn sync_local_tinyfish_artifacts_removes_stale_managed_plugin_dirs() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let stale_plugin = mgr.generated_plugins_dir().join("tinyfish-full");
    fs::create_dir_all(stale_plugin.join("hooks")).unwrap();
    fs::write(stale_plugin.join("hooks").join("hooks.json"), "{}").unwrap();
    let unmanaged_plugin = mgr.generated_plugins_dir().join("notes");
    fs::create_dir_all(&unmanaged_plugin).unwrap();

    mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

    assert!(!stale_plugin.exists());
    assert!(unmanaged_plugin.exists());
}

#[test]
fn sync_local_tinyfish_artifacts_keeps_unmanaged_tinyfish_prefixed_dirs() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let custom_plugin = mgr.generated_plugins_dir().join("tinyfish-custom");
    fs::create_dir_all(&custom_plugin).unwrap();

    mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

    assert!(custom_plugin.exists());
}

#[test]
fn sync_local_tinyfish_artifacts_keeps_legacy_tinyfish_settings_files() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let legacy_dir = mgr.base_dir().join("generated").join("settings");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_file = legacy_dir.join("config.tinyfish.json");
    fs::write(&legacy_file, "{}").unwrap();

    mgr.sync_local_tinyfish_artifacts(&[]).unwrap();

    assert!(legacy_file.exists());
}

#[test]
fn generated_plugin_file_names_are_shared_by_mode_and_shell() {
    assert_eq!(
        ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::Full).as_deref(),
        Some("tinyfish-full")
    );
    assert_eq!(
        ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::FetchOnly).as_deref(),
        Some("tinyfish-fetch-only")
    );
    assert_eq!(
        ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::SearchOnly).as_deref(),
        Some("tinyfish-search-only")
    );
    assert_eq!(
        ProfileManager::tinyfish_plugin_dir_name(TinyfishMode::None),
        None
    );
    assert!(ProfileManager::is_managed_generated_plugin_dir_name(
        "tinyfish-full"
    ));
    assert!(ProfileManager::is_managed_generated_plugin_dir_name(
        "tinyfish-fetch-only"
    ));
    assert!(ProfileManager::is_managed_generated_plugin_dir_name(
        "tinyfish-search-only"
    ));
    assert!(!ProfileManager::is_managed_generated_plugin_dir_name(
        "notes"
    ));
    assert!(!ProfileManager::is_managed_generated_plugin_dir_name(
        "tinyfish-custom"
    ));
}

#[test]
fn mcp_server_crud_links_only_lightweight_profiles() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "codex-sessions".into(),
            server_type: "stdio".into(),
            command: Some("codex-sessions-mcp".into()),
            ..Default::default()
        })
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-mcp"), LightweightEnv::default())
        .unwrap();
    let full_src = make_claude_dir(&tmp.path().join("fake-claude-mcp"));
    let full = mgr
        .add_profile_from("full", Some("full-mcp"), &full_src)
        .unwrap();

    let linked = mgr
        .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
        .unwrap();
    assert_eq!(linked.mcp_server_ids, vec![server.id.clone()]);
    assert!(
        mgr.set_profile_mcps(&full.id, std::slice::from_ref(&server.id))
            .unwrap_err()
            .to_string()
            .contains("lightweight")
    );
    assert!(
        mgr.remove_mcp_server(&server.id)
            .unwrap_err()
            .to_string()
            .contains("used by profiles")
    );
    let refs = mgr.list_profiles_using_mcp(&server.id).unwrap();
    assert_eq!(refs[0].name, "lite");
}

#[test]
fn mcp_plugin_generation_writes_mcp_json_and_manifest() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let mut env = HashMap::new();
    env.insert("GITHUB_TOKEN".into(), "${GITHUB_TOKEN}".into());
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "github".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            env,
            always_load: Some(false),
            disabled: Some(false),
            ..Default::default()
        })
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-mcp-json"), LightweightEnv::default())
        .unwrap();
    let linked = mgr
        .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
        .unwrap();
    let servers = mgr.profile_mcp_servers(&linked).unwrap();
    let plugin_root = mgr
        .upsert_local_profile_mcp_plugin(&linked, &servers)
        .unwrap();
    assert!(
        plugin_root
            .join(".claude-plugin")
            .join("plugin.json")
            .exists()
    );
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(plugin_root.join(".mcp.json")).unwrap()).unwrap();
    let compat_config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(plugin_root.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(config, compat_config);
    assert_eq!(
        config["$schema"].as_str(),
        Some("https://json.schemastore.org/claude-code-settings.json")
    );
    assert_eq!(
        config["mcpServers"]["github"]["command"].as_str(),
        Some("npx")
    );
    assert_eq!(
        config["mcpServers"]["github"]["env"]["GITHUB_TOKEN"].as_str(),
        Some("${GITHUB_TOKEN}")
    );
    assert_eq!(
        config["mcpServers"]["github"]["alwaysLoad"].as_bool(),
        Some(false)
    );
}

#[test]
fn mcp_export_import_and_replace_round_trip() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "github".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            always_load: Some(true),
            ..Default::default()
        })
        .unwrap();
    let exported = mgr
        .export_mcp_config(std::slice::from_ref(&server.id), false)
        .unwrap();
    let config: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(
        config["mcpServers"]["github"]["command"].as_str(),
        Some("npx")
    );

    let other_tmp = TempDir::new().unwrap();
    let other = make_manager(&other_tmp);
    let imported = other.import_mcp_config(&exported, false).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].name, "github");
    assert_eq!(
        imported[0].args,
        vec!["-y", "@modelcontextprotocol/server-github"]
    );

    let replacement = serde_json::json!({
        "mcpServers": {
            "github": {
                "type": "stdio",
                "command": "node",
                "args": ["server.js"]
            }
        }
    });
    other
        .import_mcp_config(&replacement.to_string(), true)
        .unwrap();
    let updated = other.get_mcp_server("github").unwrap();
    assert_eq!(updated.command.as_deref(), Some("node"));
    assert_eq!(updated.args, vec!["server.js"]);
}

#[test]
fn smart_paste_import_skip_existing_is_atomic_for_other_errors() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    mgr.add_mcp_server(McpServerInput {
        name: "github".into(),
        server_type: "stdio".into(),
        command: Some("existing-github".into()),
        ..Default::default()
    })
    .unwrap();

    let inputs = ProfileManager::parse_mcp_smart_paste_inputs(
        r#"
        {
          "mcpServers": {
            "github": {
              "type": "stdio",
              "command": "replacement-should-skip"
            },
            "broken": {
              "type": "stdio"
            },
            "tavily": {
              "type": "http",
              "url": "https://tavily.ivanli.cc/mcp"
            }
          }
        }
        "#,
    )
    .unwrap();

    let err = mgr.import_mcp_servers_skip_existing(inputs).unwrap_err();
    assert!(err.to_string().contains("broken"));

    let servers = mgr.list_mcp_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "github");
    assert_eq!(servers[0].command.as_deref(), Some("existing-github"));
}

#[test]
fn mcp_validate_reports_missing_runtime_command() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let command = format!("missing-cswitch-command-{}", Uuid::new_v4());
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "missing".into(),
            server_type: "stdio".into(),
            command: Some(command.clone()),
            ..Default::default()
        })
        .unwrap();
    let issues = mgr
        .validate_mcp_servers(std::slice::from_ref(&server.id), false)
        .unwrap();
    assert!(
        issues
            .iter()
            .any(|issue| issue.level == DiagnosticLevel::Warn && issue.message.contains(&command))
    );
}

#[test]
fn inspect_config_counts_registry_and_generated_artifacts() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "filesystem".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            ..Default::default()
        })
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-inspect"), LightweightEnv::default())
        .unwrap();
    let linked = mgr
        .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
        .unwrap();
    let servers = mgr.profile_mcp_servers(&linked).unwrap();
    mgr.upsert_local_profile_mcp_plugin(&linked, &servers)
        .unwrap();

    let inspection = mgr.inspect_config().unwrap();
    assert_eq!(inspection.profiles, 1);
    assert_eq!(inspection.lightweight_profiles, 1);
    assert_eq!(inspection.mcp_servers, 1);
    assert_eq!(inspection.linked_mcp_refs, 1);
    assert_eq!(inspection.generated_mcp_plugins, 1);
}

#[test]
fn doctor_reports_stale_mcp_plugin_state() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "filesystem".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            ..Default::default()
        })
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lite-doctor"), LightweightEnv::default())
        .unwrap();
    mgr.set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
        .unwrap();
    let report = mgr.doctor_report().unwrap();
    assert!(report.items.iter().any(|item| {
        item.level == DiagnosticLevel::Warn
            && item.area == "mcp"
            && item.message.contains("artifacts have not been generated")
    }));
}

#[test]
fn resolve_project_profile_reads_parent_marker() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let profile = mgr
        .create_lightweight_profile("project profile", Some("proj"), LightweightEnv::default())
        .unwrap();
    let project_dir = tmp.path().join("project");
    let nested_dir = project_dir.join("src").join("bin");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(project_dir.join(".cswitch-profile"), "proj\n").unwrap();

    let selected = mgr
        .resolve_project_profile(&nested_dir)
        .unwrap()
        .expect("marker should select profile");
    assert_eq!(selected.id, profile.id);
    assert!(mgr.resolve_project_profile(tmp.path()).unwrap().is_none());
}

#[test]
fn statusline_info_reports_profile_provider_and_mcps() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider_with_key_name(
            "OpenRouter",
            "https://openrouter.example.invalid/api",
            "Team",
            "sk-test",
        )
        .unwrap();
    let key_id = provider.keys.keys().next().cloned().unwrap();
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "filesystem".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            ..Default::default()
        })
        .unwrap();
    let profile = mgr
        .create_lightweight_profile("work", Some("wrk"), LightweightEnv::default())
        .unwrap();
    mgr.set_provider(&profile.id, &provider.id, &key_id)
        .unwrap();
    mgr.set_profile_mcps(&profile.id, std::slice::from_ref(&server.id))
        .unwrap();

    let info = mgr.statusline_info(Some("wrk"), None).unwrap();
    assert_eq!(info.profile_name.as_deref(), Some("work"));
    assert_eq!(info.profile_alias.as_deref(), Some("wrk"));
    assert_eq!(info.provider_name.as_deref(), Some("OpenRouter"));
    assert_eq!(info.key_name.as_deref(), Some("Team"));
    assert_eq!(info.mcp_names, vec!["filesystem"]);
    assert!(!info.project_marker);

    let project_dir = tmp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".cswitch-profile"), "wrk\n").unwrap();
    let project_info = mgr.statusline_info(None, Some(&project_dir)).unwrap();
    assert_eq!(project_info.profile_name.as_deref(), Some("work"));
    assert!(project_info.project_marker);
}

#[test]
fn config_bundle_export_redacts_secrets_and_imports_with_replace() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider_with_key_name(
            "Provider",
            "https://provider.example.invalid",
            "Default",
            "secret-key",
        )
        .unwrap();
    let key_id = provider.keys.keys().next().cloned().unwrap();
    let mut mcp_env = HashMap::new();
    mcp_env.insert("GITHUB_TOKEN".into(), "ghp-secret-token".into());
    mcp_env.insert("TOKENIZERS_PARALLELISM".into(), "false".into());
    let mut mcp_headers = HashMap::new();
    mcp_headers.insert("Authorization".into(), "Bearer mcp-header-secret".into());
    mcp_headers.insert("X-Mode".into(), "portable".into());
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "github".into(),
            server_type: "stdio".into(),
            command: Some("npx".into()),
            env: mcp_env,
            headers: mcp_headers,
            oauth: Some(serde_json::json!({
                "clientId": "client-id",
                "clientSecret": "oauth-secret",
                "scopes": ["read", "write"]
            })),
            ..Default::default()
        })
        .unwrap();
    let profile = mgr
        .create_lightweight_profile(
            "bundle",
            Some("bun"),
            LightweightEnv {
                auth_token: Some("profile-secret".into()),
                base_url: Some("https://provider.example.invalid".into()),
                ..Default::default()
            },
        )
        .unwrap();
    mgr.set_provider(&profile.id, &provider.id, &key_id)
        .unwrap();
    mgr.set_profile_mcps(&profile.id, std::slice::from_ref(&server.id))
        .unwrap();

    let redacted = mgr.export_config_bundle(&[], false).unwrap();
    assert!(!redacted.contains("secret-key"));
    assert!(!redacted.contains("profile-secret"));
    assert!(!redacted.contains("ghp-secret-token"));
    assert!(!redacted.contains("mcp-header-secret"));
    assert!(!redacted.contains("oauth-secret"));
    let bundle: ConfigBundle = serde_json::from_str(&redacted).unwrap();
    assert!(!bundle.secrets_included);
    assert_eq!(bundle.profiles.len(), 1);
    assert_eq!(
        bundle.providers[0].keys.values().next().unwrap().api_key,
        ""
    );
    assert_eq!(bundle.mcp_servers[0].env["GITHUB_TOKEN"], "");
    assert_eq!(bundle.mcp_servers[0].env["TOKENIZERS_PARALLELISM"], "false");
    assert_eq!(bundle.mcp_servers[0].headers["Authorization"], "");
    assert_eq!(bundle.mcp_servers[0].headers["X-Mode"], "portable");
    assert_eq!(
        bundle.mcp_servers[0].oauth.as_ref().unwrap()["clientSecret"].as_str(),
        Some("")
    );
    assert_eq!(
        bundle.mcp_servers[0].oauth.as_ref().unwrap()["clientId"].as_str(),
        Some("client-id")
    );

    let with_secrets = mgr.export_config_bundle(&[], true).unwrap();
    assert!(with_secrets.contains("secret-key"));
    assert!(with_secrets.contains("ghp-secret-token"));
    assert!(with_secrets.contains("mcp-header-secret"));
    assert!(with_secrets.contains("oauth-secret"));

    let other_tmp = TempDir::new().unwrap();
    let other = make_manager(&other_tmp);
    let plan = other
        .plan_config_bundle_import(&with_secrets, false)
        .unwrap();
    assert_eq!(plan.summary.profiles_added, 1);
    assert_eq!(plan.summary.providers_added, 1);
    assert_eq!(plan.summary.mcp_servers_added, 1);
    assert_eq!(plan.profiles_add.len(), 1);
    assert!(plan.profiles_update.is_empty());
    assert!(other.list_profiles().unwrap().is_empty());

    let summary = other.import_config_bundle(&with_secrets, false).unwrap();
    assert_eq!(summary.profiles_added, 1);
    assert_eq!(summary.providers_added, 1);
    assert_eq!(summary.mcp_servers_added, 1);
    assert_eq!(other.list_profiles().unwrap()[0].name, "bundle");

    let plan = other
        .plan_config_bundle_import(&with_secrets, true)
        .unwrap();
    assert_eq!(plan.summary.profiles_updated, 1);
    assert_eq!(plan.summary.providers_updated, 1);
    assert_eq!(plan.summary.mcp_servers_updated, 1);
    assert!(plan.profiles_add.is_empty());
    assert_eq!(plan.profiles_update.len(), 1);

    let conflict_plan = other
        .plan_config_bundle_import(&with_secrets, false)
        .unwrap();
    assert_eq!(conflict_plan.conflict_count(), 3);
    assert_eq!(conflict_plan.summary.profiles_conflicted, 1);
    assert_eq!(conflict_plan.summary.providers_conflicted, 1);
    assert_eq!(conflict_plan.summary.mcp_servers_conflicted, 1);
    assert!(
        other
            .import_config_bundle(&with_secrets, false)
            .unwrap_err()
            .to_string()
            .contains("Use --replace")
    );

    let summary = other.import_config_bundle(&with_secrets, true).unwrap();
    assert_eq!(summary.profiles_updated, 1);
    assert_eq!(summary.providers_updated, 1);
    assert_eq!(summary.mcp_servers_updated, 1);

    let redacted_from_other = other.export_config_bundle(&[], false).unwrap();
    other
        .import_config_bundle(&redacted_from_other, true)
        .unwrap();
    let preserved_provider = other.get_provider(&provider.id).unwrap();
    assert_eq!(
        preserved_provider
            .keys
            .get(&key_id)
            .map(|key| key.api_key.as_str()),
        Some("secret-key")
    );
    let preserved_mcp = other.get_mcp_server(&server.id).unwrap();
    assert_eq!(
        preserved_mcp.env.get("GITHUB_TOKEN").map(String::as_str),
        Some("ghp-secret-token")
    );
    assert_eq!(
        preserved_mcp
            .env
            .get("TOKENIZERS_PARALLELISM")
            .map(String::as_str),
        Some("false")
    );
    assert_eq!(
        preserved_mcp
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer mcp-header-secret")
    );
    assert_eq!(
        preserved_mcp.headers.get("X-Mode").map(String::as_str),
        Some("portable")
    );
    assert_eq!(
        preserved_mcp.oauth.as_ref().unwrap()["clientSecret"].as_str(),
        Some("oauth-secret")
    );

    let scoped = mgr
        .export_config_bundle(std::slice::from_ref(&profile.id), false)
        .unwrap();
    let scoped_bundle: ConfigBundle = serde_json::from_str(&scoped).unwrap();
    assert_eq!(scoped_bundle.profiles.len(), 1);
    assert_eq!(scoped_bundle.providers.len(), 1);
    assert_eq!(scoped_bundle.mcp_servers.len(), 1);
    assert_eq!(scoped_bundle.profiles[0].id, profile.id);
    assert_eq!(scoped_bundle.providers[0].keys.len(), 1);
    assert!(scoped_bundle.providers[0].keys.contains_key(&key_id));

    let validation = mgr.validate_config_bundle(&scoped).unwrap();
    assert_eq!(validation.profiles, 1);
    assert_eq!(validation.error_count(), 0);
}

#[test]
fn scoped_config_export_includes_only_selected_provider_keys() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider_with_key_name(
            "Shared",
            "https://shared.example.invalid",
            "Team A",
            "secret-a",
        )
        .unwrap();
    let key_a = provider.keys.keys().next().cloned().unwrap();
    let key_b = mgr.add_key(&provider.id, "Team B", "secret-b").unwrap().id;
    let first = mgr
        .create_lightweight_profile("first", Some("first"), LightweightEnv::default())
        .unwrap();
    let second = mgr
        .create_lightweight_profile("second", Some("second"), LightweightEnv::default())
        .unwrap();
    mgr.set_provider(&first.id, &provider.id, &key_a).unwrap();
    mgr.set_provider(&second.id, &provider.id, &key_b).unwrap();

    let scoped = mgr
        .export_config_bundle(std::slice::from_ref(&first.id), true)
        .unwrap();
    assert!(scoped.contains("secret-a"));
    assert!(!scoped.contains("secret-b"));
    let bundle: ConfigBundle = serde_json::from_str(&scoped).unwrap();
    assert_eq!(bundle.providers.len(), 1);
    assert_eq!(bundle.providers[0].keys.len(), 1);
    assert!(bundle.providers[0].keys.contains_key(&key_a));
    assert!(!bundle.providers[0].keys.contains_key(&key_b));
}

#[test]
fn config_import_rejects_profiles_with_missing_provider_references() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let bundle = ConfigBundle {
        schema: "https://github.com/m2selfA/claude-switch/config-bundle/v1".into(),
        exported_at: Utc::now(),
        profiles: vec![Profile {
            id: Uuid::new_v4().to_string(),
            name: "broken".into(),
            alias: Some("broken".into()),
            added: Utc::now(),
            last_used: None,
            kind: ProfileKind::Lightweight,
            env: Some(LightweightEnv::default()),
            launch_args: None,
            provider_id: Some("missing-provider".into()),
            key_id: Some("missing-key".into()),
            mcp_server_ids: Vec::new(),
        }],
        providers: Vec::new(),
        mcp_servers: Vec::new(),
        secrets_included: true,
    };
    let content = serde_json::to_string(&bundle).unwrap();

    let err = mgr
        .plan_config_bundle_import(&content, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing provider"), "{err}");
    let err = mgr
        .import_config_bundle(&content, false)
        .unwrap_err()
        .to_string();
    assert!(err.contains("missing provider"), "{err}");
    assert!(mgr.load_registry().unwrap().profiles.is_empty());
}

#[test]
fn generated_launchers_include_mcp_plugin_dir() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let server = mgr
        .add_mcp_server(McpServerInput {
            name: "codex-sessions".into(),
            server_type: "stdio".into(),
            command: Some("codex-sessions-mcp".into()),
            ..Default::default()
        })
        .unwrap();
    let lite = mgr
        .create_lightweight_profile("lite", Some("lmcp"), LightweightEnv::default())
        .unwrap();
    let linked = mgr
        .set_profile_mcps(&lite.id, std::slice::from_ref(&server.id))
        .unwrap();

    let cmd = mgr.generate_cmd_content(&linked).unwrap();
    assert!(cmd.contains("%USERPROFILE%\\.claude-switch\\generated\\mcps\\cswitch-mcp-profile-"));
    assert!(cmd.contains("--plugin-dir \"%_MCP_PLUGIN_DIR%\""));

    let sh = mgr.generate_sh_content(&linked).unwrap();
    assert!(sh.contains("$HOME/.claude-switch/generated/mcps/cswitch-mcp-profile-"));
    assert!(sh.contains("MCP_PLUGIN_ARGS=(--plugin-dir"));
}

#[test]
fn provider_backed_cmd_settings_use_updated_provider_key() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let provider = mgr
        .add_provider(
            "Example",
            "https://proxy.example.com/path!section/%5E/^v2",
            "old-token",
        )
        .unwrap();
    let key_id = provider.keys.keys().next().unwrap().clone();
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp"),
            LightweightEnv {
                auth_token: Some("inline-token".into()),
                base_url: Some("https://inline.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    mgr.set_provider(&lite.id, &provider.id, &key_id).unwrap();
    mgr.update_key(
        &provider.id,
        &key_id,
        "Default",
        "new-token!bang%20caret^\"value",
    )
    .unwrap();
    let linked = mgr.get_profile(&lite.id).unwrap();
    let content = mgr.generate_cmd_content(&linked).unwrap();

    assert!(!content.contains("old-token"));
    assert!(!content.contains("inline-token"));
    for var_name in ["_SETTINGS", "_TF_SETTINGS"] {
        let json = unescape_generated_cmd_set_value(cmd_set_value(&content, var_name));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["env"]["ANTHROPIC_AUTH_TOKEN"].as_str(),
            Some("new-token!bang%20caret^\"value")
        );
        assert_eq!(
            parsed["env"]["ANTHROPIC_BASE_URL"].as_str(),
            Some("https://proxy.example.com/path!section/%5E/^v2")
        );
    }
}

#[test]
fn generate_cmd_content_uses_plugin_dir_and_inline_tf_settings() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_cmd_content(&lite).unwrap();
    assert!(content.contains("setlocal EnableExtensions DisableDelayedExpansion"));
    assert!(content.contains("goto build_settings"));
    assert!(content.contains(":build_settings"));
    assert!(content.contains("if defined _TF goto launch_with_hooks_plain"));
    assert!(content.contains("set \"_SETTINGS={\\\"env\\\":{"));
    assert!(content.contains("set \"_TF_SETTINGS={\\\"env\\\":{"));
    assert!(content.contains("set \"_TF=\""));
    assert!(content.contains(
        "set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-full\""
    ));
    assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-full.powershell.txt\""));
    assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
    assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
    assert!(!content.contains("SubagentStart"));
    assert!(!content.contains("PreToolUse"));
    assert!(content.contains("PowerShell(tinyfish:*)"));
    assert_eq!(content.matches("\\\"ANTHROPIC_AUTH_TOKEN\\\"").count(), 2);
    assert!(content.contains("--settings \"%_SETTINGS%\""));
    assert!(!content.contains("_TF_SETTINGS_FILE="));
}

#[test]
fn generate_cmd_content_assigns_parseable_json_settings() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp"),
            LightweightEnv {
                auth_token: Some("sk-test!bang%20caret^value".into()),
                base_url: Some("https://new-api.example.com/path!section/%5E/^v2".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_cmd_content(&lite).unwrap();

    for var_name in ["_SETTINGS", "_TF_SETTINGS"] {
        let json = unescape_generated_cmd_set_value(cmd_set_value(&content, var_name));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["env"]["ANTHROPIC_AUTH_TOKEN"].as_str(),
            Some("sk-test!bang%20caret^value")
        );
        assert_eq!(
            parsed["env"]["ANTHROPIC_BASE_URL"].as_str(),
            Some("https://new-api.example.com/path!section/%5E/^v2")
        );
        assert!(!json.contains("^!"));
    }
    assert!(!content.contains("call claude --settings"));
}

#[test]
fn generate_cmd_content_includes_tf_prompt() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_cmd_content(&lite).unwrap();
    assert!(content.contains("where tinyfish >nul 2>&1 && set \"_TF=1\""));
    assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-full.powershell.txt\""));
    assert!(content.contains(
        "set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-full\""
    ));
    assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
    assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
    assert!(!content.contains("--append-system-prompt \"%_TF_PROMPT%\""));
    assert!(!content.contains("rate limited by tinyfish"));
    assert!(!content.contains("run via Bash"));
}

#[test]
fn generate_sh_content_switches_between_base_and_hook_settings() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_sh_content(&lite).unwrap();
    assert!(content.contains("command -v tinyfish"));
    assert!(content.contains("TF_SP_ARGS=("));
    assert!(content.contains("SETTINGS_ENV="));
    assert!(content.contains("BASE_SETTINGS="));
    assert!(content.contains("TF_SETTINGS="));
    assert!(content.contains("SETTINGS_ARG=(--settings \"$BASE_SETTINGS\")"));
    assert!(
        content.contains("TF_PLUGIN_DIR=\"$HOME/.claude-switch/generated/plugins/tinyfish-full\"")
    );
    assert!(content.contains(
        "TF_PROMPT_FILE=\"$HOME/.claude-switch/generated/prompts/tinyfish-full.bash.txt\""
    ));
    assert!(content.contains("TF_PLUGIN_ARGS=(--plugin-dir \"$TF_PLUGIN_DIR\")"));
    assert!(content.contains("TF_SP_ARGS=(--append-system-prompt-file \"$TF_PROMPT_FILE\")"));
    assert!(content.contains("SETTINGS_ARG=(--settings \"$TF_SETTINGS\")"));
    assert!(content.contains("BASE_SETTINGS=\"${SETTINGS_ENV}\""));
    assert!(!content.contains("HOOK_SETTINGS="));
    assert!(content.contains("Bash(tinyfish:*)"));
    assert_eq!(content.matches("\"ANTHROPIC_AUTH_TOKEN\"").count(), 1);
    assert!(!content.contains("run via Bash"));
    assert!(!content.contains("PowerShell tool"));

    let settings_env_line = find_line(&content, "SETTINGS_ENV=");
    let settings_env =
        unquote_single_quoted_shell_literal(settings_env_line.trim_start_matches("SETTINGS_ENV="));
    let base_settings_line = find_line(&content, "BASE_SETTINGS=");
    let base_tail = unquote_single_quoted_shell_literal(
        base_settings_line.trim_start_matches("BASE_SETTINGS=\"${SETTINGS_ENV}\""),
    );
    let tf_settings_line = find_line(&content, "TF_SETTINGS=");
    let tf_tail = unquote_single_quoted_shell_literal(
        tf_settings_line.trim_start_matches("TF_SETTINGS=\"${SETTINGS_ENV}\""),
    );

    let base_settings_json = format!("{settings_env}{base_tail}");
    let tf_settings_json = format!("{settings_env}{tf_tail}");
    let base_json: serde_json::Value = serde_json::from_str(&base_settings_json).unwrap();
    let tf_json: serde_json::Value = serde_json::from_str(&tf_settings_json).unwrap();
    assert!(base_json.get("permissions").is_none());
    let allow = tf_json["permissions"]["allow"].as_array().unwrap();
    assert_eq!(allow.len(), 1);
    assert_eq!(allow[0].as_str(), Some("Bash(tinyfish:*)"));
}

#[test]
fn generate_cmd_content_deepseek_fetch_only_prompt() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "ds-prof",
            Some("ds"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://api.deepseek.com/anthropic".into()),
                model: Some("deepseek-v4".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_cmd_content(&lite).unwrap();
    assert!(content.contains(
            "set \"_TF_PLUGIN_DIR=%USERPROFILE%\\.claude-switch\\generated\\plugins\\tinyfish-fetch-only\""
        ));
    assert!(content.contains("set \"_TF_PROMPT_FILE=%USERPROFILE%\\.claude-switch\\generated\\prompts\\tinyfish-fetch-only.powershell.txt\""));
    assert!(content.contains("--plugin-dir \"%_TF_PLUGIN_DIR%\""));
    assert!(content.contains("--append-system-prompt-file \"%_TF_PROMPT_FILE%\""));
    assert!(!content.contains("WebFetch"));
    assert!(!content.contains("WebSearch"));
}

#[test]
fn generate_cmd_content_native_provider_skips_tinyfish() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "native-prof",
            Some("native"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://anyrouter.top".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let content = mgr.generate_cmd_content(&lite).unwrap();
    assert!(!content.contains("_TF_PLUGIN_DIR="));
    assert!(!content.contains("_TF_PROMPT_FILE="));
    assert!(!content.contains("tinyfish:*)"));
}

#[test]
fn generate_cmd_content_respects_no_extras_when_tinyfish_missing() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let mut lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp-noextras"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    lite.launch_args = Some(vec!["--dangerously-skip-permissions".into()]);

    let content = mgr.generate_cmd_content(&lite).unwrap();

    assert!(content.contains("if defined _TF if defined _E goto launch_with_hooks_extras"));
    assert!(content.contains("if defined _TF goto launch_with_hooks_plain"));
    assert!(content.contains("if defined _E goto launch_with_extras"));
}

#[cfg(windows)]
#[test]
fn generated_cmd_subagent_hook_does_not_trigger_cmd_parse_error() {
    let tmp = TempDir::new().unwrap();
    let mgr = make_manager(&tmp);
    let lite = mgr
        .create_lightweight_profile(
            "proxy-prof",
            Some("pp-win-cmd"),
            LightweightEnv {
                auth_token: Some("sk-test".into()),
                base_url: Some("https://new-api.example.com".into()),
                model: Some("claude-sonnet".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let mut content = mgr.generate_cmd_content(&lite).unwrap();
    content = content.replace(
            "claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\" %_LAUNCH_ARGS% %_R%",
            "echo launched hooks extras",
        );
    content = content.replace(
        "claude --settings \"%_SETTINGS%\" %_LAUNCH_ARGS% %_R%",
        "echo launched extras",
    );
    content = content.replace(
            "claude --settings \"%_TF_SETTINGS%\" --plugin-dir \"%_TF_PLUGIN_DIR%\" --append-system-prompt-file \"%_TF_PROMPT_FILE%\" %_R%",
            "echo launched hooks plain",
        );
    content = content.replace(
        "claude --settings \"%_SETTINGS%\" %_R%",
        "echo launched plain",
    );

    let shim_path = tmp.path().join("claude-cstcloud.cmd");
    fs::write(&shim_path, content).unwrap();

    let output = std::process::Command::new("cmd")
        .args(["/c", shim_path.to_string_lossy().as_ref(), "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(output.status.success());
    assert!(combined.contains("launched"));
    assert!(!combined.contains("The system cannot find the file specified."));
}

#[test]
fn remove_remote_plugin_dir_reports_runner_errors() {
    let result = ProfileManager::remove_remote_plugin_dir_with_runner(
        "host",
        "/tmp/tinyfish-full",
        RemoteOs::Unix,
        |_| anyhow::bail!("permission denied"),
    );
    assert!(result.is_err());
}
