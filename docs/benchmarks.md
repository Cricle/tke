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
| `find_paths` | pathlist | 8151 | 96 | 8055 | 98.8% |
| `fd_paths` | pathlist | 8151 | 96 | 8055 | 98.8% |
| `tree_paths` | pathlist | 3926 | 96 | 3830 | 97.6% |
| `git_diff` | diff | 3691 | 232 | 3459 | 93.7% |
| `cargo_build` | log | 796 | 201 | 595 | 74.7% |
| `pytest_run` | log | 827 | 214 | 613 | 74.1% |
| `npm_test` | log | 735 | 196 | 539 | 73.3% |
| `dotnet_test` | log | 827 | 214 | 613 | 74.1% |
| `go_test` | log | 705 | 169 | 536 | 76.0% |
| `ninja_build` | log | 796 | 203 | 593 | 74.5% |
| `ps_table` | table | 655 | 153 | 502 | 76.6% |
| `systemctl_table` | table | 682 | 126 | 556 | 81.5% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 93.7% |
| file | 9 | 77.9% |
| generic | 1 | 0.0% |
| log | 12 | 74.4% |
| pathlist | 6 | 96.6% |
| search | 2 | 89.9% |
| table | 3 | 70.6% |

Built-in rollout/task benchmarks:

| Task | Mode | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `codex_api_trace_rollout_savings` | api | 5389 | 505 | 4884 | 90.6% |
| `codex_api_trace_default_tool_coverage` | api | 4021 | 769 | 3252 | 80.9% |
| `codex_interactive_trace_selected_search_stage` | interactive | 2913 | 570 | 2343 | 80.4% |
| `codex_interactive_trace_selected_find_stage` | interactive | 8125 | 96 | 8029 | 98.8% |
| `codex_interactive_trace_selected_build_stage` | interactive | 1102 | 230 | 872 | 79.1% |
| `claude_bash_trace_selected_search_stage` | api | 2416 | 556 | 1860 | 77.0% |
| `claude_bash_trace_selected_find_stage` | api | 8164 | 135 | 8029 | 98.3% |
| `claude_bash_trace_selected_build_stage` | api | 1141 | 269 | 872 | 76.4% |
| `claude_rtk_hook_trace_selected_find_stage` | api | 8159 | 130 | 8029 | 98.4% |
| `claude_rtk_hook_trace_selected_search_stage` | api | 2137 | 416 | 1721 | 80.5% |
| `claude_rtk_hook_trace_selected_build_stage` | api | 1135 | 263 | 872 | 76.8% |

Task-mode comparison for Claude-oriented stable synthetic traces:

| Scenario | TKE task savings | RTK hook task savings | TKE fragments kept | RTK hook fragments kept |
| --- | --- | --- | --- | --- |
| find/pathlist | `8029` (98.3%) | `8029` (98.4%) | `4/4` | `6/6` |
| search | `1860` (77.0%) | `1721` (80.5%) | `3/3` | `4/4` |
| build/log | `872` (76.4%) | `872` (76.8%) | `5/5` | `5/5` |

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
