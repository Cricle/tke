# tke

Local token shaving shim for AI coding CLIs.

## What it does

- Runs as a `Rust` binary with minimal dependencies.
- Activates only in the current shell session by prepending a shim directory to `PATH`.
- Wraps agent CLIs like `codex` and downstream tool commands like `cat`, `rg`, `git`, `cargo`.
- Covers common code-reading commands by default, including `cat`, `sed`, `rg`, `grep`, `find`, `fd`, `bat`, `nl`, `ls`, `tree`, `awk`, `cut`, `sort`, `uniq`, `wc`, and `xargs`.
- Converts long tool input/output into compact JSON blocks prefixed with `__TKE__`.
- Tags each normalized block with a profile such as `file`, `search`, `pathlist`, `diff`, `log`, `table`, or `stacktrace`.
- Rewrites nested `codex exec --json` `command_execution` event payloads so long `aggregated_output`/`stdout`/`stderr` fields are normalized too.
- Mirrors interactive `codex` rollout JSONL files into `.tke/interactive/` after a TTY session exits, rewriting nested command output there too.
- For shell pipelines, prefers the highest-value stage semantically, e.g. `rg` search output over upstream `cat` or downstream `head`.
- Compresses common high-frequency CLI table/list output aggressively, especially `ps`, `ss`, `systemctl`, and `docker ps`.
- Compresses large Linux path-list output aggressively, especially `find`, `fd`, and `tree` style file discovery output.
- Falls back to the real command when not in agent context or when a command is whitelisted.

## Quick start

```bash
cargo build --release
eval "$(./target/release/tke activate codex claude)"
```

Then run `codex` or `claude` normally inside that shell. Their subprocess calls to wrapped tools will be normalized into machine-readable JSON when the output is large enough.

On Windows, pick the shell explicitly:

```powershell
.\target\release\tke.exe activate --shell powershell codex | Out-String | Invoke-Expression
```

```cmd
for /f "delims=" %i in ('target\\release\\tke.exe activate --shell cmd codex') do %i
```

## Commands

```bash
tke activate [--shim-dir PATH] [agent ...]
tke deactivate
tke capture-interactive [--source PATH] [--output PATH]
tke compare-rollout [--source PATH]
tke benchmark-commands [--check]
tke package-release
```

`capture-interactive` rewrites a saved `codex` rollout JSONL into machine-readable form. By default it reads the latest rollout under `CODEX_HOME/sessions` or `~/.codex/sessions` and writes the mirrored file into `.tke/interactive/` in the current project.

`compare-rollout` reads a raw `codex` rollout, computes the rewritten in-memory version, and prints a JSON report with byte and approximate token savings. This is the fastest way to measure how much `tke` is cutting down tool output for real sessions.

`benchmark-commands` runs a built-in benchmark suite for the default high-frequency command families that `tke` optimizes, including code reading, search, path discovery, table/list output, diff, and build/test logs. It also scans local rollout corpus files such as `.tmp-*.jsonl` and `.tke/interactive/*.jsonl`, and prints a JSON summary of byte and approximate token savings.

`benchmark-commands --check` validates the current benchmark report against built-in expectations:

- `compress` cases must clear a minimum savings bar
- `pass_through` cases must remain unchanged
- rewritten corpus cases must not rewrite for negligible gain

`package-release` creates a local release archive under `dist/` and writes a sibling SHA-256 checksum file.

## Interactive codex

Interactive `codex` uses a TTY UI, so `tke` does not try to rewrite the live screen stream. Instead:

- Wrapped subprocess tools such as `cat`, `rg`, `git`, and `cargo` still run through `tke`.
- When the `codex` TTY session exits normally, `tke` finds the newest rollout JSONL and writes a rewritten mirror into `.tke/interactive/`.
- If needed, you can force the rewrite later with `tke capture-interactive`.

The mirrored rollout keeps the original event structure, but nested command output fields like `aggregated_output` are converted into `__TKE__{...}` envelopes.

On Windows, shim creation uses `.cmd` wrappers instead of Unix symlinks. The rollout rewriting logic is the same.

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

## JSON shape

Normalized blocks include:

- `k`: coarse command kind
- `sc`: selected command from a shell pipeline, when applicable
- `sr`: semantic role of the selected command, such as `search`, `source`, `filter`, `summarize`
- `p`: detected trim profile
- `m[*].k`: per-chunk role such as `file`, `hunk`, `result`, `signal`, `frame`, `fold`, `block`, `snippet`
- `o`: omitted line ranges
- `st`: byte/line statistics
