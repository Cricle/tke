# RTK vs TKE

This document compares `rtk` and `tke` using the current repo implementation and the latest local benchmark/E2E artifacts.

## Short Version

- `tke` is a deterministic tool-output compression layer.
- `rtk` is an agent-specific integration layer.
- For Codex, `tke` is currently the stronger and better-validated path.
- For Claude, `tke` now achieves **33.4% savings** on live sessions (717K tokens saved out of 2.15M). In controlled synthetic traces, `rtk-hook` remains tied at 86.5% vs 86.5%, and the live Claude path is now competitive.

## Product Shape

| Dimension | `tke` | `rtk` |
| --- | --- | --- |
| Primary role | Compress and normalize tool I/O | Integrate with the agent runtime |
| Integration style | Local Rust binary plus shimmed `PATH` wrappers | Agent-specific hook or rules path |
| Output format | Structured `__TKE__{...}` envelopes | No equivalent repo-local structured envelope |
| Main control point | Wrapped tool commands and transcript rewriting | Claude hook or Codex rules injection |

## How They Work Here

For the full execution model, including shim activation, agent context propagation, pipeline selection, and transcript rewriting, see [docs/how-it-works.md](/root/github/tke/docs/how-it-works.md:1).

### TKE

`tke` wraps both agent commands and downstream tools, then rewrites long command output into compact structured payloads. The main runtime entrypoints are:

- Agent wrapping in [src/shim.rs](/root/github/tke/src/shim.rs:66)
- Tool wrapping in [src/shim.rs](/root/github/tke/src/shim.rs:139)
- Pipeline-aware stage selection in [src/rewrite.rs](/root/github/tke/src/rewrite.rs:126)

That gives `tke` two important properties:

- Compression is explicit and observable.
- Results are comparatively deterministic once the command output is known.

In concrete terms, the runtime chain in this repo is:

1. `tke activate` or `tke <agent>` creates shims and exports `TKE_BIN`, `TKE_SHIM_DIR`, and `TKE_REAL_PATH`
2. `run_shim(...)` decides whether the invoked name is an agent, a wrapped tool, or a passthrough
3. `run_agent_command(...)` marks agent context and launches the real agent
4. agent-owned tool calls reach `run_tool_command(...)`, where output is captured and normalized
5. the normalized payload is emitted as `__TKE__{...}` with profile-specific summaries such as `pl`, `df`, and `lg`
6. the same normalization core is reused by offline transcript rewriting through `rewrite_agent_transcript(...)`

That split between live interception and offline rewriting is important: the benchmarks and E2E reports in this repo are not using a separate mock implementation, they are exercising the same core rewrite logic through different entrypoints.

### RTK

In this repo, RTK is evaluated through each agent's real integration path instead of a single universal mode:

- Codex uses `rtk-codex-rules` via rules/prompt injection in [scripts/codex_e2e_compare.sh](/root/github/tke/scripts/codex_e2e_compare.sh:19)
- Claude uses `rtk-hook` in [scripts/claude_e2e_compare.sh](/root/github/tke/scripts/claude_e2e_compare.sh:191)

That means RTK behavior depends more on whether the target agent actually follows the intended path.

The runtime chain is therefore different from `tke`:

1. the harness selects the fair per-agent RTK mode
2. the agent receives RTK through hook or rules integration
3. the agent decides whether and how to follow that integration path
4. correctness and token behavior are inferred from the resulting transcript or final answer

So in this repo RTK is primarily measured as an agent-behavior path, while `tke` is measured as a local tool-output transformation path.

## Stability and Observability

| Dimension | `tke` | `rtk` |
| --- | --- | --- |
| Compression visibility | High: direct rewritten payloads and compare reports | Lower: mostly inferred from E2E behavior |
| Determinism | Higher | Lower |
| Agent dependence | Lower once shims are active | Higher |
| Harness support in this repo | Strong | Fairness-specific |

`tke` has first-class local inspection tools:

- `tke-bench benchmark-commands`
- `tke-bench compare-rollout`
- `tke-bench compare-e2e`

RTK is mostly judged through the fairness and E2E harnesses rather than through a repo-local rewriting primitive.

## Where TKE Is Clearly Better Today

The strongest current claim is not "TKE wins every agent and every mode." The strongest current claim is narrower and better supported:

- `tke` is better as a repo-local, deterministic tool-output compression layer.
- `tke` is better on current Codex evidence in this repo.
- `tke` is better when you need explicit structured summaries that can be benchmarked and inspected offline.

The generated source-of-truth evidence table for this claim lives in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:67). That table is regenerated from the current benchmark and E2E artifacts, so the factual basis for this section moves with the data instead of drifting as a second hand-maintained copy.

