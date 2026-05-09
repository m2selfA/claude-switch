use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub email: Option<String>,
    pub added: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Registry {
    pub profiles: HashMap<String, Profile>,
}

// ── ProfileManager ────────────────────────────────────────────────────────────

pub struct ProfileManager {
    #[allow(dead_code)]
    pub base_dir: PathBuf,
    pub profiles_dir: PathBuf,
    registry_path: PathBuf,
}

impl ProfileManager {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let base_dir = home.join(".claude-switch");
        let profiles_dir = base_dir.join("profiles");
        let registry_path = base_dir.join("registry.json");
        fs::create_dir_all(&profiles_dir)?;
        Ok(Self { base_dir, profiles_dir, registry_path })
    }

    // ── Registry I/O ─────────────────────────────────────────────────────────

    pub fn load_registry(&self) -> Result<Registry> {
        if !self.registry_path.exists() {
            return Ok(Registry::default());
        }
        let content = fs::read_to_string(&self.registry_path)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn save_registry(&self, registry: &Registry) -> Result<()> {
        let content = serde_json::to_string_pretty(registry)?;
        fs::write(&self.registry_path, content)?;
        Ok(())
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Returns all profiles sorted alphabetically by name.
    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let registry = self.load_registry()?;
        let mut profiles: Vec<Profile> = registry.profiles.into_values().collect();
        profiles.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(profiles)
    }

    /// Add a profile from an explicit source directory.
    /// Used both by `add_profile` (which sources `~/.claude`) and by tests.
    pub fn add_profile_from(&self, name: &str, src: &Path) -> Result<Profile> {
        if !src.exists() {
            bail!("Source directory '{}' does not exist.", src.display());
        }
        let dest = self.profiles_dir.join(name);
        if dest.exists() {
            bail!("Profile '{}' already exists. Use --force to overwrite.", name);
        }
        let profile = self.copy_and_build_profile(name, src)?;
        self.upsert_profile(profile.clone())?;
        Ok(profile)
    }

    /// Same as `add_profile_from` but overwrites an existing profile.
    pub fn add_profile_from_force(&self, name: &str, src: &Path) -> Result<Profile> {
        if !src.exists() {
            bail!("Source directory '{}' does not exist.", src.display());
        }
        let dest = self.profiles_dir.join(name);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        let profile = self.copy_and_build_profile(name, src)?;
        self.upsert_profile(profile.clone())?;
        Ok(profile)
    }

    /// Add the current logged-in session as a named profile.
    /// Copies `~/.claude/` dir and `~/.claude.json` from the home root.
    pub fn add_profile(&self, name: &str) -> Result<Profile> {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let src = home.join(".claude");
        if !src.exists() {
            bail!("~/.claude does not exist. Is Claude Code installed and logged in?");
        }
        let mut profile = self.add_profile_from(name, &src)?;
        self.copy_extra_credentials(&home, name, &mut profile)?;
        Ok(profile)
    }

    /// Same as `add_profile` but overwrites an existing profile.
    pub fn add_profile_force(&self, name: &str) -> Result<Profile> {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        let src = home.join(".claude");
        if !src.exists() {
            bail!("~/.claude does not exist. Is Claude Code installed and logged in?");
        }
        let mut profile = self.add_profile_from_force(name, &src)?;
        self.copy_extra_credentials(&home, name, &mut profile)?;
        Ok(profile)
    }

    /// Copy `~/.claude.json` from the home root (outside `~/.claude/`).
    /// This file contains oauthAccount metadata including the account email.
    fn copy_extra_credentials(
        &self,
        home: &Path,
        name: &str,
        profile: &mut Profile,
    ) -> Result<()> {
        let dest = self.profile_dir(name);

        let home_claude_json = home.join(".claude.json");
        if home_claude_json.exists() {
            fs::copy(&home_claude_json, dest.join(".claude.json"))?;
            if profile.email.is_none() {
                profile.email = read_email_from_dir(&dest);
                self.upsert_profile(profile.clone())?;
            }
        }

        Ok(())
    }

    pub fn remove_profile(&self, name: &str) -> Result<()> {
        let mut registry = self.load_registry()?;
        if !registry.profiles.contains_key(name) {
            bail!("Profile '{}' not found.", name);
        }
        let dest = self.profiles_dir.join(name);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        registry.profiles.remove(name);
        self.save_registry(&registry)
    }

    pub fn get_profile(&self, name: &str) -> Result<Profile> {
        let registry = self.load_registry()?;
        registry
            .profiles
            .get(name)
            .cloned()
            .context(format!("Profile '{}' not found.", name))
    }

    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(name)
    }

    /// Launch `claude` with `CLAUDE_CONFIG_DIR` pointed at the named profile.
    pub fn launch_claude(&self, name: &str, args: &[String]) -> Result<()> {
        let profile_dir = self.profile_dir(name);
        if !profile_dir.exists() {
            bail!(
                "Profile directory for '{}' not found. Re-add it with: cswitch add {}",
                name,
                name
            );
        }
        let mut registry = self.load_registry()?;
        if let Some(p) = registry.profiles.get_mut(name) {
            p.last_used = Some(Utc::now());
        }
        self.save_registry(&registry)?;

        let status = std::process::Command::new("claude")
            .args(args)
            .env("CLAUDE_CONFIG_DIR", &profile_dir)
            .status()
            .context("Failed to launch claude. Is it installed and in your PATH?")?;

        std::process::exit(status.code().unwrap_or(0));
    }

    /// Create an empty profile directory and launch Claude into it.
    /// Claude will detect no credentials and trigger its own login flow.
    /// After the user authenticates and exits Claude, we read the email
    /// from whatever config Claude wrote and register the profile.
    pub fn login_profile(&self, name: &str) -> Result<()> {
        let profile_dir = self.profiles_dir.join(name);
        if profile_dir.exists() {
            let has_files = profile_dir.read_dir()?.next().is_some();
            if has_files {
                bail!("Profile '{}' already exists. Remove it first or pick a different name.", name);
            }
        }
        fs::create_dir_all(&profile_dir)?;

        println!("Launching Claude Code — please log in with the account for profile '{}'.", name);
        println!("After logging in, exit Claude (Ctrl-C or /exit) to finish setup.\n");

        let status = std::process::Command::new("claude")
            .env("CLAUDE_CONFIG_DIR", &profile_dir)
            .status()
            .context("Failed to launch claude. Is it installed and in your PATH?")?;

        // Claude has exited — check if it wrote any config files
        let email = read_email_from_dir(&profile_dir);
        let profile = Profile {
            name: name.to_string(),
            email: email.clone(),
            added: Utc::now(),
            last_used: Some(Utc::now()),
        };
        self.upsert_profile(profile)?;

        let display_email = email.as_deref().unwrap_or("unknown");
        println!("\nProfile '{}' registered (account: {}).", name, display_email);
        println!("Launch with: cswitch use {}", name);

        std::process::exit(status.code().unwrap_or(0));
    }

    /// Print shell alias/function lines for all managed profiles.
    /// Auto-detects platform: bash/zsh on Unix, PowerShell on Windows.
    /// On Linux with ~/.bashrc.d/, writes to 38-claude-switch.sh and prints info.
    #[cfg(target_os = "windows")]
    pub fn generate_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        if profiles.is_empty() {
            return Ok("# No profiles found. Add one with: cswitch add <name>".to_string());
        }
        self.generate_powershell_aliases(&profiles)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn generate_aliases(&self) -> Result<String> {
        let profiles = self.list_profiles()?;
        if profiles.is_empty() {
            return Ok("# No profiles found. Add one with: cswitch add <name>".to_string());
        }
        self.generate_shell_aliases_with_bashrc_d(&profiles)
    }

    #[cfg(not(target_os = "windows"))]
    fn generate_shell_aliases_with_bashrc_d(&self, profiles: &[Profile]) -> Result<String> {
        use dirs::home_dir;

        let aliases_content = self.generate_shell_aliases(profiles)?;
        let home = home_dir().context("Cannot determine home directory")?;
        let bashrc_d = home.join(".bashrc.d");

        if bashrc_d.exists() && bashrc_d.is_dir() {
            // Write to ~/.bashrc.d/38-claude-switch.sh
            fs::create_dir_all(&bashrc_d)?;
            let alias_file = bashrc_d.join("38-claude-switch.sh");
            fs::write(&alias_file, &aliases_content)?;
            Ok(format!(
                "{}\n\n# Aliases written to: ~/.bashrc.d/38-claude-switch.sh\n# Add 'source ~/.bashrc.d/38-claude-switch.sh' to your ~/.bashrc if not already done.",
                aliases_content
            ))
        } else {
            Ok(aliases_content)
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn generate_shell_aliases(&self, profiles: &[Profile]) -> Result<String> {
        let mut lines = vec![
            "# claude-switch aliases — auto-generated by `cswitch aliases`".to_string(),
            "# Source this file from your ~/.bashrc or ~/.bash_profile:".to_string(),
            "#   source ~/.bashrc.d/38-claude-switch.sh".to_string(),
            String::new(),
        ];
        for p in profiles {
            let dir = self.profile_dir(&p.name);
            let comment = p
                .email
                .as_deref()
                .map(|e| format!("  # {}", e))
                .unwrap_or_default();
            lines.push(format!(
                "alias claude-{}=\"CLAUDE_CONFIG_DIR='{}' claude\"{}",
                p.name,
                dir.display(),
                comment
            ));
        }
        Ok(lines.join("\n"))
    }

    fn generate_powershell_aliases(&self, profiles: &[Profile]) -> Result<String> {
        let mut lines = vec![
            "# claude-switch aliases — add to your PowerShell profile".to_string(),
            "# Run: notepad $PROFILE  to edit your profile".to_string(),
            "# Generated by: cswitch aliases".to_string(),
            String::new(),
        ];
        for p in profiles {
            let dir = self.profile_dir(&p.name);
            let comment = p
                .email
                .as_deref()
                .map(|e| format!("  # {}", e))
                .unwrap_or_default();
            lines.push(format!(
                "function claude-{} {{ $env:CLAUDE_CONFIG_DIR='{}'; claude @args }}{}",
                p.name,
                dir.display(),
                comment
            ));
        }
        Ok(lines.join("\n"))
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn copy_and_build_profile(&self, name: &str, src: &Path) -> Result<Profile> {
        let dest = self.profiles_dir.join(name);
        copy_dir_all(src, &dest)?;
        let email = read_email_from_dir(&dest);
        Ok(Profile { name: name.to_string(), email, added: Utc::now(), last_used: None })
    }

    fn upsert_profile(&self, profile: Profile) -> Result<()> {
        let mut registry = self.load_registry()?;
        registry.profiles.insert(profile.name.clone(), profile);
        self.save_registry(&registry)
    }
}

// ── First-run detection ───────────────────────────────────────────────────────

/// Account details read from the live `~/.claude` directory.
pub struct DetectedAccount {
    pub email: Option<String>,
    #[allow(dead_code)]
    pub config_dir: std::path::PathBuf,
}

/// Try to read the currently logged-in Claude account.
/// Checks both `~/.claude/` (config dir) and `~/.claude.json` (home root)
/// since on macOS the account metadata lives at the root, not inside the dir.
/// Returns `None` if neither exists.
pub fn detect_current_account() -> Option<DetectedAccount> {
    let home = dirs::home_dir()?;
    let config_dir = home.join(".claude");
    if !config_dir.exists() {
        return None;
    }
    // Try ~/.claude/ first, then fallback to ~/.claude.json at home root
    let email = read_email_from_dir(&config_dir)
        .or_else(|| read_email_from_home_root(&home));
    Some(DetectedAccount { email, config_dir })
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    let dir = match fs::read_dir(src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Warning: cannot read '{}': {}", src.display(), e);
            return Ok(());
        }
    };
    for entry in dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: cannot read entry in '{}': {}", src.display(), e);
                continue;
            }
        };
        let dest_path = dst.join(entry.file_name());

        let is_symlink = entry
            .path()
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        if is_symlink {
            if let Ok(target) = fs::read_link(&entry.path()) {
                if !copy_symlink(&target, &dest_path) {
                    if target.is_dir() {
                        copy_dir_all(&target, &dest_path)?;
                    } else if let Err(e) = fs::copy(&target, &dest_path) {
                        eprintln!("Warning: cannot copy '{}': {}", entry.path().display(), e);
                    }
                }
            }
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                eprintln!("Warning: cannot stat '{}': {}", entry.path().display(), e);
                continue;
            }
        };

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else if let Err(e) = fs::copy(entry.path(), &dest_path) {
            eprintln!("Warning: cannot copy '{}': {}", entry.path().display(), e);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn copy_symlink(target: &Path, dest: &Path) -> bool {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let result = if target.is_dir() {
        symlink_dir(target, dest)
    } else {
        symlink_file(target, dest)
    };
    result.is_ok()
}

#[cfg(not(target_os = "windows"))]
fn copy_symlink(target: &Path, dest: &Path) -> bool {
    std::os::unix::fs::symlink(target, dest).is_ok()
}


/// Read email from `~/.claude.json` at home root (macOS stores account metadata here).
fn read_email_from_home_root(home: &Path) -> Option<String> {
    let path = home.join(".claude.json");
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(email) = val
                .get("oauthAccount")
                .and_then(|o| o.get("emailAddress"))
                .and_then(|e| e.as_str())
            {
                return Some(email.to_string());
            }
        }
    }
    None
}

