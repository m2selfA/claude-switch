# claude-switch

Multi-account profile manager for [Claude Code](https://code.claude.com).

Switch between multiple Claude accounts without logging out. Each profile is fully isolated — use different accounts in different terminals simultaneously.

Two isolation modes:
- **Full** — copies `~/.claude` into an isolated directory per profile
- **Lightweight** — stores API key, base URL, and model settings as env vars

## Install

### Cargo (requires Rust)

```bash
cargo install cswitch
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
| `cswitch use <name> [-- <claude-args>]` | Launch Claude Code with a profile; stored launch args are enabled by default |
| `cswitch use --no-extras <name>` | Launch without the profile's stored launch args |
| `cswitch list` | List all saved profiles |
| `cswitch info <name>` | Show details for a profile |
| `cswitch remove <name>` | Delete a profile |
| `cswitch aliases [--local] [--remote <host>]... [--verbose]` | Generate/sync local shell aliases and shims, and/or sync self-contained shims to remote hosts via sftp (`--remote` is repeatable) |
| `cswitch provider list` | List shared API providers |
| `cswitch provider add <name> --url <url> --key <key>` | Add a shared provider with an initial key |
| `cswitch provider keys <provider-id>` | List keys for a provider |
| `cswitch provider add-key <provider-id> --name <name> --key <key>` | Add another key to a provider |
| `cswitch provider edit <provider-id> [--name <name>] [--url <url>]` | Edit a provider's name or base URL |
| `cswitch provider edit-key <provider-id> <key-id> [--name <name>] [--key <key>]` | Edit a provider key's name or token |
| `cswitch provider remove <provider-id>` | Remove a provider |
| `cswitch provider remove-key <provider-id> <key-id>` | Remove a key from a provider |
| `cswitch provider link <profile> --provider <provider-id> --key <key-id>` | Link a lightweight profile to a provider key |
| `cswitch provider unlink <profile>` | Remove provider/key association from a profile |
| `cswitch --help` | Full CLI help |

## Interactive TUI

Run `cswitch` with no arguments to open the TUI.

### TUI keybindings

| Key | Action |
|---|---|
| `Ctrl+P/N` | Navigate lists and selections |
| `↑/↓` | Compatibility navigation keys |
| `Enter` | Launch Claude with selected profile and stored launch args |
| `Shift+Enter` | Launch without stored launch args |
| `/` | Search profiles by name or alias |
| `t` | Add lightweight profile from provider/key |
| `Ctrl+Y` | Smart input provider/key from clipboard in Provider Manager |
| `Provider Manager: t` | Discover models from provider-aware candidate endpoints; failure does not prove the provider is unusable, and manual model names may still work |
| `a` | Add full profile (directory isolation) |
| `e` | Edit profile (name/alias/flags, or full model editor for lite) |
| `m` | Toggle [1m] suffix on model slots |
| `Tab` | Move to the next field, or complete/cycle values in model/provider editing flows |
| `Ctrl+P/N` | Move between fields in multi-step forms; in model test, also browse fetched models when the model field is focused |
| `Ctrl+A/E/B/F` | Move cursor in text fields |
| `Ctrl+H/D/K/U/W` | Edit text in Emacs style |
| `Ctrl+G` | Cancel or go back |
| `Shift+Tab` | Switch between profile and provider managers |
| `r` | Refresh — re-copy ~/.claude into selected |
| `d` | Delete selected profile |
| `?` | Help overlay |
| `q` / `Esc` | Quit |

`j/k` is still accepted in some pure list views for compatibility, but it is no longer the documented primary navigation scheme.

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

On Windows, `cswitch aliases` outputs PowerShell functions for your `$PROFILE`, **and** syncs self-contained `.cmd` files into `~/.local/bin` (`%USERPROFILE%\.local\bin`). Each `.cmd` is fully stand-alone (no dependency on `cswitch`) and supports `--no-extras` to skip stored launch args. The files are maintained automatically — added, updated, and cleaned up when profiles change.

On Linux/macOS, if `~/.varusers/bin/` exists, `cswitch aliases` syncs self-contained bash scripts there instead of printing aliases. Each script is executable, stand-alone, supports `--no-extras`, and is automatically maintained when profiles change. If the directory doesn't exist, the command falls back to printing bash/zsh aliases as before.

You can also sync shims to remote machines with:

```bash
# Sync to a single remote host
cswitch aliases --remote my-host

# Sync to multiple remote hosts at once
cswitch aliases --remote host1 --remote host2

# Sync both locally and remotely
cswitch aliases --local --remote my-host
```

This uses your existing local `sftp` command. Without `--local` or `--remote`, aliases are generated locally as before. `--remote` is repeatable for batch syncing multiple hosts. Remote sync currently supports lightweight profiles only and skips full directory-isolated profiles. It probes the remote OS first and keeps the default output concise; add `--verbose` to see per-stage and per-file sync details:
- remote Unix-like hosts receive shell shims in `~/.varusers/bin`
- remote Windows hosts receive `.cmd` shims in `%USERPROFILE%\.local\bin`

Only managed shim files with the generated `claude-` prefix are considered for stale cleanup.

## How profiles are stored

Profiles are tracked in `~/.claude-switch/registry.json`, keyed by UUID. Each profile has:
- `id` — UUID v4 (stable internal key)
- `name` — display name (supports Chinese, spaces, any characters)
- `alias` — optional short CLI-friendly name (alphanumeric, `-`, `_`)
- `kind` — `full` or `lightweight`
- `launch_args` — optional CLI flags passed to claude on launch (e.g. `--dangerously-skip-permissions`)
- `provider_id` / `key_id` — optional shared provider/key reference for lightweight profiles

Shared providers are tracked in the same registry. A provider stores a base URL and one or more named API keys; lightweight profiles can link to a specific key while keeping their model and extra env-var settings separate.

Clipboard-driven provider import is also available in the TUI Provider Manager via `Ctrl+Y`.

### Full profiles

Stored in `~/.claude-switch/profiles/<alias-or-name>/`. Launch sets `CLAUDE_CONFIG_DIR` to that directory.

### Lightweight profiles

No directory — env vars (token, base URL, model IDs, extras) are stored in the registry and passed via `--settings` JSON on launch. Token and base URL can also come from a linked shared provider/key.

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
