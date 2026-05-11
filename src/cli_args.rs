// Known Claude Code CLI flags — canonical list.
//
// HOW TO UPDATE:
//   1. Fetch the latest docs:
//        tinyfish fetch content get --format markdown "https://code.claude.com/docs/en/cli-reference"
//   2. Extract every `--flag` from the markdown tables and add to the appropriate
//      category below. Include short form (e.g. `-p`) if present.
//   3. Run `cargo test` to verify.
//
// Last synced: 2026-05-11 from https://code.claude.com/docs/en/cli-reference

/// All known Claude Code CLI flags, grouped by category.
/// Used for autocomplete hints in the TUI when editing launch arguments.
#[allow(dead_code)]
pub const KNOWN_CLI_FLAGS: &[(&str, &[&str])] = &[
    // ── Core ─────────────────────────────────────────────────────────────────
    ("Core", &[
        "-p, --print",
        "-c, --continue",
        "-r, --resume",
        "-n, --name",
        "--session-id",
        "--fork-session",
        "--from-pr",
        "-v, --version",
    ]),
    // ── Model / effort ──────────────────────────────────────────────────────
    ("Model / effort", &[
        "--model",
        "--effort",
        "--fallback-model",
        "--betas",
        "--max-turns",
        "--max-budget-usd",
    ]),
    // ── Permission modes ────────────────────────────────────────────────────
    ("Permission modes", &[
        "--permission-mode",
        "--dangerously-skip-permissions",
        "--allow-dangerously-skip-permissions",
        "--allowedTools",
        "--disallowedTools",
        "--tools",
        "--permission-prompt-tool",
    ]),
    // ── System prompt ───────────────────────────────────────────────────────
    ("System prompt", &[
        "--system-prompt",
        "--system-prompt-file",
        "--append-system-prompt",
        "--append-system-prompt-file",
    ]),
    // ── Output / debugging ──────────────────────────────────────────────────
    ("Output / debugging", &[
        "--debug",
        "--debug-file",
        "--verbose",
        "--output-format",
        "--input-format",
        "--include-partial-messages",
        "--include-hook-events",
        "--replay-user-messages",
        "--json-schema",
    ]),
    // ── Session / config ────────────────────────────────────────────────────
    ("Session / config", &[
        "--add-dir",
        "--settings",
        "--setting-sources",
        "--mcp-config",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--disable-slash-commands",
        "--bare",
        "--init",
        "--init-only",
        "--maintenance",
        "--exclude-dynamic-system-prompt-sections",
    ]),
    // ── Plugins / subagents ─────────────────────────────────────────────────
    ("Plugins / subagents", &[
        "--agent",
        "--agents",
        "--plugin-dir",
        "--plugin-url",
        "--teammate-mode",
    ]),
    // ── Environment / UI ────────────────────────────────────────────────────
    ("Environment / UI", &[
        "--ide",
        "--chrome",
        "--no-chrome",
        "--channels",
        "--dangerously-load-development-channels",
        "--teleport",
        "--remote",
        "--remote-control",
        "--remote-control-session-name-prefix",
        "--tmux",
        "--worktree", "-w",
    ]),
];

/// Return a flat, sorted, deduplicated slice of every known flag name
/// (long form only, e.g. `--dangerously-skip-permissions`).
pub fn all_flag_names() -> &'static [&'static str] {
    use std::sync::OnceLock;
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut v: Vec<&str> = Vec::new();
        for (_, flags) in KNOWN_CLI_FLAGS {
            for f in *flags {
                // Extract the long form: last comma-separated part
                if let Some(long) = f.split(", ").last() {
                    v.push(long);
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    })
    .as_slice()
}

/// Common preset launch-arg combinations for quick selection.
#[allow(dead_code)]
pub const LAUNCH_PRESETS: &[(&str, &str)] = &[
    ("unsafe", "--dangerously-skip-permissions"),
    ("plan", "--permission-mode plan"),
    ("accept-edits", "--permission-mode acceptEdits"),
    ("verbose", "--verbose"),
    ("bare", "--bare"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_flags() {
        let mut seen = HashSet::new();
        for (cat, flags) in KNOWN_CLI_FLAGS {
            for f in *flags {
                assert!(seen.insert(f), "duplicate flag '{f}' in category '{cat}'");
            }
        }
    }

    #[test]
    fn all_flag_names_non_empty() {
        assert!(!all_flag_names().is_empty());
    }
}