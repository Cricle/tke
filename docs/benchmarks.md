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
| `rg_code` | search | 2257 | 250 | 2007 | 88.9% |
| `grep_code` | search | 2257 | 252 | 2005 | 88.8% |
| `find_paths` | pathlist | 8151 | 137 | 8014 | 98.3% |
| `fd_paths` | pathlist | 8151 | 137 | 8014 | 98.3% |
| `tree_paths` | pathlist | 3926 | 137 | 3789 | 96.5% |
| `git_diff` | diff | 3691 | 222 | 3469 | 94.0% |
| `cargo_build` | log | 796 | 170 | 626 | 78.6% |
| `pytest_run` | log | 827 | 182 | 645 | 78.0% |
| `npm_test` | log | 735 | 166 | 569 | 77.4% |
| `dotnet_test` | log | 827 | 182 | 645 | 78.0% |
| `go_test` | log | 705 | 139 | 566 | 80.3% |
| `ninja_build` | log | 796 | 172 | 624 | 78.4% |
| `ps_table` | table | 655 | 153 | 502 | 76.6% |
| `systemctl_table` | table | 682 | 126 | 556 | 81.5% |

Profile averages:

| Profile | Cases | Average token savings |
| --- | --- | --- |
| diff | 1 | 94.0% |
| file | 9 | 77.9% |
| generic | 1 | 0.0% |
| log | 12 | 78.3% |
| pathlist | 6 | 96.1% |
| search | 2 | 88.9% |
| table | 3 | 70.8% |

Built-in rollout/task benchmarks:

| Task | Mode | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | --- | --- | --- | --- | --- |
| `codex_api_trace_rollout_savings` | api | 5389 | 512 | 4877 | 90.5% |
| `codex_api_trace_default_tool_coverage` | api | 4021 | 777 | 3244 | 80.7% |
| `codex_interactive_trace_selected_search_stage` | interactive | 2913 | 570 | 2343 | 80.4% |
| `codex_interactive_trace_selected_find_stage` | interactive | 8125 | 137 | 7988 | 98.3% |
| `codex_interactive_trace_selected_build_stage` | interactive | 1102 | 200 | 902 | 81.9% |
| `claude_bash_trace_selected_search_stage` | api | 2416 | 526 | 1890 | 78.2% |
| `claude_bash_trace_selected_find_stage` | api | 8164 | 176 | 7988 | 97.8% |
| `claude_bash_trace_selected_build_stage` | api | 1141 | 239 | 902 | 79.1% |

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
| `rtk-codex-rules` | 2 | 0 | 0 | 0 | 2 | 11 |
| `tke` | 4 | 4 | 0 | 0 | 0 | 6257 |

## RTK Fair Comparison

RTK must be compared through each agent's real integration path:

- Codex: `rtk-codex-rules`
- Claude: `rtk-hook`

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `fairfind` | `rtk-codex-rules` | ungraded | 0 | `wrong_and_not_saved` |
| `fairrg` | `rtk-codex-rules` | ungraded | 11 | `saved_but_wrong` |

## Claude Real E2E

Generated from:

```bash
./target/release/tke compare-e2e --agent claude --source .tmp-claude-e2e
```

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | --- | --- |
| `findcase` | `rtk-hook` | gateway_error | 0 | `wrong_and_not_saved` |
| `findcase` | `tke` | fail | 0 | `wrong_and_not_saved` |

Compatibility notes:

- `Claude + tke` currently defaults to compatibility mode in live CLI usage. This keeps agent and tool I/O transparent unless `TKE_CLAUDE_LIVE_TOOLS=1` is set.
- The offline transcript rewriter and compare reports still measure potential savings on saved Claude stream JSONL output.
- `gateway_error` means the gateway returned a transient upstream failure such as Cloudflare 504; treat those samples as infrastructure noise rather than a correctness verdict on the harness itself.

Claude aggregate by mode:

| Variant | Cases | Pass | Fail | Gateway | Ungraded | Total tool tokens saved |
| --- | --- | --- | --- | --- | --- | --- |
| `rtk-hook` | 1 | 0 | 0 | 1 | 0 | 0 |
| `tke` | 1 | 0 | 1 | 0 | 0 | 0 |

## Claude Live Probes

These runs exercise the live `tke` Claude path directly and are tracked separately from the formal raw-vs-variant compare table so transient gateway failures do not overwrite the last known-good live result.

| Case | Run name | OK | Completed | Error statuses |
| --- | --- | --- | --- | --- |
| `buildcase` | `livebuild` | yes | yes | - |
| `livediff` | `livediff` | yes | yes | - |
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

Successful live compatibility probes: `compatfind`, `livebuild`, `livediff`, `livefind`, `liverg`.
