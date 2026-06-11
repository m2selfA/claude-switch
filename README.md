# claude-switch

Multi-account profile manager for [Claude Code](https://code.claude.com).

Switch between multiple Claude accounts without logging out. Each profile is fully isolated — use different accounts in different terminals simultaneously.

Two isolation modes:
- **Full** — copies `~/.claude` into an isolated directory per profile
- **Lightweight** — stores API key, base URL, and model settings as env vars

## Install

### Cargo (requires Rust)

```bash
cargo install cswitch --version 0.8.2
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/m2selfA/claude-switch/releases).

```bash
# macOS (Apple Silicon)
curl -fsSL https://github.com/m2selfA/claude-switch/releases/latest/download/cc-switch-aarch64-apple-darwin.tar.gz | tar xz
sudo mv cswitch /usr/local/bin/

# macOS (Intel)
curl -fsSL https://github.com/m2selfA/claude-switch/releases/latest/download/cc-switch-x86_64-apple-darwin.tar.gz | tar xz
sudo mv cswitch /usr/local/bin/

# Linux
curl -fsSL https://github.com/m2selfA/claude-switch/releases/latest/download/cc-switch-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv cswitch /usr/local/bin/
```

### From source

```bash
git clone https://github.com/m2selfA/claude-switch.git
cd claude-switch
cargo install --path .
```

## Quick start

```bash
# Save current ~/.claude as a full profile
cswitch add --full work

# Create a lightweight profile (env-var based, interactive prompts)
cswitch add my-api-key --alias mykey

# Launch with a profile (stored launch args are enabled by default)
cswitch use work

# Or create a provider-backed lightweight profile and link a key
cswitch provider add openrouter --url https://openrouter.ai/api --key sk-...
cswitch provider link mykey --provider prov_12345678 --key key_12345678

# Duplicate an existing profile with a new name
cswitch duplicate work "work copy"

# Add a hosted plugin marketplace, install a plugin, and link it to a profile
cswitch plugin marketplace add owner/repo
cswitch plugin install my-plugin@owner-repo
cswitch plugin link mykey my-plugin@owner-repo

# Add an MCP server and attach it to a lightweight profile
cswitch mcp add filesystem --command npx --arg -y --arg @modelcontextprotocol/server-filesystem --arg '${CLAUDE_PROJECT_DIR:-.}'
cswitch mcp link mykey filesystem

# Diagnose the local registry and generated files
cswitch doctor
cswitch config inspect
cswitch statusline --dir .

# Or open the interactive TUI
cswitch
```

## Commands

| Command | Description |
|---|---|
| `cswitch` | Open interactive TUI |
| `cswitch add <name>` | Add a lightweight profile (env vars, interactive prompts) |
| `cswitch add --full <name>` | Add a full profile (copies ~/.claude) |
| `cswitch add --alias <a> <name>` | Add with a short CLI-friendly alias |
| `cswitch duplicate <source> <new-name> [--alias <alias>]` | Duplicate a profile with a new name and optional alias |
| `cswitch use <name> [-- <claude-args>]` | Launch Claude Code with a profile; stored launch args are enabled by default |
| `cswitch use --no-extras <name>` | Launch without the profile's stored launch args |
| `cswitch use --local-gateway-mode <mode> <name> [-- <claude-args>]` | For localhost/LAN lightweight profiles, choose `auto`, `search-fetch`, `fetch-only`, or `gateway-only` for that launch |
| `cswitch list` | List all saved profiles |
| `cswitch info <name>` | Show details for a profile |
| `cswitch remove <name>` | Delete a profile |
| `cswitch aliases [--local] [--remote <host>]... [--verbose]` | Generate/sync local shell aliases and launchers, and/or sync remote launchers plus any required TinyFish prompt/plugin companion files via sftp (`--remote` is repeatable) |
| `cswitch doctor [--json] [--strict]` | Diagnose registry links, generated artifacts, MCP entries, and local runtime availability |
| `cswitch config inspect [--json]` | Show registry paths, generated artifact paths, and object counts |
| `cswitch config settings show [--json]` | Show persisted global claude-switch settings |
| `cswitch config settings set [--allow-local-runtime-hot-switch] [--json]` | Update the legacy localhost/LAN runtime override toggle |
| `cswitch config settings set --plugin-github-mirror-base-url <url> [--json]` | Set the preferred HTTPS mirror for GitHub-backed hosted plugin fetches |
| `cswitch config settings set --clear-plugin-github-mirror-base-url [--json]` | Disable the hosted-plugin GitHub mirror override |
| `cswitch config export [--profile <profile>]... [-o <file>] [--include-secrets]` | Export a portable config bundle; secrets are redacted unless explicitly included |
| `cswitch config export --format paseo [--profile <profile>]... [-o <file>] [--providers-only\|--full-config] [--include-secrets] [--with-extras] [--strict-model-discovery]` | Export selected profiles as Paseo `agents.providers` JSON |
| `cswitch config import <file> [--replace] [--dry-run] [--json]` | Import a portable config bundle, or preview the add/update plan without writing |
| `cswitch config validate <file> [--json] [--strict]` | Validate a config bundle before importing it |
| `cswitch config recover-shims <shim-dir> [--write] [--replace] [--json]` | Recover registry profiles/providers from generated `claude-*` shim files |
| `cswitch config migrate-auth [--write] [--json] [--remote <host>]...` | Preview or migrate Claude token-based settings auth to `apiKeyHelper`, locally and optionally on remotes |
| `cswitch paseo export [--profile <profile>]... [-o <file>] [--providers-only\|--full-config] [--include-secrets] [--with-extras] [--strict-model-discovery]` | Export selected profiles directly as Paseo `agents.providers` JSON |
| `cswitch statusline [--profile <profile>] [--dir <path>] [--json]` | Print a compact current-profile summary for prompts/status bars |
| `cswitch shell hook [--shell <auto\|powershell\|bash\|zsh\|fish>]` | Print a shell wrapper that auto-selects project profiles from marker files |
| `cswitch shell current [--dir <path>]` | Resolve the project profile selected by `.cswitch-profile` or `.claudeprofile` |
| `cswitch provider list` | List shared API providers |
| `cswitch provider add <name> --url <url> --key <key>` | Add a shared provider with an initial key |
| `cswitch provider keys <provider-id>` | List keys for a provider |
| `cswitch provider add-key <provider-id> --name <name> --key <key>` | Add another key to a provider |
| `cswitch provider edit <provider-id> [--name <name>] [--url <url>]` | Edit a provider's name or base URL |
| `cswitch provider edit-key <provider-id> <key-id> [--name <name>] [--key <key>]` | Edit a provider key's name or token |
| `cswitch provider rename-key <provider-id> <key-id> --name <name>` | Rename a provider key without changing its token |
| `cswitch provider remove <provider-id>` | Remove a provider |
| `cswitch provider remove-key <provider-id> <key-id>` | Remove a key from a provider |
| `cswitch provider link <profile> --provider <provider-id> --key <key-id>` | Link a lightweight profile to a provider key |
| `cswitch provider unlink <profile>` | Remove provider/key association from a profile |
| `cswitch mcp list` | List saved MCP servers |
| `cswitch mcp add <name> --command <cmd> [--arg <arg>]...` | Add a stdio MCP server |
| `cswitch mcp add <name> --type http --url <url> [--header KEY=VALUE]...` | Add a remote MCP server |
| `cswitch mcp link <profile> <mcp>... [--replace]` | Select MCP servers for a lightweight profile |
| `cswitch mcp unlink <profile> <mcp>...` | Remove selected MCP servers from a lightweight profile |
| `cswitch mcp export [<mcp>...] [--all] [-o <file>]` | Export saved MCP servers as Claude-compatible `mcp.json` content |
| `cswitch mcp import <file> [--replace]` | Import MCP servers from a Claude-compatible `mcp.json` / `.mcp.json` file |
| `cswitch mcp validate [<mcp>...] [--all] [--strict]` | Validate saved MCP server entries and report missing commands, stale fields, deprecated SSE, and disabled servers |
| `cswitch plugin marketplace list` | List configured hosted plugin marketplaces |
| `cswitch plugin marketplace add <locator> [--replace]` | Add a hosted plugin marketplace from `owner/repo`, a git URL, or a local path |
| `cswitch plugin marketplace update <query>` | Refresh a configured hosted plugin marketplace cache |
| `cswitch plugin marketplace remove <query>` | Remove a hosted plugin marketplace |
| `cswitch plugin list` | List installed hosted plugins |
| `cswitch plugin show <query>` | Show one installed hosted plugin |
| `cswitch plugin install [<query>] [--marketplace <marketplace>] [--force]` | Install a hosted plugin from configured marketplaces |
| `cswitch plugin update [<query>]` | Update one installed hosted plugin, or every installed hosted plugin when omitted |
| `cswitch plugin uninstall <query> [--prune]` | Remove a hosted plugin and optionally prune orphaned dependencies |
| `cswitch plugin prune` | Remove orphaned dependency-only hosted plugin installs |
| `cswitch plugin link <profile> <plugin>... [--replace]` | Select hosted plugins for a profile |
| `cswitch plugin unlink <profile> <plugin>... [--all]` | Remove selected hosted plugins from a profile |
| `cswitch process list` | List runtime-managed lightweight Claude sessions |
| `cswitch process inspect <session-id>` | Show PID, provider, paths, and timestamps for one runtime-managed session |
| `cswitch process switch <session-id> --provider <id> --key <id> --model <id>` | Hot-switch a running runtime-managed session to a different provider/key/model |
| `cswitch process gc` | Remove stale runtime session directories |
| `cswitch --help` | Full CLI help |

## Interactive TUI

Run `cswitch` with no arguments to open the TUI.

### TUI keybindings

| Key | Action |
|---|---|
| `Ctrl+P/N` | Navigate lists and selections |
| `↑/↓` | Compatibility navigation keys |
| `Enter` | Launch the selected profile with stored launch args |
| `Shift+Enter` | Launch without stored launch args |
| `g` | For localhost/LAN lightweight profiles, open the explicit local gateway mode picker |
| `/` | Search profiles by name or alias |
| `t` | Add lightweight profile from provider/key |
| `T` | Batch-test all non-official provider keys from Profile Manager using one editable Anthropic prompt, with per-base-URL spacing and sorted results |
| `M` | Select MCP servers for the highlighted lightweight profile |
| `P` | Select hosted plugins for the highlighted profile |
| `c` | Duplicate the selected profile with prefilled name and alias |
| `Ctrl+Y` | Smart input provider/key from clipboard in Provider Manager |
| `MCP Manager: Ctrl+Y` | Smart input/import MCP JSON from clipboard |
| `MCP Manager: a/e/d` | Add, edit, or delete MCP servers |
| `Plugin Manager: a` | Open the hosted plugin install picker from configured marketplaces |
| `Plugin Manager: u/d` | Update or remove the selected hosted plugin |
| `Plugin Manager: Enter` | Refresh the linked-profile detail for the selected hosted plugin |
| `Provider Manager: t` | Discover models from provider-aware candidate endpoints; tries multiple URL patterns (e.g. `/v1`, `/api`, `/compatible-mode/v1`, `/anthropic` suffixes) and falls back to the root; failure does not prove the provider is unusable, and manual model names may still work |
| `a` | Add full profile (directory isolation) |
| `e` | Edit profile (name/alias/flags, or full model editor for lite) |
| `m` | Toggle [1m] suffix on model slots |
| `Tab` | Move to the next field, or complete/cycle values in model/provider editing flows |
| `Ctrl+P/N` | Move between fields in multi-step forms; in model test, Up/Down browse fetched models when the model field is focused |
| `Ctrl+A/E/B/F` | Move cursor in text fields |
| `Ctrl+H/D/K/U/W` | Edit text in Emacs style |
| `Ctrl+G` | Cancel or go back |
| `Shift+Tab` | Switch managers from manager/search/provider-key-list views |
| `r` | Refresh — re-copy ~/.claude into selected |
| `d` | Delete selected profile |
| `?` | Help overlay |
| `q` / `Esc` | Quit |

`j/k` is still accepted in some pure list views for compatibility, but it is no longer the documented primary navigation scheme.

For localhost/LAN lightweight profiles, plain `cswitch use <name>` and the unsuffixed generated shim stay on the self-contained inline-settings path by default. Use `g` in the TUI or `--local-gateway-mode auto` when you explicitly want the dynamic TinyFish auto-routing behavior back.

From Public Site provider-test results, `Shift+S` opens the runtime process switch picker. The Anthropic outcome popup also exposes the same action on `s`.

## Hosted plugins

Hosted plugins are managed independently from MCP servers. First add one or more plugin marketplaces, then install hosted plugins from those marketplaces, and finally link installed plugins to specific profiles.

```bash
cswitch plugin marketplace add owner/repo
cswitch plugin marketplace list
cswitch plugin install my-plugin@owner-repo
cswitch plugin list
cswitch plugin show my-plugin@owner-repo
cswitch plugin link mykey my-plugin@owner-repo
```

Use `cswitch plugin update` to refresh one installed hosted plugin or all installed hosted plugins, `cswitch plugin uninstall <plugin>` to remove one, and `cswitch plugin prune` to clean up orphaned dependency-only installs. In the TUI, `Shift+Tab` cycles into the Plugin Manager page, where `a` opens install, `u` updates, and `d` removes the selected hosted plugin.

## Runtime-managed sessions

For provider-backed lightweight profiles that are not local/self-hosted, `cswitch` can keep a runtime session directory under `~/.claude-switch/runtime/` and hot-switch an already running Claude process to a different provider, key, or model without restarting it.

```bash
cswitch process list
cswitch process inspect rt_12345678
cswitch process switch rt_12345678 --provider prov_12345678 --key key_12345678 --model claude-sonnet-4-20250514
cswitch process gc
```

Local/self-hosted lightweight profiles on `localhost`, `*.localhost`, `127.*`, `::1`, `10.*`, `192.168.*`, and `172.16-31.*` bypass runtime sessions by default. They launch directly with an inline `apiKeyHelper`; use explicit local gateway modes when you need TinyFish-assisted search/fetch routing.

## Shell aliases

Generate aliases so you can launch profiles directly:

```bash
cswitch aliases >> ~/.zshrc   # or ~/.bashrc
source ~/.zshrc
```

If `~/.bashrc.d/` exists, aliases are written to `~/.bashrc.d/38-claude-switch.sh`. Add to your `~/.bashrc`:

```bash
source ~/.bashrc.d/38-claude-switch.sh
```

This produces commands like:

```bash
claude-work       # launch with the "work" profile
claude-personal   # launch with the "personal" profile
```

On Windows, `cswitch aliases` outputs PowerShell functions for your `$PROFILE`, **and** syncs generated `.cmd` launchers into `~/.local/bin` (`%USERPROFILE%\.local\bin`). Ordinary lightweight profiles stay self-contained. Localhost/LAN lightweight profiles also default to a self-contained inline-settings launcher with no TinyFish plugin content unless you use an explicit local gateway mode (`claude-<alias>-search-fetch`, `claude-<alias>-fetch-only`, `claude-<alias>-gateway`, or `cswitch use --local-gateway-mode ...`). Profiles with selected MCP servers receive generated Claude plugin directories under `~/.claude-switch/generated/mcps/`, and the launcher passes them through `--plugin-dir` so MCP selections can combine cleanly with TinyFish plugins when needed. Each generated TinyFish plugin includes both `.claude-plugin/plugin.json` and `hooks/hooks.json`. Set `CLAUDE_SWITCH_TINYFISH=off` in a lightweight profile extra to suppress TinyFish injection for that profile. The launchers support `--no-extras`, and all generated files are added, updated, and cleaned up automatically when profiles change.

On Linux/macOS, if `~/.varusers/bin/` exists, `cswitch aliases` syncs generated bash launchers there instead of printing aliases. Non-TinyFish lightweight profiles remain self-contained. Localhost/LAN lightweight profiles also default to a self-contained inline-settings launcher with no TinyFish plugin content unless you use an explicit local gateway mode (`claude-<alias>-search-fetch`, `claude-<alias>-fetch-only`, `claude-<alias>-gateway`, or `cswitch use --local-gateway-mode ...`). Profiles with selected MCP servers receive generated Claude plugin directories under `~/.claude-switch/generated/mcps/`, and the launcher passes them through `--plugin-dir` so MCP selections can combine cleanly with TinyFish plugins when needed. Each generated TinyFish plugin includes both `.claude-plugin/plugin.json` and `hooks/hooks.json`. Set `CLAUDE_SWITCH_TINYFISH=off` in a lightweight profile extra to suppress TinyFish injection for that profile. Each script is executable, supports `--no-extras`, and is automatically maintained when profiles change. If the directory doesn't exist, the command falls back to printing bash/zsh aliases as before.

## Project auto-switch

`cswitch shell hook` prints a shell wrapper for `claude` that looks upward from the current directory for `.cswitch-profile` or `.claudeprofile`. The first non-empty, non-comment line is treated as a profile name, alias, or id. When a marker is found, the wrapper launches `cswitch use <profile> -- ...`; otherwise it falls back to the normal `claude` executable.

```bash
# bash/zsh
eval "$(cswitch shell hook --shell bash)"
printf '%s\n' mykey > .cswitch-profile
claude
```

```powershell
# PowerShell
cswitch shell hook --shell powershell | Add-Content $PROFILE
"mykey" | Set-Content .cswitch-profile
claude
```

Use `cswitch shell current --dir <path>` to debug which profile a project marker resolves to.

`cswitch statusline --dir <path>` uses the same marker resolution and prints a compact prompt/status-bar string. Use `--profile <name-or-alias>` to force a specific profile, or `--json` for prompt engines that prefer structured output.

## Diagnostics

`cswitch doctor` checks the registry, provider/key references, profile directories, MCP links, hosted plugin links, generated MCP plugin folders, and whether `claude` is on `PATH`. It reports warnings without failing by default; add `--strict` to make warnings or errors return a non-zero exit code. `cswitch config inspect --json` is intended for scripts that need the exact storage paths and object counts.

`cswitch config inspect` now reports plugin-related state as well, including the plugins root, configured plugin marketplace count, installed hosted plugin count, linked hosted-plugin references, generated MCP plugin count, and generated TinyFish directory count.

`cswitch config settings show` / `set` exposes the persisted global policy toggle for the legacy localhost/LAN runtime override plus the optional plugin GitHub mirror override used for GitHub-backed hosted plugin fetches. The local-runtime toggle is kept for compatibility, but local/self-hosted lightweight profiles still launch directly and do not regain dynamic process hot-switch support.

`cswitch config migrate-auth` previews a migration from token-based Claude settings auth to `apiKeyHelper`. Add `--write` to apply it, and repeat `--remote <host>` to migrate `~/.claude/settings.json` on remote machines as well.

```bash
cswitch config settings show
cswitch config settings set --allow-local-runtime-hot-switch
cswitch config settings set --plugin-github-mirror-base-url https://mirror.example.invalid/github
cswitch config settings set --clear-plugin-github-mirror-base-url
cswitch config migrate-auth
cswitch config migrate-auth --write --remote my-host
```

For automation or smoke tests, set `CLAUDE_SWITCH_HOME` to an isolated user-home root. `cswitch` will read/write `$CLAUDE_SWITCH_HOME/.claude-switch` and place generated local shims under that same root, instead of touching the real home directory.

## Config bundles

`cswitch config export` writes profiles, providers, saved MCP servers, plugin marketplaces, and installed hosted plugins into a portable bundle. Profile API tokens, provider keys, and likely MCP secrets in `env`, `headers`, and `oauth` are redacted by default; use `--include-secrets` only for trusted local backups. Add one or more `--profile <name-or-alias>` options to export only selected profiles plus their referenced provider keys, MCP servers, hosted plugins, and plugin marketplaces.

```bash
cswitch config export -o cswitch-bundle.json
cswitch config export --profile mg-ds --profile mykey -o selected-profiles.json
cswitch config validate selected-profiles.json --strict
cswitch config import selected-profiles.json --dry-run
cswitch config export --include-secrets -o cswitch-private-backup.json
cswitch config import cswitch-bundle.json --replace
```

`config import --dry-run` prints the exact add/update plan without writing `registry.json`; add `--json` for automation. On a real import, `--json` prints the applied import summary instead of the human-readable report. Validation and import summaries now include plugin marketplace and installed hosted plugin counts alongside profiles, providers, and MCP servers.

Full profile directories are not copied by config bundles; they only contain the registry metadata. Use shell/remote shim sync or normal file backup tools for full isolated `~/.claude` profile directories.

`config recover-shims` is a fallback for rebuilding registry entries from previously generated `claude-*` launchers. It previews by default and redacts recovered tokens from output; add `--write` to update `registry.json`. Written recoveries create a timestamped `registry.json.bak-*` first, synthesize providers and keys from recovered base URLs and tokens, and link recovered lightweight profiles to those keys.

```bash
cswitch config recover-shims ./shims
cswitch config recover-shims ./shims --write --replace
```

## Paseo export

`cswitch paseo export` and `cswitch config export --format paseo` turn saved profiles into Paseo-compatible provider entries. By default they emit an `agents` fragment:

```json
{
  "agents": {
    "providers": {
      "csw-work": {
        "extends": "claude",
        "label": "work",
        "command": ["cswitch", "use", "--no-extras", "<profile-id>", "--"]
      }
    }
  }
}
```

Use `--providers-only` to emit just the providers map, or `--full-config` to wrap the export with Paseo `$schema` and `version`. Without `--include-secrets`, exported providers stay on the `cswitch use` wrapper path and do not write API keys into the Paseo JSON. With `--include-secrets`, `claude-switch` prefers a self-contained `claude ... --settings ... --plugin-dir ...` command when the profile can be flattened safely; otherwise it warns on stderr and falls back to the wrapper form. `--strict-model-discovery` turns model-discovery warnings into a hard failure.

```bash
cswitch paseo export --profile work -o paseo-providers.json
cswitch config export --format paseo --providers-only --profile work
cswitch paseo export --include-secrets --with-extras --full-config -o paseo.config.json
```

## MCP import/export

`cswitch mcp export` writes a Claude-compatible JSON document with `$schema` and `mcpServers`. With no MCP names it exports all saved servers; pass specific ids/names to export a subset, or `-o mcp.json` to write a file.

```bash
cswitch mcp export github filesystem -o .mcp.json
cswitch mcp import .mcp.json --replace
cswitch mcp validate --all
```

Imported server names become the keys under `mcpServers`, matching Claude Code conventions. `--replace` updates existing same-name registry entries; without it, duplicate names fail fast.

You can also sync shims to remote machines with:

```bash
# Sync to a single remote host
cswitch aliases --remote my-host

# Sync to multiple remote hosts at once
cswitch aliases --remote host1 --remote host2

# Sync both locally and remotely
cswitch aliases --local --remote my-host
```

This uses your existing local `sftp` command. Without `--local` or `--remote`, aliases are generated locally as before. `--remote` is repeatable for batch syncing multiple hosts. Remote sync currently supports lightweight profiles only and skips full directory-isolated profiles. It probes the remote OS first and syncs both the launchers and any required TinyFish prompt/plugin companion files; add `--verbose` to see per-stage and per-file sync details:
- remote Unix-like hosts receive shell shims in `~/.varusers/bin`
- remote Windows hosts receive `.cmd` shims in `%USERPROFILE%\.local\bin`
- remote hosts also receive any required generated MCP plugin directories for profiles that selected MCP servers

Only managed shim files with the generated `claude-` prefix are considered for stale cleanup.

## How profiles are stored

Profiles are tracked in `~/.claude-switch/registry.json`, keyed by UUID. Each profile has:
- `id` — UUID v4 (stable internal key)
- `name` — display name (supports Chinese, spaces, any characters)
- `alias` — optional short CLI-friendly name (alphanumeric, `-`, `_`)
- `kind` — `full` or `lightweight`
- `launch_args` — optional CLI flags passed to claude on launch (e.g. `--dangerously-skip-permissions`)
- `provider_id` / `key_id` — optional shared provider/key reference for lightweight profiles
- `mcp_server_ids` — selected MCP servers for lightweight profiles
- `plugin_ids` — linked hosted plugins for this profile, stored as installed plugin ids

Shared providers are tracked in the same registry. A provider stores a base URL and one or more named API keys; lightweight profiles can link to a specific key while keeping their model and extra env-var settings separate.

Clipboard-driven provider import is also available in the TUI Provider Manager via `Ctrl+Y`.

Shared MCP servers are tracked in the same registry under `mcp_servers`. They support stdio, http, streamable-http, and sse entries with Claude-compatible fields such as `command`, `args`, `env`, `cwd`, `url`, `headers`, `oauth`, `headersHelper`, `timeout`, `alwaysLoad`, and `disabled`. Lightweight profiles can select one or more MCP servers; full profiles keep their own isolated Claude directory and do not use registry MCP selections.

Hosted plugin marketplaces and installed hosted plugins are tracked in the same registry under `plugin_marketplaces` and `installed_plugins`. Installed hosted plugins are unpacked under `~/.claude-switch/plugins/installed/<marketplace>/<plugin>/`, and lightweight or full profiles can link them through `plugin_ids`.

### Full profiles

Stored in `~/.claude-switch/profiles/<alias-or-name>/`. Launch sets `CLAUDE_CONFIG_DIR` to that directory.

### Lightweight profiles

No dedicated profile directory — env vars (token, base URL, model IDs, extras) are stored in the registry and passed via `--settings` on launch. For ordinary profiles this remains an inline settings payload. Localhost/LAN lightweight profiles also stay on that inline-settings path by default, while explicit local gateway modes can still materialize managed TinyFish plugin files under `~/.claude-switch/generated/`. Other TinyFish-enhanced profiles likewise use managed plugin files written as standard Claude plugin directories containing `.claude-plugin/plugin.json` and `hooks/hooks.json`, while TinyFish-specific permissions remain inline in `--settings`. If a lightweight profile selects MCP servers, `cswitch` generates a separate profile-scoped plugin under `~/.claude-switch/generated/mcps/` containing `.mcp.json` plus a compatibility `mcp.json`, then passes it to Claude with `--plugin-dir`. Token and base URL can also come from a linked shared provider/key.

`extras` may also include the reserved control entry `CLAUDE_SWITCH_TINYFISH=off` to keep a profile on the normal inline-settings path even when its `base_url` would otherwise trigger TinyFish plugin generation.

Nothing in `~/.claude` is modified.

## Running multiple profiles simultaneously

```bash
# Terminal 1
cswitch use work

# Terminal 2
cswitch use personal
```

## License

MIT
