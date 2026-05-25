#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
TKE_BIN="${TKE_BIN:-$ROOT/target/release/tke}"

python3 - "$ROOT" "$TKE_BIN" <<'PY'
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
tke_bin = pathlib.Path(sys.argv[2])


def run_json(args):
    data = subprocess.check_output(args, cwd=root, text=True)
    return json.loads(data)


def try_compare(agent, sources):
    present = [src for src in sources if (root / src).exists()]
    if not present:
        return None
    args = [str(tke_bin), "compare-e2e", "--agent", agent]
    for src in present:
        args.extend(["--source", src])
    return run_json(args)


def load_attempts(agent_dirs):
    out = {}
    for agent_dir in agent_dirs:
        root_dir = root / agent_dir
        if not root_dir.exists():
            continue
        for path in sorted(root_dir.glob("*.attempt.json")):
            try:
                item = json.loads(path.read_text())
            except Exception:
                continue
            key = (item.get("name", "-"), item.get("mode", "-"))
            out[key] = item
    return [out[key] for key in sorted(out)]


benchmark = run_json([str(tke_bin), "benchmark-commands"])
codex = try_compare(
    "codex",
    [".tmp-codex-e2e", ".tmp-codex-e2e-real", ".tmp-codex-e2e-fair"],
)
claude = try_compare("claude", [".tmp-claude-e2e", ".tmp-claude-e2e-fair"])
claude_attempts = load_attempts([".tmp-claude-e2e", ".tmp-claude-e2e-fair"])


def pct(value):
    return f"{value * 100:.1f}%"


def signed_int(value):
    return f"{value:+d}"


def signed_pp(value):
    return f"{value * 100:+.1f} pp"


def md_table(headers, rows):
    out = []
    out.append("| " + " | ".join(headers) + " |")
    out.append("| " + " | ".join(["---"] * len(headers)) + " |")
    for row in rows:
        out.append("| " + " | ".join(row) + " |")
    return "\n".join(out)


selected_cases = [
    "cat_code",
    "sed_code",
    "bat_code",
    "nl_code",
    "rg_code",
    "grep_code",
    "find_paths",
    "fd_paths",
    "tree_paths",
    "git_diff",
    "cargo_build",
    "pytest_run",
    "npm_test",
    "dotnet_test",
    "go_test",
    "ninja_build",
    "python_json",
    "python_paths",
    "python_table",
    "ps_table",
    "systemctl_table",
]
bench_cases = {case["name"]: case for case in benchmark["cases"]}
bench_rows = []
for name in selected_cases:
    case = bench_cases.get(name)
    if not case:
        continue
    bench_rows.append(
        [
            f"`{case['name']}`",
            case["profile"],
            str(case["raw_tokens"]),
            str(case["rewritten_tokens"]),
            str(case["tokens_saved"]),
            pct(case["tokens_saved_ratio"]),
        ]
    )

profile_buckets = {}
for case in benchmark["cases"]:
    bucket = profile_buckets.setdefault(case["profile"], [])
    bucket.append(case["tokens_saved_ratio"])
profile_rows = [
    [profile, str(len(values)), pct(sum(values) / len(values))]
    for profile, values in sorted(profile_buckets.items())
]

compress_cases = [case for case in benchmark["cases"] if case["expected"] == "compress"]
compress_case_count = len(compress_cases)
compress_tokens_saved = sum(case["tokens_saved"] for case in compress_cases)
compress_raw_tokens = sum(case["raw_tokens"] for case in compress_cases)
compress_ratio = pct(compress_tokens_saved / compress_raw_tokens) if compress_raw_tokens else "0.0%"

task_case_count = len(benchmark["tasks"])
task_tokens_saved = sum(task["tokens_saved"] for task in benchmark["tasks"])
task_raw_tokens = sum(task["raw_tokens"] for task in benchmark["tasks"])
task_ratio = pct(task_tokens_saved / task_raw_tokens) if task_raw_tokens else "0.0%"

profile_totals = {}
for case in compress_cases:
    entry = profile_totals.setdefault(
        case["profile"],
        {"cases": 0, "tokens_saved": 0, "raw_tokens": 0},
    )
    entry["cases"] += 1
    entry["tokens_saved"] += case["tokens_saved"]
    entry["raw_tokens"] += case["raw_tokens"]

profile_total_rows = []
for profile, entry in sorted(profile_totals.items()):
    ratio = pct(entry["tokens_saved"] / entry["raw_tokens"]) if entry["raw_tokens"] else "0.0%"
    profile_total_rows.append(
        [
            f"`{profile}`",
            str(entry["cases"]),
            str(entry["tokens_saved"]),
            ratio,
        ]
    )