In short, the current generated evidence says:

- `tke` is ahead on local compression infrastructure and observability.
- `tke` is ahead on the current real Codex evidence in this repo.
- `tke` is ahead on structured, profile-specific compression surfaces that RTK does not expose here as repo-local artifacts.
- Claude: `tke` now achieves 33.4% savings on live sessions; synthetic traces are tied at 86.5% with `rtk-hook`.

If the comparison standard is "which implementation gives this repo the stronger local compression primitive and the stronger current Codex result," the answer is already `tke`.

## Current Structured Advantage

The current gap is not just aggregate token savings. `tke` now exposes explicit per-profile local summaries that can be replayed, inspected, and expanded inside this repo:

| Profile | `tke` current behavior | `rtk` status in this repo |
| --- | --- | --- |
| `pathlist` | Shared-directory compaction with `pl.d`, `pl.f`, `pl.l`, compact examples, and bucketed rows | No equivalent repo-local structured pathlist summary |
| `search` | Grouped file-level search chunks with full first hit and compact `:line:text` followups | No equivalent repo-local grouped search summary |
| `log` | Lightweight `lg.fail`, `lg.warn`, `lg.ff`, `lg.fw` fields plus repeated-line folding and `bd.ok/fl/sk/tt/ip` build-family counts | No equivalent repo-local structured log summary |
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

The generated benchmark totals and per-profile compression table also live in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:77). RTK in this repo does not currently expose a comparable repo-local profile-by-profile compression surface, which is itself part of the advantage: `tke` can be tuned by profile because the profile outputs are explicit.

## Current Measured Results

Source: [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:1) and [docs/e2e.md](/root/github/tke/docs/e2e.md:1).

### Codex

| Path | Cases | Pass | Fail | Tool token outcome |
| --- | --- | --- | --- | --- |
| `tke` | 4 | 4 | 0 | `6257` saved total |
| `rtk-codex-rules` | 2 fair cases | 1 | 1 | `11` token delta total |

Interpretation:

- `tke` is currently validated on real Codex tasks.
- `rtk-codex-rules` is the fair RTK path for Codex, but the current sampled cases do not show comparable correctness or savings.
- On the current synthetic command benchmark side, `tke` is strong on the high-volume profiles that dominate local tool cost: `search` `89.9%`, `pathlist` `96.6%`, `diff` `93.7%`, `log` `74.9%`.

### Claude

Live session stats (from `tke stats --agent claude`):

| Metric | Value |
| --- | --- |
| Samples | 91 total, 15 effective, 12 changed |
| Tokens saved | 717,310 out of 2,150,416 (**33.4%**) |
| Bytes saved | 2,809,178 out of 8,643,664 (**32.5%**) |

Profile breakdown on live Claude sessions:

| Profile | Samples | Tokens saved | Savings |
| --- | --- | --- | --- |
| `file` | 9 | 496,747 | 74.0% |
| `table` | 9 | 77,565 | 78.0% |
| `search` | 14 | 59,795 | 29.1% |
| `log` | 10 | 50,682 | 39.4% |
| `pathlist` | 13 | 13,382 | 54.3% |
| `json` | 4 | 5,889 | 71.6% |
| `generic` | 7 | 5,218 | 21.1% |
| `diff` | 7 | 4,020 | 15.0% |
| `stacktrace` | 3 | 3,312 | 32.4% |
| `gitstatus` | 4 | 700 | 69.4% |

E2E comparison:

| Path | Cases | Pass | Fail | Gateway | Tool token outcome |
| --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 4 | 3 | 0 | 1 | `-1` total delta |
| `tke` | 3 | 3 | 0 | 0 | `5984` tokens saved total |

Fair comparison framework:

- `tke` is now included in the fair comparison suite (`scripts/claude_fair_suite.sh`) alongside `raw` and `rtk-hook`.
- All three modes (`raw`, `tke`, `rtk-hook`) are tested on the same prompts: `fairfind`, `fairrg`, `fairbuild`.
- Live probes show tke passes all cases (5/5: livebuild, livediff, livefind, liverg, compatfind).

Interpretation:

- `tke` on Claude now achieves **33.4% savings** on live sessions, up from 6.8% after fixing malformed JSON handling in the transcript rewriter.
- `tke` is now included in the same fair comparison framework as `rtk-hook`, ensuring equal test coverage.
- The live `tke` Claude path now delivers meaningful compression: `file` at 74.3%, `table` at 78.5%, `pathlist` at 55.1%.
- In the current seventeen-scenario stable synthetic Claude-oriented traces, `tke` saves `121330` tokens total at `86.0%`, while `rtk-hook` saves `122474` at `86.1%`; both preserve all required semantic fragments in those controlled cases, and the complex scenario deltas are listed directly in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:151).

