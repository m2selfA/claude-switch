use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

const CLAUDE_SWITCH_HOME_ENV: &str = "CLAUDE_SWITCH_HOME";
const MISSING_PROFILE: &str = "__stderr_stream_test_missing_profile__";
const MISSING_PROFILE_ID: &str = "__stderr_stream_test_missing_profile_id__";

fn cswitch_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cswitch"))
}

fn run_cswitch(args: &[&str]) -> std::process::Output {
    let home = TempDir::new().unwrap();
    Command::new(cswitch_bin())
        .args(args)
        .env(CLAUDE_SWITCH_HOME_ENV, home.path())
        .output()
        .unwrap()
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn use_missing_profile_reports_only_to_stderr() {
    let output = run_cswitch(&["use", MISSING_PROFILE]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr_text(&output).contains(&format!("Profile '{MISSING_PROFILE}' not found")));
}

#[test]
fn invalid_local_gateway_mode_reports_only_to_stderr() {
    let output = run_cswitch(&["use", MISSING_PROFILE, "--local-gateway-mode", "bad-mode"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr_text(&output).contains("Invalid --local-gateway-mode 'bad-mode'"));
}

#[test]
fn shim_launch_without_profile_id_reports_only_to_stderr() {
    let output = run_cswitch(&["shim", "launch"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr_text(&output).contains("Missing --profile-id for shim launch"));
}

#[test]
fn shim_launch_missing_profile_reports_only_to_stderr() {
    let output = run_cswitch(&["shim", "launch", "--profile-id", MISSING_PROFILE_ID]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(stderr_text(&output).contains(&format!("Profile '{MISSING_PROFILE_ID}' not found")));
}

#[test]
fn shim_launch_probe_is_silent() {
    let output = run_cswitch(&["shim", "launch", "--probe"]);

    assert!(output.status.success());
    assert!(stdout_text(&output).is_empty());
    assert!(stderr_text(&output).is_empty());
}