task_rows = []
for task in benchmark["tasks"]:
    task_rows.append(
        [
            f"`{task['name']}`",
            task["mode"],
            str(task["raw_tokens"]),
            str(task["rewritten_tokens"]),
            str(task["tokens_saved"]),
            pct(task["tokens_saved_ratio"]),
        ]
    )


def task_by_name(name):
    return next((task for task in benchmark["tasks"] if task["name"] == name), None)


comparison_task_specs = [
    ("find/pathlist", "claude_bash_trace_selected_find_stage", "claude_rtk_hook_trace_selected_find_stage"),
    ("search", "claude_bash_trace_selected_search_stage", "claude_rtk_hook_trace_selected_search_stage"),
    ("diff", "claude_bash_trace_selected_diff_stage", "claude_rtk_hook_trace_selected_diff_stage"),
    ("build/log", "claude_bash_trace_selected_build_stage", "claude_rtk_hook_trace_selected_build_stage"),
    ("complex/triage", "claude_bash_trace_complex_triage_task", "claude_rtk_hook_trace_complex_triage_task"),
    ("complex/code-trace", "claude_bash_trace_complex_code_trace_task", "claude_rtk_hook_trace_complex_code_trace_task"),
    ("complex/stacktrace", "claude_bash_trace_complex_stacktrace_task", "claude_rtk_hook_trace_complex_stacktrace_task"),
    ("complex/stacktrace-diff", "claude_bash_trace_complex_stacktrace_diff_task", "claude_rtk_hook_trace_complex_stacktrace_diff_task"),
    ("complex/root-cause", "claude_bash_trace_complex_root_cause_task", "claude_rtk_hook_trace_complex_root_cause_task"),
    ("answer-consistency", "claude_bash_trace_answer_consistency_task", "claude_rtk_hook_trace_answer_consistency_task"),
    ("candidate-root-cause", "claude_bash_trace_candidate_root_cause_task", "claude_rtk_hook_trace_candidate_root_cause_task"),
    ("misleading-signal", "claude_bash_trace_misleading_signal_task", "claude_rtk_hook_trace_misleading_signal_task"),
    ("cross-file-causality", "claude_bash_trace_cross_file_causality_task", "claude_rtk_hook_trace_cross_file_causality_task"),
    ("negative-evidence", "claude_bash_trace_negative_evidence_task", "claude_rtk_hook_trace_negative_evidence_task"),
    ("temporal-causality", "claude_bash_trace_temporal_causality_task", "claude_rtk_hook_trace_temporal_causality_task"),
    ("symbol-collision", "claude_bash_trace_symbol_collision_task", "claude_rtk_hook_trace_symbol_collision_task"),
    ("reversal", "claude_bash_trace_reversal_task", "claude_rtk_hook_trace_reversal_task"),
]