/// Extract the account email from a Claude config directory.
/// Checks `.claude.json` → `oauthAccount.emailAddress`, then
/// `.credentials.json` → `claudeAiOauth.email` as fallback.
fn read_email_from_dir(dir: &Path) -> Option<String> {
    for filename in &[".claude.json", "claude.json"] {
        if let Ok(content) = fs::read_to_string(dir.join(filename)) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(email) = val
                    .get("oauthAccount")
                    .and_then(|o| o.get("emailAddress"))
                    .and_then(|e| e.as_str())
                {
                    return Some(email.to_string());
                }
            }
        }
    }
    if let Ok(content) = fs::read_to_string(dir.join(".credentials.json")) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(email) = val
                .get("claudeAiOauth")
                .and_then(|o| o.get("email"))
                .and_then(|e| e.as_str())
            {
                return Some(email.to_string());
            }
        }
    }
    None
}

// ══════════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Construct a ProfileManager fully isolated inside a temp directory.
    fn make_manager(tmp: &TempDir) -> ProfileManager {
        let base_dir = tmp.path().join(".claude-switch");
        let profiles_dir = base_dir.join("profiles");
        let registry_path = base_dir.join("registry.json");
        fs::create_dir_all(&profiles_dir).unwrap();
        ProfileManager { base_dir, profiles_dir, registry_path }
    }

    /// Populate a fake `~/.claude` directory with the two files Claude Code
    /// actually writes: `.claude.json` and `.credentials.json`.
    fn make_claude_dir(root: &Path, email: &str) -> PathBuf {
        let dir = root.to_path_buf();
        fs::create_dir_all(&dir).unwrap();

        // .claude.json — contains oauthAccount block
        let claude_json = serde_json::json!({
            "oauthAccount": {
                "emailAddress": email,
                "accountUuid": "uuid-0000-test"
            },
            "someOtherConfig": true
        });
        fs::write(
            dir.join(".claude.json"),
            serde_json::to_string_pretty(&claude_json).unwrap(),
        )
        .unwrap();

        // .credentials.json — contains OAuth tokens
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

    /// Same but email is only in `.credentials.json` to test the fallback path.
    fn make_claude_dir_creds_only(root: &Path, email: &str) -> PathBuf {
        let dir = root.to_path_buf();
        fs::create_dir_all(&dir).unwrap();

        let creds_json = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "tok",
                "email": email
            }
        });
        fs::write(
            dir.join(".credentials.json"),
            serde_json::to_string_pretty(&creds_json).unwrap(),
        )
        .unwrap();

        dir
    }

    // ── read_email_from_dir ───────────────────────────────────────────────────

    #[test]
    fn email_read_from_claude_json() {
        let tmp = TempDir::new().unwrap();
        let dir = make_claude_dir(tmp.path(), "oauth@test.com");
        assert_eq!(read_email_from_dir(&dir), Some("oauth@test.com".into()));
    }

    #[test]
    fn email_fallback_to_credentials_json() {
        let tmp = TempDir::new().unwrap();
        let dir = make_claude_dir_creds_only(tmp.path(), "creds@test.com");
        assert_eq!(read_email_from_dir(&dir), Some("creds@test.com".into()));
    }

    #[test]
    fn email_returns_none_when_no_config_files() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        assert_eq!(read_email_from_dir(tmp.path()), None);
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
        assert_eq!(fs::read_to_string(dst.join("sub/deep/leaf.txt")).unwrap(), "leaf");
    }

    // ── Registry I/O ──────────────────────────────────────────────────────────

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

        let mut reg = Registry::default();
        reg.profiles.insert(
            "work".into(),
            Profile {
                name: "work".into(),
                email: Some("work@acme.com".into()),
                added: Utc::now(),
                last_used: None,
            },
        );
        mgr.save_registry(&reg).unwrap();

        let loaded = mgr.load_registry().unwrap();
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles["work"].email.as_deref(), Some("work@acme.com"));
        assert!(loaded.profiles["work"].last_used.is_none());
    }

    // ── add_profile_from ──────────────────────────────────────────────────────

    #[test]
    fn add_profile_copies_files_into_profiles_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "u@test.com");

        mgr.add_profile_from("work", &src).unwrap();

        let dest = mgr.profile_dir("work");
        assert!(dest.join(".claude.json").exists(), ".claude.json missing");
        assert!(dest.join(".credentials.json").exists(), ".credentials.json missing");
    }

    #[test]
    fn add_profile_records_email_from_config() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "email@test.com");

        let p = mgr.add_profile_from("personal", &src).unwrap();

        assert_eq!(p.email.as_deref(), Some("email@test.com"));
    }

    #[test]
    fn add_profile_records_entry_in_registry() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "x@y.com");

        mgr.add_profile_from("slot", &src).unwrap();

        let reg = mgr.load_registry().unwrap();
        assert!(reg.profiles.contains_key("slot"));
    }

    #[test]
    fn add_profile_stores_none_email_when_config_unreadable() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);

        // Source dir exists but contains no recognisable config files
        let src = tmp.path().join("empty-claude");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("something-unrelated.txt"), "hi").unwrap();

        let p = mgr.add_profile_from("mystery", &src).unwrap();
        assert!(p.email.is_none());
    }

    #[test]
    fn add_profile_errors_on_nonexistent_source() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let err = mgr
            .add_profile_from("bad", &tmp.path().join("does-not-exist"))
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[test]
    fn add_profile_errors_on_duplicate_without_force() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "a@b.com");

        mgr.add_profile_from("dup", &src).unwrap();
        let err = mgr.add_profile_from("dup", &src).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    // ── add_profile_from_force ────────────────────────────────────────────────

    #[test]
    fn force_add_overwrites_existing_profile() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);

        let src = make_claude_dir(&tmp.path().join("v1"), "first@test.com");
        mgr.add_profile_from("slot", &src).unwrap();

        // Change source to a different account
        let src2 = make_claude_dir(&tmp.path().join("v2"), "second@test.com");
        mgr.add_profile_from_force("slot", &src2).unwrap();

        let reg = mgr.load_registry().unwrap();
        assert_eq!(reg.profiles["slot"].email.as_deref(), Some("second@test.com"));
        // Old files replaced
        let content = fs::read_to_string(mgr.profile_dir("slot").join(".claude.json")).unwrap();
        assert!(content.contains("second@test.com"));
    }

    #[test]
    fn force_add_works_when_profile_does_not_yet_exist() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "new@test.com");

        let p = mgr.add_profile_from_force("brand-new", &src).unwrap();
        assert_eq!(p.name, "brand-new");
        assert_eq!(p.email.as_deref(), Some("new@test.com"));
    }

    // ── list_profiles ─────────────────────────────────────────────────────────

    #[test]
    fn list_profiles_returns_empty_vec_when_none_added() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        assert!(mgr.list_profiles().unwrap().is_empty());
    }

    #[test]
    fn list_profiles_returns_sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);

        for name in &["zebra", "alpha", "mango"] {
            let src = make_claude_dir(
                &tmp.path().join(format!("src-{name}")),
                &format!("{name}@test.com"),
            );
            mgr.add_profile_from(name, &src).unwrap();
        }

        let profiles = mgr.list_profiles().unwrap();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mango", "zebra"]);
    }

    // ── remove_profile ────────────────────────────────────────────────────────

    #[test]
    fn remove_profile_deletes_directory_and_registry_entry() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "del@test.com");
        mgr.add_profile_from("to-delete", &src).unwrap();

        mgr.remove_profile("to-delete").unwrap();

        assert!(!mgr.profile_dir("to-delete").exists());
        assert!(!mgr.load_registry().unwrap().profiles.contains_key("to-delete"));
    }

    #[test]
    fn remove_profile_errors_when_profile_not_found() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let err = mgr.remove_profile("ghost").unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn remove_profile_leaves_other_profiles_intact() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);

        for name in &["keep", "delete-me"] {
            let src = make_claude_dir(&tmp.path().join(name), &format!("{name}@x.com"));
            mgr.add_profile_from(name, &src).unwrap();
        }

        mgr.remove_profile("delete-me").unwrap();

        let profiles = mgr.list_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "keep");
    }

    // ── get_profile ───────────────────────────────────────────────────────────

    #[test]
    fn get_profile_returns_correct_entry() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "found@test.com");
        mgr.add_profile_from("found", &src).unwrap();

        let p = mgr.get_profile("found").unwrap();
        assert_eq!(p.name, "found");
        assert_eq!(p.email.as_deref(), Some("found@test.com"));
    }

    #[test]
    fn get_profile_errors_when_missing() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let err = mgr.get_profile("nope").unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    // ── profile_dir ───────────────────────────────────────────────────────────

    #[test]
    fn profile_dir_returns_correct_path() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        assert_eq!(mgr.profile_dir("foo"), mgr.profiles_dir.join("foo"));
    }

    // ── generate_aliases ──────────────────────────────────────────────────────

    #[test]
    fn generate_aliases_when_empty_returns_hint() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let out = mgr.generate_aliases().unwrap();
        assert!(out.contains("No profiles"), "{out}");
    }

    #[test]
    fn generate_aliases_includes_all_profiles_with_config_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);

        for name in &["work", "personal"] {
            let src = make_claude_dir(&tmp.path().join(name), &format!("{name}@x.com"));
            mgr.add_profile_from(name, &src).unwrap();
        }

        let out = mgr.generate_aliases().unwrap();
        #[cfg(target_os = "windows")]
        {
            assert!(out.contains("function claude-work"), "{out}");
            assert!(out.contains("function claude-personal"), "{out}");
            assert!(out.contains("$env:CLAUDE_CONFIG_DIR="), "{out}");
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(out.contains("alias claude-work="), "{out}");
            assert!(out.contains("alias claude-personal="), "{out}");
            assert!(out.contains("CLAUDE_CONFIG_DIR="), "{out}");
        }
    }

    // ── login_profile ──────────────────────────────────────────────────────

    #[test]
    fn login_profile_rejects_existing_nonempty_dir() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake-claude"), "a@b.com");
        mgr.add_profile_from("taken", &src).unwrap();

        // login_profile should refuse because the dir is non-empty
        let err = mgr.login_profile("taken").unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    // ── read_email_from_home_root ─────────────────────────────────────────

    #[test]
    fn read_email_from_home_root_finds_oauth_account() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let claude_json = serde_json::json!({
            "oauthAccount": {
                "emailAddress": "root@test.com",
                "accountUuid": "uuid"
            },
            "numStartups": 42
        });
        fs::write(
            root.join(".claude.json"),
            serde_json::to_string_pretty(&claude_json).unwrap(),
        )
        .unwrap();

        assert_eq!(read_email_from_home_root(root), Some("root@test.com".into()));
    }

    #[test]
    fn read_email_from_home_root_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(read_email_from_home_root(tmp.path()), None);
    }

    #[test]
    fn generate_aliases_includes_email_comment() {
        let tmp = TempDir::new().unwrap();
        let mgr = make_manager(&tmp);
        let src = make_claude_dir(&tmp.path().join("fake"), "me@work.com");
        mgr.add_profile_from("work", &src).unwrap();

        let out = mgr.generate_aliases().unwrap();
        assert!(out.contains("# me@work.com"), "{out}");
    }
}
