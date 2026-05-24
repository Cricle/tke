# RTK vs TKE

This document compares `rtk` and `tke` using the current repo implementation and the latest local benchmark/E2E artifacts.

## Short Version

- `tke` is a deterministic tool-output compression layer.
- `rtk` is an agent-specific integration layer.
- For Codex, `tke` is currently the stronger and better-validated path.
- For Claude, `rtk-hook` is currently the more stable fairness path, but not a better compression path.

## Product Shape

| Dimension | `tke` | `rtk` |
| --- | --- | --- |
| Primary role | Compress and normalize tool I/O | Integrate with the agent runtime |
| Integration style | Local Rust binary plus shimmed `PATH` wrappers | Agent-specific hook or rules path |
| Output format | Structured `__TKE__{...}` envelopes | No equivalent repo-local structured envelope |
| Main control point | Wrapped tool commands and transcript rewriting | Claude hook or Codex rules injection |

## How They Work Here

### TKE

`tke` wraps both agent commands and downstream tools, then rewrites long command output into compact structured payloads. The main runtime entrypoints are:

- Agent wrapping in [src/shim.rs](/root/github/tke/src/shim.rs:66)
- Tool wrapping in [src/shim.rs](/root/github/tke/src/shim.rs:139)
- Pipeline-aware stage selection in [src/rewrite.rs](/root/github/tke/src/rewrite.rs:126)

That gives `tke` two important properties:

- Compression is explicit and observable.
- Results are comparatively deterministic once the command output is known.

### RTK

In this repo, RTK is evaluated through each agent's real integration path instead of a single universal mode:

- Codex uses `rtk-codex-rules` via rules/prompt injection in [scripts/codex_e2e_compare.sh](/root/github/tke/scripts/codex_e2e_compare.sh:19)
- Claude uses `rtk-hook` in [scripts/claude_e2e_compare.sh](/root/github/tke/scripts/claude_e2e_compare.sh:191)

That means RTK behavior depends more on whether the target agent actually follows the intended path.

## Stability and Observability

| Dimension | `tke` | `rtk` |
| --- | --- | --- |
| Compression visibility | High: direct rewritten payloads and compare reports | Lower: mostly inferred from E2E behavior |
| Determinism | Higher | Lower |
| Agent dependence | Lower once shims are active | Higher |
| Harness support in this repo | Strong | Fairness-specific |

`tke` has first-class local inspection tools:

- `tke benchmark-commands`
- `tke compare-rollout`
- `tke compare-e2e`

RTK is mostly judged through the fairness and E2E harnesses rather than through a repo-local rewriting primitive.

## Current Measured Results

Source: [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:66) and [docs/e2e.md](/root/github/tke/docs/e2e.md:9).

### Codex

| Path | Cases | Pass | Fail | Tool token outcome |
| --- | --- | --- | --- | --- |
| `tke` | 4 | 4 | 0 | `6257` saved total |
| `rtk-codex-rules` | 2 fair cases | 0 | 2 | `11` token delta total |

Interpretation:

- `tke` is currently validated on real Codex tasks.
- `rtk-codex-rules` is the fair RTK path for Codex, but the current sampled cases do not show comparable correctness or savings.

### Claude

| Path | Cases | Pass | Fail | Gateway | Tool token outcome |
| --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 4 | 3 | 0 | 1 | `-1` total delta |
| `tke` | 1 | 0 | 1 | 0 | `0` total delta |

Interpretation:

- `rtk-hook` is currently the stable fairness path for Claude.
- `tke` on Claude currently prioritizes compatibility by default and should not yet be treated as equally mature live compression.

## Important Fairness Cases

From the current fair comparison table in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:86):

| Agent | Case | Raw | RTK path | Tool token delta | Verdict |
| --- | --- | --- | --- | --- | --- |
| `codex` | `fairfind` | fail | fail | `0` | `wrong_and_not_saved` |
| `codex` | `fairrg` | fail | fail | `11` | `saved_but_wrong` |
| `claude` | `fairbuild` | pass | pass | `-1` | `correct_but_not_saved` |
| `claude` | `fairfind` | fail | pass | `0` | `correct_but_not_saved` |
| `claude` | `fairrg` | pass | pass | `0` | `correct_but_not_saved` |

The key signal is that Claude RTK currently helps more with path correctness than with measurable tool-output compression, while Codex `tke` already delivers strong measured savings on real tasks.

## Practical Recommendation

### Use `tke` when:

- You want deterministic tool-output compression.
- You need direct local observability into what was rewritten.
- You are optimizing Codex workflows today.

### Use `rtk` when:

- You need the agent's native RTK integration path.
- You are evaluating Claude through its official hook-style path.
- You care more about fair agent-path comparison than about guaranteed local compression.

## Bottom Line

There is no single global winner independent of agent:

- For Codex, `tke` is clearly ahead today.
- For Claude, `rtk-hook` is currently the more stable path, but not the stronger compression path.
- `rtk` and `tke` should be treated as different layers, not as interchangeable implementations of the same thing.