comparison_task_rows = []
comparison_task_totals = {
    "tke_raw_tokens": 0,
    "tke_rewritten_tokens": 0,
    "tke_saved_tokens": 0,
    "tke_required_fragments": 0,
    "tke_preserved_fragments": 0,
    "rtk_raw_tokens": 0,
    "rtk_rewritten_tokens": 0,
    "rtk_saved_tokens": 0,
    "rtk_required_fragments": 0,
    "rtk_preserved_fragments": 0,
}
comparison_verdict_rows = []
for label, tke_name, rtk_name in comparison_task_specs:
    tke_task = task_by_name(tke_name)
    rtk_task = task_by_name(rtk_name)
    if not tke_task or not rtk_task:
        continue
    comparison_task_totals["tke_raw_tokens"] += tke_task["raw_tokens"]
    comparison_task_totals["tke_rewritten_tokens"] += tke_task["rewritten_tokens"]
    comparison_task_totals["tke_saved_tokens"] += tke_task["tokens_saved"]
    comparison_task_totals["tke_required_fragments"] += len(tke_task["required_fragments"])
    comparison_task_totals["tke_preserved_fragments"] += len(tke_task["preserved_fragments"])
    comparison_task_totals["rtk_raw_tokens"] += rtk_task["raw_tokens"]
    comparison_task_totals["rtk_rewritten_tokens"] += rtk_task["rewritten_tokens"]
    comparison_task_totals["rtk_saved_tokens"] += rtk_task["tokens_saved"]
    comparison_task_totals["rtk_required_fragments"] += len(rtk_task["required_fragments"])
    comparison_task_totals["rtk_preserved_fragments"] += len(rtk_task["preserved_fragments"])
    comparison_task_rows.append(
        [
            label,
            f"`{tke_task['tokens_saved']}` ({pct(tke_task['tokens_saved_ratio'])})",
            f"`{rtk_task['tokens_saved']}` ({pct(rtk_task['tokens_saved_ratio'])})",
            f"`{len(tke_task['preserved_fragments'])}/{len(tke_task['required_fragments'])}`",
            f"`{len(rtk_task['preserved_fragments'])}/{len(rtk_task['required_fragments'])}`",
        ]
    )
    if tke_task["tokens_saved"] > rtk_task["tokens_saved"]:
        token_winner = "`tke`"
    elif tke_task["tokens_saved"] < rtk_task["tokens_saved"]:
        token_winner = "`rtk-hook`"
    else:
        token_winner = "`tie`"
    if tke_task["tokens_saved_ratio"] > rtk_task["tokens_saved_ratio"]:
        ratio_winner = "`tke`"
    elif tke_task["tokens_saved_ratio"] < rtk_task["tokens_saved_ratio"]:
        ratio_winner = "`rtk-hook`"
    else:
        ratio_winner = "`tie`"
    tke_fragment_ok = len(tke_task["preserved_fragments"]) == len(tke_task["required_fragments"])
    rtk_fragment_ok = len(rtk_task["preserved_fragments"]) == len(rtk_task["required_fragments"])
    if tke_fragment_ok and rtk_fragment_ok:
        fragment_status = "`both full`"
    elif tke_fragment_ok:
        fragment_status = "`tke full`"
    elif rtk_fragment_ok:
        fragment_status = "`rtk-hook full`"
    else:
        fragment_status = "`partial`"

    token_delta = rtk_task["tokens_saved"] - tke_task["tokens_saved"]
    ratio_delta = rtk_task["tokens_saved_ratio"] - tke_task["tokens_saved_ratio"]
    near_tie_token_limit = max(250, int(max(tke_task["tokens_saved"], rtk_task["tokens_saved"]) * 0.02))
    near_tie_ratio_limit = 0.005
    if tke_fragment_ok and rtk_fragment_ok and abs(token_delta) <= near_tie_token_limit and abs(ratio_delta) <= near_tie_ratio_limit:
        practical_verdict = "`near-tie`"
    elif tke_fragment_ok and not rtk_fragment_ok:
        practical_verdict = "`tke`"
    elif rtk_fragment_ok and not tke_fragment_ok:
        practical_verdict = "`rtk-hook`"
    elif token_winner == ratio_winner:
        practical_verdict = token_winner
    else:
        practical_verdict = "`mixed`"
    comparison_verdict_rows.append(
        [
            label,
            f"`{signed_int(token_delta)}`",
            f"`{signed_pp(ratio_delta)}`",
            fragment_status,
            practical_verdict,
        ]
    )


def ratio_pct(saved, total):
    if total == 0:
        return "0.0%"
    return pct(saved / total)


comparison_totals_rows = []
if comparison_task_rows:
    comparison_totals_rows = [
        [
            "`tke`",
            str(comparison_task_totals["tke_raw_tokens"]),
            str(comparison_task_totals["tke_rewritten_tokens"]),
            str(comparison_task_totals["tke_saved_tokens"]),
            ratio_pct(
                comparison_task_totals["tke_saved_tokens"],
                comparison_task_totals["tke_raw_tokens"],
            ),
            f"`{comparison_task_totals['tke_preserved_fragments']}/{comparison_task_totals['tke_required_fragments']}`",
        ],
        [
            "`rtk-hook`",
            str(comparison_task_totals["rtk_raw_tokens"]),
            str(comparison_task_totals["rtk_rewritten_tokens"]),
            str(comparison_task_totals["rtk_saved_tokens"]),
            ratio_pct(
                comparison_task_totals["rtk_saved_tokens"],
                comparison_task_totals["rtk_raw_tokens"],
            ),
            f"`{comparison_task_totals['rtk_preserved_fragments']}/{comparison_task_totals['rtk_required_fragments']}`",
        ],
    ]


def collect_variant_rows(report, wanted_modes=None):
    rows = []
    if not report:
        return rows
    for case in report["cases"]:
        case_name = normalize_case_name(case["name"])
        for variant in case["variants"]:
            mode = variant["mode"]
            if wanted_modes and mode not in wanted_modes:
                continue
            correctness = variant["sample"]["correctness"]["status"]
            rows.append(
                [
                    f"`{case_name}`",
                    f"`{mode}`",
                    correctness,
                    "-" if correctness == "gateway_error" else str(variant["tool_tokens_saved"]),
                    "`gateway_error`" if correctness == "gateway_error" else f"`{variant['verdict']}`",
                ]
            )
    return rows


