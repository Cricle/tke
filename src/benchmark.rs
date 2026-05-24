use crate::adapter::rewrite_agent_transcript;
use crate::app::{AppError, Config};
use crate::rollout_stats::RolloutOutputStats;
use crate::trim::{BenchmarkExpectation, BenchmarkSpec, BenchmarkTaskSpec, BenchmarkTaskStep};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) use crate::rollout_stats::{collect_rollout_output_stats, rollout_string_haystack};

#[derive(Serialize)]
pub(crate) struct RolloutCompareReport {
    pub(crate) v: u8,
    pub(crate) source: String,
    pub(crate) changed: bool,
    pub(crate) raw_fields: usize,
    pub(crate) raw_bytes: usize,
    pub(crate) raw_tokens: usize,
    pub(crate) rewritten_fields: usize,
    pub(crate) rewritten_bytes: usize,
    pub(crate) rewritten_tokens: usize,
    pub(crate) bytes_saved: isize,
    pub(crate) tokens_saved: isize,
    pub(crate) bytes_saved_ratio: f64,
    pub(crate) tokens_saved_ratio: f64,
}

impl RolloutCompareReport {
    pub(crate) fn from_stats(
        source: &Path,
        changed: bool,
        raw: RolloutOutputStats,
        rewritten: RolloutOutputStats,
    ) -> Self {
        let bytes_saved = raw.bytes as isize - rewritten.bytes as isize;
        let tokens_saved = raw.approx_tokens as isize - rewritten.approx_tokens as isize;
        let bytes_saved_ratio = ratio(bytes_saved, raw.bytes);
        let tokens_saved_ratio = ratio(tokens_saved, raw.approx_tokens);
        Self {
            v: 1,
            source: source.display().to_string(),
            changed,
            raw_fields: raw.fields,
            raw_bytes: raw.bytes,
            raw_tokens: raw.approx_tokens,
            rewritten_fields: rewritten.fields,
            rewritten_bytes: rewritten.bytes,
            rewritten_tokens: rewritten.approx_tokens,
            bytes_saved,
            tokens_saved,
            bytes_saved_ratio,
            tokens_saved_ratio,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct BenchmarkReport {
    pub(crate) v: u8,
    pub(crate) cases: Vec<BenchmarkCaseReport>,
    pub(crate) tasks: Vec<BenchmarkTaskReport>,
    pub(crate) corpus: Vec<CorpusCaseReport>,
}

#[derive(Serialize)]
pub(crate) struct BenchmarkCaseReport {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) profile: String,
    pub(crate) expected: String,
    pub(crate) changed: bool,
    pub(crate) raw_bytes: usize,
    pub(crate) raw_tokens: usize,
    pub(crate) rewritten_bytes: usize,
    pub(crate) rewritten_tokens: usize,
    pub(crate) bytes_saved: isize,
    pub(crate) tokens_saved: isize,
    pub(crate) bytes_saved_ratio: f64,
    pub(crate) tokens_saved_ratio: f64,
}

#[derive(Serialize)]
pub(crate) struct BenchmarkTaskReport {
    pub(crate) name: String,
    pub(crate) mode: String,
    pub(crate) objective: String,
    pub(crate) changed: bool,
    pub(crate) raw_bytes: usize,
    pub(crate) raw_tokens: usize,
    pub(crate) rewritten_bytes: usize,
    pub(crate) rewritten_tokens: usize,
    pub(crate) bytes_saved: isize,
    pub(crate) tokens_saved: isize,
    pub(crate) bytes_saved_ratio: f64,
    pub(crate) tokens_saved_ratio: f64,
    pub(crate) required_fragments: Vec<String>,
    pub(crate) preserved_fragments: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct CorpusCaseReport {
    source: String,
    changed: bool,
    raw_bytes: usize,
    raw_tokens: usize,
    rewritten_bytes: usize,
    rewritten_tokens: usize,
    bytes_saved: isize,
    tokens_saved: isize,
    bytes_saved_ratio: f64,
    tokens_saved_ratio: f64,
}

fn ratio(saved: isize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        saved as f64 / total as f64
    }
}

pub(crate) fn build_benchmark_report(config: &Config) -> Result<BenchmarkReport, AppError> {
    let cases = benchmark_specs()
        .into_iter()
        .map(|spec| benchmark_case_report(spec, config))
        .collect::<Result<Vec<_>, _>>()?;
    let tasks = benchmark_task_specs()
        .into_iter()
        .map(|spec| benchmark_task_report(spec, config))
        .collect::<Result<Vec<_>, _>>()?;
    let corpus = benchmark_corpus_reports(config)?;
    Ok(BenchmarkReport {
        v: 1,
        cases,
        tasks,
        corpus,
    })
}

fn benchmark_case_report(
    spec: BenchmarkSpec,
    config: &Config,
) -> Result<BenchmarkCaseReport, AppError> {
    let payload = spec.sample;
    let jsonl = build_exec_rollout(&spec.command, &payload, &spec.call_id);
    let rewritten = rewrite_agent_transcript(&jsonl, config)?;
    let raw_stats = collect_rollout_output_stats(&jsonl, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&jsonl);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    let report = RolloutCompareReport::from_stats(
        Path::new(&format!("/tmp/{}.jsonl", spec.name)),
        rewritten.is_some(),
        raw_stats,
        rewritten_stats,
    );
    Ok(BenchmarkCaseReport {
        name: spec.name,
        command: spec.command,
        profile: spec.profile,
        expected: spec.expected.as_str().to_owned(),
        changed: report.changed,
        raw_bytes: report.raw_bytes,
        raw_tokens: report.raw_tokens,
        rewritten_bytes: report.rewritten_bytes,
        rewritten_tokens: report.rewritten_tokens,
        bytes_saved: report.bytes_saved,
        tokens_saved: report.tokens_saved,
        bytes_saved_ratio: report.bytes_saved_ratio,
        tokens_saved_ratio: report.tokens_saved_ratio,
    })
}

fn benchmark_task_report(
    spec: BenchmarkTaskSpec,
    config: &Config,
) -> Result<BenchmarkTaskReport, AppError> {
    let rewritten = rewrite_agent_transcript(&spec.rollout, config)?;
    let raw_stats = collect_rollout_output_stats(&spec.rollout, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&spec.rollout);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    let report = RolloutCompareReport::from_stats(
        Path::new(&format!("/tmp/{}.jsonl", spec.name)),
        rewritten.is_some(),
        raw_stats,
        rewritten_stats,
    );
    let haystack = rollout_string_haystack(rewritten_text);
    let preserved_fragments = spec
        .required_fragments
        .iter()
        .filter(|fragment| haystack.contains(fragment.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    Ok(BenchmarkTaskReport {
        name: spec.name,
        mode: spec.mode,
        objective: spec.objective,
        changed: report.changed,
        raw_bytes: report.raw_bytes,
        raw_tokens: report.raw_tokens,
        rewritten_bytes: report.rewritten_bytes,
        rewritten_tokens: report.rewritten_tokens,
        bytes_saved: report.bytes_saved,
        tokens_saved: report.tokens_saved,
        bytes_saved_ratio: report.bytes_saved_ratio,
        tokens_saved_ratio: report.tokens_saved_ratio,
        required_fragments: spec.required_fragments,
        preserved_fragments,
    })
}

fn benchmark_corpus_reports(config: &Config) -> Result<Vec<CorpusCaseReport>, AppError> {
    let mut reports = Vec::new();
    for source in discover_benchmark_corpus_paths()? {
        if !source.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&source)?;
        let rewritten = rewrite_agent_transcript(&raw, config)?;
        let raw_stats = collect_rollout_output_stats(&raw, config);
        let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
        let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
        let report = RolloutCompareReport::from_stats(
            &source,
            rewritten.is_some(),
            raw_stats,
            rewritten_stats,
        );
        reports.push(CorpusCaseReport {
            source: source.display().to_string(),
            changed: report.changed,
            raw_bytes: report.raw_bytes,
            raw_tokens: report.raw_tokens,
            rewritten_bytes: report.rewritten_bytes,
            rewritten_tokens: report.rewritten_tokens,
            bytes_saved: report.bytes_saved,
            tokens_saved: report.tokens_saved,
            bytes_saved_ratio: report.bytes_saved_ratio,
            tokens_saved_ratio: report.tokens_saved_ratio,
        });
    }
    Ok(reports)
}

fn discover_benchmark_corpus_paths() -> Result<Vec<PathBuf>, AppError> {
    let mut out = Vec::new();
    for candidate in [
        PathBuf::from(".tmp-benchmark-rollout.jsonl"),
        PathBuf::from(".tmp-real-large-rollout.jsonl"),
    ] {
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    let cwd = env::current_dir()?;
    let interactive = cwd.join(".tke").join("interactive");
    if interactive.is_dir() {
        for entry in fs::read_dir(interactive)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub(crate) fn benchmark_report_check(report: &BenchmarkReport) -> Result<(), AppError> {
    for case in &report.cases {
        match case.expected.as_str() {
            "compress" => {
                if case.tokens_saved_ratio < 0.20 {
                    return Err(AppError::Usage(format!(
                        "benchmark check failed: case `{}` saved too little token ratio ({:.3})",
                        case.name, case.tokens_saved_ratio
                    )));
                }
            }
            "pass_through" => {
                if case.changed || case.tokens_saved != 0 {
                    return Err(AppError::Usage(format!(
                        "benchmark check failed: case `{}` should pass through unchanged",
                        case.name
                    )));
                }
            }
            _ => {}
        }
    }
    for task in &report.tasks {
        if task.tokens_saved_ratio < 0.20 {
            return Err(AppError::Usage(format!(
                "benchmark check failed: codex task `{}` saved too little token ratio ({:.3})",
                task.name, task.tokens_saved_ratio
            )));
        }
        if task.required_fragments.len() != task.preserved_fragments.len() {
            return Err(AppError::Usage(format!(
                "benchmark check failed: codex task `{}` lost required result fragments",
                task.name
            )));
        }
    }
    for case in &report.corpus {
        if case.changed && case.tokens_saved_ratio < 0.10 {
            return Err(AppError::Usage(format!(
                "benchmark check failed: corpus `{}` rewrote with too little gain ({:.3})",
                case.source, case.tokens_saved_ratio
            )));
        }
    }
    Ok(())
}

pub(crate) fn build_exec_rollout(command: &str, output: &str, call_id: &str) -> String {
    [
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "exec_command",
                "arguments": serde_json::json!({
                    "cmd": command,
                    "yield_time_ms": 1000
                }).to_string(),
                "call_id": call_id
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": call_id,
                "output": format!(
                    "Chunk ID: bench\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 1000\nOutput:\n{output}\n"
                )
            }
        })
        .to_string(),
    ]
    .join("\n")
}

fn build_exec_rollout_steps(steps: &[BenchmarkTaskStep], final_answer: &str) -> String {
    let mut lines = Vec::new();
    for step in steps {
        lines.push(
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": serde_json::json!({
                        "cmd": step.command,
                        "yield_time_ms": 1000
                    }).to_string(),
                    "call_id": step.call_id
                }
            })
            .to_string(),
        );
        lines.push(
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": step.call_id,
                    "output": format!(
                        "Chunk ID: task\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 1000\nOutput:\n{}\n",
                        step.output
                    )
                }
            })
            .to_string(),
        );
    }
    lines.push(
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": final_answer
                    }
                ]
            }
        })
        .to_string(),
    );
    lines.join("\n")
}

