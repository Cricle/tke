# Benchmarks

This file is generated from the current local benchmark and E2E artifacts.

## Synthetic Command Benchmarks

Generated from:

```bash
./target/release/tke-bench benchmark-commands --check
```

| Command case | Profile | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `cat_code` | file | 743 | 166 | 577 | 77.7% |
| `sed_code` | file | 743 | 171 | 572 | 77.0% |
| `bat_code` | file | 743 | 173 | 570 | 76.7% |
| `nl_code` | file | 1058 | 195 | 863 | 81.6% |
| `rg_code` | search | 2257 | 235 | 2022 | 89.6% |
| `grep_code` | search | 2257 | 239 | 2018 | 89.4% |
| `find_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `fd_paths` | pathlist | 8151 | 93 | 8058 | 98.9% |
| `tree_paths` | pathlist | 3926 | 93 | 3833 | 97.6% |
| `git_diff` | diff | 3691 | 623 | 3068 | 83.1% |
| `cargo_build` | log | 884 | 205 | 679 | 76.8% |
| `pytest_run` | log | 453 | 189 | 264 | 58.3% |
| `npm_test` | log | 863 | 182 | 681 | 78.9% |
| `dotnet_test` | log | 800 | 152 | 648 | 81.0% |
| `go_test` | log | 1215 | 127 | 1088 | 89.5% |
| `ninja_build` | log | 1248 | 144 | 1104 | 88.5% |
| `python_json` | json | 6730 | 82 | 6648 | 98.8% |
| `python_paths` | pathlist | 1976 | 94 | 1882 | 95.2% |
| `python_table` | table | 159 | 78 | 81 | 50.9% |
| `ps_table` | table | 655 | 152 | 503 | 76.8% |
| `systemctl_table` | table | 682 | 125 | 557 | 81.7% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 83.1% |
| file | 12 | 77.7% |
| generic | 1 | 0.0% |
| json | 4 | 98.7% |
| log | 27 | 82.6% |
| pathlist | 8 | 96.7% |
| search | 3 | 89.5% |
| table | 10 | 68.0% |

Built-in rollout/task benchmarks:

| Task | Mode | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `codex_api_trace_rollout_savings` | api | 5389 | 670 | 4719 | 87.6% |
| `codex_api_trace_default_tool_coverage` | api | 4695 | 829 | 3866 | 82.3% |
| `codex_interactive_trace_selected_search_stage` | interactive | 2913 | 616 | 2297 | 78.9% |
| `codex_interactive_trace_selected_find_stage` | interactive | 8125 | 93 | 8032 | 98.9% |
| `codex_interactive_trace_selected_build_stage` | interactive | 1102 | 193 | 909 | 82.5% |
| `claude_bash_trace_selected_search_stage` | api | 2380 | 545 | 1835 | 77.1% |
| `claude_bash_trace_selected_find_stage` | api | 8125 | 93 | 8032 | 98.9% |
| `claude_bash_trace_selected_diff_stage` | api | 3665 | 623 | 3042 | 83.0% |
| `claude_bash_trace_selected_build_stage` | api | 1102 | 193 | 909 | 82.5% |
| `claude_bash_trace_complex_triage_task` | api | 14938 | 1306 | 13632 | 91.3% |
| `claude_bash_trace_complex_code_trace_task` | api | 7647 | 1396 | 6251 | 81.7% |
| `claude_bash_trace_complex_stacktrace_task` | api | 2732 | 752 | 1980 | 72.5% |
| `claude_bash_trace_complex_stacktrace_diff_task` | api | 6505 | 1391 | 5114 | 78.6% |
| `claude_bash_trace_complex_root_cause_task` | api | 10897 | 1491 | 9406 | 86.3% |
| `claude_bash_trace_answer_consistency_task` | api | 10414 | 1342 | 9072 | 87.1% |
| `claude_bash_trace_candidate_root_cause_task` | api | 11248 | 1645 | 9603 | 85.4% |
| `claude_bash_trace_misleading_signal_task` | api | 10463 | 1637 | 8826 | 84.4% |
| `claude_bash_trace_cross_file_causality_task` | api | 14605 | 1700 | 12905 | 88.4% |
| `claude_bash_trace_negative_evidence_task` | api | 6834 | 1308 | 5526 | 80.9% |
| `claude_bash_trace_temporal_causality_task` | api | 13830 | 1706 | 12124 | 87.7% |
| `claude_bash_trace_symbol_collision_task` | api | 5515 | 914 | 4601 | 83.4% |
| `claude_bash_trace_reversal_task` | api | 10174 | 1702 | 8472 | 83.3% |
| `claude_rtk_hook_trace_selected_find_stage` | api | 8125 | 93 | 8032 | 98.9% |
| `claude_rtk_hook_trace_selected_search_stage` | api | 2104 | 404 | 1700 | 80.8% |
| `claude_rtk_hook_trace_selected_diff_stage` | api | 3665 | 623 | 3042 | 83.0% |
| `claude_rtk_hook_trace_selected_build_stage` | api | 1102 | 193 | 909 | 82.5% |
| `claude_rtk_hook_trace_complex_triage_task` | api | 14938 | 1306 | 13632 | 91.3% |
| `claude_rtk_hook_trace_complex_code_trace_task` | api | 7647 | 1396 | 6251 | 81.7% |
| `claude_rtk_hook_trace_complex_stacktrace_task` | api | 2819 | 768 | 2051 | 72.8% |
| `claude_rtk_hook_trace_complex_stacktrace_diff_task` | api | 6592 | 1407 | 5185 | 78.7% |
| `claude_rtk_hook_trace_complex_root_cause_task` | api | 11014 | 1503 | 9511 | 86.4% |
| `claude_rtk_hook_trace_answer_consistency_task` | api | 10531 | 1354 | 9177 | 87.1% |
| `claude_rtk_hook_trace_candidate_root_cause_task` | api | 11382 | 1652 | 9730 | 85.5% |
| `claude_rtk_hook_trace_misleading_signal_task` | api | 10598 | 1646 | 8952 | 84.5% |
| `claude_rtk_hook_trace_cross_file_causality_task` | api | 14741 | 1710 | 13031 | 88.4% |
| `claude_rtk_hook_trace_negative_evidence_task` | api | 6951 | 1320 | 5631 | 81.0% |
| `claude_rtk_hook_trace_temporal_causality_task` | api | 13947 | 1718 | 12229 | 87.7% |
| `claude_rtk_hook_trace_symbol_collision_task` | api | 5631 | 924 | 4707 | 83.6% |
| `claude_rtk_hook_trace_reversal_task` | api | 10430 | 1726 | 8704 | 83.5% |

## Why TKE Is Better Today

This section is generated from the current benchmark and E2E artifacts. The claim is intentionally narrow: it records where the current repo evidence already favors `tke` directly.

| Evidence area | `tke` result | `rtk` result in this repo | Why this matters |
| --- | --- | --- | --- |
| Built-in local compression benchmarks | `65/34` cases, `109872` tokens saved, `91.8%` | No equivalent repo-local tool-output benchmark runner in this repo | `tke` can be measured locally and repeatedly without depending on agent compliance |
| Built-in rollout/task traces | `39` traces, `263627` tokens saved, `86.3%` | RTK participates only through the fairness/synthetic harness subset wired here | `tke` has broader measured coverage inside the repo |
| Codex real E2E | `4/4` pass, `6257` tool tokens saved | `1/2` pass, `11` token delta | Current real Codex evidence favors `tke` clearly |
| Structured output surface | `pathlist`, `search`, `diff`, `log`, `table`, and `file` profiles emit inspectable `__TKE__{...}` summaries | No equivalent repo-local structured envelope | `tke` gives a concrete artifact that tooling can compare and audit |
| Claude stable synthetic traces | `121330` tokens saved at `86.0%` | `122474` tokens saved at `86.1%` | `rtk-hook` currently leads on both absolute token savings and ratio in the stable synthetic Claude traces, while `tke` remains competitive on fragment retention |

Current built-in totals:

| Scope | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| Default compress benchmarks | 65 | 109872 | 91.8% |
| Built-in rollout/task traces | 39 | 263627 | 86.3% |

Per-profile compression totals:

| Profile | Cases | Tokens saved | Savings ratio |
| --- | --- | --- | --- |
| `diff` | 1 | 3068 | 83.1% |
| `file` | 12 | 7181 | 77.8% |
| `json` | 4 | 26597 | 98.7% |
| `log` | 27 | 21668 | 83.9% |
| `pathlist` | 8 | 42154 | 98.4% |
| `search` | 3 | 6058 | 89.5% |
| `table` | 10 | 3146 | 71.6% |

Claude-oriented stable synthetic summary:

| Path | Raw tokens | Rewritten tokens | Tokens saved | Savings | Fragments kept |
| --- | --- | --- | --- | --- | --- |
| `tke` | 141074 | 19744 | 121330 | 86.0% | `218/218` |
| `rtk-hook` | 142217 | 19743 | 122474 | 86.1% | `221/221` |

Task-mode comparison for Claude-oriented stable synthetic traces:

| Scenario | TKE task savings | RTK hook task savings | TKE fragments kept | RTK hook fragments kept |
| --- | --- | --- | --- | --- |
| find/pathlist | `8032` (98.9%) | `8032` (98.9%) | `4/4` | `6/6` |
| search | `1835` (77.1%) | `1700` (80.8%) | `3/3` | `4/4` |
| diff | `3042` (83.0%) | `3042` (83.0%) | `6/6` | `6/6` |
| build/log | `909` (82.5%) | `909` (82.5%) | `5/5` | `5/5` |
| complex/triage | `13632` (91.3%) | `13632` (91.3%) | `11/11` | `11/11` |
| complex/code-trace | `6251` (81.7%) | `6251` (81.7%) | `11/11` | `11/11` |
| complex/stacktrace | `1980` (72.5%) | `2051` (72.8%) | `9/9` | `9/9` |
| complex/stacktrace-diff | `5114` (78.6%) | `5185` (78.7%) | `12/12` | `12/12` |
| complex/root-cause | `9406` (86.3%) | `9511` (86.4%) | `13/13` | `13/13` |
| answer-consistency | `9072` (87.1%) | `9177` (87.1%) | `15/15` | `15/15` |
| candidate-root-cause | `9603` (85.4%) | `9730` (85.5%) | `20/20` | `20/20` |
| misleading-signal | `8826` (84.4%) | `8952` (84.5%) | `20/20` | `20/20` |
| cross-file-causality | `12905` (88.4%) | `13031` (88.4%) | `19/19` | `19/19` |
| negative-evidence | `5526` (80.9%) | `5631` (81.0%) | `17/17` | `17/17` |
| temporal-causality | `12124` (87.7%) | `12229` (87.7%) | `19/19` | `19/19` |
| symbol-collision | `4601` (83.4%) | `4707` (83.6%) | `15/15` | `15/15` |
| reversal | `8472` (83.3%) | `8704` (83.5%) | `19/19` | `19/19` |

Scenario deltas and practical verdicts:

Positive deltas mean `rtk-hook` saved more than `tke`. `near-tie` means both paths kept all required fragments and the gap stayed within a small practical band (`<=250` tokens or `<=2%` of scenario savings, and `<=0.5 pp` ratio gap).

| Scenario | Token delta (RTK-TKE) | Ratio delta (RTK-TKE) | Fragment status | Practical verdict |
| --- | --- | --- | --- | --- |
| find/pathlist | `+0` | `+0.0 pp` | `both full` | `near-tie` |
| search | `-135` | `+3.7 pp` | `both full` | `mixed` |
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
./target/release/tke-bench compare-e2e --agent codex \
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
| `rtk-codex-rules` | 2 | 1 | 1 | 0 | 0 | 11 |
| `tke` | 4 | 4 | 0 | 0 | 0 | 6257 |

## RTK Fair Comparison

RTK must be compared through each agent's real integration path:

- Codex: `rtk-codex-rules`
- Claude: `rtk-hook`

| Agent | Case | Raw | RTK path | Raw tool tokens | RTK tool tokens | Tool token delta | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `codex` | `fairfind` | pass | pass | 68 | 68 | 0 | `correct_but_not_saved` |
| `codex` | `fairrg` | fail | fail | 12 | 1 | 11 | `saved_but_wrong` |
| `claude` | `fairbuild` | fail | fail | 1175 | 8 | 1167 | `saved_but_wrong` |
| `claude` | `fairfind` | pass | fail | 79 | 59 | 20 | `saved_but_wrong` |
| `claude` | `fairrg` | pass | pass | 5727 | 930 | 4797 | `saved_and_correct` |

Accuracy and compression scorecard:

| Scope | Path | Cases | Accuracy | Compression rate | Semantic retention | Token outcome |
| --- | --- | --- | --- | --- | --- | --- |
| `Claude synthetic` | `tke` | 17 | `n/a` | 86.0% | `218/218` | 121330 |
| `Claude synthetic` | `rtk-hook` | 17 | `n/a` | 86.1% | `221/221` | 122474 |
| `codex` | `rtk-codex-rules` | 2 | 50.0% | `n/a` | `pass=1 fail=1 gateway=0 ungraded=0` | 11 |
| `claude` | `tke` | 3 | 33.3% | `n/a` | `pass=1 fail=2 gateway=0 ungraded=0` | 5984 |

Fair-path aggregate by agent:

| Agent | Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool token delta |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `codex` | `rtk-codex-rules` | 2 | 1 | 1 | 0 | 0 | 11 |
| `claude` | `tke` | 3 | 1 | 2 | 0 | 0 | 5984 |

Codex RTK variant rows:

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `fairfind` | `rtk-codex-rules` | pass | 0 | `correct_but_not_saved` |
| `fairrg` | `rtk-codex-rules` | fail | 11 | `saved_but_wrong` |

## Claude Real E2E

Generated from:

```bash
./target/release/tke-bench compare-e2e --agent claude \
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