def collect_summary_rows(report, wanted_modes=None):
    if not report:
        return []
    totals = {}
    for case in report["cases"]:
        for variant in case["variants"]:
            mode = variant["mode"]
            if wanted_modes and mode not in wanted_modes:
                continue
            entry = totals.setdefault(
                mode,
                {
                    "cases": 0,
                    "pass": 0,
                    "fail": 0,
                    "gateway_error": 0,
                    "ungraded": 0,
                    "tool_tokens_saved": 0,
                },
            )
            entry["cases"] += 1
            status = variant["sample"]["correctness"]["status"]
            if status in ("pass", "fail", "gateway_error", "ungraded"):
                entry[status] += 1
            if status != "gateway_error":
                entry["tool_tokens_saved"] += variant["tool_tokens_saved"]
    rows = []
    for mode, entry in sorted(totals.items()):
        rows.append(
            [
                f"`{mode}`",
                str(entry["cases"]),
                str(entry["pass"]),
                str(entry["fail"]),
                str(entry["gateway_error"]),
                str(entry["ungraded"]),
                str(entry["tool_tokens_saved"]),
            ]
        )
    return rows


def parse_int_cell(value):
    try:
        return int(value)
    except Exception:
        return 0


def summary_map(rows):
    out = {}
    for row in rows:
        out[row[0].strip("`")] = {
            "cases": parse_int_cell(row[1]),
            "pass": parse_int_cell(row[2]),
            "fail": parse_int_cell(row[3]),
            "gateway": parse_int_cell(row[4]),
            "ungraded": parse_int_cell(row[5]),
            "tokens": parse_int_cell(row[6]),
        }
    return out


def collect_fair_compare_rows(report, agent_label, variant_mode):
    rows = []
    if not report:
        return rows
    for case in sorted(report["cases"], key=lambda item: normalize_case_name(item["name"])):
        case_name = normalize_case_name(case["name"])
        if not case_name.startswith("fair"):
            continue
        baseline = case["baseline"]
        variant = next((item for item in case["variants"] if item["mode"] == variant_mode), None)
        if variant is None:
            continue
        raw_status = baseline["correctness"]["status"]
        variant_status = variant["sample"]["correctness"]["status"]
        rows.append(
            [
                f"`{agent_label}`",
                f"`{case_name}`",
                raw_status,
                variant_status,
                str(baseline["tool_tokens"]),
                str(variant["sample"]["tool_tokens"]),
                str(variant["tool_tokens_saved"]),
                f"`{variant['verdict']}`",
            ]
        )
    return rows


def collect_fair_compare_summary_rows(items):
    rows = []
    for agent_label, report, variant_mode in items:
        if not report:
            continue
        totals = {
            "cases": 0,
            "pass": 0,
            "fail": 0,
            "gateway_error": 0,
            "ungraded": 0,
            "tool_tokens_saved": 0,
        }
        for case in report["cases"]:
            case_name = normalize_case_name(case["name"])
            if not case_name.startswith("fair"):
                continue
            variant = next((item for item in case["variants"] if item["mode"] == variant_mode), None)
            if variant is None:
                continue
            totals["cases"] += 1
            status = variant["sample"]["correctness"]["status"]
            if status in totals:
                totals[status] += 1
            if status != "gateway_error":
                totals["tool_tokens_saved"] += variant["tool_tokens_saved"]
        if totals["cases"] == 0:
            continue
        rows.append(
            [
                f"`{agent_label}`",
                f"`{variant_mode}`",
                str(totals["cases"]),
                str(totals["pass"]),
                str(totals["fail"]),
                str(totals["gateway_error"]),
                str(totals["ungraded"]),
                str(totals["tool_tokens_saved"]),
            ]
        )
    return rows


def normalize_case_name(name):
    if name.startswith("livefind") or name.startswith("compatfind"):
        return "findcase"
    if name.startswith("livebuild"):
        return "buildcase"
    if name.startswith("livediff"):
        return "diffcase"
    if name.startswith("liverg"):
        return "rgcase"
    return name


codex_tke_rows = collect_variant_rows(codex, {"tke"})
codex_rtk_rows = collect_variant_rows(codex, {"rtk-codex-rules"})
claude_rows = collect_variant_rows(claude, {"tke", "rtk-hook"})
codex_summary_rows = collect_summary_rows(codex, {"tke", "rtk-codex-rules", "rtk-direct"})
claude_summary_rows = collect_summary_rows(claude, {"tke", "rtk-hook"})
codex_summary_by_mode = summary_map(codex_summary_rows)
claude_summary_by_mode = summary_map(claude_summary_rows)
fair_compare_rows = (
    collect_fair_compare_rows(codex, "codex", "rtk-codex-rules")
    + collect_fair_compare_rows(claude, "claude", "rtk-hook")
)
fair_compare_summary_rows = collect_fair_compare_summary_rows(
    [
        ("codex", codex, "rtk-codex-rules"),
        ("claude", claude, "rtk-hook"),
    ]
)