fn build_command_execution_rollout_steps(
    steps: &[BenchmarkTaskStep],
    final_answer: &str,
) -> String {
    let mut lines = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        lines.push(
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "id": format!("item_{}", idx + 1),
                    "type": "command_execution",
                    "command": step.command,
                    "aggregated_output": format!("{}\n", step.output),
                    "exit_code": 0,
                    "status": "completed"
                }
            })
            .to_string(),
        );
    }
    lines.push(
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": final_answer
                    }
                ]
            }
        })
        .to_string(),
    );
    lines.join("\n")
}

fn build_claude_tool_rollout_steps(steps: &[BenchmarkTaskStep], final_answer: &str) -> String {
    let mut lines = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        let tool_id = format!("toolu_{}", idx + 1);
        lines.push(
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": tool_id,
                            "name": "Bash",
                            "input": {
                                "command": step.command
                            }
                        }
                    ]
                }
            })
            .to_string(),
        );
        lines.push(
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": tool_id,
                            "content": [
                                {
                                    "type": "text",
                                    "text": format!("{}\n", step.output)
                                }
                            ]
                        }
                    ]
                }
            })
            .to_string(),
        );
    }
    lines.push(
        serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "text",
                        "text": final_answer
                    }
                ]
            }
        })
        .to_string(),
    );
    lines.join("\n")
}

