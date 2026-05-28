# tke

Local token-shaving shim for AI coding CLIs. Wraps agent tool commands (`cat`, `rg`, `git`, `cargo`, etc.) and compresses large output into compact `__TKE__{...}` JSON envelopes.

Detailed docs: [how-it-works](docs/how-it-works.md) | [benchmarks](docs/benchmarks.md) | [E2E matrix](docs/e2e.md) | [RTK vs TKE](docs/rtk-vs-tke.md)

## Quick Start

```bash
cargo build --release
./target/release/tke install
tke codex
tke stats
```

`tke <agent>` creates shims and launches the agent in one command. No `eval` needed.

```bash
tke codex
tke claude
tk codex                    # short alias
```

For a persistent shell session:

```bash
eval "$(./target/release/tke activate codex claude)"
```

On Windows:

```powershell
.\target\release\tke.exe activate --shell powershell codex | Out-String | Invoke-Expression
```

Platform notes: Linux/macOS install to `~/.local/bin` by default. Windows installs to `%LOCALAPPDATA%\Microsoft\WindowsApps` with `.exe` shim entries. See [docs/how-it-works.md](docs/how-it-works.md#platform-notes) for details.

## Commands

```bash
tke <agent> [args ...]
tke install [--bin-dir PATH]
tke activate [--shim-dir PATH] [agent ...]
tke run [--shim-dir PATH] <agent> [args ...]
tke tty [--shim-dir PATH] <command> [args ...]   # PTY attach for non-TTY hosts
tke deactivate
tke capture-interactive [--source PATH] [--output PATH]
tke compare-rollout [--source PATH]
tke stats [--source PATH]... [--limit N] [--profile NAME] [--command NAME] [--by day|profile|command] [--json]
tke compare-e2e [--source DIR]... [--agent codex|claude]
tke benchmark-commands [--check]
```

## Config

Optional config file: `.tke/config.json`

```json
{
  "min_trim_bytes": 2048,
  "max_body_lines": 120,
  "head_lines": 16,
  "tail_lines": 16,
  "match_context": 2,
  "max_matches": 6,
  "trim_agent_output": true,
  "json_prefix": "__TKE__",
  "agent_commands": ["codex", "claude"],
  "tool_commands": ["cat", "rg", "git", "cargo"],
  "whitelist_commands": [],
  "whitelist_extensions": [".json", ".toml"],
  "whitelist_paths": []
}
```

## JSON Shape

Normalized blocks include:

- `k`: command kind
- `sc`: selected pipeline stage
- `sr`: semantic role (`search`, `source`, `filter`, `build`)
- `p`: trim profile (`file`, `search`, `pathlist`, `diff`, `log`, `table`)
- `m[*].k`: per-chunk role (`file`, `hunk`, `result`, `frame`, `fold`)
- `o`: omitted line ranges
- `st`: byte/line statistics

## CI

GitHub Actions runs `cargo fmt --check`, `cargo test --quiet`, `cargo build --release`, and `tke benchmark-commands --check` on push/PR. Release binaries are built for tagged pushes (`v*`).