def count_rows(rows):
    return sum(1 for _ in rows)


rtk_scorecard_rows = []
if comparison_totals_rows:
    tke_summary = comparison_totals_rows[0]
    rtk_summary = comparison_totals_rows[1]
    tke_synth_saved = parse_int_cell(tke_summary[3])
    rtk_synth_saved = parse_int_cell(rtk_summary[3])
    tke_synth_fragments = tke_summary[5]
    rtk_synth_fragments = rtk_summary[5]
    rtk_scorecard_rows.extend(
        [
            [
                "`Claude synthetic`",
                "`tke`",
                str(count_rows(comparison_task_rows)),
                "`n/a`",
                tke_summary[4],
                tke_synth_fragments,
                str(tke_synth_saved),
            ],
            [
                "`Claude synthetic`",
                "`rtk-hook`",
                str(count_rows(comparison_task_rows)),
                "`n/a`",
                rtk_summary[4],
                rtk_synth_fragments,
                str(rtk_synth_saved),
            ],
        ]
    )
for row in fair_compare_summary_rows:
    agent = row[0]
    variant = row[1]
    cases = row[2]
    passed = parse_int_cell(row[3])
    failed = parse_int_cell(row[4])
    gateway = parse_int_cell(row[5])
    ungraded = parse_int_cell(row[6])
    total_delta = row[7]
    decided = passed + failed
    accuracy = "n/a"
    if decided > 0:
        accuracy = pct(passed / decided)
    fragments = f"`pass={passed} fail={failed} gateway={gateway} ungraded={ungraded}`"
    rtk_scorecard_rows.append(
        [
            agent,
            variant,
            cases,
            accuracy,
            "`n/a`",
            fragments,
            total_delta,
        ]
    )

evidence_rows = [
    [
        "Built-in local compression benchmarks",
        f"`{compress_case_count}/34` cases, `{compress_tokens_saved}` tokens saved, `{compress_ratio}`",
        "No equivalent repo-local tool-output benchmark runner in this repo",
        "`tke` can be measured locally and repeatedly without depending on agent compliance",
    ],
    [
        "Built-in rollout/task traces",
        f"`{task_case_count}` traces, `{task_tokens_saved}` tokens saved, `{task_ratio}`",
        "RTK participates only through the fairness/synthetic harness subset wired here",
        "`tke` has broader measured coverage inside the repo",
    ],
]

codex_tke_summary = codex_summary_by_mode.get("tke")
codex_rtk_summary = codex_summary_by_mode.get("rtk-codex-rules")
if codex_tke_summary:
    tke_cell = f"`{codex_tke_summary['pass']}/{codex_tke_summary['cases']}` pass, `{codex_tke_summary['tokens']}` tool tokens saved"
else:
    tke_cell = "missing"
if codex_rtk_summary:
    rtk_cell = f"`{codex_rtk_summary['pass']}/{codex_rtk_summary['cases']}` pass, `{codex_rtk_summary['tokens']}` token delta"
else:
    rtk_cell = "missing"
evidence_rows.append(
    [
        "Codex real E2E",
        tke_cell,
        rtk_cell,
        "Current real Codex evidence favors `tke` clearly",
    ]
)
evidence_rows.append(
    [
        "Structured output surface",
        "`pathlist`, `search`, `diff`, `log`, `table`, and `file` profiles emit inspectable `__TKE__{...}` summaries",
        "No equivalent repo-local structured envelope",
        "`tke` gives a concrete artifact that tooling can compare and audit",
    ]
)
if comparison_totals_rows:
    tke_summary = comparison_totals_rows[0]
    rtk_summary = comparison_totals_rows[1]
    tke_saved = parse_int_cell(tke_summary[3])
    rtk_saved = parse_int_cell(rtk_summary[3])
    if tke_saved > rtk_saved:
        comparison_claim = "`tke` currently leads on absolute token savings, while `rtk-hook` may still differ on ratio depending on task mix"
    elif tke_saved < rtk_saved:
        comparison_claim = "`rtk-hook` currently leads on both absolute token savings and ratio in the stable synthetic Claude traces, while `tke` remains competitive on fragment retention"
    else:
        comparison_claim = "The two paths are currently tied on absolute token savings, with any remaining difference coming from ratio or fragment details"
    evidence_rows.append(
        [
            "Claude stable synthetic traces",
            f"`{tke_summary[3]}` tokens saved at `{tke_summary[4]}`",
            f"`{rtk_summary[3]}` tokens saved at `{rtk_summary[4]}`",
            comparison_claim,
        ]
    )