pub(crate) fn benchmark_specs() -> Vec<BenchmarkSpec> {
    vec![
        BenchmarkSpec {
            name: "cat_code".to_owned(),
            command: "cat src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cat_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "sed_code".to_owned(),
            command: "sed -n 1,180p src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_sed_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "bat_code".to_owned(),
            command: "bat --style=plain src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_bat_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "nl_code".to_owned(),
            command: "nl -ba src/lib.rs | sed -n 1,180p".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_nl_1".to_owned(),
            sample: repeated_benchmark_numbered_code(180),
        },
        BenchmarkSpec {
            name: "awk_code".to_owned(),
            command: "awk '{print}' src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_awk_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "cut_code".to_owned(),
            command: "cut -c1-120 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cut_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "head_code".to_owned(),
            command: "head -n 180 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_head_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "tail_code".to_owned(),
            command: "tail -n 180 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_tail_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "rg_code".to_owned(),
            command: "rg -n 'fn|struct|impl|enum' src".to_owned(),
            profile: "search".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_rg_1".to_owned(),
            sample: repeated_benchmark_search(),
        },
        BenchmarkSpec {
            name: "grep_code".to_owned(),
            command: "grep -n 'fn\\|struct\\|impl\\|enum' src/lib.rs".to_owned(),
            profile: "search".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_grep_1".to_owned(),
            sample: repeated_benchmark_search(),
        },
        BenchmarkSpec {
            name: "find_paths".to_owned(),
            command: "find /root/project -type f | head -n 500".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_find_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "fd_paths".to_owned(),
            command: "fd . /root/project | head -n 500".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_fd_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "tree_paths".to_owned(),
            command: "tree -a -L 6 /root/project".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_tree_1".to_owned(),
            sample: repeated_benchmark_paths(240),
        },
        BenchmarkSpec {
            name: "sort_paths".to_owned(),
            command: "find /root/project -type f | sort".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_sort_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "uniq_paths".to_owned(),
            command: "find /root/project -type f | uniq".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_uniq_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "ls_long".to_owned(),
            command: "ls -l /root/project/src".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ls_1".to_owned(),
            sample: repeated_benchmark_ls_long(),
        },
        BenchmarkSpec {
            name: "ls_names".to_owned(),
            command: "ls /root/project/src".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ls_names_1".to_owned(),
            sample: repeated_benchmark_ls_names(120),
        },
        BenchmarkSpec {
            name: "wc_summary".to_owned(),
            command: "wc -l src/lib.rs README.md".to_owned(),
            profile: "generic".to_owned(),
            expected: BenchmarkExpectation::PassThrough,
            call_id: "bench_wc_1".to_owned(),
            sample: repeated_benchmark_wc(),
        },
        BenchmarkSpec {
            name: "git_diff".to_owned(),
            command: "git diff -- src/lib.rs".to_owned(),
            profile: "diff".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_git_1".to_owned(),
            sample: repeated_benchmark_diff(),
        },
        BenchmarkSpec {
            name: "cargo_build".to_owned(),
            command: "cargo build".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cargo_1".to_owned(),
            sample: repeated_benchmark_build_log("cargo"),
        },
        BenchmarkSpec {
            name: "pytest_run".to_owned(),
            command: "pytest -q".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_pytest_1".to_owned(),
            sample: repeated_benchmark_build_log("pytest"),
        },
        BenchmarkSpec {
            name: "npm_test".to_owned(),
            command: "npm test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_npm_1".to_owned(),
            sample: repeated_benchmark_build_log("npm"),
        },
        BenchmarkSpec {
            name: "pnpm_test".to_owned(),
            command: "pnpm test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_pnpm_1".to_owned(),
            sample: repeated_benchmark_build_log("pnpm"),
        },
        BenchmarkSpec {
            name: "yarn_test".to_owned(),
            command: "yarn test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_yarn_1".to_owned(),
            sample: repeated_benchmark_build_log("yarn"),
        },
        BenchmarkSpec {
            name: "dotnet_test".to_owned(),
            command: "dotnet test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_dotnet_1".to_owned(),
            sample: repeated_benchmark_build_log("dotnet"),
        },
        BenchmarkSpec {
            name: "go_test".to_owned(),
            command: "go test ./...".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_go_1".to_owned(),
            sample: repeated_benchmark_build_log("go"),
        },
        BenchmarkSpec {
            name: "cmake_build".to_owned(),
            command: "cmake --build build".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cmake_1".to_owned(),
            sample: repeated_benchmark_build_log("cmake"),
        },
        BenchmarkSpec {
            name: "ctest_run".to_owned(),
            command: "ctest --test-dir build --output-on-failure".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ctest_1".to_owned(),
            sample: repeated_benchmark_build_log("ctest"),
        },
        BenchmarkSpec {
            name: "make_build".to_owned(),
            command: "make -j8 test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_make_1".to_owned(),
            sample: repeated_benchmark_build_log("make"),
        },
        BenchmarkSpec {
            name: "ninja_build".to_owned(),
            command: "ninja -C build test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ninja_1".to_owned(),
            sample: repeated_benchmark_build_log("ninja"),
        },
        BenchmarkSpec {
            name: "node_test".to_owned(),
            command: "node --test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_node_1".to_owned(),
            sample: repeated_benchmark_build_log("node"),
        },
        BenchmarkSpec {
            name: "ps_table".to_owned(),
            command: "ps aux --sort=-%cpu | head -n 25".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ps_1".to_owned(),
            sample: repeated_benchmark_ps(),
        },
        BenchmarkSpec {
            name: "systemctl_table".to_owned(),
            command: "systemctl list-units --type=service --all --no-pager | head -n 40".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_systemctl_1".to_owned(),
            sample: repeated_benchmark_systemctl(),
        },
        BenchmarkSpec {
            name: "xargs_cat".to_owned(),
            command: "printf '/root/project/src/lib.rs\\n' | xargs cat".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_xargs_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
    ]
}

pub(crate) fn benchmark_task_specs() -> Vec<BenchmarkTaskSpec> {
    let savings_answer = "RolloutCompareReport::from_stats computes tokens_saved as raw.approx_tokens - rewritten.approx_tokens and tokens_saved_ratio via ratio(tokens_saved, raw.approx_tokens).";
    let coverage_answer = "DEFAULT_TOOL_COMMANDS covers cat, sed, rg, grep, git, cargo, pytest, npm, pnpm, yarn, tail, head, ls, find, fd, bat, nl, awk, cut, sort, uniq, wc, tree, and xargs.";
    let pipeline_answer = "The selected stage is rg, so the normalized payload preserves sc=rg and sr=search instead of keeping the upstream cat or downstream head stage.";
    let find_pipeline_answer = "The selected stage is find, so the normalized payload preserves sc=find and sr=search and keeps the path list summary instead of the tail stage semantics.";
    let build_pipeline_answer = "The selected stage is cargo, so the normalized payload preserves sc=cargo and sr=build and keeps the log/error summary instead of the tail stage semantics.";
    let diff_pipeline_answer = "The selected stage is git diff, so the normalized payload preserves sc=git and p=diff and keeps the diff file summary instead of raw hunks.";
    let complex_triage_answer = "The combined triage trace preserves find pathlist, rg search, git diff summary, and cargo log summary across four Bash tool invocations.";
    let complex_code_trace_answer = "The combined code-trace preserves file summary, search summary, git diff summary, and cargo log summary across four Bash tool invocations.";
    let complex_stacktrace_answer = "The combined stacktrace triage preserves python traceback summary, rg search summary, and cargo log summary across three Bash tool invocations.";
    let complex_stacktrace_diff_answer = "The combined debug trace preserves python traceback summary, git diff summary, rg search summary, and cargo log summary across four Bash tool invocations.";
    let rtk_hook_find_answer = "The RTK hook sample preserves the same find pathlist-stage metadata and semantic answer fragments for Claude-style hook integrations.";
    let rtk_hook_search_answer = "The RTK hook sample preserves the same rg search-stage metadata and semantic answer fragments for Claude-style hook integrations.";
    let rtk_hook_diff_answer = "The RTK hook sample preserves the same git diff-stage metadata and semantic answer fragments for Claude-style hook integrations.";
    let rtk_hook_build_answer = "The RTK hook sample preserves the same cargo build-stage metadata and semantic answer fragments for Claude-style hook integrations.";
    let rtk_hook_complex_triage_answer = "The RTK hook triage trace preserves find pathlist, rg search, git diff summary, and cargo log summary across four Bash tool invocations.";
    let rtk_hook_complex_code_trace_answer = "The RTK hook code-trace preserves file summary, search summary, git diff summary, and cargo log summary across four Bash tool invocations.";
    let rtk_hook_complex_stacktrace_answer = "The RTK hook stacktrace triage preserves python traceback summary, rg search summary, and cargo log summary across three Bash tool invocations.";
    let rtk_hook_complex_stacktrace_diff_answer = "The RTK hook debug trace preserves python traceback summary, git diff summary, rg search summary, and cargo log summary across four Bash tool invocations.";
    vec![
        BenchmarkTaskSpec {
            name: "codex_api_trace_rollout_savings".to_owned(),
            mode: "api".to_owned(),
            objective: "Find where rollout token savings are computed and summarize the exact formula.".to_owned(),
            required_fragments: vec![
                "RolloutCompareReport::from_stats".to_owned(),
                "raw.approx_tokens - rewritten.approx_tokens".to_owned(),
                "ratio(tokens_saved, raw.approx_tokens)".to_owned(),
                savings_answer.to_owned(),
            ],
            rollout: build_exec_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "task_savings_1".to_owned(),
                        command: "rg -n \"RolloutCompareReport|tokens_saved|tokens_saved_ratio\" src/lib.rs".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "2346:struct RolloutCompareReport {",
                                "2364:        let tokens_saved = raw.approx_tokens as isize - rewritten.approx_tokens as isize;",
                                "2366:        let tokens_saved_ratio = ratio(tokens_saved, raw.approx_tokens);",
                            ],
                            180,
                            "src/lib.rs",
                            "tokens_saved_helper",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "task_savings_2".to_owned(),
                        command: "sed -n '2358,2376p' src/lib.rs".to_owned(),
                        output: repeated_task_code_block(
                            &[
                                "impl RolloutCompareReport {",
                                "    fn from_stats(",
                                "        source: &Path,",
                                "        changed: bool,",
                                "        raw: RolloutOutputStats,",
                                "        rewritten: RolloutOutputStats,",
                                "    ) -> Self {",
                                "        let bytes_saved = raw.bytes as isize - rewritten.bytes as isize;",
                                "        let tokens_saved = raw.approx_tokens as isize - rewritten.approx_tokens as isize;",
                                "        let bytes_saved_ratio = ratio(bytes_saved, raw.bytes);",
                                "        let tokens_saved_ratio = ratio(tokens_saved, raw.approx_tokens);",
                                "        Self {",
                                "            v: 1,",
                                "        }",
                                "    }",
                                "}",
                            ],
                            18,
                        ),
                    },
                ],
                savings_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "codex_api_trace_default_tool_coverage".to_owned(),
            mode: "api".to_owned(),
            objective: "Read the codebase defaults and list the common code-reading tool commands enabled by default.".to_owned(),
            required_fragments: vec![
                "DEFAULT_TOOL_COMMANDS".to_owned(),
                "cat, sed, rg, grep, git, cargo".to_owned(),
                "tree, and xargs".to_owned(),
                coverage_answer.to_owned(),
            ],
            rollout: build_exec_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "task_tools_1".to_owned(),
                        command: "rg -n \"DEFAULT_TOOL_COMMANDS|default_tool_commands_cover_core_agent_workflows\" src/lib.rs".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "12:const DEFAULT_TOOL_COMMANDS: &[&str] = &[",
                                "4518:    fn default_tool_commands_cover_core_agent_workflows() {",
                                "4524:            \"cat\", \"sed\", \"rg\", \"grep\", \"git\", \"cargo\", \"pytest\", \"npm\", \"pnpm\", \"yarn\",",
                            ],
                            160,
                            "src/lib.rs",
                            "tool_command_check",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "task_tools_2".to_owned(),
                        command: "sed -n '10,18p' src/lib.rs".to_owned(),
                        output: repeated_task_code_block(
                            &[
                                "const DEFAULT_TOOL_COMMANDS: &[&str] = &[",
                                "    \"cat\", \"sed\", \"rg\", \"grep\", \"git\", \"cargo\", \"pytest\", \"npm\", \"pnpm\", \"yarn\", \"tail\", \"head\",",
                                "    \"ls\", \"find\", \"fd\", \"bat\", \"nl\", \"awk\", \"cut\", \"sort\", \"uniq\", \"wc\", \"tree\", \"xargs\",",
                                "];",
                            ],
                            24,
                        ),
                    },
                ],
                coverage_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "codex_interactive_trace_selected_search_stage".to_owned(),
            mode: "interactive".to_owned(),
            objective: "Verify which stage is preserved when codex reads a file through a cat | rg | head search pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
            ],
            rollout: build_command_execution_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "task_tty_1".to_owned(),
                        command: "/bin/bash -lc 'cat /tmp/tke-codex/huge.txt | rg -n \"SECTION 599\" | head -n 1'".to_owned(),
                        output: repeated_lines("2397:SECTION 599", 180),
                    },
                    BenchmarkTaskStep {
                        call_id: "task_tty_2".to_owned(),
                        command: "/bin/bash -lc 'sed -n \"4821,4841p\" src/lib.rs'".to_owned(),
                        output: repeated_task_code_block(
                            &[
                                "fn codex_event_replay_preserves_selected_search_stage() {",
                                "    let nested = value_from_json(",
                                "        value[\"item\"][\"aggregated_output\"]",
                                "            .as_str()",
                                "            .expect(\"aggregated_output\")",
                                "            .trim_start_matches(\"__TKE__\"),",
                                "    );",
                                "    assert_eq!(nested[\"sc\"], \"rg\");",
                                "    assert_eq!(nested[\"sr\"], \"search\");",
                                "}",
                            ],
                            20,
                        ),
                    },
                ],
                pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "codex_interactive_trace_selected_find_stage".to_owned(),
            mode: "interactive".to_owned(),
            objective: "Verify which stage is preserved when codex lists files through a find | head pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"find\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"p\":\"pathlist\"".to_owned(),
                find_pipeline_answer.to_owned(),
            ],
            rollout: build_command_execution_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "task_find_1".to_owned(),
                    command: "/bin/bash -lc 'find /root/project -type f | head -n 500'".to_owned(),
                    output: repeated_benchmark_paths(500),
                }],
                find_pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "codex_interactive_trace_selected_build_stage".to_owned(),
            mode: "interactive".to_owned(),
            objective: "Verify which stage is preserved when codex inspects build output through a cargo test | tail pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"cargo\"".to_owned(),
                "\"sr\":\"build\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "error: test failed, to rerun pass --lib".to_owned(),
                build_pipeline_answer.to_owned(),
            ],
            rollout: build_command_execution_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "task_build_1".to_owned(),
                    command: "/bin/bash -lc 'cargo test -- --nocapture | tail -n 80'".to_owned(),
                    output: format!(
                        "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                        repeated_lines("test parser::case ... ok", 120)
                    ),
                }],
                build_pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_selected_search_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify which stage is preserved when Claude reads a file through a cat | rg | head search pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                pipeline_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_task_1".to_owned(),
                        command: "cat /tmp/tke-codex/huge.txt | rg -n \"SECTION 599\" | head -n 1".to_owned(),
                        output: repeated_lines("2397:SECTION 599", 180),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_2".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_selected_find_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify which stage is preserved when Claude lists files through a find | head pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"find\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"p\":\"pathlist\"".to_owned(),
                find_pipeline_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_task_find_1".to_owned(),
                    command: "find /root/project -type f | head -n 500".to_owned(),
                    output: repeated_benchmark_paths(500),
                }],
                find_pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_selected_diff_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify which stage is preserved when Claude inspects git diff output.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"add\":3".to_owned(),
                "\"del\":1".to_owned(),
                diff_pipeline_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_task_diff_1".to_owned(),
                    command: "git diff -- src/lib.rs".to_owned(),
                    output: repeated_benchmark_diff(),
                }],
                diff_pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_selected_build_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify which stage is preserved when Claude inspects build output through a cargo test | tail pipeline.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"cargo\"".to_owned(),
                "\"sr\":\"build\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "error: test failed, to rerun pass --lib".to_owned(),
                build_pipeline_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_task_build_1".to_owned(),
                    command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                    output: format!(
                        "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                        repeated_lines("test parser::case ... ok", 120)
                    ),
                }],
                build_pipeline_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_complex_triage_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that Claude preserves the important summaries across a multi-step triage flow with file discovery, search, diff inspection, and build-log follow-up.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"find\"".to_owned(),
                "\"p\":\"pathlist\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                complex_triage_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_task_triage_find_1".to_owned(),
                        command: "find /root/project -type f | head -n 500".to_owned(),
                        output: repeated_benchmark_paths(500),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_triage_search_1".to_owned(),
                        command: "rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:2538:                \"result\": \"STAGE=rg -n \\\"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\\\" src\\nFILE=src/tests.rs\\nKIND=search\"",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            120,
                            "src/tests.rs",
                            "triage_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_triage_diff_1".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_triage_build_1".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                complex_triage_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_complex_code_trace_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that Claude preserves the important summaries across a multi-step code-trace flow with file read, search, diff inspection, and build-log follow-up.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"sed\"".to_owned(),
                "\"p\":\"file\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                complex_code_trace_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_task_code_trace_file_1".to_owned(),
                        command: "sed -n '1,180p' src/lib.rs".to_owned(),
                        output: repeated_benchmark_code(180),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_code_trace_search_1".to_owned(),
                        command: "rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:2538:                \"result\": \"STAGE=rg -n \\\"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\\\" src\\nFILE=src/tests.rs\\nKIND=search\"",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            120,
                            "src/tests.rs",
                            "code_trace_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_code_trace_diff_1".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_code_trace_build_1".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                complex_code_trace_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_complex_stacktrace_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that Claude preserves traceback structure, search context, and build-log follow-up across a multi-step debugging flow.".to_owned(),
            required_fragments: vec![
                "\"p\":\"stacktrace\"".to_owned(),
                "\"k\":\"summary\"".to_owned(),
                "\"k\":\"frame\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                complex_stacktrace_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_1".to_owned(),
                        command: "python script.py".to_owned(),
                        output: [
                            "Traceback (most recent call last):",
                            "  File \"app.py\", line 10, in <module>",
                            "  File \"svc.py\", line 20, in run",
                            "ValueError: boom",
                        ]
                        .join("\n"),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_2".to_owned(),
                        command: "rg -n \"ValueError|run|compare-e2e\" src tests".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:208:        \"Traceback (most recent call last):\",",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            90,
                            "src/tests.rs",
                            "stacktrace_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_3".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                complex_stacktrace_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_bash_trace_complex_stacktrace_diff_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that Claude preserves traceback structure, diff summary, search context, and build-log follow-up across a deeper debugging flow.".to_owned(),
            required_fragments: vec![
                "\"p\":\"stacktrace\"".to_owned(),
                "\"k\":\"summary\"".to_owned(),
                "\"k\":\"frame\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                complex_stacktrace_diff_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_diff_1".to_owned(),
                        command: "python script.py".to_owned(),
                        output: [
                            "Traceback (most recent call last):",
                            "  File \"app.py\", line 10, in <module>",
                            "  File \"svc.py\", line 20, in run",
                            "ValueError: boom",
                        ]
                        .join("\n"),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_diff_2".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_diff_3".to_owned(),
                        command: "rg -n \"ValueError|run|compare-e2e\" src tests".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:208:        \"Traceback (most recent call last):\",",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            90,
                            "src/tests.rs",
                            "stacktrace_diff_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_task_stacktrace_diff_4".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                complex_stacktrace_diff_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_selected_find_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves find pathlist-stage semantics for Claude-style hook integrations.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"find\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"p\":\"pathlist\"".to_owned(),
                "\"d\":\"/root/project/target/debug/incremental/tke\"".to_owned(),
                "\"f\":\"build-artifact-0000.o\"".to_owned(),
                rtk_hook_find_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_rtk_hook_task_find_1".to_owned(),
                    command: "find /root/project -type f | head -n 500".to_owned(),
                    output: repeated_benchmark_paths(500),
                }],
                rtk_hook_find_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_selected_search_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves rg search-stage semantics for Claude-style hook integrations.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "src/tests.rs".to_owned(),
                rtk_hook_search_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_rtk_hook_task_search_1".to_owned(),
                    command: "rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src".to_owned(),
                    output: repeated_task_search_output(
                        &[
                            "src/tests.rs:2538:                \"result\": \"STAGE=rg -n \\\"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\\\" src\\nFILE=src/tests.rs\\nKIND=search\"",
                            "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                            "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                        ],
                        120,
                        "src/tests.rs",
                        "rtk_hook_search_trace",
                    ),
                }],
                rtk_hook_search_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_selected_diff_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves git diff-stage semantics for Claude-style hook integrations.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"add\":3".to_owned(),
                "\"del\":1".to_owned(),
                rtk_hook_diff_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_rtk_hook_task_diff_1".to_owned(),
                    command: "git diff -- src/lib.rs".to_owned(),
                    output: repeated_benchmark_diff(),
                }],
                rtk_hook_diff_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_selected_build_stage".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves cargo build-stage semantics for Claude-style hook integrations.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"cargo\"".to_owned(),
                "\"sr\":\"build\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "error: test failed, to rerun pass --lib".to_owned(),
                rtk_hook_build_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[BenchmarkTaskStep {
                    call_id: "claude_rtk_hook_task_build_1".to_owned(),
                    command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                    output: format!(
                        "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                        repeated_lines("test parser::case ... ok", 120)
                    ),
                }],
                rtk_hook_build_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_complex_triage_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves the important summaries across a multi-step triage flow with file discovery, search, diff inspection, and build-log follow-up.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"find\"".to_owned(),
                "\"p\":\"pathlist\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                rtk_hook_complex_triage_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_triage_find_1".to_owned(),
                        command: "find /root/project -type f | head -n 500".to_owned(),
                        output: repeated_benchmark_paths(500),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_triage_search_1".to_owned(),
                        command: "rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:2538:                \"result\": \"STAGE=rg -n \\\"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\\\" src\\nFILE=src/tests.rs\\nKIND=search\"",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            120,
                            "src/tests.rs",
                            "triage_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_triage_diff_1".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_triage_build_1".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                rtk_hook_complex_triage_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_complex_code_trace_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves the important summaries across a multi-step code-trace flow with file read, search, diff inspection, and build-log follow-up.".to_owned(),
            required_fragments: vec![
                "\"sc\":\"sed\"".to_owned(),
                "\"p\":\"file\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                rtk_hook_complex_code_trace_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_code_trace_file_1".to_owned(),
                        command: "sed -n '1,180p' src/lib.rs".to_owned(),
                        output: repeated_benchmark_code(180),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_code_trace_search_1".to_owned(),
                        command: "rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:2538:                \"result\": \"STAGE=rg -n \\\"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\\\" src\\nFILE=src/tests.rs\\nKIND=search\"",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            120,
                            "src/tests.rs",
                            "code_trace_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_code_trace_diff_1".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_code_trace_build_1".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                rtk_hook_complex_code_trace_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_complex_stacktrace_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves traceback structure, search context, and build-log follow-up across a multi-step debugging flow.".to_owned(),
            required_fragments: vec![
                "\"p\":\"stacktrace\"".to_owned(),
                "\"k\":\"summary\"".to_owned(),
                "\"k\":\"frame\"".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                rtk_hook_complex_stacktrace_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_1".to_owned(),
                        command: "python script.py".to_owned(),
                        output: [
                            "Traceback (most recent call last):",
                            "  File \"app.py\", line 10, in <module>",
                            "  File \"svc.py\", line 20, in run",
                            "ValueError: boom",
                        ]
                        .join("\n"),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_2".to_owned(),
                        command: "rg -n \"ValueError|run|compare-e2e\" src tests".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:208:        \"Traceback (most recent call last):\",",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            90,
                            "src/tests.rs",
                            "rtk_stacktrace_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_3".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                rtk_hook_complex_stacktrace_answer,
            ),
        },
        BenchmarkTaskSpec {
            name: "claude_rtk_hook_trace_complex_stacktrace_diff_task".to_owned(),
            mode: "api".to_owned(),
            objective: "Verify that the RTK hook path preserves traceback structure, diff summary, search context, and build-log follow-up across a deeper debugging flow.".to_owned(),
            required_fragments: vec![
                "\"p\":\"stacktrace\"".to_owned(),
                "\"k\":\"summary\"".to_owned(),
                "\"k\":\"frame\"".to_owned(),
                "\"sc\":\"git\"".to_owned(),
                "\"p\":\"diff\"".to_owned(),
                "\"df\":".to_owned(),
                "\"sc\":\"rg\"".to_owned(),
                "\"sr\":\"search\"".to_owned(),
                "\"sc\":\"cargo\"".to_owned(),
                "\"p\":\"log\"".to_owned(),
                "\"lg\":".to_owned(),
                rtk_hook_complex_stacktrace_diff_answer.to_owned(),
            ],
            rollout: build_claude_tool_rollout_steps(
                &[
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_diff_1".to_owned(),
                        command: "python script.py".to_owned(),
                        output: [
                            "Traceback (most recent call last):",
                            "  File \"app.py\", line 10, in <module>",
                            "  File \"svc.py\", line 20, in run",
                            "ValueError: boom",
                        ]
                        .join("\n"),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_diff_2".to_owned(),
                        command: "git diff -- src/lib.rs".to_owned(),
                        output: repeated_benchmark_diff(),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_diff_3".to_owned(),
                        command: "rg -n \"ValueError|run|compare-e2e\" src tests".to_owned(),
                        output: repeated_task_search_output(
                            &[
                                "src/tests.rs:208:        \"Traceback (most recent call last):\",",
                                "src/e2e_report.rs:167:    let rewritten = rewrite_agent_transcript(&raw_text, config)?;",
                                "src/app.rs:289:        Some(\"compare-e2e\") => parse_compare_e2e(args),",
                            ],
                            90,
                            "src/tests.rs",
                            "rtk_stacktrace_diff_search_trace",
                        ),
                    },
                    BenchmarkTaskStep {
                        call_id: "claude_rtk_hook_task_stacktrace_diff_4".to_owned(),
                        command: "cargo test -- --nocapture | tail -n 80".to_owned(),
                        output: format!(
                            "{}\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n",
                            repeated_lines("test parser::case ... ok", 120)
                        ),
                    },
                ],
                rtk_hook_complex_stacktrace_diff_answer,
            ),
        },
    ]
}

