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
| `git_diff` | diff | 3691 | 616 | 3075 | 83.3% |
| `cargo_build` | log | 884 | 195 | 689 | 77.9% |
| `pytest_run` | log | 453 | 185 | 268 | 59.2% |
| `npm_test` | log | 863 | 179 | 684 | 79.3% |
| `dotnet_test` | log | 800 | 148 | 652 | 81.5% |
| `go_test` | log | 1215 | 122 | 1093 | 90.0% |
| `ninja_build` | log | 1248 | 135 | 1113 | 89.2% |
| `python_json` | json | 6730 | 77 | 6653 | 98.9% |
| `python_paths` | pathlist | 1976 | 94 | 1882 | 95.2% |
| `python_table` | table | 159 | 73 | 86 | 54.1% |
| `ps_table` | table | 655 | 146 | 509 | 77.7% |
| `systemctl_table` | table | 682 | 111 | 571 | 83.7% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 83.3% |
| file | 12 | 79.7% |
| generic | 1 | 0.0% |
| json | 4 | 98.8% |
| log | 27 | 83.2% |
| pathlist | 8 | 96.7% |
| search | 3 | 89.9% |
| table | 10 | 69.9% |

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
| `claude_bash_trace_selected_diff_stage` | api | 3665 | 616 | 3049 | 83.2% |
| `claude_bash_trace_selected_build_stage` | api | 1102 | 185 | 917 | 83.2% |
| `claude_bash_trace_complex_triage_task` | api | 14938 | 1270 | 13668 | 91.5% |
| `claude_bash_trace_complex_code_trace_task` | api | 7647 | 1353 | 6294 | 82.3% |
| `claude_bash_trace_complex_stacktrace_task` | api | 2732 | 732 | 2000 | 73.2% |
| `claude_bash_trace_complex_stacktrace_diff_task` | api | 6505 | 1364 | 5141 | 79.0% |
| `claude_bash_trace_complex_root_cause_task` | api | 10897 | 1447 | 9450 | 86.7% |
| `claude_bash_trace_answer_consistency_task` | api | 10414 | 1306 | 9108 | 87.5% |
| `claude_bash_trace_candidate_root_cause_task` | api | 11248 | 1597 | 9651 | 85.8% |
| `claude_bash_trace_misleading_signal_task` | api | 10463 | 1588 | 8875 | 84.8% |
| `claude_bash_trace_cross_file_causality_task` | api | 14605 | 1644 | 12961 | 88.7% |
| `claude_bash_trace_negative_evidence_task` | api | 6834 | 1254 | 5580 | 81.7% |
| `claude_bash_trace_temporal_causality_task` | api | 13830 | 1650 | 12180 | 88.1% |
| `claude_bash_trace_symbol_collision_task` | api | 5515 | 878 | 4637 | 84.1% |
| `claude_bash_trace_reversal_task` | api | 10174 | 1651 | 8523 | 83.8% |
| `claude_rtk_hook_trace_selected_find_stage` | api | 8125 | 93 | 8032 | 98.9% |
| `claude_rtk_hook_trace_selected_search_stage` | api | 2104 | 383 | 1721 | 81.8% |
| `claude_rtk_hook_trace_selected_diff_stage` | api | 3665 | 616 | 3049 | 83.2% |
| `claude_rtk_hook_trace_selected_build_stage` | api | 1102 | 185 | 917 | 83.2% |
| `claude_rtk_hook_trace_complex_triage_task` | api | 14938 | 1270 | 13668 | 91.5% |
| `claude_rtk_hook_trace_complex_code_trace_task` | api | 7647 | 1353 | 6294 | 82.3% |
| `claude_rtk_hook_trace_complex_stacktrace_task` | api | 2819 | 748 | 2071 | 73.5% |
| `claude_rtk_hook_trace_complex_stacktrace_diff_task` | api | 6592 | 1380 | 5212 | 79.1% |
| `claude_rtk_hook_trace_complex_root_cause_task` | api | 11014 | 1459 | 9555 | 86.8% |
| `claude_rtk_hook_trace_answer_consistency_task` | api | 10531 | 1318 | 9213 | 87.5% |
| `claude_rtk_hook_trace_candidate_root_cause_task` | api | 11382 | 1604 | 9778 | 85.9% |
| `claude_rtk_hook_trace_misleading_signal_task` | api | 10598 | 1597 | 9001 | 84.9% |
| `claude_rtk_hook_trace_cross_file_causality_task` | api | 14741 | 1654 | 13087 | 88.8% |
| `claude_rtk_hook_trace_negative_evidence_task` | api | 6951 | 1266 | 5685 | 81.8% |
| `claude_rtk_hook_trace_temporal_causality_task` | api | 13947 | 1662 | 12285 | 88.1% |
| `claude_rtk_hook_trace_symbol_collision_task` | api | 5631 | 888 | 4743 | 84.2% |
| `claude_rtk_hook_trace_reversal_task` | api | 10430 | 1675 | 8755 | 83.9% |