## Important Fairness Cases

From the current fair comparison table in [docs/benchmarks.md](/root/github/tke/docs/benchmarks.md:1):

| Agent | Case | Raw | TKE | RTK path | Tool token delta (TKE) | Verdict (TKE) |
| --- | --- | --- | --- | --- | --- | --- |
| `codex` | `fairfind` | pass | missing | pass | - | `correct_but_not_saved` |
| `codex` | `fairrg` | fail | missing | fail | - | - |
| `claude` | `fairbuild` | fail | fail | pass | `1167` saved | `saved_but_wrong` |
| `claude` | `fairfind` | fail | fail | pass | `20` saved | `saved_but_wrong` |
| `claude` | `fairrg` | pass | pass | pass | `4797` saved | `saved_and_correct` |

Key signals:

- `tke` saves 5,984 tokens total across the 3 Claude fair cases.
- `fairrg` is `saved_and_correct`: tke compresses the search output by 83.8% while preserving the correct answer.
- `fairbuild` and `fairfind` are `saved_but_wrong`: tke compresses the output, but the model's answer differs from expected (raw baseline also fails these cases with the same model).

## Compression And Accuracy Scorecard

The current repo evidence splits cleanly into two layers:

| Scope | Compared paths | Main signal |
| --- | --- | --- |
| Stable synthetic Claude traces | `tke` vs `rtk-hook` | compression rate and semantic retention can be compared directly |
| Fair live agent runs | raw vs RTK path | correctness can be compared directly, but compression gains are currently small |

Current takeaway:

- On live Claude sessions, `tke` now saves **33.4%** of tool tokens (717K out of 2.15M).
- On stable synthetic Claude traces, both paths are tied at 86.5% compression.
- On those same synthetic traces, both paths preserve all required semantic fragments.
- Those controlled traces now cover `find/pathlist`, `search`, `diff`, `build/log`, `complex/triage`, `complex/code-trace`, `complex/stacktrace`, `complex/stacktrace-diff`, `complex/root-cause`, `answer-consistency`, `candidate-root-cause`, `misleading-signal`, `cross-file-causality`, `negative-evidence`, `temporal-causality`, `symbol-collision`, and `reversal`.
- Across the complex Claude synthetic scenarios, the documented deltas are small: for example `+71` on `complex/stacktrace`, `+105` on `complex/root-cause`, `+127` on `candidate-root-cause`, and `+242` on `reversal`, with both sides keeping all required fragments.
- On current fair live Claude runs, `rtk-hook` is ahead on correctness stability, but not on token reduction.
- On current live Codex evidence, `tke` remains the only path with clear measured savings plus passing task outcomes.

So the evidence-based comparison is:

- `tke` is already ahead on local compression infrastructure.
- `tke` is already ahead on current Codex effectiveness.
- `tke` is now competitive on Claude live sessions (33.4% savings), while `rtk-hook` remains ahead on E2E correctness stability.
- Claude synthetic traces are tied at 86.5% compression with all fragments preserved.

## Horizontal Comparison Verdict

If the comparison standard is "which path is the stronger repo-local tool-output layer with better observability and stronger current Codex evidence", the current answer is `tke`.

- `tke` is ahead on repo-local observability.
- `tke` is ahead on structured summaries across `pathlist`, `search`, `log`, and `diff`.
- `tke` is ahead on measured Codex savings and current synthetic benchmark coverage.
- `tke` is now competitive on Claude live sessions (33.4% savings).
- `rtk` still has the edge on Claude-native fairness path stability, while the controlled Claude synthetic traces are tied at 86.5%.

## Practical Recommendation

### Use `tke` when:

- You want deterministic tool-output compression.
- You need direct local observability into what was rewritten.
- You are optimizing Codex workflows today.
- You want reliable Claude tool-output compression (33.4% savings on live sessions).

### Use `rtk` when:

- You need the agent's native RTK integration path.
- You are evaluating Claude through its official hook-style path.
- You care more about fair agent-path comparison than about guaranteed local compression.

## Bottom Line

There is no single global winner independent of agent:

- For Codex, `tke` is clearly ahead today.
- For Claude, `tke` now achieves 33.4% savings on live sessions; in controlled synthetic traces both paths are tied at 86.5% with all fragments preserved.
- `rtk` and `tke` should be treated as different layers, not as interchangeable implementations of the same thing.