benchmarks_md = [
    "# Benchmarks",
    "",
    "This file is generated from the current local benchmark and E2E artifacts.",
    "",
    "## Synthetic Command Benchmarks",
    "",
    "Generated from:",
    "",
    "```bash",
    "./target/release/tke benchmark-commands --check",
    "```",
    "",
    md_table(
        ["Command case", "Profile", "Raw tokens", "Rewritten tokens", "Tokens saved", "Savings"],
        bench_rows,
    ),
    "",
    "Profile averages:",
    "",
    md_table(["Profile", "Cases", "Average token savings"], profile_rows),
    "",
    "Built-in rollout/task benchmarks:",
    "",
    md_table(
        ["Task", "Mode", "Raw tokens", "Rewritten tokens", "Tokens saved", "Savings"],
        task_rows,
    ),
    "",
    "## Why TKE Is Better Today",
    "",
    "This section is generated from the current benchmark and E2E artifacts. The claim is intentionally narrow: it records where the current repo evidence already favors `tke` directly.",
    "",
    md_table(
        ["Evidence area", "`tke` result", "`rtk` result in this repo", "Why this matters"],
        evidence_rows,
    ),
    "",
    "Current built-in totals:",
    "",
    md_table(
        ["Scope", "Cases", "Tokens saved", "Savings ratio"],
        [
            ["Default compress benchmarks", str(compress_case_count), str(compress_tokens_saved), compress_ratio],
            ["Built-in rollout/task traces", str(task_case_count), str(task_tokens_saved), task_ratio],
        ],
    ),
    "",
    "Per-profile compression totals:",
    "",
    md_table(
        ["Profile", "Cases", "Tokens saved", "Savings ratio"],
        profile_total_rows,
    ),
    "",
    "Claude-oriented stable synthetic summary:",
    "",
    md_table(
        ["Path", "Raw tokens", "Rewritten tokens", "Tokens saved", "Savings", "Fragments kept"],
        comparison_totals_rows,
    ),
    "",
    "Task-mode comparison for Claude-oriented stable synthetic traces:",
    "",
    md_table(
        ["Scenario", "TKE task savings", "RTK hook task savings", "TKE fragments kept", "RTK hook fragments kept"],
        comparison_task_rows,
    ),
    "",
    "Scenario deltas and practical verdicts:",
    "",
    "Positive deltas mean `rtk-hook` saved more than `tke`. `near-tie` means both paths kept all required fragments and the gap stayed within a small practical band (`<=250` tokens or `<=2%` of scenario savings, and `<=0.5 pp` ratio gap).",
    "",
    md_table(
        ["Scenario", "Token delta (RTK-TKE)", "Ratio delta (RTK-TKE)", "Fragment status", "Practical verdict"],
        comparison_verdict_rows,
    ),
    "",
    "## Structured Summary Coverage",
    "",
    "The current local comparison is broader than raw token totals alone. `tke` now emits repo-local structured summaries for several high-volume profiles that RTK does not expose as equivalent local envelope fields in this repo:",
    "",
    md_table(
        ["Profile", "Current `tke` structure", "Current RTK position in this repo"],
        [
            ["`pathlist`", "`pl.d` shared dir, compact `pl.f`/`pl.l`, examples", "No equivalent repo-local structured summary"],
            ["`search`", "Grouped file chunks with full first hit and compact followups", "No equivalent repo-local structured summary"],
            ["`log`", "`lg.fail`, `lg.warn`, `lg.first_fail`, `lg.first_warn`", "No equivalent repo-local structured summary"],
            ["`diff`", "`df.f[].p/add/del` per-file summaries", "No equivalent repo-local structured summary"],
        ],
    ),
]

if codex:
    benchmarks_md.extend(
        [
            "",
            "## Codex Real E2E",
            "",
            "Generated from:",
            "",
            "```bash",
            "./target/release/tke compare-e2e --agent codex \\",
            "  --source .tmp-codex-e2e \\",
            "  --source .tmp-codex-e2e-real \\",
            "  --source .tmp-codex-e2e-fair",
            "```",
            "",
            md_table(
                ["Case", "Variant", "Correct", "Tool token savings", "Verdict"],
                codex_tke_rows or [["(no tke cases found)", "-", "-", "-", "-"]],
            ),
            "",
            "Codex aggregate by mode:",
            "",
            md_table(
                ["Variant", "Cases", "Pass", "Fail", "Gateway", "Ungraded", "Total tool tokens saved"],
                codex_summary_rows or [["-", "-", "-", "-", "-", "-", "-"]],
            ),
        ]
    )