## Why TKE Is Better Today

This section is generated from the current benchmark and E2E artifacts. The claim is intentionally narrow: it records where the current repo evidence already favors `tke` directly.

| Evidence area | `tke` result | `rtk` result in this repo | Why this matters |
| --- | --- | --- | --- |
| Built-in local compression benchmarks | `65/34` cases, `110406` tokens saved, `92.2%` | No equivalent repo-local tool-output benchmark runner in this repo | `tke` can be measured locally and repeatedly without depending on agent compliance |
| Built-in rollout/task traces | `39` traces, `265109` tokens saved, `86.8%` | RTK participates only through the fairness/synthetic harness subset wired here | `tke` has broader measured coverage inside the repo |
| Codex real E2E | `4/4` pass, `6257` tool tokens saved | `0/2` pass, `11` token delta | Current real Codex evidence favors `tke` clearly |
| Structured output surface | `pathlist`, `search`, `diff`, `log`, `table`, and `file` profiles emit inspectable `__TKE__{...}` summaries | No equivalent repo-local structured envelope | `tke` gives a concrete artifact that tooling can compare and audit |
| Claude stable synthetic traces | `121971` tokens saved at `86.5%` | `123066` tokens saved at `86.5%` | `rtk-hook` currently leads on both absolute token savings and ratio in the stable synthetic Claude traces, while `tke` remains competitive on fragment retention |

Current built-in totals:

| Scope | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| Default compress benchmarks | 65 | 110406 | 92.2% |
| Built-in rollout/task traces | 39 | 265109 | 86.8% |

Per-profile compression totals:

| Profile | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| `diff` | 1 | 3075 | 83.3% |
| `file` | 12 | 7409 | 80.3% |
| `json` | 4 | 26628 | 98.8% |
| `log` | 27 | 21836 | 84.5% |
| `pathlist` | 8 | 42154 | 98.4% |
| `search` | 3 | 6089 | 89.9% |
| `table` | 10 | 3215 | 73.1% |

Claude-oriented stable synthetic summary:

| Path | Raw tokens | Rewritten tokens | Tokens saved | Savings | Fragments kept |
| --- | --- | --- | --- | --- | --- |
| `tke` | 141074 | 19103 | 121971 | 86.5% | `218/218` |
| `rtk-hook` | 142217 | 19151 | 123066 | 86.5% | `221/221` |

Task-mode comparison for Claude-oriented stable synthetic traces:

