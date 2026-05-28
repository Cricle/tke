# How TKE Works

This document explains the execution model behind `tke` in this repo, and how that differs from the RTK comparison paths used here.

## Short Model

`tke` is a local command-interception layer:

1. it injects shim executables into `PATH`
2. it decides whether the current command is an agent, a wrapped tool, or a passthrough
3. it runs the real command
4. it captures large tool output
5. it rewrites that output into a compact `__TKE__{...}` envelope
6. it leaves the final answer generation to the agent

RTK in this repo is not a local output-rewriter. It is evaluated through agent-specific integration paths such as Claude hook mode and Codex rules mode.

## TKE Runtime Flow

### 1. Activation

Entry points:

- [README.md](/root/github/tke/README.md:1)
- [src/app.rs](/root/github/tke/src/app.rs:332)
- [src/trim.rs](/root/github/tke/src/trim.rs:537)

When you run:

```bash
tke codex
```

or:

```bash
eval "$(tke activate codex claude)"
```

`tke` creates a shim directory and prepends it to `PATH`. The generated activate script also exports:

- `TKE_BIN`: the real `tke` binary path
- `TKE_SHIM_DIR`: the shim directory
- `TKE_REAL_PATH`: the original non-shim `PATH`
- `TKE_AGENT_CMDS`: wrapped agent names
- `TKE_TOOL_CMDS`: wrapped tool names

That means later calls to `codex`, `claude`, `rg`, `git`, `cargo`, `jq`, `curl`, `python3`, `docker`, `ps`, and similar names first hit a `tke` shim instead of the system binary.

### 2. Shim Dispatch

Main dispatch:

- [src/app.rs](/root/github/tke/src/app.rs:372)
- [src/app.rs](/root/github/tke/src/app.rs:389)

When a shim is invoked, `tke` resolves the real command and chooses one of three paths:

- agent path: `run_agent_command(...)`
- tool path: `run_tool_command(...)`
- passthrough path: run the real command unchanged

The decision depends on:

- whether the current name is configured as an agent command
- whether the current shell is already inside agent context
- whether the current name is configured as a wrapped tool
- whether the command is explicitly whitelisted

So the important behavior is: tools are only normalized when they are being called from inside an agent session, not during ordinary shell usage.

### 3. Agent Launch

Agent runtime:

- [src/shim.rs](/root/github/tke/src/shim.rs:58)

When the invoked command is an agent like `codex` or `claude`, `tke`:

- sets `TKE_AGENT_CONTEXT=1`
- records the active agent in `TKE_ACTIVE_AGENT`
- restores the original tool path through `TKE_REAL_PATH`
- launches the real agent binary

Two modes matter here:

- interactive TTY mode: the agent UI is passed through live
- captured mode: `tke` can rewrite agent stdout/stderr before emitting it

For Claude, tool output is compressed via PATH shims (always active in agent context). Agent output is passed through. For Codex TTY sessions, `tke` mainly rewrites the saved rollout after the session exits.

### 4. Tool Interception And Normalization

Tool runtime:

- [src/shim.rs](/root/github/tke/src/shim.rs:123)
- [src/trim.rs](/root/github/tke/src/trim.rs:721)

When an agent subprocess runs a wrapped tool like `rg`, `find`, `git diff`, `cargo test`, `curl`, `jq`, `docker ps`, or `python script.py`, `tke`:

1. resolves the real binary
2. captures stdout/stderr
3. classifies the command into a profile
4. decides whether the payload is worth trimming
5. emits either raw output or a compact envelope

The envelope is prefixed with:

```text
__TKE__
```

and then a JSON object describing the normalized result.

Typical fields include:

- `cmd`: the emitted command name
- `sc`: the semantically selected stage name
- `sr`: the semantic role such as `search` or `build`
- `p`: the selected profile such as `file`, `search`, `pathlist`, `diff`, or `log`
- `st`: size stats
- `pl`: path-list summary
- `df`: diff summary
- `lg`: log summary

This is the core reason `tke` is observable: the compressed form is explicit and machine-readable instead of being hidden behind prompt-side behavior.

### 5. Profile-Specific Compression

Key profile code:

- [src/search_profile.rs](/root/github/tke/src/search_profile.rs:7)
- [src/path_profile.rs](/root/github/tke/src/path_profile.rs:11)
- [src/log_profile.rs](/root/github/tke/src/log_profile.rs:37)
- [src/trim.rs](/root/github/tke/src/trim.rs:243)

`tke` does not use one generic summarizer for everything. It chooses a profile and then applies profile-specific logic:

