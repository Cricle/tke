# Benchmarks

This file is generated from the current local benchmark and E2E artifacts.

## Synthetic Command Benchmarks

Generated from:

```bash
./target/release/tke benchmark-commands --check
```

| Command case | Profile | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `cat_code` | file | 743 | 161 | 582 | 78.3% |
| `sed_code` | file | 743 | 164 | 579 | 77.9% |
| `bat_code` | file | 743 | 165 | 578 | 77.8% |
| `nl_code` | file | 1058 | 189 | 869 | 82.1% |
| `rg_code` | search | 2257 | 226 | 2031 | 90.0% |
| `grep_code` | search | 2257 | 228 | 2029 | 89.9% |
| `find_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `fd_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `tree_paths` | pathlist | 3926 | 93 | 3833 | 97.6% |
| `git_diff` | diff | 3691 | 232 | 3459 | 93.7% |
| `cargo_build` | log | 796 | 197 | 599 | 75.3% |
| `pytest_run` | log | 827 | 210 | 617 | 74.6% |
| `npm_test` | log | 735 | 192 | 543 | 73.9% |
| `dotnet_test` | log | 827 | 210 | 617 | 74.6% |
| `go_test` | log | 705 | 162 | 543 | 77.0% |
| `ninja_build` | log | 796 | 199 | 597 | 75.0% |
| `ps_table` | table | 655 | 153 | 502 | 76.6% |
| `systemctl_table` | table | 682 | 126 | 556 | 81.5% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 93.7% |
| file | 9 | 77.9% |
| generic | 1 | 0.0% |
| log | 12 | 74.9% |
| pathlist | 6 | 96.7% |
| search | 2 | 89.9% |
| table | 3 | 70.6% |

Built-in rollout/task benchmarks:

| Task | Mode | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `codex_api_trace_rollout_savings` | api | 5389 | 505 | 4884 | 90.6% |
| `codex_api_trace_default_tool_coverage` | api | 4021 | 769 | 3252 | 80.9% |
| `codex_interactive_trace_selected_search_stage` | interactive | 2913 | 570 | 2343 | 80.4% |
| `codex_interactive_trace_selected_find_stage` | interactive | 8125 | 93 | 8032 | 98.9% |
| `codex_interactive_trace_selected_build_stage` | interactive | 1102 | 226 | 876 | 79.5% |
| `claude_bash_trace_selected_search_stage` | api | 2416 | 552 | 1864 | 77.2% |
| `claude_bash_trace_selected_find_stage` | api | 8164 | 132 | 8032 | 98.4% |
| `claude_bash_trace_selected_diff_stage` | api | 3700 | 267 | 3433 | 92.8% |
| `claude_bash_trace_selected_build_stage` | api | 1141 | 265 | 876 | 76.8% |
| `claude_bash_trace_complex_triage_task` | api | 14972 | 961 | 14011 | 93.6% |
| `claude_bash_trace_complex_code_trace_task` | api | 7682 | 1045 | 6637 | 86.4% |
| `claude_bash_trace_complex_stacktrace_task` | api | 2768 | 809 | 1959 | 70.8% |
| `claude_bash_trace_complex_stacktrace_diff_task` | api | 6544 | 1060 | 5484 | 83.8% |
| `claude_bash_trace_complex_root_cause_task` | api | 10934 | 1141 | 9793 | 89.6% |
| `claude_bash_trace_answer_consistency_task` | api | 10463 | 1012 | 9451 | 90.3% |
| `claude_bash_trace_candidate_root_cause_task` | api | 11320 | 1269 | 10051 | 88.8% |
| `claude_rtk_hook_trace_selected_find_stage` | api | 8159 | 127 | 8032 | 98.4% |
| `claude_rtk_hook_trace_selected_search_stage` | api | 2137 | 416 | 1721 | 80.5% |
| `claude_rtk_hook_trace_selected_diff_stage` | api | 3697 | 264 | 3433 | 92.9% |
| `claude_rtk_hook_trace_selected_build_stage` | api | 1135 | 259 | 876 | 77.2% |
| `claude_rtk_hook_trace_complex_triage_task` | api | 14972 | 961 | 14011 | 93.6% |
| `claude_rtk_hook_trace_complex_code_trace_task` | api | 7682 | 1045 | 6637 | 86.4% |
| `claude_rtk_hook_trace_complex_stacktrace_task` | api | 2855 | 825 | 2030 | 71.1% |
| `claude_rtk_hook_trace_complex_stacktrace_diff_task` | api | 6631 | 1076 | 5555 | 83.8% |
| `claude_rtk_hook_trace_complex_root_cause_task` | api | 11051 | 1153 | 9898 | 89.6% |
| `claude_rtk_hook_trace_answer_consistency_task` | api | 10572 | 1016 | 9556 | 90.4% |
| `claude_rtk_hook_trace_candidate_root_cause_task` | api | 11428 | 1250 | 10178 | 89.1% |

