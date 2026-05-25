# tke

Local token shaving shim for AI coding CLIs.

More detailed benchmark and real E2E notes live under [docs/README.md](./docs/README.md).

If you want the internal execution model instead of usage notes, start with [docs/how-it-works.md](./docs/how-it-works.md).

## What it does

- Runs as a `Rust` binary with minimal dependencies.
- Activates only in the current shell session by prepending a shim directory to `PATH`.
- Wraps agent CLIs like `codex` and downstream tool commands like `cat`, `rg`, `git`, `cargo`.
- Covers common code-reading commands by default, including `cat`, `sed`, `rg`, `grep`, `find`, `fd`, `bat`, `nl`, `ls`, `tree`, `awk`, `cut`, `sort`, `uniq`, `wc`, and `xargs`.
- Converts long tool input/output into compact JSON blocks prefixed with `__TKE__`.
- Tags each normalized block with a profile such as `file`, `search`, `pathlist`, `diff`, `log`, `table`, or `stacktrace`.
- Rewrites nested `codex exec --json` `command_execution` event payloads so long `aggregated_output`/`stdout`/`stderr` fields are normalized too.
- Mirrors interactive `codex` rollout JSONL files into `.tke/interactive/` after a TTY session exits, rewriting nested command output there too.
- For offline transcript rewriting, prefers the highest-value stage semantically inside shell pipelines, e.g. `rg` search output over upstream `cat` or downstream `head`.
- Compresses common high-frequency CLI table/list output aggressively, especially `ps`, `ss`, `systemctl`, and `docker ps`.
- Compresses large Linux path-list output aggressively, especially `find`, `fd`, and `tree` style file discovery output.
- Falls back to the real command when not in agent context or when a command is whitelisted.

## How it works

At a high level, `tke` is a local interception layer rather than a prompt-only integration:

1. `tke activate` or `tke <agent>` prepends a shim directory to `PATH`
2. agent and tool names such as `codex`, `claude`, `rg`, `git`, and `cargo` resolve to `tke` shims first
3. the shim decides whether the current process is an agent launch, an agent-owned tool call, or a passthrough
4. wrapped tool output is captured and normalized into a compact `__TKE__{...}` payload when it is large enough
5. saved rollouts and JSONL transcripts can be rewritten offline with the same normalization core

That means `tke` works at the tool-output layer, not only at the final-answer layer. The detailed runtime flow, pipeline handling, and RTK comparison model are documented in [docs/how-it-works.md](./docs/how-it-works.md).

## Quick start

```bash
cargo build --release
./target/release/tke install
tke codex
tke stats
```

For one-shot use, put the agent name directly after `tke`. It creates the shim env and launches the agent in one command.

```bash
tke codex
tke codex exec --json "Reply with exactly OK."
tk codex
```

`tke run codex ...` still works as the explicit form.

`tke install` is cross-platform:

- Linux: installs to `~/.local/bin` by default
- macOS: installs to `~/.local/bin` by default
- Windows: installs to `%LOCALAPPDATA%\\Microsoft\\WindowsApps` by default and creates `tk.cmd`

If you want to keep a shell session activated for repeated agent runs, use:

```bash
eval "$(./target/release/tke activate codex claude)"
```

Then run `codex` or `claude` normally inside that shell. Their subprocess calls to wrapped tools will be normalized into machine-readable JSON when the output is large enough.

For stability, live shell shims only rewrite the final emitting stage of selected multi-stage pipelines. The preserved `sc`/`sr` metadata still points at the semantically selected stage, so `cat file | rg ... | head` is emitted as a compact `head`-stage result annotated with `sc=rg` and `sr=search`, while `cargo test | tail` is emitted with `sc=cargo` and `sr=build`. Unsupported or ambiguous pipelines still pass through unchanged.

On Windows, pick the shell explicitly:

```powershell
.\target\release\tke.exe activate --shell powershell codex | Out-String | Invoke-Expression
```

```cmd
for /f "delims=" %i in ('target\\release\\tke.exe activate --shell cmd codex') do %i
```

## Commands