- `file`: keep useful head/body/tail ranges for code reads
- `search`: keep the first strong hits and compact follow-up hits by file
- `pathlist`: collapse repeated directory structure into compact lists
- `diff`: summarize file-level adds/deletes instead of keeping full hunks
- `log`: keep first failures/warnings and fold repeated noise
- `table`: shrink large list/table outputs such as `ps`, `ss`, `netstat`, `systemctl`, `docker ps`, and `df -h`

That structure is why `tke` can stay deterministic: once command kind and raw output are known, the rewritten form is largely fixed.

## Pipeline Handling

Pipeline selection logic:

- [src/rewrite.rs](/root/github/tke/src/rewrite.rs:126)

Pipelines are handled semantically rather than literally. For example:

```bash
cat file | rg pattern | head -n 20
```

The final emitting stage may be `head`, but the semantically important stage is `rg`. In those cases `tke` preserves the semantic stage metadata through fields like:

- `sc=rg`
- `sr=search`

For live shell interception, `tke` only normalizes selected pipeline shapes when the stage ownership is unambiguous. Otherwise it falls back to passthrough.

This is a stability tradeoff:

- fewer surprising rewrites
- less risk of breaking shell behavior
- slightly less aggressive live compression in ambiguous pipelines

## Offline Transcript Rewriting

Transcript adapter:

- [src/adapter.rs](/root/github/tke/src/adapter.rs:9)
- [src/rollout_io.rs](/root/github/tke/src/rollout_io.rs:49)

`tke` also rewrites saved agent transcripts after the fact. This is what powers:

- `tke capture-interactive`
- `tke compare-rollout`
- `tke compare-e2e`
- `tke benchmark-commands`

The adapter reads JSONL event streams and rewrites nested tool output fields in place.

In this repo it understands both:

- Codex-style `exec_command` and `function_call_output`
- Claude-style `tool_use` and `tool_result`

The flow is:

1. parse each JSONL line
2. track tool call ids and the originating shell command
3. detect nested stdout/stderr payloads
4. normalize those payloads with the same trimming logic
5. write back the original event shape with only the large payload fields replaced

That gives this repo a useful property: live shims, saved rollouts, synthetic benchmarks, and E2E comparisons all reuse the same normalization core.

## Why TKE Is Usually More Observable Than RTK Here

In this repo, `tke` owns the tool-output layer directly:

- it sees the actual raw command output
- it emits an explicit normalized envelope
- it can be benchmarked offline without depending on agent compliance

RTK here is evaluated differently:

- Claude fairness path: `rtk-hook`
- Codex fairness path: `rtk-codex-rules`

Those are agent integration paths, not repo-local output rewriters. So the observed result depends more on:

- whether the agent follows the intended integration path
- whether the hook/rules path is actually exercised
- whether the final transcript exposes measurable tool-token changes

That is why `tke` and `rtk` are not interchangeable in this repo:

- `tke` is a local deterministic compression layer
- RTK is an agent-path integration layer

## End-To-End Data Flow

### TKE path

```text
user shell
  -> tke activate / tke <agent>
  -> PATH shim
  -> agent process
  -> wrapped tool process
  -> raw stdout/stderr captured
  -> profile classification
  -> __TKE__{...} normalized payload
  -> agent reads compact payload
  -> final answer
```

### RTK path in this repo

```text
user shell
  -> agent-specific RTK integration
  -> agent decides whether and how to use that integration
  -> tool execution / transcript behavior depends on agent path
  -> final answer
```

The RTK path may still improve correctness or agent behavior, but it does not provide the same repo-local structured compression primitive that `tke` provides here.

## Practical Implications

- If you want stable local measurement of tool-token reduction, `tke` is the stronger fit.
- If you want to compare official agent-native RTK behavior, you must use the per-agent fairness paths instead of assuming one universal RTK mode.
- If you want both correctness and token efficiency, the right workflow in this repo is to measure `tke` and RTK separately, then compare them through `compare-e2e` and `benchmark-commands`.

## Platform Notes

### Windows

- `tke codex`, `tk codex`, and `tke tty codex` work against npm-installed Codex CLI on Windows.
- Runtime shims do not default to `./.tke/shims`; one-shot runs use temporary shim directories.
- `tke activate` defaults to a temp shim directory on Windows unless `--shim-dir` is provided.
- Shims are generated as `.exe` entries rather than `.cmd` wrappers for closer Linux-like dispatch.
- `tke install` defaults to `%LOCALAPPDATA%\Microsoft\WindowsApps` and creates `tk.cmd`.

### Linux / macOS

- `tke install` defaults to `~/.local/bin`.
- Linux release artifacts are static `musl` builds.
- macOS ships native Darwin binaries for Intel and Apple Silicon.
- PTY attach (`tke tty`) uses native Linux PTY via the `nix` crate.