if codex_rtk_rows or fair_compare_rows:
    benchmarks_md.extend(
        [
            "",
            "## RTK Fair Comparison",
            "",
            "RTK must be compared through each agent's real integration path:",
            "",
            "- Codex: `rtk-codex-rules`",
            "- Claude: `rtk-hook`",
            "",
            md_table(
                ["Agent", "Case", "Raw", "RTK path", "Raw tool tokens", "RTK tool tokens", "Tool token delta", "Verdict"],
                fair_compare_rows or [["-", "-", "-", "-", "-", "-", "-", "-"]],
            ),
            "",
            "Accuracy and compression scorecard:",
            "",
            md_table(
                ["Scope", "Path", "Cases", "Accuracy", "Compression rate", "Semantic retention", "Token outcome"],
                rtk_scorecard_rows or [["-", "-", "-", "-", "-", "-"]],
            ),
        ]
    )
    if fair_compare_summary_rows:
        benchmarks_md.extend(
            [
                "",
                "Fair-path aggregate by agent:",
                "",
                md_table(
                    ["Agent", "Variant", "Cases", "Pass", "Fail", "Gateway", "Ungraded", "Total tool token delta"],
                    fair_compare_summary_rows,
                ),
                "",
                "Codex RTK variant rows:",
                "",
                md_table(
                    ["Case", "Variant", "Correct", "Tool token savings", "Verdict"],
                    codex_rtk_rows,
                ),
            ]
        )

if claude:
    benchmarks_md.extend(
        [
            "",
            "## Claude Real E2E",
            "",
            "Generated from:",
            "",
            "```bash",
            "./target/release/tke compare-e2e --agent claude \\",
            "  --source .tmp-claude-e2e \\",
            "  --source .tmp-claude-e2e-fair",
            "```",
            "",
            md_table(
                ["Case", "Variant", "Correct", "Tool token savings", "Verdict"],
                claude_rows or [["(no claude cases found)", "-", "-", "-", "-"]],
            ),
            "",
            "Compatibility notes:",
            "",
            "- `Claude + tke` currently defaults to compatibility mode in live CLI usage. This keeps agent and tool I/O transparent unless `TKE_CLAUDE_LIVE_TOOLS=1` is set.",
            "- The offline transcript rewriter and compare reports still measure potential savings on saved Claude stream JSONL output.",
            "- `gateway_error` means the gateway returned a transient upstream failure such as Cloudflare 504; treat those samples as infrastructure noise rather than a correctness verdict on the harness itself.",
            "",
            "Claude aggregate by mode:",
            "",
            md_table(
                ["Variant", "Cases", "Pass", "Fail", "Gateway", "Ungraded", "Total tool tokens saved"],
                claude_summary_rows or [["-", "-", "-", "-", "-", "-", "-"]],
            ),
        ]
    )

if claude_attempts:
    live_attempt_rows = []
    live_probe_rows = []
    live_ok_names = []
    live_not_ok_names = []
    fair_attempt_rows = []
    for item in claude_attempts:
        statuses = ",".join(str(status) for status in item.get("error_statuses", [])) or "-"
        name = item.get("name", "-")
        if name.startswith("live"):
            live_probe_rows.append(
                [
                    f"`{normalize_case_name(name)}`",
                    f"`{name}`",
                    "yes" if item.get("ok") else "no",
                    "yes" if item.get("completed") else "no",
                    statuses,
                ]
            )
            if item.get("ok"):
                live_ok_names.append(name)
            else:
                live_not_ok_names.append(name)
            live_attempt_rows.append(
                [
                    f"`{name}`",
                    f"`{item.get('mode', '-')}`",
                    "yes" if item.get("ok") else "no",
                    "yes" if item.get("completed") else "no",
                    "yes" if item.get("result_is_error") else "no",
                    statuses,
                ]
            )
        elif name.startswith("compat"):
            if item.get("ok"):
                live_ok_names.append(name)
            else:
                live_not_ok_names.append(name)
            live_attempt_rows.append(
                [
                    f"`{name}`",
                    f"`{item.get('mode', '-')}`",
                    "yes" if item.get("ok") else "no",
                    "yes" if item.get("completed") else "no",
                    "yes" if item.get("result_is_error") else "no",
                    statuses,
                ]
            )
        elif name.startswith("fair"):
            fair_attempt_rows.append(
                [
                    f"`{normalize_case_name(name)}`",
                    f"`{item.get('mode', '-')}`",
                    "yes" if item.get("ok") else "no",
                    "yes" if item.get("completed") else "no",
                    "yes" if item.get("result_is_error") else "no",
                    statuses,
                ]
            )
        else:
            live_attempt_rows.append(
                [
                    f"`{name}`",
                    f"`{item.get('mode', '-')}`",
                    "yes" if item.get("ok") else "no",
                    "yes" if item.get("completed") else "no",
                    "yes" if item.get("result_is_error") else "no",
                    statuses,
                ]
            )
    benchmarks_md.extend(
        [
            "",
            "## Claude Live Probes",
            "",
            "These runs exercise the live `tke` Claude path directly and are tracked separately from the formal raw-vs-variant compare table so transient gateway failures do not overwrite the last known-good live result.",
            "",
            md_table(
                ["Case", "Run name", "OK", "Completed", "Error statuses"],
                live_probe_rows or [["-", "-", "-", "-", "-"]],
            ),
            "",
            "Claude attempt summary:",
            "",
            md_table(
                ["Case", "Mode", "OK", "Completed", "Result error", "Error statuses"],
                live_attempt_rows or [["-", "-", "-", "-", "-", "-"]],
            ),
        ]
    )
    if fair_attempt_rows:
        benchmarks_md.extend(
            [
                "",
                "Claude fair-attempt summary:",
                "",
                md_table(
                    ["Case", "Mode", "OK", "Completed", "Result error", "Error statuses"],
                    fair_attempt_rows,
                ),
            ]
        )
    if live_ok_names or live_not_ok_names:
        notes = []
        if live_ok_names:
            notes.append(
                "Successful live compatibility probes: "
                + ", ".join(f"`{name}`" for name in sorted(live_ok_names))
                + "."
            )
        if live_not_ok_names:
            notes.append(
                "Still unstable or incomplete: "
                + ", ".join(f"`{name}`" for name in sorted(live_not_ok_names))
                + "."
            )
        benchmarks_md.extend(["", *notes])

