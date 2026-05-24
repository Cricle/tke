# Benchmarks

This file records the current benchmark and real-task comparison results available in the repository as of `2026-05-24`.

## Synthetic Command Benchmarks

Current synthetic benchmark report was generated from:

```bash
./target/release/tke benchmark-commands --check
```

Selected command results:

| Command case | Raw tokens | Rewritten tokens | Tokens saved | Savings |
| --- | ---: | ---: | ---: | ---: |
| `cat_code` | 743 | 161 | 582 | 78.3% |
| `sed_code` | 743 | 164 | 579 | 77.9% |
| `rg_code` | 2257 | 250 | 2007 | 88.9% |
| `find_paths` | 8151 | 55 | 8096 | 99.3% |
| `fd_paths` | 8151 | 55 | 8096 | 99.3% |
| `tree_paths` | 3926 | 55 | 3871 | 98.6% |
| `ls_names` | 428 | 36 | 392 | 91.6% |
| `cargo_build` | 796 | 170 | 626 | 78.6% |
| `pytest_run` | 827 | 182 | 645 | 78.0% |
| `dotnet_test` | 827 | 182 | 645 | 78.0% |
| `go_test` | 705 | 139 | 566 | 80.3% |
| `ninja_build` | 796 | 172 | 624 | 78.4% |

Notable behavior:

- Code-reading commands such as `cat`, `sed`, `head`, `tail`, `bat`, and `nl` consistently save around `72%` to `82%` of estimated tokens.
- Search output such as `rg` and `grep` is usually reduced by around `88%`.
- Path discovery output such as `find`, `fd`, and `tree` is the strongest case, often exceeding `98%` savings.
- Build and test logs across Rust, Python, Node, .NET, Go, CMake, Make, and Ninja usually land around `77%` to `80%` savings.

## Codex Real E2E

Current real Codex comparison was generated from:

```bash
./target/release/tke compare-e2e --agent codex \
  --source .tmp-codex-e2e \
  --source .tmp-codex-e2e-real \
  --source .tmp-codex-e2e-fair
```

Stable real-task results:

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | ---: | --- |
| `findcase` | `tke` | yes | 27 | `saved_and_correct` |
| `buildcase` | `tke` | yes | 893 | `saved_and_correct` |
| `rgcase` | `tke` | yes | 5337 | `saved_and_correct` |
| `realtask` | `tke` | yes | 0 | `correct_but_not_saved` |

Notes:

- `rgcase` is currently the strongest stable real code-reading win in this repo.
- `realtask` is correct but not large enough to trigger meaningful compression in the current output.

## RTK Comparison

RTK must be compared through the integration path each agent actually uses:

- Claude Code: `rtk-hook`
- Codex: `rtk-codex-rules`

Current fair Codex RTK result:

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | ---: | --- |
| `fairfind` | `rtk-codex-rules` | ungraded correctness, same final answer shape | 0 | `wrong_and_not_saved` against raw-output savings baseline |

Interpretation:

- The official Codex fairness path was exercised.
- In the sampled real task, Codex still effectively executed the raw command path rather than yielding an RTK-compressed tool payload.
- As a result, the measured tool-output savings for the RTK Codex fairness path were `0` in that sample.

This does not claim RTK is generally ineffective. It only records what happened in the tested Codex real task and harness.

## Claude Real E2E

Current fair Claude comparison was generated from:

```bash
./target/release/tke compare-e2e --agent claude --source .tmp-claude-e2e
```

| Case | Variant | Correct | Tool token savings | Verdict |
| --- | --- | --- | ---: | --- |
| `findcase` | `rtk-hook` | yes | 0 | `correct_but_not_saved` |
| `findcase` | `tke` | no | 67 | `saved_but_wrong` |

Interpretation:

- `raw` and `rtk-hook` are currently the stable Claude paths in this repo.
- The tested `Claude + tke` path is not yet considered production-safe because the model failed correctness in real execution.

## Recommended Reading Order

- Start with `README.md` for usage.
- Read `docs/e2e.md` for correctness and fairness.
- Use this file when you need measured savings numbers.