fn repeated_benchmark_code(lines: usize) -> String {
    (0..lines)
        .map(|idx| match idx % 6 {
            0 => format!("pub struct Struct{idx} {{"),
            1 => format!("    field_{idx}: usize,"),
            2 => "}".to_owned(),
            3 => format!("pub fn function_{idx}() {{"),
            4 => format!("    println!(\"{{}}\", {idx});"),
            _ => "}".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_numbered_code(lines: usize) -> String {
    repeated_benchmark_code(lines)
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("{:>6}\t{line}", idx + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_search() -> String {
    (0..140)
        .map(|idx| format!("src/lib.rs:{}:pub fn alpha_{}() {{}}", idx + 1, idx))
        .chain((0..80).map(|idx| format!("src/main.rs:{}:pub struct Beta{};", idx + 1, idx)))
        .chain((0..40).map(|idx| format!("tests/lib.rs:{}:impl Gamma{} {{}}", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_paths(count: usize) -> String {
    (0..count)
        .map(|idx| format!("/root/project/target/debug/incremental/tke/build-artifact-{idx:04}.o"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_ls_long() -> String {
    let mut rows = vec!["total 160".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "-rw-r--r-- 1 root root {:>5} May 23 17:{:02} module_{idx:02}.rs",
            1024 + idx * 13,
            idx % 60
        ));
    }
    rows.join("\n")
}

fn repeated_benchmark_ls_names(count: usize) -> String {
    (0..count)
        .map(|idx| {
            if idx % 5 == 0 {
                format!("module_{idx:03}")
            } else {
                format!("module_{idx:03}.rs")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_wc() -> String {
    [
        "  3816 115848 src/lib.rs",
        "    82   3580 README.md",
        "  3898 119428 total",
    ]
    .join("\n")
}

fn repeated_benchmark_diff() -> String {
    let mut rows = vec![
        "diff --git a/src/lib.rs b/src/lib.rs".to_owned(),
        "index 1234567..89abcde 100644".to_owned(),
        "--- a/src/lib.rs".to_owned(),
        "+++ b/src/lib.rs".to_owned(),
    ];
    for idx in 0..120 {
        rows.push(format!(
            "@@ -{},3 +{},6 @@ pub fn function_{}() {{",
            idx * 10 + 1,
            idx * 10 + 1,
            idx
        ));
        rows.push("-    old_call();".to_owned());
        rows.push("+    new_call();".to_owned());
        rows.push(format!("+    extra_line_{}();", idx));
        rows.push("+    trace_call();".to_owned());
        rows.push(" }".to_owned());
    }
    rows.join("\n")
}

fn repeated_benchmark_build_log(kind: &str) -> String {
    let mut rows = Vec::new();
    for idx in 0..120 {
        rows.push(format!("{kind}: step {idx:03} finished"));
    }
    rows.push(format!("{kind}: warning: deprecated config key"));
    rows.push(format!("{kind}: error: build failed at target 007"));
    rows.join("\n")
}

fn repeated_benchmark_ps() -> String {
    let mut rows = vec![
        "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND".to_owned(),
    ];
    for idx in 0..24 {
        rows.push(format!(
            "root        {:>4}  {:>3}.{:1}  1.2 357624 101588 pts/1   Sl+  08:08   0:00 /usr/bin/process-{} --flag value",
            3000 + idx,
            9 - (idx / 3),
            idx % 10,
            idx
        ));
    }
    rows.join("\n")
}

fn repeated_benchmark_systemctl() -> String {
    let mut rows =
        vec!["UNIT                         LOAD   ACTIVE SUB     DESCRIPTION".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "service-{idx:02}.service      loaded active running Sample Service {idx:02}"
        ));
    }
    rows.join("\n")
}

pub(crate) fn repeated_lines(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|idx| format!("{prefix} // line {idx}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_task_search_output(
    highlights: &[&str],
    total_lines: usize,
    path: &str,
    symbol: &str,
) -> String {
    let mut rows = highlights
        .iter()
        .map(|line| (*line).to_owned())
        .collect::<Vec<_>>();
    while rows.len() < total_lines {
        let idx = rows.len() + 1;
        rows.push(format!(
            "{path}:{}:fn {symbol}_{idx:03}() {{ let value = {}; }}",
            2400 + idx,
            idx
        ));
    }
    rows.join("\n")
}

fn repeated_task_code_block(lines: &[&str], repeats: usize) -> String {
    let mut rows = Vec::new();
    for _ in 0..repeats {
        rows.extend(lines.iter().map(|line| (*line).to_owned()));
    }
    rows.join("\n")
}

pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    usize::max(1, chars.div_ceil(4))
}
