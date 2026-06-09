use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

mod api_test;
mod artifacts;
mod auth_migration;
mod config_bundle;
mod diagnostics;
mod launch;
mod local_shims;
mod mcp;
mod mcp_config;
mod mcp_registry;
mod mcp_secrets;
mod mcp_validation;
mod profiles;
mod providers;
mod remote_alias_sync;
mod remote_sftp;
mod runtime_sessions;
mod shim_recovery;
mod shims;
mod storage;
mod tinyfish;
mod types;
mod url_match;
pub use self::api_test::*;
pub(crate) use self::launch::LaunchOptions;
pub(crate) use self::tinyfish::tinyfish_available;
use self::tinyfish::{
    LightweightRuntimeArtifacts, TinyfishToolShell, build_lightweight_runtime_artifacts,
    build_lightweight_runtime_artifacts_with_local_gateway_mode, build_lightweight_settings,
    build_lightweight_settings_with_local_gateway_mode, native_tinyfish_tool_shell,
    tinyfish_plugin_script_file_name, tinyfish_statusline_script_file_name,
};
pub use self::types::*;
pub(crate) use self::url_match::{base_url_host, is_local_runtime_base_url};

const CLAUDE_SWITCH_HOME_ENV: &str = "CLAUDE_SWITCH_HOME";

fn escape_cmd_json_fragment(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len() * 2);
    for ch in fragment.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '^' => out.push_str("^^"),
            _ => out.push(ch),
        }
    }
    out
}

fn assign_cmd_json_var(lines: &mut Vec<String>, var_name: &str, json: &str) {
    let escaped = escape_cmd_json_fragment(json);
    lines.push(format!("set \"{var_name}={escaped}\""));
}

#[cfg(target_os = "windows")]
fn local_command_pathexts() -> Vec<OsString> {
    env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(OsString::from)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| {
            vec![
                OsString::from(".COM"),
                OsString::from(".EXE"),
                OsString::from(".BAT"),
                OsString::from(".CMD"),
            ]
        })
}

fn local_command_candidates_for_paths(program: &str, paths: Option<OsString>) -> Vec<OsString> {
    let program = program.trim();
    if program.is_empty() {
        return Vec::new();
    }
    if program.contains('/') || program.contains('\\') {
        return vec![OsString::from(program)];
    }

    let mut candidates = Vec::new();
    if let Some(paths) = paths {
        #[cfg(target_os = "windows")]
        let has_extension = Path::new(program).extension().is_some();
        #[cfg(target_os = "windows")]
        let pathexts = (!has_extension).then(local_command_pathexts);

        for dir in env::split_paths(&paths) {
            #[cfg(target_os = "windows")]
            if let Some(pathexts) = pathexts.as_ref() {
                for ext in pathexts {
                    let mut file_name = OsString::from(program);
                    file_name.push(ext);
                    candidates.push(dir.join(&file_name).into_os_string());
                }
            }
            candidates.push(dir.join(program).into_os_string());
        }
    }

    candidates.push(OsString::from(program));
    candidates
}

fn local_command_candidates(program: &str) -> Vec<OsString> {
    local_command_candidates_for_paths(program, env::var_os("PATH"))
}

fn with_local_command_candidates_for_paths<T, F>(
    program: &str,
    paths: Option<OsString>,
    mut runner: F,
) -> io::Result<T>
where
    F: FnMut(&OsStr) -> io::Result<T>,
{
    let candidates = local_command_candidates_for_paths(program, paths);
    let direct_program = OsString::from(program.trim());
    if candidates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "local command name is empty",
        ));
    }

    let mut last_error = None;
    for candidate in candidates {
        if candidate != direct_program && !Path::new(&candidate).is_file() {
            continue;
        }
        match runner(candidate.as_os_str()) {
            Ok(value) => return Ok(value),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Failed to resolve local command '{}'", program.trim()),
        )
    }))
}

fn with_local_command_candidates<T, F>(program: &str, mut runner: F) -> io::Result<T>
where
    F: FnMut(&OsStr) -> io::Result<T>,
{
    with_local_command_candidates_for_paths(program, env::var_os("PATH"), move |resolved| {
        runner(resolved)
    })
}

fn build_local_command(program: &str) -> Command {
    let program = program.trim();
    for candidate in local_command_candidates(program) {
        if Path::new(&candidate).is_file() {
            return Command::new(candidate);
        }
    }
    Command::new(program)
}

fn local_command_exists(program: &str) -> bool {
    let program = program.trim();
    if program.is_empty() {
        return false;
    }
    if program.contains('/') || program.contains('\\') {
        return Path::new(program).is_file();
    }

    local_command_candidates(program)
        .into_iter()
        .filter(|candidate| candidate != program)
        .any(|candidate| Path::new(&candidate).is_file())
}

// ── ProfileManager ────────────────────────────────────────────────────────────

const CMD_MARKER: &str = ":: Generated by cswitch (claude-switch) — do not edit manually";
const SH_MARKER: &str = "# Generated by cswitch (claude-switch) — do not edit manually";

pub struct ProfileManager {
    pub profiles_dir: PathBuf,
    registry_path: PathBuf,
}

impl ProfileManager {}

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
            if let Ok(target) = fs::read_link(entry.path())
                && !copy_symlink(&target, &dest_path)
            {
                if target.is_dir() {
                    copy_dir_all(&target, &dest_path)?;
                } else if let Err(e) = fs::copy(&target, &dest_path) {
                    eprintln!("Warning: cannot copy '{}': {}", entry.path().display(), e);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOs {
    Unix,
    Windows,
}

// ══════════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests;