```bash
tke <agent> [args ...]
tke install [--bin-dir PATH]
tke activate [--shim-dir PATH] [agent ...]
tke run [--shim-dir PATH] <agent> [args ...]
tke tty [--shim-dir PATH] <command> [args ...]
tke deactivate
tke capture-interactive [--source PATH] [--output PATH]
tke compare-rollout [--source PATH]
tke stats [--source PATH]... [--limit N]
tke compare-e2e [--source DIR]... [--agent codex|claude]
tke benchmark-commands [--check]
```

There is no `tke release` subcommand. Release artifacts come from `cargo build --release` locally and the GitHub Actions release workflow for tagged builds.

`tke <agent> ...` is the recommended low-friction entrypoint. It wraps a single agent launch without requiring `eval` or shell state changes.

Examples:

```bash
tke install
tke codex
tke codex exec --json "Reply with exactly OK."
tke claude
tk codex
tke claude
```

`capture-interactive` rewrites a saved `codex` rollout JSONL into machine-readable form. By default it reads the latest rollout under `CODEX_HOME/sessions` or `~/.codex/sessions` and writes the mirrored file into `.tke/interactive/` in the current project.

`compare-rollout` reads a raw `codex` rollout, computes the rewritten in-memory version, and prints a JSON report with byte and approximate token savings. This is the fastest way to measure how much `tke` is cutting down tool output for real sessions.

`stats` is the main user-facing savings summary for real usage. By default it scans:

- `CODEX_HOME/sessions` or `~/.codex/sessions`
- `.tke/interactive/`

and prints a human-readable summary with:

- total raw vs rewritten bytes and approximate tokens
- total bytes saved and tokens saved
- overall savings ratios
- how many real rollout samples changed
- the latest effective savings sample
- per-day savings summary
- recent rollout sample lines

If you want machine-readable output, add `--json`.

Examples:

```bash
tke stats
tke stats --limit 20
tke stats --json --limit 20
tke stats --source ~/.codex/sessions --source ./.tke/interactive
```

`compare-e2e` scans real E2E artifacts under `.tmp-claude-e2e` and `.tmp-codex-e2e` by default, groups them by case name, and treats `raw` as the baseline against one or more variants such as `tke`, `rtk-hook`, `rtk-codex-rules`, or `rtk-direct`. The JSON report includes:

- baseline vs variant canonical tool payload bytes and estimated tokens
- whether each variant payload already contained `__TKE__`
- baseline vs variant final answer text
- structured correctness verdicts such as `saved_and_correct` and `wrong_and_not_saved`
- per-agent summary counts and total tool token savings
- rollout rewrite savings per sample

Mode notes:

- `tke`: `tke` shim/activation path
- `rtk-hook`: RTK transparent hook path, currently meaningful for Claude-style hook integrations
- `rtk-codex-rules`: RTK official Codex path via `AGENTS.md + RTK.md` prompt-level instructions
- `rtk-direct`: explicit `rtk ...` invocation path, used where the agent integration is rules/prompt driven instead of transparent shell rewriting

Examples:

```bash
tke compare-e2e
tke compare-e2e --agent claude
tke compare-e2e --source .tmp-claude-e2e --source .tmp-codex-e2e
bash scripts/codex_real_suite.sh /root/github/tke
```

## Native PTY Attach

If your current host process has `bash` but does not have a real interactive TTY, commands like `tke codex` may still fail because the agent checks whether `stdin` is a terminal.

For those environments, use:

```bash
tke tty codex
```

`tke tty <command>` uses a native Linux PTY attach path so the wrapped command sees a real terminal rather than a plain pipe. This path is system-level rather than agent-specific, so it also works for other terminal-bound programs:

```bash
tke tty bash
tke tty claude
```

This is a compatibility path for hosts where `stdin` is not a real TTY. It is useful for getting terminal-bound commands to run, but it should not be treated as a guaranteed full-fidelity replacement for a native local terminal UI.

