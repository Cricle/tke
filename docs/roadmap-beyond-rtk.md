# Roadmap Beyond RTK

This document turns the current `rtk` vs `tke` comparison into an execution plan.

The goal is not to imitate RTK more closely. The goal is to build a path that is:

- more stable than RTK
- more measurable than RTK
- competitive with RTK on tool-output token cost without sacrificing correctness

## Strategy

Do not compete with RTK at the prompt or hook layer.

Instead, push `tke` further as a deterministic local compression layer:

- compress at the tool I/O boundary
- preserve correctness with aggressive fallback
- measure every gain with local reports
- keep agent-specific behavior out of the core design whenever possible

## Product Position

### What RTK is good at

- Agent-native integration paths
- Claude hook-style workflows
- Fairness comparisons across supported agent entrypoints

### What TKE should own

- Deterministic command-output compression
- Stable local rollout rewriting
- Agent-independent savings on high-volume tool traffic
- Replayable local measurement and regression checking

## Success Criteria

A path is only "better than RTK" if it wins on all three of these dimensions:

### 1. Correctness

- equal or higher pass rate on benchmark and E2E tasks
- no regressions in checked result fields
- safe fallback when compression confidence is low

### 2. Stability

- lower dependence on agent-specific runtime quirks
- lower rerun variance
- lower sensitivity to hook/rules activation issues

### 3. Savings

- similar total tool-token savings on real tasks is sufficient if correctness and stability are better
- keep per-profile savings strong on file/search/pathlist/log/diff outputs
- prefer savings consistency across reruns over chasing marginal peak compression

## Design Principles

### Prefer deterministic local transforms

If the command output is the same, the compressed result should be the same.

### Prefer explicit observability

Every compression decision should be inspectable through local artifacts like:

- `tke-bench benchmark-commands`
- `tke-bench compare-rollout`
- `tke-bench compare-e2e`

### Prefer fallback over cleverness

If a profile is ambiguous, the output is too short, or the pipeline cannot be attributed safely, pass through the original output.

### Keep agent-specific logic narrow

Agent-specific handling should stay at the boundary. The compression core should stay tool- and transcript-centric.

## Execution Plan

## Recently Landed

The roadmap is no longer starting from zero. The following items are already implemented in the current tree:

- `pathlist` shared-directory compaction with compact first/last entries and bucketed examples
- `search` grouped-prefix compaction that keeps a full first hit per file and compact followup hits
- lightweight structured `log` summaries with failure and warning counts plus first samples
- lightweight structured `diff` summaries with per-file add/delete counts

That changes the baseline: the next phases should extend these summaries and add deduplication, not re-argue whether local structure is possible.

## Phase 1: Harden the Current Winning Path

Priority: highest

Scope:

- keep Codex + `tke` as the production path
- preserve Claude compatibility-first defaults
- tighten fallback rules instead of pushing more live Claude rewriting by default

Implementation themes:

- audit all profiles for low-confidence passthrough behavior
- expand regression cases for pipeline selection and transcript rewriting
- keep RTK as a fairness baseline, not the primary product path

Expected outcome:

- higher stability without changing the product shape

## Phase 2: Increase Savings on High-Volume Outputs

Priority: highest

Target profiles:

- search
- pathlist
- diff
- log
- table

Implementation themes:

- extend the shipped path-list compaction with multi-root and mixed-parent cases
- extend the shipped search summarization with denser hit clustering and better file ordering
- extend the shipped diff summaries from file counts toward symbol and hunk summaries
- extend the shipped log summaries from fail/warn samples toward richer test/build outcome extraction

Expected outcome:

- larger savings on the command families that dominate tool token cost

## Phase 3: Add Session-Level Deduplication

Priority: high

Problem:

Many agent sessions repeat the same file listings, repeated search outputs, and repeated build/test fragments.

Implementation themes:

- assign stable short references to repeated normalized payloads
- emit abbreviated repeat references inside the same session
- keep replay/debug artifacts rich enough to reconstruct the original context

Expected outcome:

- a second layer of savings beyond single-command trimming

## Phase 4: Strengthen Structured Log Compression

Priority: high

Problem:

Large logs still carry repeated low-value lines even after the current `fail`/`warn` summary pass.

Implementation themes:

- extract failing test names and first error location more reliably
- keep the current warning/failure summary, then deepen it with tool-specific fields
- normalize common build/test tools more deeply:
  - `cargo`
  - `pytest`
  - `npm` / `pnpm` / `yarn`
  - `go test`
  - `cmake` / `ctest` / `make` / `ninja`

Expected outcome:

- better savings with lower correctness risk than generic log clipping

## Phase 5: Expand the Benchmark Corpus

Priority: medium

Problem:

The current RTK fairness comparison is directionally useful, but still narrow.

Implementation themes:

- add more fair cases for search, path discovery, diff inspection, and test/build follow-up
- add rerun sampling to measure variance instead of single-run anecdotes
- keep agent-specific fair paths explicit:
  - Codex: `rtk-codex-rules`
  - Claude: `rtk-hook`

Expected outcome:

- stronger evidence for product claims

## Phase 6: Claude Shadow Mode Before Broader Enablement

Priority: medium

Problem:

Claude live compression is not yet mature enough to become the default path.

Implementation themes:

- keep default Claude behavior compatibility-first
- run shadow compare on real Claude sessions
- only graduate specific live-compression paths after repeated correct results

Expected outcome:

- reduced risk while still collecting evidence for future rollout

## Measurement Plan

Every phase should be judged with the same scorecard.

### Correctness Metrics

- E2E pass/fail
- checked field match rate
- semantic result match rate

### Stability Metrics

- rerun consistency
- gateway-noise isolation
- rate of fallback-to-raw behavior

### Savings Metrics

- total tool tokens saved
- average token savings ratio by profile
- real-task savings by agent and mode

## What Not To Do

- do not make prompt engineering the core strategy
- do not depend on rules injection as the main compression mechanism
- do not force live Claude compression to become default before repeated evidence
- do not collapse RTK and TKE into a single benchmark number across agents

## Recommended Current Positioning

Today, the practical product message should be:

- `tke` is the primary path for deterministic local tool-output compression
- `tke` is already validated most strongly on Codex
- `rtk` remains a useful fairness baseline and Claude integration reference
- `tke` already has a broader local structured-summary layer than RTK in this repo
- the next gains should come from deeper local compression and session-level deduplication, not from copying RTK's integration model

## Immediate Next Build Steps

If implementation starts now, the best order is:

1. strengthen fallback rules and regression coverage
2. deepen the shipped `search`, `pathlist`, `diff`, and `log` summaries
3. add session-level deduplication
4. expand benchmark and fairness cases
5. continue Claude shadow-mode evaluation
