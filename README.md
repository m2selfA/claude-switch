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

# Launch with a profile (with stored launch args)
cswitch use -e work

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
| `cswitch use [-e] <name> [-- <claude-args>]` | Launch Claude Code with a profile; `-e` enables stored launch args |
| `cswitch list` | List all saved profiles |
| `cswitch info <name>` | Show details for a profile |
| `cswitch remove <name>` | Delete a profile |
| `cswitch aliases` | Print shell aliases for all profiles |
| `cswitch --help` | Full CLI help |

## Interactive TUI

Run `cswitch` with no arguments to open the TUI.

### TUI keybindings

| Key | Action |
|---|---|
| `↑/↓` or `j/k` | Navigate profiles |
| `Enter` | Launch Claude with selected profile |
| `Shift+Enter` | Launch with stored launch args |
| `/` | Search profiles by name or alias |
| `t` | Add lightweight profile (env vars) |
| `a` | Add full profile (directory isolation) |
| `e` | Edit profile (name/alias/flags, or full model editor for lite) |
| `m` | Toggle [1m] suffix on model slots |
| `Tab` | Complete: cycle model IDs / env vars / CLI flags |
| `r` | Refresh — re-copy ~/.claude into selected |
| `d` | Delete selected profile |
| `?` | Help overlay |
| `q` / `Esc` | Quit |

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

On Windows, `cswitch aliases` outputs PowerShell functions. Add them to your `$PROFILE`.

## How profiles are stored

Profiles are tracked in `~/.claude-switch/registry.json`, keyed by UUID. Each profile has:
- `id` — UUID v4 (stable internal key)
- `name` — display name (supports Chinese, spaces, any characters)
- `alias` — optional short CLI-friendly name (alphanumeric, `-`, `_`)
- `kind` — `full` or `lightweight`
- `launch_args` — optional CLI flags passed to claude on launch (e.g. `--dangerously-skip-permissions`)

### Full profiles

Stored in `~/.claude-switch/profiles/<alias-or-name>/`. Launch sets `CLAUDE_CONFIG_DIR` to that directory.

### Lightweight profiles

No directory — env vars (token, base URL, model IDs, extras) are stored in the registry and passed via `--settings` JSON on launch.

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