`benchmark-commands` runs a built-in benchmark suite for the default high-frequency command families that `tke` optimizes, including code reading, search, path discovery, table/list output, diff, and build/test logs. It also includes fixed "real codex task" rollout benchmarks that simulate multi-step agent work on the same objective, and scans local rollout corpus files such as `.tmp-*.jsonl` and `.tke/interactive/*.jsonl`. The output is a JSON summary of byte and approximate token savings.

For the Claude local harness itself, `bash scripts/verify_claude_harness.sh` verifies the isolated shell setup, `TKE_REAL_PATH`, and cargo/rustup cache wiring without making an external API call.

## Fair RTK Comparisons

RTK does not integrate with every agent in the same way:

- Claude Code: transparent hook (`rtk-hook`)
- Codex: prompt/rules injection via `AGENTS.md + RTK.md` (`rtk-codex-rules`)

For that reason, a fair RTK comparison should not treat `rtk-direct` as the official path for Codex. Use:

```bash
scripts/codex_fair_compare.sh /root/github/tke raw findcase /tmp/tke-codex-fair-find-prompt.txt
scripts/codex_fair_compare.sh /root/github/tke rtk-codex-rules findcase /tmp/tke-codex-fair-find-prompt.txt
CLAUDE_API_KEY=... CLAUDE_BASE_URL=... scripts/claude_smoke_try.sh /root/github/tke raw fairsmoke /tmp/tke-claude-fair-smoke-prompt.txt
CLAUDE_API_KEY=... CLAUDE_BASE_URL=... scripts/claude_smoke_try.sh /root/github/tke rtk-hook fairsmoke /tmp/tke-claude-fair-smoke-prompt.txt
```

`claude_smoke_try.sh` always writes an `*.attempt.json` status record so transient gateway failures can be tracked instead of silently blocking the workflow.

## Real E2E Findings

Current real-task findings in this repo are:

- Codex + `tke`: verified correct on real `findcase` and `buildcase`, while reducing tool payload size materially.
- Codex + `rtk-codex-rules`: the official `AGENTS.md + RTK.md` path was exercised fairly, but in the sampled real task Codex still executed the raw command path rather than an RTK-prefixed command, so no measured tool-output savings were observed there.
- Claude + `raw`: verified correct on the real `findcase` Bash task.
- Claude + `rtk-hook`: verified correct on the same real `findcase` Bash task after fixing the local comparison harness.
- Claude + `tke`: default live usage now stays in compatibility mode on the tested `claude-opus-4-6` gateway path. Experimental live compression is only enabled with `TKE_CLAUDE_LIVE_TOOLS=1`, and should still be judged case by case in the benchmark docs.

Practical interpretation:

- `tke` is currently validated first for Codex.
- RTK fairness for Codex should be judged via `rtk-codex-rules`, not `rtk-direct`.
- RTK fairness for Claude should be judged via `rtk-hook`.
- Claude-specific `tke` compression is intentionally split into a stable compatibility default and an experimental live-compression path.

`benchmark-commands --check` validates the current benchmark report against built-in expectations:

- `compress` cases must clear a minimum savings bar
- built-in codex task rollouts must also clear a minimum savings bar and preserve required result fragments
- `pass_through` cases must remain unchanged
- rewritten corpus cases must not rewrite for negligible gain

## CI and Release

GitHub Actions includes:

- `CI`: runs on push to `main` and on pull requests, and checks `cargo fmt --check`, `cargo test --quiet`, `cargo build --release`, and `tke benchmark-commands --check`
- `Release`: runs when pushing a tag like `v0.1.0`, builds release binaries and uploads GitHub Release assets

Release assets currently include:

- Linux: `x86_64-unknown-linux-musl` static binary archive
- macOS: `x86_64-apple-darwin` archive
- macOS: `aarch64-apple-darwin` archive
- Windows: `x86_64-pc-windows-msvc` archive with static CRT via `-C target-feature=+crt-static`

Notes:

- Linux release artifacts are static `musl` builds.
- macOS does not use `musl`; the release workflow ships native Darwin binaries for Intel and Apple Silicon.

To publish:

```bash
git tag v0.1.0
git push origin v0.1.0
```

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