| Scenario | TKE task savings | RTK hook task savings | TKE fragments kept | RTK hook fragments kept |
| --- | --- | --- | --- | --- |
| find/pathlist | `8032` (98.9%) | `8032` (98.9%) | `4/4` | `6/6` |
| search | `1905` (80.0%) | `1721` (81.8%) | `3/3` | `4/4` |
| diff | `3049` (83.2%) | `3049` (83.2%) | `6/6` | `6/6` |
| build/log | `917` (83.2%) | `917` (83.2%) | `5/5` | `5/5` |
| complex/triage | `13668` (91.5%) | `13668` (91.5%) | `11/11` | `11/11` |
| complex/code-trace | `6294` (82.3%) | `6294` (82.3%) | `11/11` | `11/11` |
| complex/stacktrace | `2000` (73.2%) | `2071` (73.5%) | `9/9` | `9/9` |
| complex/stacktrace-diff | `5141` (79.0%) | `5212` (79.1%) | `12/12` | `12/12` |
| complex/root-cause | `9450` (86.7%) | `9555` (86.8%) | `13/13` | `13/13` |
| answer-consistency | `9108` (87.5%) | `9213` (87.5%) | `15/15` | `15/15` |
| candidate-root-cause | `9651` (85.8%) | `9778` (85.9%) | `20/20` | `20/20` |
| misleading-signal | `8875` (84.8%) | `9001` (84.9%) | `20/20` | `20/20` |
| cross-file-causality | `12961` (88.7%) | `13087` (88.8%) | `19/19` | `19/19` |
| negative-evidence | `5580` (81.7%) | `5685` (81.8%) | `17/17` | `17/17` |
| temporal-causality | `12180` (88.1%) | `12285` (88.1%) | `19/19` | `19/19` |
| symbol-collision | `4637` (84.1%) | `4743` (84.2%) | `15/15` | `15/15` |
| reversal | `8523` (83.8%) | `8755` (83.9%) | `19/19` | `19/19` |

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
| complex/stacktrace-diff | `+71` | `+0.0 pp` | `both full` | `near-tie` |
| complex/root-cause | `+105` | `+0.0 pp` | `both full` | `near-tie` |
| answer-consistency | `+105` | `+0.0 pp` | `both full` | `near-tie` |
| candidate-root-cause | `+127` | `+0.1 pp` | `both full` | `near-tie` |
| misleading-signal | `+126` | `+0.1 pp` | `both full` | `near-tie` |
| cross-file-causality | `+126` | `+0.0 pp` | `both full` | `near-tie` |
| negative-evidence | `+105` | `+0.1 pp` | `both full` | `near-tie` |
| temporal-causality | `+105` | `+0.0 pp` | `both full` | `near-tie` |
| symbol-collision | `+106` | `+0.2 pp` | `both full` | `near-tie` |
| reversal | `+232` | `+0.2 pp` | `both full` | `near-tie` |

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
| `claude` | `fairbuild` | fail | fail | 1175 | 8 | 1167 | `saved_but_wrong` |
| `claude` | `fairfind` | fail | fail | 79 | 59 | 20 | `saved_but_wrong` |
| `claude` | `fairrg` | pass | pass | 5727 | 930 | 4797 | `saved_and_correct` |

Accuracy and compression scorecard:

| Scope | Path | Cases | Accuracy | Compression rate | Semantic retention | Token outcome |
| --- | --- | --- | --- | --- | --- | --- |
| `Claude synthetic` | `tke` | 17 | `n/a` | 86.5% | `218/218` | 121971 |
| `Claude synthetic` | `rtk-hook` | 17 | `n/a` | 86.5% | `221/221` | 123066 |
| `codex` | `rtk-codex-rules` | 2 | 0.0% | `n/a` | `pass=0 fail=2 gateway=0 ungraded=0` | 11 |
| `claude` | `tke` | 3 | 33.3% | `n/a` | `pass=1 fail=2 gateway=0 ungraded=0` | 5984 |

Fair-path aggregate by agent:

| Agent | Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool token delta |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `codex` | `rtk-codex-rules` | 2 | 0 | 2 | 0 | 0 | 11 |
| `claude` | `tke` | 3 | 1 | 2 | 0 | 0 | 5984 |

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
| `fairbuild` | `tke` | fail | 1167 | `saved_but_wrong` |
| `fairfind` | `tke` | fail | 20 | `saved_but_wrong` |
| `fairrg` | `tke` | pass | 4797 | `saved_and_correct` |
| `findcase` | `rtk-hook` | gateway_error | - | `gateway_error` |
| `findcase` | `tke` | fail | 0 | `wrong_and_not_saved` |

Compatibility notes:

- `Claude + tke` compresses tool output via PATH shims. Agent output (TTY or `-p`) is passed through. Tool shims are always active when running inside a `tke` session.
- The offline transcript rewriter and compare reports still measure potential savings on saved Claude stream JSONL output.
- `gateway_error` means the gateway returned a transient upstream failure such as Cloudflare 504; treat those samples as infrastructure noise rather than a correctness verdict on the harness itself.

Claude aggregate by mode:

| Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool tokens saved |
| --- | --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 1 | 0 | 0 | 1 | 0 | 0 |
| `tke` | 4 | 1 | 3 | 0 | 0 | 5984 |

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
| `fairbuild` | `rtk-hook` | no | no | no | - |
| `fairbuild` | `tke` | no | no | no | - |
| `fairfind` | `raw` | yes | yes | no | - |
| `fairfind` | `rtk-hook` | no | no | no | - |
| `fairrg` | `raw` | yes | yes | no | - |
| `fairrg` | `rtk-hook` | no | no | no | - |

Successful live compatibility probes: `compatfind`, `livebuild`, `livediff`, `livefind`, `liverg`.