(root / "docs/benchmarks.md").write_text("\n".join(benchmarks_md) + "\n")


def build_status_map(report):
    status = {}
    if not report:
        return status
    for case in report["cases"]:
        entry = status.setdefault(normalize_case_name(case["name"]), {"raw": "missing"})
        entry["raw"] = case["baseline"]["correctness"]["status"]
        for variant in case["variants"]:
            entry[variant["mode"]] = variant["sample"]["correctness"]["status"]
    return status


codex_status = build_status_map(codex)
claude_status = build_status_map(claude)


def pass_label(status):
    if status == "gateway_error":
        return "gateway_error"
    if status == "pass":
        return "pass"
    if status == "fail":
        return "fail"
    if status == "ungraded":
        return "ungraded"
    return "missing"


codex_rows = []
for name in sorted(codex_status):
    item = codex_status[name]
    notes = []
    if item.get("tke") == "pass":
        notes.append("stable tke case")
    if item.get("rtk-codex-rules") == "ungraded":
        notes.append("fair RTK sample")
    codex_rows.append(
        [
            f"`{name}`",
            pass_label(item.get("raw")),
            pass_label(item.get("tke")),
            pass_label(item.get("rtk-codex-rules")),
            ", ".join(notes) or "-",
        ]
    )

claude_rows_e2e = []
for name in sorted(claude_status):
    item = claude_status[name]
    notes = []
    if item.get("tke") == "pass":
        notes.append("live tke correct")
    elif item.get("tke") == "fail":
        notes.append("experimental live tke path")
    elif item.get("tke") == "gateway_error":
        notes.append("gateway noise on raw compare path")
    if item.get("rtk-hook") == "pass":
        notes.append("fair RTK hook path")
    elif item.get("rtk-hook") == "gateway_error":
        notes.append("gateway noise on RTK hook path")
    claude_rows_e2e.append(
        [
            f"`{name}`",
            pass_label(item.get("raw")),
            pass_label(item.get("tke")),
            pass_label(item.get("rtk-hook")),
            ", ".join(notes) or "-",
        ]
    )

e2e_md = [
    "# Real E2E Matrix",
    "",
    "This file is generated from the current local E2E artifacts.",
    "",
    "## Stable Cases",
    "",
    "### Codex",
    "",
    md_table(["Case", "Raw", "TKE", "RTK Rules", "Notes"], codex_rows or [["(no codex cases found)", "-", "-", "-", "-"]]),
    "",
    "### Claude",
    "",
    md_table(["Case", "Raw", "TKE", "RTK Hook", "Notes"], claude_rows_e2e or [["(no claude cases found)", "-", "-", "-", "-"]]),
    "",
    "## Fairness Rules",
    "",
    "- Codex vs RTK must use `rtk-codex-rules`.",
    "- Claude vs RTK must use `rtk-hook`.",
    "- `rtk-direct` is not the official fairness path for Codex.",
    "",
    "## Current Repo Verdict",
    "",
    "- Codex remains the primary validated live-compression path.",
    "- Claude currently prioritizes stable compatibility over live compression by default.",
    "- RTK results must be reported per agent integration mode, not as one universal number.",
]

(root / "docs/e2e.md").write_text("\n".join(e2e_md) + "\n")
PY
