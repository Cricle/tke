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
| `nl_code` | file | 1058 | 34 | 1024 | 96.8% |
| `rg_code` | search | 2257 | 226 | 2031 | 90.0% |
| `grep_code` | search | 2257 | 228 | 2029 | 89.9% |
| `find_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `fd_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `tree_paths` | pathlist | 3926 | 93 | 3833 | 97.6% |
| `git_diff` | diff | 3691 | 232 | 3459 | 93.7% |
| `cargo_build` | log | 884 | 195 | 689 | 77.9% |
| `pytest_run` | log | 453 | 185 | 268 | 59.2% |
| `npm_test` | log | 863 | 179 | 684 | 79.3% |
| `dotnet_test` | log | 800 | 148 | 652 | 81.5% |
| `go_test` | log | 1215 | 122 | 1093 | 90.0% |
| `ninja_build` | log | 1248 | 135 | 1113 | 89.2% |
| `python_json` | json | 6730 | 77 | 6653 | 98.9% |
| `python_paths` | pathlist | 1976 | 94 | 1882 | 95.2% |
| `python_table` | table | 159 | 81 | 78 | 49.1% |
| `ps_table` | table | 655 | 154 | 501 | 76.5% |
| `systemctl_table` | table | 682 | 139 | 543 | 79.6% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 93.7% |
| file | 12 | 79.7% |
| generic | 1 | 0.0% |
| json | 4 | 98.8% |
| log | 27 | 83.2% |
| pathlist | 8 | 96.7% |
| search | 3 | 89.9% |
| table | 10 | 67.1% |

Built-in rollout/task benchmarks:

| Task | Mode | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `codex_api_trace_rollout_savings` | api | 5389 | 505 | 4884 | 90.6% |
| `codex_api_trace_default_tool_coverage` | api | 4695 | 799 | 3896 | 83.0% |
| `codex_interactive_trace_selected_search_stage` | interactive | 2913 | 570 | 2343 | 80.4% |
| `codex_interactive_trace_selected_find_stage` | interactive | 8125 | 93 | 8032 | 98.9% |
| `codex_interactive_trace_selected_build_stage` | interactive | 1102 | 185 | 917 | 83.2% |
| `claude_bash_trace_selected_search_stage` | api | 2380 | 475 | 1905 | 80.0% |
| `claude_bash_trace_selected_find_stage` | api | 8125 | 93 | 8032 | 98.9% |
| `claude_bash_trace_selected_diff_stage` | api | 3665 | 232 | 3433 | 93.7% |
| `claude_bash_trace_selected_build_stage` | api | 1102 | 185 | 917 | 83.2% |
| `claude_bash_trace_complex_triage_task` | api | 14938 | 886 | 14052 | 94.1% |
| `claude_bash_trace_complex_code_trace_task` | api | 7647 | 969 | 6678 | 87.3% |
| `claude_bash_trace_complex_stacktrace_task` | api | 2732 | 732 | 2000 | 73.2% |
| `claude_bash_trace_complex_stacktrace_diff_task` | api | 6505 | 980 | 5525 | 84.9% |
| `claude_bash_trace_complex_root_cause_task` | api | 10897 | 1063 | 9834 | 90.2% |
| `claude_bash_trace_answer_consistency_task` | api | 10414 | 922 | 9492 | 91.1% |
| `claude_bash_trace_candidate_root_cause_task` | api | 11248 | 1173 | 10075 | 89.6% |
| `claude_bash_trace_misleading_signal_task` | api | 10463 | 1166 | 9297 | 88.9% |
| `claude_bash_trace_cross_file_causality_task` | api | 14605 | 1262 | 13343 | 91.4% |
| `claude_bash_trace_negative_evidence_task` | api | 6834 | 1254 | 5580 | 81.7% |
| `claude_bash_trace_temporal_causality_task` | api | 13830 | 1265 | 12565 | 90.9% |
| `claude_bash_trace_symbol_collision_task` | api | 5515 | 878 | 4637 | 84.1% |
| `claude_bash_trace_reversal_task` | api | 10174 | 1257 | 8917 | 87.6% |
| `claude_rtk_hook_trace_selected_find_stage` | api | 8125 | 93 | 8032 | 98.9% |
| `claude_rtk_hook_trace_selected_search_stage` | api | 2104 | 383 | 1721 | 81.8% |
| `claude_rtk_hook_trace_selected_diff_stage` | api | 3665 | 232 | 3433 | 93.7% |
| `claude_rtk_hook_trace_selected_build_stage` | api | 1102 | 185 | 917 | 83.2% |
| `claude_rtk_hook_trace_complex_triage_task` | api | 14938 | 886 | 14052 | 94.1% |
| `claude_rtk_hook_trace_complex_code_trace_task` | api | 7647 | 969 | 6678 | 87.3% |
| `claude_rtk_hook_trace_complex_stacktrace_task` | api | 2819 | 748 | 2071 | 73.5% |
| `claude_rtk_hook_trace_complex_stacktrace_diff_task` | api | 6592 | 996 | 5596 | 84.9% |
| `claude_rtk_hook_trace_complex_root_cause_task` | api | 11014 | 1075 | 9939 | 90.2% |
| `claude_rtk_hook_trace_answer_consistency_task` | api | 10531 | 934 | 9597 | 91.1% |
| `claude_rtk_hook_trace_candidate_root_cause_task` | api | 11382 | 1180 | 10202 | 89.6% |
| `claude_rtk_hook_trace_misleading_signal_task` | api | 10598 | 1175 | 9423 | 88.9% |
| `claude_rtk_hook_trace_cross_file_causality_task` | api | 14741 | 1272 | 13469 | 91.4% |
| `claude_rtk_hook_trace_negative_evidence_task` | api | 6951 | 1266 | 5685 | 81.8% |
| `claude_rtk_hook_trace_temporal_causality_task` | api | 13947 | 1277 | 12670 | 90.8% |
| `claude_rtk_hook_trace_symbol_collision_task` | api | 5631 | 888 | 4743 | 84.2% |
| `claude_rtk_hook_trace_reversal_task` | api | 10430 | 1271 | 9159 | 87.8% |