## Why TKE Is Better Today

This section is generated from the current benchmark and E2E artifacts. The claim is intentionally narrow: it records where the current repo evidence already favors `tke` directly.

| Evidence area | `tke` result | `rtk` result in this repo | Why this matters |
| --- | --- | --- | --- |
| Built-in local compression benchmarks | `33/34` cases, `57793` tokens saved, `91.1%` | No equivalent repo-local tool-output benchmark runner in this repo | `tke` can be measured locally and repeatedly without depending on agent compliance |
| Built-in rollout/task traces | `27` traces, `162905` tokens saved, `89.5%` | RTK participates only through the fairness/synthetic harness subset wired here | `tke` has broader measured coverage inside the repo |
| Codex real E2E | `4/4` pass, `6257` tool tokens saved | `0/2` pass, `11` token delta | Current real Codex evidence favors `tke` clearly |
| Structured output surface | `pathlist`, `search`, `diff`, `log`, `table`, and `file` profiles emit inspectable `__TKE__{...}` summaries | No equivalent repo-local structured envelope | `tke` gives a concrete artifact that tooling can compare and audit |
| Claude stable synthetic traces | `71591` tokens saved at `89.4%` | `71927` tokens saved at `89.6%` | `rtk-hook` currently leads on both absolute token savings and ratio in the stable synthetic Claude traces, while `tke` remains competitive on fragment retention |

Current built-in totals:

| Scope | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| Default compress benchmarks | 33 | 57793 | 91.1% |
| Built-in rollout/task traces | 27 | 162905 | 89.5% |

Per-profile compression totals:

| Profile | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| `diff` | 1 | 3459 | 93.7% |
| `file` | 9 | 5468 | 78.1% |
| `log` | 12 | 6998 | 74.9% |
| `pathlist` | 6 | 36439 | 98.6% |
| `search` | 2 | 4060 | 89.9% |
| `table` | 3 | 1369 | 71.5% |

Claude-oriented stable synthetic summary:

| Path | Raw tokens | Rewritten tokens | Tokens saved | Savings | Fragments kept |
| --- | --- | --- | --- | --- | --- |
| `tke` | 80104 | 8513 | 71591 | 89.4% | `109/109` |
| `rtk-hook` | 80319 | 8392 | 71927 | 89.6% | `112/112` |

Task-mode comparison for Claude-oriented stable synthetic traces:

| Scenario | TKE task savings | RTK hook task savings | TKE fragments kept | RTK hook fragments kept |
| --- | --- | --- | --- | --- |
| find/pathlist | `8032` (98.4%) | `8032` (98.4%) | `4/4` | `6/6` |
| search | `1864` (77.2%) | `1721` (80.5%) | `3/3` | `4/4` |
| diff | `3433` (92.8%) | `3433` (92.9%) | `6/6` | `6/6` |
| build/log | `876` (76.8%) | `876` (77.2%) | `5/5` | `5/5` |
| complex/triage | `14011` (93.6%) | `14011` (93.6%) | `11/11` | `11/11` |
| complex/code-trace | `6637` (86.4%) | `6637` (86.4%) | `11/11` | `11/11` |
| complex/stacktrace | `1959` (70.8%) | `2030` (71.1%) | `9/9` | `9/9` |
| complex/stacktrace-diff | `5484` (83.8%) | `5555` (83.8%) | `12/12` | `12/12` |
| complex/root-cause | `9793` (89.6%) | `9898` (89.6%) | `13/13` | `13/13` |
| answer-consistency | `9451` (90.3%) | `9556` (90.4%) | `15/15` | `15/15` |
| candidate-root-cause | `10051` (88.8%) | `10178` (89.1%) | `20/20` | `20/20` |

