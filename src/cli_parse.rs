use anyhow::{Result, bail};

pub(crate) fn parse_key_values(
    entries: &[String],
    flag_name: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for entry in entries {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("{} expects KEY=VALUE, got '{}'.", flag_name, entry))?;
        let key = key.trim();
        if key.is_empty() {
            bail!("{} entry has an empty key.", flag_name);
        }
        map.insert(key.to_string(), value.trim().to_string());
    }
    Ok(map)
}

pub(crate) fn parse_optional_json(
    input: Option<&str>,
    flag_name: &str,
) -> Result<Option<serde_json::Value>> {
    input
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|err| anyhow::anyhow!("{} must be valid JSON: {}", flag_name, err))
        })
        .transpose()
}

pub(crate) fn default_shell_name() -> String {
    #[cfg(windows)]
    {
        "powershell".to_string()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL")
            .ok()
            .and_then(|shell| {
                std::path::Path::new(&shell)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "bash".to_string())
    }
}

fn normalize_auto_shell_name(shell: &str) -> String {
    let normalized = shell.trim().to_ascii_lowercase();
    if normalized.contains("pwsh") || normalized.contains("powershell") {
        "powershell".to_string()
    } else if normalized.contains("fish") {
        "fish".to_string()
    } else if normalized.contains("zsh") {
        "zsh".to_string()
    } else {
        "bash".to_string()
    }
}

pub(crate) fn render_shell_hook(shell: &str) -> Result<String> {
    let shell = if shell == "auto" {
        normalize_auto_shell_name(&default_shell_name())
    } else {
        shell.to_ascii_lowercase()
    };
    match shell.as_str() {
        "powershell" | "pwsh" => Ok(r#"function claude {
    $profile = $null
    $profile = (& cswitch shell current --dir (Get-Location).Path 2>$null)
    if ($LASTEXITCODE -eq 0 -and $profile) {
        & cswitch use $profile -- @args
        return
    }
    $cmd = Get-Command claude.exe -CommandType Application -ErrorAction SilentlyContinue
    if (-not $cmd) {
        throw "claude.exe was not found on PATH"
    }
    & $cmd.Source @args
}"#
        .to_string()),
        "bash" | "zsh" => Ok(r#"# claude-switch project auto-profile hook
claude() {
  local _cswitch_profile
  _cswitch_profile="$(command cswitch shell current --dir "${PWD}" 2>/dev/null)" || _cswitch_profile=""
  if [ -n "${_cswitch_profile}" ]; then
    command cswitch use "${_cswitch_profile}" -- "$@"
    return
  fi
  command claude "$@"
}"#
        .to_string()),
        "fish" => Ok(r#"function claude
    set -l _cswitch_profile (command cswitch shell current --dir (pwd) 2>/dev/null)
    if test $status -eq 0 -a -n "$_cswitch_profile"
        command cswitch use "$_cswitch_profile" -- $argv
        return
    end
    command claude $argv
end"#
            .to_string()),
        other => bail!(
            "Unsupported shell '{}'. Use auto, powershell, bash, zsh, or fish.",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalize_auto_shell_name_lowercases_and_recognizes_zsh() {
        assert_eq!(super::normalize_auto_shell_name("Zsh"), "zsh");
    }

    #[test]
    fn normalize_auto_shell_name_falls_back_to_bash_for_sh() {
        assert_eq!(super::normalize_auto_shell_name("sh"), "bash");
    }

    #[test]
    fn bash_shell_hook_invokes_cswitch_as_command() {
        let hook = super::render_shell_hook("bash").unwrap();
        assert!(hook.contains("$(command cswitch shell current"));
        assert!(hook.contains("command cswitch use"));
    }

    #[test]
    fn fish_shell_hook_invokes_cswitch_as_command() {
        let hook = super::render_shell_hook("fish").unwrap();
        assert!(hook.contains("(command cswitch shell current"));
        assert!(hook.contains("command cswitch use"));
    }

    #[cfg(not(windows))]
    struct ShellEnvGuard {
        old_shell: Option<std::ffi::OsString>,
    }

    #[cfg(not(windows))]
    impl ShellEnvGuard {
        fn set(shell: &str) -> Self {
            let old_shell = std::env::var_os("SHELL");
            unsafe {
                std::env::set_var("SHELL", shell);
            }
            Self { old_shell }
        }
    }

    #[cfg(not(windows))]
    impl Drop for ShellEnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(old_shell) = &self.old_shell {
                    std::env::set_var("SHELL", old_shell);
                } else {
                    std::env::remove_var("SHELL");
                }
            }
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn auto_shell_hook_accepts_mixed_case_zsh_basename() {
        let _guard = ShellEnvGuard::set("/usr/local/bin/Zsh");
        let hook = super::render_shell_hook("auto").unwrap();
        assert!(hook.contains("command claude \"$@\""));
    }

    #[cfg(not(windows))]
    #[test]
    fn auto_shell_hook_falls_back_to_bash_for_sh() {
        let _guard = ShellEnvGuard::set("/bin/sh");
        let hook = super::render_shell_hook("auto").unwrap();
        assert!(hook.contains("command claude \"$@\""));
    }
}