## Why TKE Is Better Today

This section is generated from the current benchmark and E2E artifacts. The claim is intentionally narrow: it records where the current repo evidence already favors `tke` directly.

| Evidence area | `tke` result | `rtk` result in this repo | Why this matters |
| --- | --- | --- | --- |
| Built-in local compression benchmarks | `65/34` cases, `110693` tokens saved, `92.4%` | No equivalent repo-local tool-output benchmark runner in this repo | `tke` can be measured locally and repeatedly without depending on agent compliance |
| Built-in rollout/task traces | `39` traces, `273741` tokens saved, `89.6%` | RTK participates only through the fairness/synthetic harness subset wired here | `tke` has broader measured coverage inside the repo |
| Codex real E2E | `4/4` pass, `6257` tool tokens saved | `0/2` pass, `11` token delta | Current real Codex evidence favors `tke` clearly |
| Structured output surface | `pathlist`, `search`, `diff`, `log`, `table`, and `file` profiles emit inspectable `__TKE__{...}` summaries | No equivalent repo-local structured envelope | `tke` gives a concrete artifact that tooling can compare and audit |
| Claude stable synthetic traces | `126282` tokens saved at `89.5%` | `127387` tokens saved at `89.6%` | `rtk-hook` currently leads on both absolute token savings and ratio in the stable synthetic Claude traces, while `tke` remains competitive on fragment retention |

Current built-in totals:

| Scope | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| Default compress benchmarks | 65 | 110693 | 92.4% |
| Built-in rollout/task traces | 39 | 273741 | 89.6% |

Per-profile compression totals:

| Profile | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| `diff` | 1 | 3459 | 93.7% |
| `file` | 12 | 7409 | 80.3% |
| `json` | 4 | 26628 | 98.8% |
| `log` | 27 | 21836 | 84.5% |
| `pathlist` | 8 | 42154 | 98.4% |
| `search` | 3 | 6089 | 89.9% |
| `table` | 10 | 3118 | 70.9% |

Claude-oriented stable synthetic summary:

| Path | Raw tokens | Rewritten tokens | Tokens saved | Savings | Fragments kept |
| --- | --- | --- | --- | --- | --- |
| `tke` | 141074 | 14792 | 126282 | 89.5% | `218/218` |
| `rtk-hook` | 142217 | 14830 | 127387 | 89.6% | `221/221` |

Task-mode comparison for Claude-oriented stable synthetic traces:

| Scenario | TKE task savings | RTK hook task savings | TKE fragments kept | RTK hook fragments kept |
| --- | --- | --- | --- | --- |
| find/pathlist | `8032` (98.9%) | `8032` (98.9%) | `4/4` | `6/6` |
| search | `1905` (80.0%) | `1721` (81.8%) | `3/3` | `4/4` |
| diff | `3433` (93.7%) | `3433` (93.7%) | `6/6` | `6/6` |
| build/log | `917` (83.2%) | `917` (83.2%) | `5/5` | `5/5` |
| complex/triage | `14052` (94.1%) | `14052` (94.1%) | `11/11` | `11/11` |
| complex/code-trace | `6678` (87.3%) | `6678` (87.3%) | `11/11` | `11/11` |
| complex/stacktrace | `2000` (73.2%) | `2071` (73.5%) | `9/9` | `9/9` |
| complex/stacktrace-diff | `5525` (84.9%) | `5596` (84.9%) | `12/12` | `12/12` |
| complex/root-cause | `9834` (90.2%) | `9939` (90.2%) | `13/13` | `13/13` |
| answer-consistency | `9492` (91.1%) | `9597` (91.1%) | `15/15` | `15/15` |
| candidate-root-cause | `10075` (89.6%) | `10202` (89.6%) | `20/20` | `20/20` |
| misleading-signal | `9297` (88.9%) | `9423` (88.9%) | `20/20` | `20/20` |
| cross-file-causality | `13343` (91.4%) | `13469` (91.4%) | `19/19` | `19/19` |
| negative-evidence | `5580` (81.7%) | `5685` (81.8%) | `17/17` | `17/17` |
| temporal-causality | `12565` (90.9%) | `12670` (90.8%) | `19/19` | `19/19` |
| symbol-collision | `4637` (84.1%) | `4743` (84.2%) | `15/15` | `15/15` |
| reversal | `8917` (87.6%) | `9159` (87.8%) | `19/19` | `19/19` |

Scenario deltas and practical verdicts:

Positive deltas mean `rtk-hook` saved more than `tke`. `near-tie` means both paths kept all required fragments and the gap stayed within a small practical band (`<=250` tokens or `<=2%` of scenario savings, and `<=0.5 pp` ratio gap).

| Scenario | Token delta (RTK-TKE) | Ratio delta (RTK-TKE) | Fragment status | Practical verdict |
| --- | --- | --- | --- | --- |
| find/pathlist | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| search | `-184` | `+1.8 pp` | `both full` | `mixed` |
| diff | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| build/log | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| complex/triage | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| complex/code-trace | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| complex/stacktrace | `+71` | `+0.3 pp` | `both full` | `near-tie` |
| complex/stacktrace-diff | `+71` | `-0.0 pp` | `both full` | `near-tie` |
| complex/root-cause | `+105` | `-0.0 pp` | `both full` | `near-tie` |
| answer-consistency | `+105` | `-0.0 pp` | `both full` | `near-tie` |
| candidate-root-cause | `+127` | `+0.1 pp` | `both full` | `near-tie` |
| misleading-signal | `+126` | `+0.1 pp` | `both full` | `near-tie` |
| cross-file-causality | `+126` | `+0.0 pp` | `both full` | `near-tie` |
| negative-evidence | `+105` | `+0.1 pp` | `both full` | `near-tie` |
| temporal-causality | `+105` | `-0.0 pp` | `both full` | `near-tie` |
| symbol-collision | `+106` | `+0.2 pp` | `both full` | `near-tie` |
| reversal | `+242` | `+0.2 pp` | `both full` | `near-tie` |

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
| `Claude synthetic` | `tke` | 17 | `n/a` | 89.5% | `218/218` | 126282 |
| `Claude synthetic` | `rtk-hook` | 17 | `n/a` | 89.6% | `221/221` | 127387 |
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