Scenario verdicts:

| Scenario | Absolute token winner | Savings-ratio winner | Fragment winner |
| --- | --- | --- | --- |
| find/pathlist | `tie` | `rtk-hook` | `tie` |
| search | `tke` | `rtk-hook` | `tie` |
| diff | `tie` | `rtk-hook` | `tie` |
| build/log | `tie` | `rtk-hook` | `tie` |
| complex/triage | `tie` | `tie` | `tie` |
| complex/code-trace | `tie` | `tie` | `tie` |
| complex/stacktrace | `rtk-hook` | `rtk-hook` | `tie` |
| complex/stacktrace-diff | `rtk-hook` | `tke` | `tie` |
| complex/root-cause | `rtk-hook` | `rtk-hook` | `tie` |
| answer-consistency | `rtk-hook` | `rtk-hook` | `tie` |
| candidate-root-cause | `rtk-hook` | `rtk-hook` | `tie` |

## Structured Summary Coverage

The current local comparison is broader than raw token totals alone. `tke` now emits repo-local structured summaries for several high-volume profiles that RTK does not expose as equivalent local envelope fields in this repo:

| Profile | Current `tke` structure | Current RTK position in this repo |
| --- | --- | --- |
| `pathlist` | `pl.d` shared dir, compact `pl.f`/`pl.l`, examples | No equivalent repo-local structured summary |
| `search` | Grouped file chunks with full first hit and compact followups | No equivalent repo-local structured summary |
| `log` | `lg.fail`, `lg.warn`, `lg.first_fail`, `lg.first_warn` | No equivalent repo-local structured summary |
| `diff` | `df.f[].p/add/del` per-file summaries | No equivalent repo-local structured summary |

## Codex Real E2E

Generated from:

```bash
./target/release/tke compare-e2e --agent codex \
  --source .tmp-codex-e2e \
  --source .tmp-codex-e2e-real \
  --source .tmp-codex-e2e-fair
```

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `buildcase` | `tke` | pass | 893 | `saved_and_correct` |
| `findcase` | `tke` | pass | 27 | `saved_and_correct` |
| `realtask` | `tke` | pass | 0 | `correct_but_not_saved` |
| `rgcase` | `tke` | pass | 5337 | `saved_and_correct` |

Codex aggregate by mode:

| Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool tokens saved |
| --- | --- | --- | --- | --- | --- | --- |
| `rtk-codex-rules` | 2 | 0 | 2 | 0 | 0 | 11 |
| `tke` | 4 | 4 | 0 | 0 | 0 | 6257 |

## RTK Fair Comparison

RTK must be compared through each agent's real integration path:

- Codex: `rtk-codex-rules`
- Claude: `rtk-hook`

| Agent | Case | Raw | RTK path | Raw tool tokens | RTK tool tokens | Tool token delta | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `codex` | `fairfind` | fail | fail | 68 | 68 | 0 | `wrong_and_not_saved` |
| `codex` | `fairrg` | fail | fail | 12 | 1 | 11 | `saved_but_wrong` |
| `claude` | `fairbuild` | pass | pass | 1357 | 1358 | -1 | `correct_but_not_saved` |
| `claude` | `fairfind` | fail | pass | 68 | 68 | 0 | `correct_but_not_saved` |
| `claude` | `fairrg` | pass | pass | 1883 | 1883 | 0 | `correct_but_not_saved` |

Accuracy and compression scorecard:

| Scope | Path | Cases | Accuracy | Compression rate | Semantic retention | Token outcome |
| --- | --- | --- | --- | --- | --- | --- |
| `Claude synthetic` | `tke` | 11 | `n/a` | 89.4% | `109/109` | 71591 |
| `Claude synthetic` | `rtk-hook` | 11 | `n/a` | 89.6% | `112/112` | 71927 |
| `codex` | `rtk-codex-rules` | 2 | 0.0% | `n/a` | `pass=0 fail=2 gateway=0 ungraded=0` | 11 |
| `claude` | `rtk-hook` | 3 | 100.0% | `n/a` | `pass=3 fail=0 gateway=0 ungraded=0` | -1 |

Fair-path aggregate by agent:

| Agent | Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool token delta |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `codex` | `rtk-codex-rules` | 2 | 0 | 2 | 0 | 0 | 11 |
| `claude` | `rtk-hook` | 3 | 3 | 0 | 0 | 0 | -1 |

Codex RTK variant rows:

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `fairfind` | `rtk-codex-rules` | fail | 0 | `wrong_and_not_saved` |
| `fairrg` | `rtk-codex-rules` | fail | 11 | `saved_but_wrong` |

## Claude Real E2E

Generated from:

```bash
./target/release/tke compare-e2e --agent claude \
  --source .tmp-claude-e2e \
  --source .tmp-claude-e2e-fair
```

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `fairbuild` | `rtk-hook` | pass | -1 | `correct_but_not_saved` |
| `fairfind` | `rtk-hook` | pass | 0 | `correct_but_not_saved` |
| `fairrg` | `rtk-hook` | pass | 0 | `correct_but_not_saved` |
| `findcase` | `rtk-hook` | gateway_error | - | `gateway_error` |
| `findcase` | `tke` | fail | 0 | `wrong_and_not_saved` |

Compatibility notes:

- `Claude + tke` currently defaults to compatibility mode in live CLI usage. This keeps agent and tool I/O transparent unless `TKE_CLAUDE_LIVE_TOOLS=1` is set.
- The offline transcript rewriter and compare reports still measure potential savings on saved Claude stream JSONL output.
- `gateway_error` means the gateway returned a transient upstream failure such as Cloudflare 504; treat those samples as infrastructure noise rather than a correctness verdict on the harness itself.

Claude aggregate by mode:

| Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool tokens saved |
| --- | --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 4 | 3 | 0 | 1 | 0 | -1 |
| `tke` | 1 | 0 | 1 | 0 | 0 | 0 |

## Claude Live Probes

These runs exercise the live `tke` Claude path directly and are tracked separately from the formal raw-vs-variant compare table so transient gateway failures do not overwrite the last known-good live result.

| Case | Run name | OK | Completed | Error statuses |
| --- | --- | --- | --- | --- |
| `buildcase` | `livebuild` | yes | yes | - |
| `diffcase` | `livediff` | yes | yes | - |
| `findcase` | `livefind` | yes | yes | - |
| `rgcase` | `liverg` | yes | yes | - |

Claude attempt summary:

| Case | Mode | OK | Completed | Result error | Error statuses |
| --- | --- | --- | --- | --- | --- |
| `compatfind` | `tke` | yes | yes | no | - |
| `livebuild` | `tke` | yes | yes | no | - |
| `livediff` | `tke` | yes | yes | no | - |
| `livefind` | `tke` | yes | yes | no | - |
| `liverg` | `tke` | yes | yes | no | - |

Claude fair-attempt summary:

| Case | Mode | OK | Completed | Result error | Error statuses |
| --- | --- | --- | --- | --- | --- |
| `fairbuild` | `raw` | yes | yes | no | - |
| `fairbuild` | `rtk-hook` | yes | yes | no | - |
| `fairfind` | `raw` | yes | yes | no | - |
| `fairfind` | `rtk-hook` | yes | yes | no | - |
| `fairrg` | `raw` | yes | yes | no | - |
| `fairrg` | `rtk-hook` | yes | yes | no | - |

Successful live compatibility probes: `compatfind`, `livebuild`, `livediff`, `livefind`, `liverg`.
