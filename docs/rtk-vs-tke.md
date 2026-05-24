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

## Current Structured Advantage

The current gap is not just aggregate token savings. `tke` now exposes explicit per-profile local summaries that can be replayed, inspected, and expanded inside this repo:

| Profile | `tke` current behavior | `rtk` status in this repo |
| --- | --- | --- |
| `pathlist` | Shared-directory compaction with `pl.d`, `pl.f`, `pl.l`, compact examples, and bucketed rows | No equivalent repo-local structured pathlist summary |
| `search` | Grouped file-level search chunks with full first hit and compact `:line:text` followups | No equivalent repo-local grouped search summary |
| `log` | Lightweight `lg.fail`, `lg.warn`, `lg.first_fail`, `lg.first_warn` fields plus repeated-line folding | No equivalent repo-local structured log summary |
| `diff` | Lightweight `df` file summaries with per-file `p`, `add`, and `del` counts | No equivalent repo-local structured diff summary |

These summaries are emitted as part of the normal `__TKE__{...}` envelope rather than as benchmark-only side data. In this repo that matters because:

- the rewritten payload is directly observable
- the compare tooling can inspect the same normalized structure
- fallback stays local and deterministic instead of depending on agent compliance

Implementation references:

- pathlist summary fields in [src/trim.rs](/root/github/tke/src/trim.rs:1372) and compaction logic in [src/path_profile.rs](/root/github/tke/src/path_profile.rs:8)
- grouped search compaction in [src/search_profile.rs](/root/github/tke/src/search_profile.rs:7)
- log summary fields in [src/trim.rs](/root/github/tke/src/trim.rs:1405) and extraction in [src/log_profile.rs](/root/github/tke/src/log_profile.rs:37)
- diff summary fields in [src/trim.rs](/root/github/tke/src/trim.rs:1416)

## Current Measured Results

Source: [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:1) and [docs/e2e.md](/root/github/tke/docs/e2e.md:1).

### Codex

| Path | Cases | Pass | Fail | Tool token outcome |
| --- | --- | --- | --- | --- |
| `tke` | 4 | 4 | 0 | `6257` saved total |
| `rtk-codex-rules` | 2 fair cases | 0 | 2 | `11` token delta total |

Interpretation:

- `tke` is currently validated on real Codex tasks.
- `rtk-codex-rules` is the fair RTK path for Codex, but the current sampled cases do not show comparable correctness or savings.
- On the current synthetic command benchmark side, `tke` is strong on the high-volume profiles that dominate local tool cost: `search` `89.9%`, `pathlist` `96.6%`, `diff` `93.7%`, `log` `74.4%`.

### Claude

| Path | Cases | Pass | Fail | Gateway | Tool token outcome |
| --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 4 | 3 | 0 | 1 | `-1` total delta |
| `tke` | 1 | 0 | 1 | 0 | `0` total delta |

Interpretation:

- `rtk-hook` is currently the stable fairness path for Claude.
- `tke` on Claude currently prioritizes compatibility by default and should not yet be treated as equally mature live compression.
- Even so, the underlying `tke` local compression primitives are broader and more inspectable than the current RTK fairness path, because they operate on normalized tool output rather than only on agent integration behavior.
- In the current stable synthetic Claude-oriented traces, `tke` saves `10761` tokens total at `91.8%`, while `rtk-hook` saves `10622` at `92.9%`; both preserve all required semantic fragments in those controlled cases.

## Important Fairness Cases

From the current fair comparison table in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:1):

| Agent | Case | Raw | RTK path | Tool token delta | Verdict |
| --- | --- | --- | --- | --- | --- |
| `codex` | `fairfind` | fail | fail | `0` | `wrong_and_not_saved` |
| `codex` | `fairrg` | fail | fail | `11` | `saved_but_wrong` |
| `claude` | `fairbuild` | pass | pass | `-1` | `correct_but_not_saved` |
| `claude` | `fairfind` | fail | pass | `0` | `correct_but_not_saved` |
| `claude` | `fairrg` | pass | pass | `0` | `correct_but_not_saved` |

The key signal is that Claude RTK currently helps more with path correctness than with measurable tool-output compression, while Codex `tke` already delivers strong measured savings on real tasks.

## Compression And Accuracy Scorecard

The current repo evidence splits cleanly into two layers:

| Scope | Compared paths | Main signal |
| --- | --- | --- |
| Stable synthetic Claude traces | `tke` vs `rtk-hook` | compression rate and semantic retention can be compared directly |
| Fair live agent runs | raw vs RTK path | correctness can be compared directly, but compression gains are currently small |

Current takeaway:

- On stable synthetic Claude traces, `rtk-hook` is slightly ahead on compression ratio, while `tke` is slightly ahead on absolute tokens saved.
- On those same synthetic traces, both paths preserve all required semantic fragments.
- On current fair live Claude runs, `rtk-hook` is ahead on correctness stability, but not on token reduction.
- On current live Codex evidence, `tke` remains the only path with clear measured savings plus passing task outcomes.

## Horizontal Comparison Verdict

If the comparison standard is "which path is more stable and more token-efficient as a local tool-output layer", the current answer is `tke`.

- `tke` wins on repo-local observability.
- `tke` wins on structured summaries across `pathlist`, `search`, `log`, and `diff`.
- `tke` wins on measured Codex savings and current synthetic benchmark coverage.
- `rtk` still wins on Claude-native fairness path stability today.

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
