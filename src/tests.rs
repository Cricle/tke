use crate::adapter::{
    rewrite_agent_transcript, rewrite_claude_jsonl, rewrite_codex_jsonl, rewrite_generic_jsonl,
};
use crate::app::{
    AppError, Config, Dispatch, default_activate_shim_dir, default_runtime_shim_dir,
    default_tool_commands, parse_dispatch,
};
use crate::benchmark::{
    RolloutCompareReport, benchmark_specs, benchmark_task_specs, build_benchmark_report,
};
use crate::e2e_report::build_e2e_compare_report;
use crate::rewrite::{
    LivePipelineDecision, classify_stage_role, extract_exec_command_output, live_pipeline_decision,
    live_pipeline_should_passthrough, looks_like_stderr_only_exec_output, parse_command_execution,
    parse_live_shell_pipeline, rewrite_command_item_fields,
};
use crate::rollout_io::{InteractiveTracker, capture_interactive};
use crate::rollout_stats::{
    collect_rollout_output_stats, collect_rollout_output_stats_detailed,
    rollout_has_relevant_tool_output, rollout_string_haystack,
};
use crate::shim::{maybe_normalize_text, normalize_text, normalize_text_with_stage};
use crate::stats::{
    UsageStatsFilter, UsageStatsGroupBy, UsageStatsSortBy, build_usage_stats_report,
};
use crate::table_profile::looks_like_table;
use crate::trim::{
    CommandKind, ShellKind, candidate_command_names, canonical_command_name, classify_command,
    create_windows_exe_shim, has_log_progress, is_failure_signal_line, is_log_signal,
    is_warning_signal, looks_like_path_list, match_terms, now_millis, read_stream_payload,
    render_activate_script, render_deactivate_script, shim_command_path,
};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn value_from_json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("json")
}

fn temp_test_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tke-{label}-{}", now_millis()))
}

fn set_env_var(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_env_var(key: &str) {
    unsafe {
        std::env::remove_var(key);
    }
}

fn repeated_lines(prefix: &str, count: usize) -> String {
    crate::benchmark_data::repeated_lines(prefix, count)
}

fn write_codex_e2e_sample(path: &Path, tool_output: &str, result: &str) {
    fs::write(
        path,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": tool_output
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": result
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write codex e2e sample");
}

fn write_claude_e2e_sample(path: &Path, tool_output: &str, result: &str) {
    fs::write(
        path,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_tool",
                            "content": tool_output
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": result
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write claude e2e sample");
}

fn write_claude_e2e_gateway_sample(path: &Path, tool_output: &str, result: &str) {
    fs::write(
        path,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_tool",
                            "content": tool_output
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "error",
                "error": {
                    "message": "origin gateway timeout"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": result
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write claude e2e gateway sample");
}

fn e2e_case_status(case: &crate::e2e_report::E2eCaseReport, mode: &str) -> String {
    case.variants
        .iter()
        .find(|variant| variant.mode == mode)
        .map(|variant| variant.sample.correctness.status.clone())
        .unwrap_or_else(|| "missing".to_owned())
}

#[test]
fn whitelist_paths_match_path_boundaries_not_arbitrary_substrings() {
    let mut cfg = Config::default();
    cfg.whitelist_paths = vec!["/tmp/tke-codex".to_owned(), "src/tests.rs".to_owned()];
    cfg.whitelist_extensions.clear();

    assert!(cfg.is_whitelisted("cat", &["/tmp/tke-codex/session/output.json".to_owned()]));
    assert!(cfg.is_whitelisted("sed", &["src/tests.rs".to_owned()]));
    assert!(!cfg.is_whitelisted("cat", &["/tmp/notke-codex/session/output.json".to_owned()]));
    assert!(!cfg.is_whitelisted("cat", &["src/tests.rs.bak".to_owned()]));
}

struct WouldBlockReader;

impl Read for WouldBlockReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::WouldBlock))
    }
}

#[test]
fn small_payload_keeps_body() {
    let cfg = Config::default();
    let text = "line1\nline2\nline3\n";
    let json = normalize_text(
        "cat",
        &["foo.rs".to_owned()],
        "stdout",
        CommandKind::File,
        text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert!(value.get("t").is_none());
    assert_eq!(value["b"][0], "line1");
}

#[test]
fn large_payload_trims_ranges() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    cfg.head_lines = 2;
    cfg.tail_lines = 2;
    let text = (0..12)
        .map(|idx| format!("line-{idx} error"))
        .collect::<Vec<_>>()
        .join("\n");
    let json = normalize_text(
        "cargo",
        &["test".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["t"], true);
    assert!(value["o"].as_array().is_some());
    assert!(value["m"].as_array().expect("matches").len() >= 1);
}

#[test]
fn match_terms_include_errors_and_args() {
    let terms = match_terms("cargo", &["test".to_owned(), "AuthFailure".to_owned()]);
    assert!(terms.contains(&"authfailure".to_owned()));
    assert!(terms.contains(&"error".to_owned()));
}

#[test]
fn compact_args_handles_unicode_boundaries() {
    let args = vec![
        "decision_view|core_research_call|recommendation|执行动作|一页纸摘要|IC主席版|reader_summary|summary"
            .to_owned(),
    ];
    let compact = crate::trim::compact_args(&args);
    assert_eq!(compact.len(), 1);
    assert!(compact[0].ends_with("..."));
}

#[test]
fn collect_log_summary_handles_unicode_boundaries() {
    let lines = vec![
        "{\"success\":true,\"data\":{\"current_step_description\":\"生成市场技术分析师报告\",\"current_step_name\":\"市场技术分析\",\"elapsed_time\":17,\"error_message\":null,\"estimated_total_time\":100,\"message\":\"市场技术分析中\",\"progress\":87,\"remaining_time\":13}}",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::log_profile::collect_log_summary(&refs);
    assert!(summary.fail + summary.warn >= 1);
    assert!(
        summary.ff.as_deref().is_some() || summary.fw.as_deref().is_some(),
        "expected a safely truncated sample"
    );
}

#[test]
fn read_stream_payload_reads_normal_input() {
    let mut reader = Cursor::new(b"hello\nworld".to_vec());
    let payload = read_stream_payload(&mut reader).expect("payload");
    assert_eq!(payload, Some(b"hello\nworld".to_vec()));
}

#[test]
fn read_stream_payload_ignores_empty_would_block() {
    let mut reader = WouldBlockReader;
    let payload = read_stream_payload(&mut reader).expect("payload");
    assert_eq!(payload, None);
}

#[test]
fn diff_profile_marks_hunks() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "diff --git a/src/lib.rs b/src/lib.rs",
        "index 123..456 100644",
        "--- a/src/lib.rs",
        "+++ b/src/lib.rs",
        "@@ -1,3 +1,4 @@",
        "-old",
        "+new",
        " unchanged",
    ]
    .join("\n");
    let json = normalize_text(
        "git",
        &["diff".to_owned()],
        "stdout",
        CommandKind::Diff,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["p"], "diff");
    assert_eq!(value["df"]["f"][0]["p"], "src/lib.rs");
    assert_eq!(value["df"]["f"][0]["add"], 1);
    assert_eq!(value["df"]["f"][0]["del"], 1);
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|chunk| chunk["k"] == "hunk")
    );
}

#[test]
fn diff_profile_emits_per_file_add_delete_summary() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "diff --git a/src/lib.rs b/src/lib.rs",
        "index 123..456 100644",
        "--- a/src/lib.rs",
        "+++ b/src/lib.rs",
        "@@ -1,2 +1,3 @@",
        "-old_line",
        "+new_line",
        "+new_call();",
        "diff --git a/src/main.rs b/src/main.rs",
        "index 999..abc 100644",
        "--- a/src/main.rs",
        "+++ b/src/main.rs",
        "@@ -10,1 +10,0 @@",
        "-removed_main",
    ]
    .join("\n");
    let json = normalize_text(
        "git",
        &["diff".to_owned(), "--".to_owned(), "src".to_owned()],
        "stdout",
        CommandKind::Diff,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["p"], "diff");
    let files = value["df"]["f"].as_array().expect("diff files");
    assert_eq!(files[0]["p"], "src/lib.rs");
    assert_eq!(files[0]["add"], 2);
    assert_eq!(files[0]["del"], 1);
    assert_eq!(files[1]["p"], "src/main.rs");
    assert!(files[1]["add"].is_null());
    assert_eq!(files[1]["del"], 1);
}

#[test]
fn stacktrace_profile_detected() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "Traceback (most recent call last):",
        "  File \"app.py\", line 10, in <module>",
        "  File \"svc.py\", line 20, in run",
        "ValueError: boom",
    ]
    .join("\n");
    let json = normalize_text(
        "python",
        &["script.py".to_owned()],
        "stderr",
        CommandKind::Generic,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["p"], "stacktrace");
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|chunk| chunk["k"] == "frame")
    );
}

#[test]
fn search_profile_prefers_result_lines() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "src/lib.rs:10:fn normalize_text(",
        "src/lib.rs:20:fn collect_profile_chunks(",
        "src/main.rs:5:fn run()",
    ]
    .join("\n");
    let json = normalize_text(
        "rg",
        &["normalize".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["p"], "search");
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .all(|chunk| chunk["k"] == "file" || chunk["k"] == "result")
    );
}

#[test]
fn search_profile_groups_results_by_file() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..40)
        .map(|idx| format!("src/lib.rs:{}:pub fn alpha_{}() {{}}", idx + 1, idx))
        .chain((0..20).map(|idx| format!("src/main.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n");
    let json = normalize_text(
        "rg",
        &["fn".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    let chunks = value["m"].as_array().expect("matches");
    assert!(chunks.iter().any(|chunk| chunk["k"] == "file"));
    let file_chunk = chunks
        .iter()
        .find(|chunk| chunk["k"] == "file")
        .expect("file chunk");
    let lines = file_chunk["l"].as_array().expect("chunk lines");
    assert_eq!(
        lines[0].as_str().expect("first line"),
        "src/lib.rs:1:pub fn alpha_0() {}"
    );
    assert!(
        lines[1].as_str().expect("second line").starts_with(":"),
        "expected compact grouped line"
    );
}

#[test]
fn search_profile_compacts_repeated_file_prefixes_after_first_line() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "src/lib.rs:10:pub fn alpha() {}",
        "src/lib.rs:11:pub fn beta() {}",
        "src/lib.rs:12:pub fn gamma() {}",
        "src/main.rs:5:pub fn run() {}",
    ]
    .join("\n");
    let json = normalize_text(
        "rg",
        &["fn".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    let chunks = value["m"].as_array().expect("matches");
    let file_chunk = chunks
        .iter()
        .find(|chunk| chunk["k"] == "file")
        .expect("file chunk");
    let lines = file_chunk["l"].as_array().expect("chunk lines");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "src/lib.rs:10:pub fn alpha() {}");
    assert_eq!(lines[1], ":11:pub fn beta() {}");
    assert_eq!(lines[2], ":12:pub fn gamma() {}");
}

#[test]
fn log_profile_folds_repeated_lines() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "Compiling crate_a v0.1.0",
        "Compiling crate_a v0.1.0",
        "Compiling crate_a v0.1.0",
        "Compiling crate_a v0.1.0",
        "error: build failed",
    ]
    .join("\n");
    let json = normalize_text(
        "cargo",
        &["build".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["p"], "log");
    assert_eq!(value["lg"]["fail"], 1);
    assert_eq!(value["lg"]["ff"], "error: build failed");
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|chunk| chunk["k"] == "fold")
    );
}

#[test]
fn log_profile_emits_failure_and_warning_counts() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "warning: deprecated item used",
        "error: build failed",
        "FAILED tests/test_parser.py::test_invalid_input",
        "ok 1 - should parse simple expression",
    ]
    .join("\n");
    let json = normalize_text(
        "cargo",
        &["test".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["p"], "log");
    assert_eq!(value["lg"]["fail"], 2);
    assert_eq!(value["lg"]["warn"], 1);
    assert_eq!(value["lg"]["ff"], "error: build failed");
    assert_eq!(value["lg"]["fw"], "warning: deprecated item used");
    assert_eq!(value["bd"]["n"], "cargo");
}

#[test]
fn log_profile_does_not_treat_zero_failed_summary_as_failure() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "test result: ok. 104 passed; 0 failed; 0 ignored; 0 measured",
        "warning: deprecated fixture used",
    ]
    .join("\n");
    let json = normalize_text(
        "cargo",
        &["test".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["p"], "log");
    assert_eq!(value["lg"]["fail"], 0);
    assert_eq!(value["lg"]["warn"], 1);
    assert!(value["lg"]["ff"].is_null());
    assert_eq!(value["lg"]["fw"], "warning: deprecated fixture used");
}

#[test]
fn build_summary_extracts_test_and_install_counts() {
    let cargo_lines = [
        "Compiling demo v0.1.0",
        "Running unittests src/lib.rs (target/debug/deps/demo)",
        "test result: FAILED. 117 passed; 3 failed; 0 ignored; 0 measured",
        "error: test failed, to rerun pass --lib",
    ];
    let cargo_refs = cargo_lines.iter().copied().collect::<Vec<_>>();
    let cargo = crate::trim::collect_build_summary("cargo", &cargo_refs).expect("cargo summary");
    assert_eq!(cargo.n, "cargo");
    assert_eq!(cargo.cp, 1);
    assert_eq!(cargo.rn, 1);
    assert_eq!(cargo.ok, 117);
    assert_eq!(cargo.fl, 3);
    assert_eq!(cargo.tt, 120);
    assert!(
        cargo
            .tr
            .as_deref()
            .is_some_and(|line| line.contains("117 passed"))
    );

    let pip_lines = [
        "Collecting demo-package",
        "Collecting helper-package",
        "Successfully installed demo-1.0 helper-2.0 toolkit-3.1",
        "warning: Retrying (Retry(total=4, connect=None))",
    ];
    let pip_refs = pip_lines.iter().copied().collect::<Vec<_>>();
    let pip = crate::trim::collect_build_summary("pip", &pip_refs).expect("pip summary");
    assert_eq!(pip.n, "pip");
    assert_eq!(pip.ip, 3);
    assert!(
        pip.e
            .iter()
            .any(|line| line.contains("Successfully installed demo-1.0 helper-2.0 toolkit-3.1"))
    );
}

#[test]
fn build_summary_prefers_numeric_failed_count_over_status_word() {
    let lines = [
        "Running integration tests",
        "test result: FAILED. 117 passed; 3 failed; 0 ignored; 0 measured",
        "error: test failed, to rerun pass --test parser",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("cargo", &refs).expect("summary");
    assert_eq!(summary.ok, 117);
    assert_eq!(summary.fl, 3);
    assert_eq!(summary.tt, 120);
}

#[test]
fn build_summary_extracts_ctest_totals() {
    let lines = [
        "Test #1: lexer_test ... Passed",
        "Test #2: parser_test ... Passed",
        "99% tests passed, 1 tests failed out of 120",
        "The following tests FAILED:",
        " 42 - parser_test (Failed)",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("ctest", &refs).expect("summary");
    assert_eq!(summary.ok, 99);
    assert_eq!(summary.fl, 1);
    assert_eq!(summary.tt, 120);
    assert!(
        summary
            .e
            .iter()
            .any(|line| line.contains("1 tests failed out of 120"))
    );
}

#[test]
fn build_summary_extracts_maven_style_counts() {
    let lines = [
        "[INFO] BUILD FAILURE",
        "[ERROR] Tests run: 120, Failures: 1, Errors: 0, Skipped: 4",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("mvn", &refs).expect("summary");
    assert_eq!(summary.fl, 1);
    assert_eq!(summary.sk, 4);
    assert_eq!(summary.tt, 120);
}

#[test]
fn build_summary_extracts_maven_error_count_when_failures_are_zero() {
    let lines = [
        "[INFO] BUILD FAILURE",
        "[ERROR] Tests run: 120, Failures: 0, Errors: 2, Skipped: 1",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("mvn", &refs).expect("summary");
    assert_eq!(summary.fl, 2);
    assert_eq!(summary.sk, 1);
    assert_eq!(summary.tt, 120);
}

#[test]
fn build_summary_extracts_dotnet_style_counts() {
    let lines = [
        "Passed TestCase.Parser",
        "Failed!  - Failed:     3, Passed:   117, Skipped:     2, Total:   122",
        "error CS1002: ; expected",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("dotnet", &refs).expect("summary");
    assert_eq!(summary.ok, 117);
    assert_eq!(summary.fl, 3);
    assert_eq!(summary.sk, 2);
    assert_eq!(summary.tt, 122);
}

#[test]
fn build_summary_keeps_zero_failed_run_non_failing() {
    let lines = [
        "Running unittests src/lib.rs (target/debug/deps/demo)",
        "test result: ok. 104 passed; 0 failed; 0 ignored; 0 measured",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("cargo", &refs).expect("summary");
    assert_eq!(summary.ok, 104);
    assert_eq!(summary.fl, 0);
    assert_eq!(summary.tt, 104);
}

#[test]
fn build_summary_counts_installed_packages_once() {
    let lines = [
        "Collecting demo-package",
        "Successfully installed demo-1.0 helper-2.0 toolkit-3.1",
        "Successfully installed demo-1.0 helper-2.0 toolkit-3.1",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("pip", &refs).expect("summary");
    assert_eq!(summary.ip, 3);
    assert_eq!(summary.e.len(), 1);
}

#[test]
fn build_summary_extracts_pytest_style_counts() {
    let lines = [
        "FAILED tests/test_parser.py::test_invalid_input - AssertionError: expected boom",
        "2 passed, 1 failed, 1 skipped, 4 total",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("pytest", &refs).expect("summary");
    assert_eq!(summary.ok, 2);
    assert_eq!(summary.fl, 1);
    assert_eq!(summary.sk, 1);
    assert_eq!(summary.tt, 4);
}

#[test]
fn build_summary_extracts_python_unittest_counts() {
    let lines = [
        "test_parser (tests.test_parser.ParserTests.test_parser) ... ok",
        "FAILED (failures=2, errors=1, skipped=3)",
        "Ran 12 tests in 0.452s",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("python", &refs).expect("summary");
    assert_eq!(summary.fl, 3);
    assert_eq!(summary.sk, 3);
    assert_eq!(summary.tt, 12);
}

#[test]
fn build_summary_extracts_cargo_ignored_counts_into_total() {
    let lines = ["test result: ok. 117 passed; 0 failed; 4 ignored; 0 measured"];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("cargo", &refs).expect("summary");
    assert_eq!(summary.ok, 117);
    assert_eq!(summary.tt, 121);
}

#[test]
fn build_summary_extracts_gradle_completed_and_failed_counts() {
    let lines = ["BUILD FAILED in 12s", "1 test completed, 1 failed"];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("gradle", &refs).expect("summary");
    assert_eq!(summary.fl, 1);
    assert_eq!(summary.tt, 1);
    assert!(
        summary
            .e
            .iter()
            .any(|line| line.contains("1 test completed, 1 failed"))
    );
}

#[test]
fn build_summary_does_not_treat_node_case_index_as_failure_count() {
    let lines = [
        "not ok 12 - parser handles invalid input",
        "error: Expected value to be truthy",
    ];
    let refs = lines.iter().copied().collect::<Vec<_>>();
    let summary = crate::trim::collect_build_summary("node", &refs).expect("summary");
    assert_eq!(summary.fl, 0);
    assert!(
        summary
            .e
            .iter()
            .any(|line| line.contains("not ok 12 - parser handles invalid input"))
    );
}

#[test]
fn log_signal_detection_uses_tokens_not_substrings() {
    assert!(is_warning_signal("warning: deprecated fixture used"));
    assert!(!is_warning_signal("forewarning markers are enabled"));
    assert!(is_failure_signal_line(
        "FAILED tests/test_parser.py::test_invalid_input"
    ));
    assert!(!is_failure_signal_line(
        "test result: ok. 104 passed; 0 failed; 0 ignored"
    ));
    assert!(is_log_signal(
        "panic: runtime error: index out of range",
        &[]
    ));
    assert!(is_log_signal(
        "src/core/parser.rs | 12 ++++++------",
        &["core parser".to_owned()]
    ));
    assert!(!is_log_signal(
        "src/core/parser.rs | 12 ++++++------",
        &["ore par".to_owned()]
    ));
}

#[test]
fn table_profile_detected_for_ps_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 proxima",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
        ]
        .join("\n");
    let json = normalize_text(
        "ps",
        &["aux".to_owned(), "--sort=-%cpu".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["p"], "table");
    assert!(value["tb"].is_object());
    assert_eq!(value["tb"]["c"][0], "USER");
    assert!(
        value["tb"]["r"]
            .as_array()
            .expect("rows")
            .iter()
            .any(|row| row["v"].to_string().to_ascii_lowercase().contains("codex"))
    );
}

#[test]
fn maybe_normalize_prefers_table_summary_when_cheaper() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 proxima",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
            "root        1063  0.0  1.6 1440068 66960 ?       Sl   07:56   0:00 cloud-monitor-agent",
            "root        2546  0.0  1.2 1403548 48180 pts/1   Sl+  08:08   0:00 node codex",
            "root         886  0.0  1.1 1313072 46252 ?       Ssl  07:56   0:00 mihomo",
            "root         710  0.0  1.4 1950568 57672 ?       Ssl  07:56   0:00 containerd",
            "root         964  0.0  2.2 2271304 91340 ?       Ssl  07:56   0:00 dockerd",
        ]
        .join("\n");
    let normalized = maybe_normalize_text(
        "ps",
        &["aux".to_owned(), "--sort=-%cpu".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
        None,
    )
    .expect("normalize");
    assert!(normalized.is_some());
    let payload = normalized.expect("payload");
    let value = value_from_json(&payload);
    assert_eq!(value["p"], "table");
}

#[test]
fn table_profile_can_trigger_below_default_trim_bytes() {
    let cfg = Config::default();
    let text = [
        "CONTAINER ID   IMAGE          STATUS       PORTS      NAMES",
        "abc123         redis:7        Up 2 hours   6379/tcp   redis-main",
        "def456         postgres:16    Up 2 hours   5432/tcp   pg-main",
        "ghi789         nginx:latest   Exited (0)              web-old",
        "jkl012         app:v1         Up 5 mins    8080/tcp   app-blue",
        "mno345         app:v2         Up 1 min     8081/tcp   app-green",
        "pqr678         worker:v2      Up 1 min                worker-green",
        "stu901         sidekiq:v2     Restarting              sidekiq-green",
        "vwx234         cron:v1        Exited (137)            cron-old",
        "yz5678         proxy:v3       Up 3 days    80/tcp     edge-proxy",
        "qwe111         admin:v1       Up 3 days    9000/tcp   admin-ui",
        "rty222         mail:v1        Up 1 day     25/tcp     smtp",
    ]
    .join("\n");
    assert!(text.len() < cfg.min_trim_bytes);
    let normalized = maybe_normalize_text(
        "docker",
        &["ps".to_owned(), "-a".to_owned(), "--no-trunc".to_owned()],
        "stdout",
        CommandKind::Generic,
        &text,
        &cfg,
        None,
    )
    .expect("normalize");
    assert!(normalized.is_some());
}

#[test]
fn compare_rollout_reports_savings_for_realish_ps_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let output = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 /root/.nvm/.../codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared --token secret",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 /opt/proxima/proxima",
            "root        2696  0.1  0.3 102660 15264 pts/1    S+   08:08   0:00 git-remote-https https://github.com/openai/plugins.git",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 /usr/bin/1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
            "root        1063  0.0  1.6 1440068 66960 ?       Sl   07:56   0:00 cloud-monitor-agent worker",
            "root        2546  0.0  1.2 1403548 48180 pts/1   Sl+  08:08   0:00 node /root/.nvm/.../codex",
            "root         886  0.0  1.1 1313072 46252 ?       Ssl  07:56   0:00 mihomo",
            "root         710  0.0  1.4 1950568 57672 ?       Ssl  07:56   0:00 containerd",
            "root         964  0.0  2.2 2271304 91340 ?       Ssl  07:56   0:00 dockerd",
            "root        2408  0.0  0.1  11028  7764 pts/1    Ss   08:08   0:00 /bin/bash",
            "root          63  0.0  0.0      0     0 ?        I    07:56   0:00 [kworker/3:1-events]",
            "root         673  0.0  1.5 1366336 60272 ?       Ssl  07:56   0:00 cloud-monitor-agent start",
            "root         685  0.0  0.1 221780  4376 ?        Ssl  07:56   0:00 rsyslogd -n -iNONE",
            "root         266  0.0  0.3  33208 13396 ?        Ss   07:56   0:00 systemd-journald",
            "root          39  0.0  0.0      0     0 ?        I    07:56   0:00 [kworker/u10:1-writeback]",
            "root        2407  0.0  0.0   7016  2532 ?        Ss   08:08   0:00 SCREEN -S t",
            "earlyoom     679  0.0  0.0   2480   884 ?        SLs  07:56   0:00 earlyoom",
            "root         668  0.0  0.8 1308624 32080 ?       Ssl  07:56   0:00 assist-client start",
            "root        1843  0.0  0.0      0     0 ?        I    08:07   0:00 [kworker/u9:1-events_unbound]",
            "root         667  0.0  0.8 1292420 33452 ?       Ssl  07:56   0:00 /usr/bin/1panel-core",
            "70          1411  0.0  0.6 175360 26788 ?        Ss   07:56   0:00 postgres",
        ]
        .join("\n");
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"ps aux --sort=-%cpu | head -n 25\",\"yield_time_ms\":1000}",
                    "call_id": "call_7"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_7",
                    "output": format!(
                        "Chunk ID: eebb60\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 757\nOutput:\n{output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let raw_stats = collect_rollout_output_stats_detailed(&jsonl, &cfg);
    let rewritten_stats = collect_rollout_output_stats_detailed(&rewritten, &cfg);
    assert!(rewritten_stats.total.approx_tokens < raw_stats.total.approx_tokens);
    assert!(rewritten_stats.total.bytes < raw_stats.total.bytes);
}

#[test]
fn file_profile_extracts_code_blocks() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "use std::io;",
        "",
        "pub fn normalize_text() {",
        "    let x = 1;",
        "    println!(\"{}\", x);",
        "}",
        "",
        "pub struct Config {",
        "    value: usize,",
        "}",
    ]
    .join("\n");
    let json = normalize_text(
        "cat",
        &["src/lib.rs".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["p"], "file");
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|chunk| chunk["k"] == "block" || chunk["k"] == "decl")
    );
}

#[test]
fn rewrites_codex_command_execution_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let line = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_1",
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
            "aggregated_output": format!("{}\n", repeated_lines("pub fn a() {}", 200)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    let rewritten = rewrite_codex_jsonl(&line, &cfg)
        .expect("rewrite")
        .expect("changed");
    let value: serde_json::Value =
        serde_json::from_str(rewritten.lines().next().expect("line")).expect("json");
    let output = value["item"]["aggregated_output"]
        .as_str()
        .expect("aggregated_output");
    assert!(output.starts_with("__TKE__"));
    let nested = value_from_json(output.trim_start_matches("__TKE__"));
    assert_eq!(nested["sc"], "cat");
    assert_eq!(nested["sr"], "source");
}

#[test]
fn parses_shell_wrapped_command_execution() {
    let parsed =
        parse_command_execution("/bin/bash -lc \"env FOO=1 cat /tmp/demo.txt | sed -n '1p'\"");
    assert_eq!(parsed.selected_stage().name, "cat");
    assert_eq!(
        parsed.selected_stage().args.first().map(String::as_str),
        Some("/tmp/demo.txt")
    );
}

#[test]
fn parses_timeout_wrapped_command_execution() {
    let parsed = parse_command_execution("timeout 30s env FOO=1 cargo test -- --nocapture");
    assert_eq!(parsed.selected_stage().name, "cargo");
    assert_eq!(parsed.selected_stage().role.as_str(), "build");
}

#[test]
fn parses_stdbuf_wrapped_search_pipeline() {
    let parsed = parse_command_execution(
        "stdbuf -oL env FOO=1 rg -n normalize_text src/tests.rs | head -n 10",
    );
    assert_eq!(parsed.selected_stage().name, "rg");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn parses_nice_wrapped_python_command() {
    let parsed = parse_command_execution("nice -n 5 python3 script.py");
    assert_eq!(parsed.selected_stage().name, "python3");
    assert_eq!(parsed.selected_stage().role.as_str(), "build");
}

#[test]
fn parses_bash_wrapped_which_pipeline() {
    let parsed = parse_command_execution("bash -lc 'env FOO=1 which cargo | head -n 1'");
    assert_eq!(parsed.selected_stage().name, "which");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_for_loop_wrapped_curl_healthcheck() {
    let parsed = parse_command_execution(
        "bash -lc 'for i in {1..20}; do curl -fsS http://127.0.0.1:8000/api/health && exit 0; sleep 1; done; exit 1'",
    );
    assert_eq!(parsed.selected_stage().name, "curl");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_for_loop_wrapped_curl_head_pipeline() {
    let parsed = parse_command_execution(
        "bash -lc 'for i in {1..20}; do curl -fsS http://127.0.0.1:5173/ | head -c 200 && exit 0; sleep 1; done; exit 1'",
    );
    assert_eq!(parsed.selected_stage().name, "curl");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn stage_selection_ignores_head_count_argument() {
    let parsed = parse_command_execution("curl -fsS http://127.0.0.1:5173/ | head -c 200");
    assert_eq!(parsed.selected_stage().name, "curl");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_python_heredoc_command() {
    let parsed = parse_command_execution(
        "python - <<'PY'\nimport json\nprint(json.dumps({'ok': True}))\nPY",
    );
    assert_eq!(parsed.selected_stage().name, "python");
    assert_eq!(parsed.selected_stage().role.as_str(), "build");
}

#[test]
fn parses_powershell_wrapped_command_execution() {
    let parsed = parse_command_execution(
        "pwsh -Command \"Get-Content /tmp/demo.txt | rg -n section | Select-Object -First 1\"",
    );
    assert_eq!(parsed.selected_stage().name, "rg");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn parses_powershell_get_content_pipeline_as_source_stage() {
    let parsed = parse_command_execution(
        "pwsh -Command \"Get-Content /tmp/demo.txt | Select-Object -First 20\"",
    );
    assert_eq!(parsed.selected_stage().name, "cat");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_macos_mdfind_pipeline_as_search_stage() {
    let parsed = parse_command_execution("mdfind kind:rust | head -n 20");
    assert_eq!(parsed.selected_stage().name, "find");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn parses_bsd_prefixed_ggrep_pipeline_as_search_stage() {
    let parsed = parse_command_execution("ggrep -n normalize_text src/tests.rs | head -n 5");
    assert_eq!(parsed.selected_stage().name, "grep");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn parses_windows_more_pipeline_as_source_stage() {
    let parsed = parse_command_execution("type README.md | more");
    assert_eq!(parsed.selected_stage().name, "cat");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_macos_open_preview_as_source_stage() {
    let parsed = parse_command_execution("qlmanage -p /tmp/demo.png");
    assert_eq!(parsed.selected_stage().name, "cat");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_powershell_get_clipboard_pipeline_as_source_stage() {
    let parsed =
        parse_command_execution("pwsh -Command \"Get-Clipboard | Select-Object -First 20\"");
    assert_eq!(parsed.selected_stage().name, "cat");
    assert_eq!(parsed.selected_stage().role.as_str(), "source");
}

#[test]
fn parses_powershell_select_object_last_as_tail_stage() {
    let parsed = parse_command_execution(
        "pwsh -Command \"Get-Content /tmp/demo.txt | Select-Object -Last 20\"",
    );
    assert_eq!(parsed.last_stage().name, "tail");
    assert_eq!(parsed.last_stage().role.as_str(), "summarize");
}

#[test]
fn parses_powershell_select_object_skip_as_filter_stage() {
    let parsed = parse_command_execution(
        "pwsh -Command \"Get-Content /tmp/demo.txt | Select-Object -Skip 10\"",
    );
    assert_eq!(parsed.last_stage().name, "awk");
    assert_eq!(parsed.last_stage().role.as_str(), "filter");
}

#[test]
fn canonical_command_name_maps_windows_aliases() {
    assert_eq!(canonical_command_name("Get-Content"), "cat");
    assert_eq!(canonical_command_name("Get-Clipboard"), "cat");
    assert_eq!(canonical_command_name("gc"), "cat");
    assert_eq!(canonical_command_name("type"), "cat");
    assert_eq!(canonical_command_name("gsed"), "sed");
    assert_eq!(canonical_command_name("Select-String"), "grep");
    assert_eq!(canonical_command_name("findstr"), "grep");
    assert_eq!(canonical_command_name("Get-ChildItem"), "ls");
    assert_eq!(canonical_command_name("dir"), "ls");
    assert_eq!(canonical_command_name("Measure-Object"), "wc");
    assert_eq!(canonical_command_name("Select-Object"), "head");
    assert_eq!(canonical_command_name("ggrep"), "grep");
    assert_eq!(canonical_command_name("gls"), "ls");
    assert_eq!(canonical_command_name("gfind"), "find");
    assert_eq!(canonical_command_name("mdfind"), "find");
    assert_eq!(canonical_command_name("mdls"), "cat");
    assert_eq!(canonical_command_name("xattr"), "cat");
    assert_eq!(canonical_command_name("pbpaste"), "cat");
    assert_eq!(canonical_command_name("ghead"), "head");
    assert_eq!(canonical_command_name("gtail"), "tail");
    assert_eq!(canonical_command_name("guniq"), "uniq");
    assert_eq!(canonical_command_name("gwc"), "wc");
    assert_eq!(canonical_command_name("gdu"), "du");
    assert_eq!(canonical_command_name("gdf"), "df");
    assert_eq!(canonical_command_name("more"), "head");
    assert_eq!(canonical_command_name("more.com"), "head");
    assert_eq!(canonical_command_name("plutil"), "jq");
    assert_eq!(canonical_command_name("open"), "cat");
    assert_eq!(canonical_command_name("qlmanage"), "cat");
}

#[test]
fn rewrites_multiple_command_output_fields() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let mut value = serde_json::json!({
        "item": {
            "type": "command_execution",
            "command": "/bin/bash -lc 'python tool.py'",
            "stdout": format!("Traceback (most recent call last):\n{}\nValueError: boom\n", repeated_lines("  File \"x.py\", line 1, in run", 80)),
            "stderr": format!("{}\n", repeated_lines("warning: noisy", 120)),
            "output": format!("{}\n", repeated_lines("pub fn alpha() {}", 180))
        }
    });
    let parsed = parse_command_execution("/bin/bash -lc 'python tool.py'");
    let changed = rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
    assert!(changed);
    assert!(
        value["item"]["stdout"]
            .as_str()
            .expect("stdout")
            .starts_with("__TKE__")
    );
    assert!(
        value["item"]["stderr"]
            .as_str()
            .expect("stderr")
            .starts_with("__TKE__")
    );
    assert!(
        value["item"]["output"]
            .as_str()
            .expect("output")
            .starts_with("__TKE__")
    );
}

#[test]
fn rewrite_is_idempotent_for_prefixed_output() {
    let cfg = Config::default();
    let mut value = serde_json::json!({
        "item": {
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
            "aggregated_output": "__TKE__{\"v\":1}"
        }
    });
    let parsed = parse_command_execution("/bin/bash -lc 'cat /tmp/demo.rs'");
    let changed = rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
    assert!(!changed);
    assert_eq!(value["item"]["aggregated_output"], "__TKE__{\"v\":1}");
}

#[test]
fn rewrite_skips_empty_fields() {
    let cfg = Config::default();
    let mut value = serde_json::json!({
        "item": {
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
            "aggregated_output": ""
        }
    });
    let parsed = parse_command_execution("/bin/bash -lc 'cat /tmp/demo.rs'");
    let changed = rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
    assert!(!changed);
}

#[test]
fn non_jsonl_input_is_not_rewritten() {
    let cfg = Config::default();
    let rewritten = rewrite_codex_jsonl("not-json\nstill-not-json\n", &cfg).expect("rewrite");
    assert!(rewritten.is_none());
}

#[test]
fn non_jsonl_input_is_not_rewritten_for_claude() {
    let cfg = Config::default();
    let rewritten = rewrite_claude_jsonl("not-json\nstill-not-json\n", &cfg).expect("rewrite");
    assert!(rewritten.is_none());
}

#[test]
fn parses_nested_shell_invocation() {
    let parsed = parse_command_execution(
        "/bin/bash -lc \"sh -c 'rg -n fn /tmp/tke-codex/big.rs | sed -n 1p'\"",
    );
    assert_eq!(parsed.selected_stage().name, "rg");
    assert!(
        parsed
            .selected_stage()
            .args
            .iter()
            .any(|arg: &String| arg.contains("/tmp/tke-codex/big.rs"))
    );
}

#[test]
fn parses_xargs_payload_command() {
    let parsed =
        parse_command_execution("/bin/bash -lc \"printf '/tmp/tke-codex/big.rs\\n' | xargs cat\"");
    assert_eq!(parsed.selected_stage().name, "cat");
}

#[test]
fn stage_selection_prefers_search_over_filter() {
    let parsed = parse_command_execution(
        "env FOO=1 echo ignore | rg -n fn /tmp/tke-codex/big.rs | sed -n 1p",
    );
    assert_eq!(parsed.selected_stage().name, "rg");
}

#[test]
fn stage_selection_prefers_search_over_source_in_mixed_pipeline() {
    let parsed =
        parse_command_execution("cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1");
    assert_eq!(parsed.selected_stage().name, "rg");
}

#[test]
fn stage_selection_treats_git_grep_as_search() {
    let parsed = parse_command_execution("git grep -n normalize_text src | head -n 20");
    assert_eq!(parsed.selected_stage().name, "git");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn live_pipeline_passthrough_skips_multi_stage_cat_rg_head_pipeline() {
    let parsed =
        parse_live_shell_pipeline("cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1");
    assert!(live_pipeline_should_passthrough(&parsed, "cat"));
    assert!(live_pipeline_should_passthrough(&parsed, "rg"));
    assert!(!live_pipeline_should_passthrough(&parsed, "head"));
}

#[test]
fn live_pipeline_passthrough_skips_multi_stage_find_head_pipeline() {
    let parsed = parse_live_shell_pipeline("find src -name '*.rs' | head -n 20");
    assert!(live_pipeline_should_passthrough(&parsed, "find"));
    assert!(!live_pipeline_should_passthrough(&parsed, "head"));
}

#[test]
fn live_pipeline_passthrough_skips_multi_stage_build_tail_pipeline() {
    let parsed = parse_live_shell_pipeline("cargo test -- --nocapture | tail -n 80");
    assert!(live_pipeline_should_passthrough(&parsed, "cargo"));
    assert!(!live_pipeline_should_passthrough(&parsed, "tail"));
}

#[test]
fn live_pipeline_decision_normalizes_last_stage_with_selected_search_metadata() {
    let parsed =
        parse_live_shell_pipeline("cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1");
    match live_pipeline_decision(&parsed, "head") {
        LivePipelineDecision::Normalize(selected) => {
            assert_eq!(selected.name, "rg");
            assert_eq!(selected.role.as_str(), "search");
        }
        _ => panic!("expected normalize"),
    }
}

#[test]
fn live_pipeline_decision_normalizes_last_stage_for_build_tail() {
    let parsed = parse_live_shell_pipeline("cargo test -- --nocapture | tail -n 80");
    match live_pipeline_decision(&parsed, "tail") {
        LivePipelineDecision::Normalize(selected) => {
            assert_eq!(selected.name, "cargo");
            assert_eq!(selected.role.as_str(), "build");
        }
        _ => panic!("expected normalize"),
    }
}

#[test]
fn default_tool_commands_cover_common_reading_tools() {
    let cfg = Config::default();
    for name in [
        "ls",
        "find",
        "fd",
        "bat",
        "nl",
        "awk",
        "cut",
        "sort",
        "uniq",
        "wc",
        "tree",
        "xargs",
        "jq",
        "curl",
        "tr",
        "perl",
        "gls",
        "gfind",
        "ggrep",
        "mdfind",
        "pbpaste",
        "Get-ChildItem",
        "Get-Content",
        "Get-Clipboard",
        "Select-String",
        "more",
        "more.com",
        "open",
        "qlmanage",
        "mdls",
        "plutil",
        "xattr",
        "gsed",
        "ghead",
        "gtail",
        "guniq",
        "gwc",
        "gdu",
        "gdf",
    ] {
        assert!(cfg.is_tool_command(name), "missing tool command {name}");
    }
}

#[test]
fn default_tool_commands_cover_core_agent_workflows() {
    let cfg = Config::default();
    for name in [
        "cat",
        "sed",
        "rg",
        "grep",
        "git",
        "cargo",
        "pytest",
        "npm",
        "pnpm",
        "yarn",
        "bun",
        "pip",
        "uv",
        "poetry",
        "mvn",
        "gradle",
        "gradlew",
        "javac",
        "java",
        "bundle",
        "composer",
        "dotnet",
        "go",
        "cmake",
        "ctest",
        "make",
        "ninja",
        "node",
        "tail",
        "head",
        "ls",
        "find",
        "fd",
        "bat",
        "nl",
        "awk",
        "cut",
        "sort",
        "uniq",
        "wc",
        "tree",
        "xargs",
        "jq",
        "curl",
        "python",
        "python3",
        "docker",
        "ps",
        "ss",
        "netstat",
        "systemctl",
        "tr",
        "perl",
        "du",
        "df",
    ] {
        assert!(cfg.is_tool_command(name), "missing tool command {name}");
    }
}

#[test]
fn stage_selection_prefers_find_for_file_discovery_pipeline() {
    let parsed = parse_command_execution("find src -name '*.rs' | head -n 20");
    assert_eq!(parsed.selected_stage().name, "find");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn stage_selection_prefers_fd_over_filter() {
    let parsed = parse_command_execution("fd normalize src | sort | head -n 10");
    assert_eq!(parsed.selected_stage().name, "fd");
    assert_eq!(parsed.selected_stage().role.as_str(), "search");
}

#[test]
fn stage_selection_prefers_build_over_tail() {
    let parsed = parse_command_execution("cargo test -- --nocapture | tail -n 80");
    assert_eq!(parsed.selected_stage().name, "cargo");
    assert_eq!(parsed.selected_stage().role.as_str(), "build");
}

#[test]
fn classify_common_code_reading_commands() {
    assert!(matches!(
        classify_command("bat", &["src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("nl", &["-ba".to_owned(), "src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "find",
            &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()]
        ),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("ls", &["-l".to_owned(), "src".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("ls", &["src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("Get-Content", &["src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "Select-String",
            &[
                "-Pattern".to_owned(),
                "normalize_text".to_owned(),
                "src/lib.rs".to_owned()
            ]
        ),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("Get-ChildItem", &["src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("Get-ChildItem", &["-Recurse".to_owned(), "src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("gls", &["src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command(
            "ggrep",
            &[
                "-n".to_owned(),
                "normalize_text".to_owned(),
                "src/tests.rs".to_owned()
            ]
        ),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command(
            "gfind",
            &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()]
        ),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("mdfind", &["kind:rust".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("Get-Clipboard", &[]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "gsed",
            &["-n".to_owned(), "1,20p".to_owned(), "src/lib.rs".to_owned()]
        ),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("mdls", &["/tmp/demo.png".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "mdls",
            &[
                "-name".to_owned(),
                "kMDItemKind".to_owned(),
                "/tmp/demo.png".to_owned()
            ]
        ),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("xattr", &["/tmp/demo.png".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("xattr", &["-l".to_owned(), "/tmp/demo.png".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("plutil", &["-p".to_owned(), "/tmp/demo.plist".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "stat",
            &["-f".to_owned(), "%N".to_owned(), "/tmp/demo.txt".to_owned()]
        ),
        CommandKind::File
    ));
    assert!(matches!(classify_command("ghead", &[]), CommandKind::File));
    assert!(matches!(classify_command("gtail", &[]), CommandKind::File));
    assert!(matches!(
        classify_command("guniq", &[]),
        CommandKind::Generic
    ));
    assert!(matches!(classify_command("gwc", &[]), CommandKind::Generic));
    assert!(matches!(classify_command("gdu", &[]), CommandKind::Log));
    assert!(matches!(classify_command("gdf", &[]), CommandKind::Log));
    assert!(matches!(
        classify_command("pbpaste", &[]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("more", &["README.md".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("open", &["README.md".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("qlmanage", &["-p".to_owned(), "/tmp/demo.png".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("dir", &["src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("tree", &["-a".to_owned(), "src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("awk", &["{print}".to_owned(), "src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("tr", &["-s".to_owned(), " ".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "perl",
            &[
                "-ne".to_owned(),
                "print".to_owned(),
                "src/lib.rs".to_owned()
            ]
        ),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("wc", &["-l".to_owned(), "src/lib.rs".to_owned()]),
        CommandKind::Generic
    ));
    assert!(matches!(
        classify_command(
            "curl",
            &["-s".to_owned(), "http://127.0.0.1/demo".to_owned()]
        ),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("jq", &[".".to_owned(), "/tmp/demo.json".to_owned()]),
        CommandKind::Generic
    ));
    assert!(matches!(
        classify_command("python3", &["script.py".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("ps", &["aux".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("docker", &["ps".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("which", &["cargo".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("git", &["show".to_owned(), "HEAD:src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command(
            "git",
            &[
                "grep".to_owned(),
                "-n".to_owned(),
                "normalize_text".to_owned()
            ]
        ),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("git", &["ls-files".to_owned(), "src".to_owned()]),
        CommandKind::Search
    ));
    assert!(matches!(
        classify_command("file", &["src/lib.rs".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("stat", &["/tmp/demo.txt".to_owned()]),
        CommandKind::File
    ));
    assert!(matches!(
        classify_command("psql", &["-c".to_owned(), "select 1".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("redis-cli", &["info".to_owned()]),
        CommandKind::Log
    ));
}

#[test]
fn classify_new_agent_commands_as_log() {
    assert!(matches!(
        classify_command("curl", &["-s".to_owned(), "http://example.com".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command(
            "wget",
            &[
                "-O".to_owned(),
                "-".to_owned(),
                "http://example.com".to_owned()
            ]
        ),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("gh", &["pr".to_owned(), "list".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("glab", &["mr".to_owned(), "list".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("docker-compose", &["up".to_owned()]),
        CommandKind::Log
    ));
    assert!(matches!(
        classify_command("pip3", &["install".to_owned(), "requests".to_owned()]),
        CommandKind::Log
    ));
}

#[test]
fn short_generic_output_never_compressed() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let single_line = "hello world";
    let result = maybe_normalize_text(
        "sort",
        &[],
        "stdout",
        CommandKind::Generic,
        single_line,
        &cfg,
        None,
    )
    .expect("no error");
    assert!(result.is_none(), "single-line Generic should passthrough");

    let two_lines = "line one\nline two";
    let result = maybe_normalize_text(
        "sort",
        &[],
        "stdout",
        CommandKind::Generic,
        two_lines,
        &cfg,
        None,
    )
    .expect("no error");
    assert!(result.is_none(), "two-line Generic should passthrough");
}

#[test]
fn empty_output_produces_passthrough() {
    let cfg = Config::default();
    let result = maybe_normalize_text("cat", &[], "stdout", CommandKind::File, "", &cfg, None)
        .expect("no error");
    assert!(result.is_none(), "empty output should return None");
}

#[test]
fn envelope_fc_field_contains_full_command() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let output = (0..20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "cargo",
        &["test".to_owned(), "--lib".to_owned()],
        "stdout",
        CommandKind::Log,
        &output,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    let fc = value["fc"].as_str().expect("fc field");
    assert_eq!(fc, "cargo test --lib", "fc should be full command");
}

#[test]
fn pipeline_normalizes_xargs_payload_command() {
    // xargs wc -l → wc is the real command, should be normalized not passed through
    let parsed = parse_command_execution("find . -name '*.rs' | xargs wc -l");
    assert!(!live_pipeline_should_passthrough(&parsed, "wc"));
    let decision = live_pipeline_decision(&parsed, "wc");
    assert!(matches!(decision, LivePipelineDecision::Normalize(_)));
}

#[test]
fn pathlist_rejects_error_messages() {
    let lines = vec![
        "error[E0308]: src/main.rs:42:10",
        "error[E0432]: src/lib.rs:15:5",
        "error[E0277]: src/trim.rs:100:20",
        "error[E0308]: src/app.rs:55:8",
    ];
    assert!(!looks_like_path_list(&lines));
}

#[test]
fn table_rejects_two_column_text() {
    let lines = vec!["name  value", "foo   123"];
    assert!(!looks_like_table(&lines));
}

#[test]
fn stage_roles_cover_default_tool_commands() {
    for (name, expected) in [
        ("cat", "source"),
        ("rg", "search"),
        ("grep", "search"),
        ("find", "search"),
        ("fd", "search"),
        ("tree", "search"),
        ("sed", "filter"),
        ("awk", "filter"),
        ("perl", "filter"),
        ("cut", "filter"),
        ("sort", "filter"),
        ("uniq", "filter"),
        ("tr", "filter"),
        ("jq", "filter"),
        ("head", "summarize"),
        ("tail", "summarize"),
        ("wc", "summarize"),
        ("du", "summarize"),
        ("df", "summarize"),
        ("cargo", "build"),
        ("bun", "build"),
        ("dotnet", "build"),
        ("go", "build"),
        ("cmake", "build"),
        ("ctest", "build"),
        ("make", "build"),
        ("ninja", "build"),
        ("node", "build"),
        ("python", "build"),
        ("python3", "build"),
        ("pip", "build"),
        ("uv", "build"),
        ("poetry", "build"),
        ("mvn", "build"),
        ("gradle", "build"),
        ("gradlew", "build"),
        ("javac", "build"),
        ("java", "build"),
        ("bundle", "build"),
        ("composer", "build"),
        ("ps", "build"),
        ("ss", "build"),
        ("netstat", "build"),
        ("systemctl", "build"),
        ("curl", "source"),
        ("docker", "source"),
        ("which", "source"),
        ("readlink", "source"),
        ("file", "source"),
        ("stat", "source"),
        ("psql", "build"),
        ("redis-cli", "build"),
    ] {
        assert_eq!(classify_stage_role(name).as_str(), expected, "{name}");
    }
}

#[test]
fn stage_roles_cover_windows_equivalent_commands() {
    for (name, expected) in [
        ("Get-Content", "source"),
        ("gc", "source"),
        ("type", "source"),
        ("Select-String", "search"),
        ("findstr", "search"),
        ("Get-ChildItem", "source"),
        ("dir", "source"),
        ("Measure-Object", "summarize"),
        ("Select-Object", "summarize"),
        ("Sort-Object", "filter"),
        ("gls", "source"),
        ("ggrep", "search"),
        ("gfind", "search"),
        ("mdfind", "search"),
        ("Get-Clipboard", "source"),
        ("gsed", "filter"),
        ("mdls", "source"),
        ("xattr", "source"),
        ("plutil", "filter"),
        ("pbpaste", "source"),
        ("ghead", "summarize"),
        ("gtail", "summarize"),
        ("guniq", "filter"),
        ("gwc", "summarize"),
        ("gdu", "summarize"),
        ("gdf", "summarize"),
        ("more", "summarize"),
        ("more.com", "summarize"),
        ("open", "source"),
        ("qlmanage", "source"),
    ] {
        assert_eq!(classify_stage_role(name).as_str(), expected, "{name}");
    }
}

#[test]
fn live_pipeline_decision_uses_canonical_current_command_name() {
    let parsed = parse_live_shell_pipeline(
        "Get-Content /tmp/demo.txt | Select-String section | Select-Object -First 1",
    );
    match live_pipeline_decision(&parsed, "Select-Object") {
        LivePipelineDecision::Normalize(selected) => {
            assert_eq!(selected.name, "grep");
            assert_eq!(selected.role.as_str(), "search");
        }
        _ => panic!("expected normalize"),
    }
}

#[test]
fn live_pipeline_decision_handles_select_object_last_via_canonical_stage() {
    let parsed = parse_live_shell_pipeline(
        "Get-Content /tmp/demo.txt | Select-String section | Select-Object -Last 1",
    );
    match live_pipeline_decision(&parsed, "Select-Object") {
        LivePipelineDecision::Normalize(selected) => {
            assert_eq!(selected.name, "grep");
            assert_eq!(selected.role.as_str(), "search");
        }
        _ => panic!("expected normalize"),
    }
}

#[test]
fn classify_common_build_and_test_commands_as_log() {
    for (name, args) in [
        ("bun", vec!["test".to_owned()]),
        ("dotnet", vec!["test".to_owned()]),
        ("go", vec!["test".to_owned(), "./...".to_owned()]),
        ("cmake", vec!["--build".to_owned(), "build".to_owned()]),
        ("ctest", vec!["--output-on-failure".to_owned()]),
        ("make", vec!["test".to_owned()]),
        (
            "ninja",
            vec!["-C".to_owned(), "build".to_owned(), "test".to_owned()],
        ),
        ("node", vec!["--test".to_owned()]),
        (
            "pip",
            vec![
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
        ),
        (
            "uv",
            vec![
                "pip".to_owned(),
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
        ),
        ("poetry", vec!["install".to_owned()]),
        ("mvn", vec!["test".to_owned()]),
        ("gradle", vec!["test".to_owned()]),
        ("gradlew", vec!["build".to_owned()]),
        ("javac", vec!["Main.java".to_owned()]),
        ("java", vec!["-jar".to_owned(), "app.jar".to_owned()]),
        ("bundle", vec!["exec".to_owned(), "rspec".to_owned()]),
        ("composer", vec!["test".to_owned()]),
    ] {
        assert!(
            matches!(classify_command(name, &args), CommandKind::Log),
            "{name}"
        );
    }
}

#[test]
fn search_profile_detected_for_find_results() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..32)
        .map(|idx| format!("src/module_{idx:03}.rs"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "find",
        &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "pathlist");
    assert!(value["pl"].is_object());
}

#[test]
fn file_profile_detected_for_numbered_code_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (1..=40)
        .map(|idx| match idx % 8 {
            1 => format!("{idx:>6}\tpub fn function_{idx}() {{"),
            2 => format!("{idx:>6}\t    let value = {idx};"),
            3 => format!("{idx:>6}\t    println!(\"{{}}\", value);"),
            4 => format!("{idx:>6}\t}}"),
            5 => format!("{idx:>6}\t"),
            6 => format!("{idx:>6}\tpub struct Struct{idx} {{"),
            7 => format!("{idx:>6}\t    field: usize,"),
            _ => format!("{idx:>6}\t}}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "nl",
        &["-ba".to_owned(), "src/lib.rs".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "file");
}

#[test]
fn pathlist_profile_detected_for_find_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..40)
        .map(|idx| format!("/root/project/src/module_{idx:03}.rs"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "find",
        &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 40);
    assert!(value["pl"]["rc"].is_null());
}

#[test]
fn pathlist_profile_detected_for_medium_find_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..16)
        .map(|idx| format!("/root/project/src/module_{idx:03}.rs"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = maybe_normalize_text(
        "find",
        &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
        None,
    )
    .expect("normalize");
    assert!(normalized.is_some());
    let value = value_from_json(&normalized.expect("payload"));
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 16);
    assert!(value["pl"]["rc"].is_null());
}

#[test]
fn pathlist_profile_compacts_real_findcase_under_default_config() {
    let cfg = Config::default();
    let text = [
        "src/tests.rs",
        "src/release.rs",
        "src/path_profile.rs",
        "src/main.rs",
        "src/benchmark.rs",
        "src/e2e_report.rs",
        "src/rollout_io.rs",
        "src/shim.rs",
        "src/trim.rs",
        "src/search_profile.rs",
        "src/rewrite.rs",
        "src/file_profile.rs",
        "src/log_profile.rs",
        "src/app.rs",
        "src/lib.rs",
        "src/adapter.rs",
        "src/rollout_stats.rs",
    ]
    .join("\n");
    let normalized = maybe_normalize_text(
        "head",
        &["-n".to_owned(), "40".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
        Some(("find", "search")),
    )
    .expect("normalize");
    assert!(normalized.is_some(), "expected real findcase to compress");
    let value = value_from_json(&normalized.expect("payload"));
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 17);
    assert!(value["pl"]["rc"].is_null());
    assert_eq!(value["pl"]["d"], "src");
}

#[test]
fn pathlist_profile_detected_for_ls_name_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..40)
        .map(|idx| {
            if idx % 7 == 0 {
                format!("module_{idx:03}")
            } else {
                format!("module_{idx:03}.rs")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = maybe_normalize_text(
        "ls",
        &["src".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
        None,
    )
    .expect("normalize");
    assert!(normalized.is_some());
    let value = value_from_json(&normalized.expect("payload"));
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 40);
    assert!(value["pl"]["rc"].is_null());
}

#[test]
fn git_status_profile_detected_for_status_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "## main...origin/main",
        " M src/main.rs",
        "A  src/lib.rs",
        " D README.md",
        "?? docs/new.md",
    ]
    .join("\n");
    let normalized = normalize_text(
        "git",
        &[
            "status".to_owned(),
            "--short".to_owned(),
            "--branch".to_owned(),
        ],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "gitstatus");
    assert_eq!(value["gs"]["br"], "main...origin/main");
    assert_eq!(value["gs"]["m"], 1);
    assert_eq!(value["gs"]["a"], 1);
    assert_eq!(value["gs"]["d"], 1);
    assert_eq!(value["gs"]["u"], 1);
}

#[test]
fn json_profile_compacts_pretty_json() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = "{\n  \"ok\": true,\n  \"items\": [\n    1,\n    2,\n    3\n  ],\n  \"name\": \"demo\"\n}\n";
    let normalized = normalize_text(
        "curl",
        &["-s".to_owned(), "http://127.0.0.1/demo".to_owned()],
        "stdout",
        CommandKind::Generic,
        text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
    let compacted = value["b"][0].as_str().expect("json body");
    let parsed = value_from_json(compacted);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["items"][1], 2);
    assert_eq!(parsed["name"], "demo");
}

#[test]
fn curl_http_json_output_uses_json_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 42\r\ndate: Fri, 22 May 2026 07:25:29 GMT\r\n\r\n{\"ok\":true,\"items\":[1,2,3],\"name\":\"demo\"}";
    let normalized = normalize_text(
        "curl",
        &[
            "-sS".to_owned(),
            "-i".to_owned(),
            "http://127.0.0.1:8800/demo".to_owned(),
        ],
        "stdout",
        CommandKind::Generic,
        text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
    let compacted = value["b"][0].as_str().expect("json body");
    let parsed = value_from_json(compacted);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["items"][2], 3);
    assert_eq!(parsed["name"], "demo");
}

#[test]
fn curl_json_lines_output_uses_json_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "{\"task_id\":\"a1\",\"status\":\"running\",\"progress\":90,\"error\":\"\"}",
        "{\"task_id\":\"b2\",\"status\":\"running\",\"progress\":89,\"error\":\"\"}",
        "{\"task_id\":\"c3\",\"status\":\"done\",\"progress\":100,\"error\":\"\"}",
    ]
    .join("\n");
    let normalized = normalize_text(
        "curl",
        &[
            "-s".to_owned(),
            "http://127.0.0.1:8000/api/tasks".to_owned(),
        ],
        "stdout",
        CommandKind::Generic,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
}

#[test]
fn curl_header_only_output_does_not_use_json_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-cache\r\nDate: Sun, 17 May 2026 10:40:15 GMT\r\n\r\n";
    let normalized = normalize_text(
        "curl",
        &[
            "-I".to_owned(),
            "-sS".to_owned(),
            "http://127.0.0.1:5173".to_owned(),
        ],
        "stdout",
        CommandKind::Generic,
        text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_ne!(value["p"], "json");
}

#[test]
fn file_profile_prefers_decl_chunks_for_large_code_reads() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "use serde::{Deserialize, Serialize};",
        "use std::collections::HashMap;",
        "",
        "pub struct Config {",
        "    field: usize,",
        "}",
        "",
        "impl Config {",
        "    pub fn load() -> Self {",
        "        Self { field: 1 }",
        "    }",
        "}",
        "",
        "pub fn helper() {",
        "    println!(\"hi\");",
        "}",
    ]
    .join("\n");
    let normalized = normalize_text(
        "cat",
        &["src/lib.rs".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "file");
    assert!(
        value["m"]
            .as_array()
            .expect("matches")
            .iter()
            .any(|chunk| chunk["k"] == "decl")
    );
}

#[test]
fn compare_rollout_reports_savings_for_path_list_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let output = (0..200)
        .map(|idx| format!("/root/project/target/debug/incremental/tke/build-artifact-{idx:03}.o"))
        .collect::<Vec<_>>()
        .join("\n");
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"find /root/project -type f | head -n 200\",\"yield_time_ms\":1000}",
                    "call_id": "call_path_1"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_path_1",
                    "output": format!(
                        "Chunk ID: demo\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 1000\nOutput:\n{output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let raw_stats = collect_rollout_output_stats_detailed(&jsonl, &cfg);
    let rewritten_stats = collect_rollout_output_stats_detailed(&rewritten, &cfg);
    assert!(rewritten_stats.total.approx_tokens < raw_stats.total.approx_tokens);
    assert!(rewritten_stats.total.bytes < raw_stats.total.bytes);
}

#[test]
fn selected_stage_metadata_is_embedded_for_search_pipeline() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let parsed =
        parse_command_execution("cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1");
    let selected = parsed.selected_stage();
    let json = normalize_text_with_stage(
        &selected.name,
        &selected.args,
        "stdout",
        classify_command(&selected.name, &selected.args),
        "2397:SECTION 599\n",
        &cfg,
        Some((&selected.name, selected.role.as_str())),
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["sc"], "rg");
    assert_eq!(value["sr"], "search");
    assert_eq!(value["p"], "search");
}

#[test]
fn normalize_search_pipeline_tail_uses_selected_search_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let parsed = parse_command_execution("find src -name '*.rs' | head -n 40");
    let selected = parsed.selected_stage();
    let text = (0..120)
        .map(|idx| format!("src/module_{idx:03}.rs"))
        .collect::<Vec<_>>()
        .join("\n");
    let json = normalize_text_with_stage(
        &selected.name,
        &selected.args,
        "stdout",
        classify_command(&selected.name, &selected.args),
        &text,
        &cfg,
        Some((&selected.name, selected.role.as_str())),
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["sc"], "find");
    assert_eq!(value["sr"], "search");
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 120);
    assert!(value["pl"]["rc"].is_null());
}

#[test]
fn normalize_build_pipeline_tail_uses_selected_build_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let parsed = parse_command_execution("cargo test -- --nocapture | tail -n 80");
    let selected = parsed.selected_stage();
    let text = repeated_lines("test parser::case ... ok", 120)
        + "\nerror: test failed, to rerun pass --lib\nwarning: deprecated assertion helper\n";
    let json = normalize_text_with_stage(
        &selected.name,
        &selected.args,
        "stdout",
        classify_command(&selected.name, &selected.args),
        &text,
        &cfg,
        Some((&selected.name, selected.role.as_str())),
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["sc"], "cargo");
    assert_eq!(value["sr"], "build");
    assert_eq!(value["p"], "log");
    let haystack = rollout_string_haystack(&json);
    assert!(haystack.contains("error: test failed"));
    assert!(haystack.contains("warning: deprecated assertion helper"));
}

#[test]
fn forced_log_profile_prefers_summary_and_signal_chunks_over_head_tail() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = repeated_lines("test parser::case ... ok", 120)
        + "\nwarning: deprecated assertion helper\nerror: test failed, to rerun pass --lib\n";
    let json = normalize_text(
        "cargo",
        &["test".to_owned(), "--".to_owned(), "--nocapture".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&json);
    assert_eq!(value["p"], "log");
    assert!(value["h"].as_array().is_none_or(|rows| rows.is_empty()));
    assert!(value["ta"].as_array().is_none_or(|rows| rows.is_empty()));
    assert_eq!(value["bd"]["n"], "cargo");
    assert_eq!(value["lg"]["warn"], 1);
    assert_eq!(value["lg"]["fail"], 1);
    let matches = value["m"].as_array().expect("matches");
    assert!(!matches.is_empty());
}

#[test]
fn codex_event_replay_preserves_selected_search_stage() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let line = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_9",
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/tke-codex/huge.txt | rg -n \"SECTION 599\" | head'",
            "aggregated_output": format!("{}\n", repeated_lines("2397:SECTION 599", 180)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    let rewritten = rewrite_codex_jsonl(&line, &cfg)
        .expect("rewrite")
        .expect("changed");
    let value = value_from_json(rewritten.lines().next().expect("line"));
    let nested = value_from_json(
        value["item"]["aggregated_output"]
            .as_str()
            .expect("aggregated_output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
}

#[test]
fn rewrites_exec_command_function_call_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1\",\"yield_time_ms\":1000}",
                    "call_id": "call_1"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{}\n",
                        repeated_lines("2:pub fn beta() {}", 160)
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let second = rewritten.lines().nth(1).expect("second line");
    let value = value_from_json(second);
    let nested = value_from_json(
        value["payload"]["output"]
            .as_str()
            .expect("output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
    assert_eq!(nested["p"], "search");
}

#[test]
fn rewrites_exec_command_error_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1 .\",\"yield_time_ms\":1000}",
                    "call_id": "call_2"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_2",
                    "output": format!(
                        "Chunk ID: 15b935\nWall time: 0.0000 seconds\nProcess exited with code 1\nOriginal token count: 500\nOutput:\n{}\n",
                        repeated_lines("head: error reading '.': Is a directory", 120)
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let second = rewritten.lines().nth(1).expect("second line");
    let value = value_from_json(second);
    let nested = value_from_json(
        value["payload"]["output"]
            .as_str()
            .expect("output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["s"], "stderr");
    assert_eq!(nested["sc"], "rg");
}

#[test]
fn rewrites_shell_command_function_call_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "shell_command",
                "arguments": "{\"command\":\"cat /tmp/demo.rs | rg -n beta | head -n 1\",\"workdir\":\"/tmp\",\"timeout_ms\":10000}",
                "call_id": "call_shell_1"
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_shell_1",
                "output": format!(
                    "Exit code: 0\nWall time: 0.9 seconds\nOutput:\n{}\n",
                    repeated_lines("2:pub fn beta() {}", 160)
                )
            }
        })
        .to_string(),
    ]
    .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let second = rewritten.lines().nth(1).expect("second line");
    let value = value_from_json(second);
    let nested = value_from_json(
        value["payload"]["output"]
            .as_str()
            .expect("output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
    assert_eq!(nested["p"], "search");
}

#[test]
fn rollout_stats_track_shell_command_function_call_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "shell_command",
                "arguments": "{\"command\":\"find src -name '*.rs' | head -n 40\",\"workdir\":\"/tmp\",\"timeout_ms\":10000}",
                "call_id": "call_shell_stats_1"
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_shell_stats_1",
                "output": format!(
                    "Exit code: 0\nWall time: 1.1 seconds\nOutput:\n{}\n",
                    repeated_lines("/tmp/project/src/lib.rs", 160)
                )
            }
        })
        .to_string(),
    ]
    .join("\n");
    assert!(rollout_has_relevant_tool_output(&jsonl));
    let stats = collect_rollout_output_stats_detailed(&jsonl, &cfg);
    assert!(stats.total.approx_tokens > 0);
    assert!(
        stats
            .records
            .iter()
            .any(|record| record.command == "find" && record.profile == "pathlist")
    );
}

#[test]
fn parses_exec_command_envelope_output() {
    let raw = "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\nhello\nworld\n";
    assert_eq!(extract_exec_command_output(raw), Some("hello\nworld\n"));
    assert!(!looks_like_stderr_only_exec_output(raw));
}

#[test]
fn stderr_only_exec_output_detects_tokenized_diagnostics() {
    let raw = "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 1\nOriginal token count: 500\nOutput:\nTraceback (most recent call last):\nPermission denied\n";
    assert!(looks_like_stderr_only_exec_output(raw));
}

#[test]
fn ignores_non_envelope_output_for_exec_command_helpers() {
    let raw = "Process exited with code 1\nerror: build failed\nOutput: missing delimiter line\n";
    assert_eq!(extract_exec_command_output(raw), None);
    assert!(!looks_like_stderr_only_exec_output(raw));
}

#[test]
fn rewrites_claude_tool_result_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
        serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "Bash",
                        "input": {
                            "command": "cat /tmp/demo.rs | rg -n beta | head -n 1"
                        }
                    }
                ]
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": format!("{}\n", repeated_lines("2:pub fn beta() {}", 160))
                    }
                ]
            }
        })
        .to_string(),
    ]
    .join("\n");
    let rewritten = rewrite_claude_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let second = rewritten.lines().nth(1).expect("second line");
    let value = value_from_json(second);
    let content = value["message"]["content"][0]["content"]
        .as_str()
        .expect("content");
    assert!(content.starts_with("__TKE__"));
    let nested = value_from_json(content.trim_start_matches("__TKE__"));
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
    assert_eq!(nested["p"], "search");
}

#[test]
fn pathlist_summary_keeps_first_last_and_skips_omitted_ranges() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "src/tests.rs",
        "src/release.rs",
        "src/path_profile.rs",
        "src/main.rs",
        "src/benchmark.rs",
        "src/e2e_report.rs",
        "src/rollout_io.rs",
        "src/shim.rs",
        "src/trim.rs",
        "src/search_profile.rs",
        "src/rewrite.rs",
        "src/file_profile.rs",
        "src/log_profile.rs",
        "src/app.rs",
        "src/lib.rs",
        "src/adapter.rs",
        "src/rollout_stats.rs",
    ]
    .join("\n");
    let normalized = normalize_text(
        "find",
        &["src".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 17);
    assert_eq!(
        value["pl"]["s"],
        "C=17, F=tests.rs, L=rollout_stats.rs, D=src"
    );
    assert_eq!(value["pl"]["d"], "src");
    assert_eq!(value["pl"]["f"], "tests.rs");
    assert_eq!(value["pl"]["l"], "rollout_stats.rs");
    assert!(
        value.get("o").is_none() || value["o"].as_array().is_some_and(|items| items.is_empty())
    );
}

#[test]
fn pathlist_rollout_haystack_exposes_first_last_and_count() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "src/tests.rs",
        "src/release.rs",
        "src/path_profile.rs",
        "src/main.rs",
        "src/benchmark.rs",
        "src/e2e_report.rs",
        "src/rollout_io.rs",
        "src/shim.rs",
        "src/trim.rs",
        "src/search_profile.rs",
        "src/rewrite.rs",
        "src/file_profile.rs",
        "src/log_profile.rs",
        "src/app.rs",
        "src/lib.rs",
        "src/adapter.rs",
        "src/rollout_stats.rs",
    ]
    .join("\n");
    let normalized = normalize_text(
        "find",
        &["src".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let haystack = rollout_string_haystack(&normalized);
    for fragment in [
        "src/tests.rs",
        "src/rollout_stats.rs",
        "C=17",
        "F=tests.rs",
        "L=rollout_stats.rs",
    ] {
        assert!(
            haystack.contains(fragment),
            "pathlist haystack missing `{fragment}`"
        );
    }
}

#[test]
fn pathlist_profile_rejects_arrow_mapping_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "src/lib.rs -> /tmp/build/lib.rs",
        "src/main.rs -> /tmp/build/main.rs",
        "src/tests.rs -> /tmp/build/tests.rs",
        "src/trim.rs -> /tmp/build/trim.rs",
    ]
    .join("\n");
    let normalized = normalize_text(
        "find",
        &["src".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_ne!(value["p"], "pathlist");
}

#[test]
fn rewrites_claude_tool_result_text_block_array() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
        serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_2",
                        "name": "bash",
                        "input": {
                            "cmd": "cargo test -- --nocapture | tail -n 80"
                        }
                    }
                ]
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_2",
                        "content": [
                            {
                                "type": "text",
                                "text": format!("{}\n", repeated_lines("error: test failed", 120))
                            }
                        ]
                    }
                ]
            }
        })
        .to_string(),
    ]
    .join("\n");
    let rewritten = rewrite_agent_transcript(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let second = rewritten.lines().nth(1).expect("second line");
    let value = value_from_json(second);
    let text = value["message"]["content"][0]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text.starts_with("__TKE__"));
    let nested = value_from_json(text.trim_start_matches("__TKE__"));
    assert_eq!(nested["sc"], "cargo");
    assert_eq!(nested["sr"], "build");
    assert_eq!(nested["p"], "log");
}

#[test]
fn generic_rewrite_alias_matches_agent_rewriter() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_generic_1",
            "type": "command_execution",
            "command": "pwsh -Command \"Get-Content /tmp/demo.rs | rg -n beta | Select-Object -First 1\"",
            "aggregated_output": format!("{}\n", repeated_lines("2:pub fn beta() {}", 160)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    let via_generic = rewrite_generic_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let via_agent = rewrite_agent_transcript(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    assert_eq!(via_generic, via_agent);
}

#[test]
fn compare_rollout_reports_savings() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let large_output = (0..80)
        .map(|idx| format!("src/demo.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx))
        .collect::<Vec<_>>()
        .join("\n");
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1\",\"yield_time_ms\":1000}",
                    "call_id": "call_3"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_3",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{large_output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let raw_stats = collect_rollout_output_stats_detailed(&jsonl, &cfg);
    let rewritten_stats = collect_rollout_output_stats_detailed(&rewritten, &cfg);
    let report = RolloutCompareReport::from_stats(
        Path::new("/tmp/demo.jsonl"),
        true,
        raw_stats,
        rewritten_stats,
    );
    assert!(report.changed);
    assert_eq!(report.raw_fields, 1);
    assert_eq!(report.rewritten_fields, 1);
    assert!(
        report.bytes_saved > 0,
        "report bytes_saved={}",
        report.bytes_saved
    );
    assert!(
        report.tokens_saved > 0,
        "report tokens_saved={}",
        report.tokens_saved
    );
}

#[test]
fn compare_rollout_reports_grouped_search_savings() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let large_output = (0..120)
        .map(|idx| format!("src/lib.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx))
        .chain((0..80).map(|idx| format!("src/main.rs:{}:pub fn gamma_{}() {{}}", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n");
    let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"rg -n 'fn' src\",\"yield_time_ms\":1000}",
                    "call_id": "call_rg_group"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_rg_group",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{large_output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
    let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
        .expect("rewrite")
        .expect("changed");
    let raw_stats = collect_rollout_output_stats(&jsonl, &cfg);
    let rewritten_stats = collect_rollout_output_stats(&rewritten, &cfg);
    assert!(rewritten_stats.approx_tokens < raw_stats.approx_tokens);
    assert!(rewritten_stats.bytes < raw_stats.bytes);
}

#[test]
fn parse_capture_interactive_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "capture-interactive".to_owned(),
            "--source".to_owned(),
            "/tmp/source.jsonl".to_owned(),
            "--output".to_owned(),
            "/tmp/out".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::CaptureInteractive { source, output } => {
            assert_eq!(source, Some(PathBuf::from("/tmp/source.jsonl")));
            assert_eq!(output, Some(PathBuf::from("/tmp/out")));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_activate_accepts_shell() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "activate".to_owned(),
            "--shell".to_owned(),
            "powershell".to_owned(),
            "codex".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Activate {
            agents,
            shim_dir,
            shell,
        } => {
            assert_eq!(agents, vec!["codex".to_owned()]);
            assert!(shim_dir.is_none());
            assert_eq!(shell, Some(ShellKind::PowerShell));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_run_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "run".to_owned(),
            "--shim-dir".to_owned(),
            "/tmp/tke-shims".to_owned(),
            "codex".to_owned(),
            "exec".to_owned(),
            "--json".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Run {
            name,
            args,
            shim_dir,
        } => {
            assert_eq!(name, "codex");
            assert_eq!(args, vec!["exec", "--json"]);
            assert_eq!(shim_dir, Some(PathBuf::from("/tmp/tke-shims")));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_tty_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "tty".to_owned(),
            "--shim-dir".to_owned(),
            "/tmp/tke-shims".to_owned(),
            "codex".to_owned(),
            "--no-alt-screen".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Tty {
            name,
            args,
            shim_dir,
        } => {
            assert_eq!(name, "codex");
            assert_eq!(args, vec!["--no-alt-screen"]);
            assert_eq!(shim_dir, Some(PathBuf::from("/tmp/tke-shims")));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_agent_alias_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "codex".to_owned(),
            "exec".to_owned(),
            "--json".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Run {
            name,
            args,
            shim_dir,
        } => {
            assert_eq!(name, "codex");
            assert_eq!(args, vec!["exec", "--json"]);
            assert!(shim_dir.is_none());
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_tk_driver_alias_dispatch() {
    let dispatch = parse_dispatch(
        "tk",
        vec!["tk".to_owned(), "codex".to_owned(), "exec".to_owned()],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Run {
            name,
            args,
            shim_dir,
        } => {
            assert_eq!(name, "codex");
            assert_eq!(args, vec!["exec"]);
            assert!(shim_dir.is_none());
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_shim_subcommand_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "shim".to_owned(),
            "rg".to_owned(),
            "-n".to_owned(),
            "fn".to_owned(),
            "src".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::ShimExec { name, args } => {
            assert_eq!(name, "rg");
            assert_eq!(args, vec!["-n", "fn", "src"]);
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_stats_dispatch() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "stats".to_owned(),
            "--source".to_owned(),
            "/tmp/rollouts".to_owned(),
            "--limit".to_owned(),
            "12".to_owned(),
            "--json".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Stats {
            sources,
            limit,
            filter,
            group_by,
            changed_only,
            refresh,
            top,
            sort_by,
            json,
        } => {
            assert_eq!(sources, vec![PathBuf::from("/tmp/rollouts")]);
            assert_eq!(limit, Some(12));
            assert!(matches!(filter, crate::stats::UsageStatsFilter::None));
            assert!(matches!(group_by, crate::stats::UsageStatsGroupBy::Day));
            assert!(!changed_only);
            assert!(!refresh);
            assert_eq!(top, 10);
            assert!(matches!(sort_by, crate::stats::UsageStatsSortBy::Saved));
            assert!(json);
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_stats_dispatch_with_profile_and_group() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "stats".to_owned(),
            "--profile".to_owned(),
            "pathlist".to_owned(),
            "--by".to_owned(),
            "command".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Stats {
            filter,
            group_by,
            changed_only,
            refresh,
            top,
            sort_by,
            ..
        } => {
            match filter {
                crate::stats::UsageStatsFilter::Profile(value) => {
                    assert_eq!(value, "pathlist");
                }
                other => panic!("unexpected filter: {other:?}"),
            }
            assert!(matches!(group_by, crate::stats::UsageStatsGroupBy::Command));
            assert!(!changed_only);
            assert!(!refresh);
            assert_eq!(top, 10);
            assert!(matches!(sort_by, crate::stats::UsageStatsSortBy::Saved));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_stats_dispatch_with_changed_only_top_and_sort() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "stats".to_owned(),
            "--command".to_owned(),
            "cargo".to_owned(),
            "--changed-only".to_owned(),
            "--top".to_owned(),
            "7".to_owned(),
            "--sort".to_owned(),
            "ratio".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Stats {
            filter,
            changed_only,
            refresh,
            top,
            sort_by,
            ..
        } => {
            match filter {
                crate::stats::UsageStatsFilter::Command(value) => {
                    assert_eq!(value, "cargo");
                }
                other => panic!("unexpected filter: {other:?}"),
            }
            assert!(changed_only);
            assert!(!refresh);
            assert_eq!(top, 7);
            assert!(matches!(sort_by, crate::stats::UsageStatsSortBy::Ratio));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_stats_dispatch_with_low_ratio_sort() {
    let dispatch = parse_dispatch(
        "tke",
        vec![
            "tke".to_owned(),
            "stats".to_owned(),
            "--by".to_owned(),
            "command".to_owned(),
            "--sort".to_owned(),
            "low-ratio".to_owned(),
        ],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Stats {
            group_by,
            refresh,
            sort_by,
            ..
        } => {
            assert!(matches!(group_by, crate::stats::UsageStatsGroupBy::Command));
            assert!(!refresh);
            assert!(matches!(sort_by, crate::stats::UsageStatsSortBy::LowRatio));
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn parse_stats_dispatch_with_refresh() {
    let dispatch = parse_dispatch(
        "tke",
        vec!["tke".to_owned(), "stats".to_owned(), "--refresh".to_owned()],
    )
    .expect("dispatch");
    match dispatch {
        Dispatch::Stats { refresh, .. } => assert!(refresh),
        other => panic!("unexpected dispatch: {other:?}"),
    }
}

#[test]
fn detailed_rollout_stats_track_profile_breakdown() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let jsonl = [
        serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "git status --short --branch",
                "aggregated_output": "__TKE__{\"v\":1,\"cmd\":\"git\",\"p\":\"gitstatus\",\"gs\":{\"br\":\"main\",\"m\":1}}"
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_2",
                "type": "command_execution",
                "command": "find src -type f",
                "aggregated_output": "__TKE__{\"v\":1,\"cmd\":\"find\",\"sc\":\"find\",\"p\":\"pathlist\",\"c\":17,\"pl\":{\"d\":\"src\"}}"
            }
        })
        .to_string(),
    ]
    .join("\n");
    let stats = collect_rollout_output_stats_detailed(&jsonl, &cfg);
    assert!(stats.breakdown.by_profile.contains_key("gitstatus"));
    assert!(stats.breakdown.by_profile.contains_key("pathlist"));
    assert!(stats.breakdown.by_command.contains_key("git"));
    assert!(stats.breakdown.by_command.contains_key("find"));
}

#[test]
fn usage_stats_report_includes_top_profiles_and_commands() {
    let base = temp_test_dir("stats-report");
    let sessions = base.join(".codex/sessions/2026/05/25");
    fs::create_dir_all(&sessions).expect("mkdir");
    let rollout = sessions.join("demo.jsonl");
    fs::write(
        &rollout,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "id": "item_1",
                    "type": "command_execution",
                    "command": "find src -type f",
                    "aggregated_output": repeated_lines("/root/project/src/lib.rs", 60)
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "id": "item_2",
                    "type": "command_execution",
                    "command": "cargo test",
                    "aggregated_output": repeated_lines("error: build failed", 40)
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write rollout");
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_usage_stats_report(
        vec![base.join(".codex/sessions")],
        Some(10),
        &UsageStatsFilter::None,
        UsageStatsGroupBy::Day,
        false,
        false,
        5,
        UsageStatsSortBy::Saved,
        &cfg,
    )
    .expect("report");
    let value = serde_json::to_value(report).expect("json");
    assert_eq!(value["samples"], 1);
    assert!(
        value["top_profiles"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        value["top_commands"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        value["bottom_profiles"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert!(
        value["bottom_commands"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
}

#[test]
fn rollout_tool_output_probe_ignores_plain_messages() {
    let jsonl = [
        serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "plain assistant text"}]
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "user",
            "message": {
                "content": [{"type": "text", "text": "plain user text"}]
            }
        })
        .to_string(),
    ]
    .join("\n");
    assert!(!rollout_has_relevant_tool_output(&jsonl));
}

#[test]
fn usage_stats_custom_sources_do_not_write_default_cache() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let base = temp_test_dir("stats-cache-skip");
    let sessions = base.join(".codex/sessions/2026/05/25");
    fs::create_dir_all(&sessions).expect("mkdir");
    let rollout = sessions.join("demo.jsonl");
    fs::write(
        &rollout,
        serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "find src -type f",
                "aggregated_output": repeated_lines("/root/project/src/lib.rs", 60)
            }
        })
        .to_string(),
    )
    .expect("write rollout");

    let project = base.join("project");
    fs::create_dir_all(project.join(".tke")).expect("project dir");
    let original_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&project).expect("chdir");

    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_usage_stats_report(
        vec![base.join(".codex/sessions")],
        Some(10),
        &UsageStatsFilter::None,
        UsageStatsGroupBy::Day,
        false,
        false,
        5,
        UsageStatsSortBy::Saved,
        &cfg,
    )
    .expect("report");
    let value = serde_json::to_value(report).expect("json");

    std::env::set_current_dir(original_cwd).expect("restore cwd");

    assert_eq!(value["samples"], 1);
    assert!(!project.join(".tke/stats-cache.json").exists());
}

#[test]
fn compare_e2e_report_pairs_raw_and_tke_samples() {
    let base = temp_test_dir("e2e-report");
    fs::create_dir_all(base.join(".tmp-claude-e2e")).expect("mkdir");
    let raw = base.join(".tmp-claude-e2e/rgcase.raw.stream.jsonl");
    let wrapped = base.join(".tmp-claude-e2e/rgcase.tke.stream.jsonl");
    fs::write(
        &raw,
        [
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "call_1",
                            "name": "Bash",
                            "input": {"command": "rg -n fn src/lib.rs"}
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_1",
                            "content": "10:fn alpha()\n11:fn beta()\n"
                        }
                    ]
                },
                "tool_use_result": {
                    "stdout": "__TKE__{\"v\":1,\"cmd\":\"rg\",\"p\":\"search\",\"h\":[\"10:fn alpha()\"],\"ta\":[\"11:fn beta()\"],\"m\":[{\"k\":\"result\",\"r\":[0,2],\"l\":[\"10:fn alpha()\",\"11:fn beta()\"]}]}"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=normalize_text, FILE=src/tests.rs, KIND=test-focus"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write raw");
    fs::write(
        &wrapped,
        [
            serde_json::json!({
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "call_1",
                            "name": "Bash",
                            "input": {"command": "rg -n fn src/lib.rs"}
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_1",
                            "content": "__TKE__{\"v\":1,\"cmd\":\"rg\",\"p\":\"search\",\"h\":[\"10:fn alpha()\"],\"ta\":[\"11:fn beta()\"],\"m\":[{\"k\":\"result\",\"r\":[0,2],\"l\":[\"10:fn alpha()\",\"11:fn beta()\"]}]}"
                        }
                    ]
                },
                "tool_use_result": {
                    "stdout": "10:fn alpha()\n11:fn beta()\n"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=normalize_text_with_stage, FILE=src/tests.rs, KIND=function"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write wrapped");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-claude-e2e")],
        Some("claude"),
        &Config::default(),
    )
    .expect("report");
    assert_eq!(report.cases.len(), 1);
    let case = &report.cases[0];
    assert_eq!(case.agent, "claude");
    assert_eq!(case.name, "rgcase");
    assert_eq!(case.baseline.mode, "raw");
    assert!(!case.baseline.tool_has_tke);
    assert_eq!(
        case.baseline.result,
        "STAGE=normalize_text, FILE=src/tests.rs, KIND=test-focus"
    );
    assert_eq!(case.baseline.correctness.status, "pass");
    assert_eq!(
        case.baseline.result_fields.get("FILE").map(String::as_str),
        Some("src/tests.rs")
    );
    assert_eq!(case.variants.len(), 1);
    let variant = &case.variants[0];
    assert_eq!(variant.mode, "tke");
    assert!(variant.sample.tool_has_tke);
    assert!(!variant.exact_result_match);
    assert!(variant.semantic_result_match);
    assert!(variant.expected_result_match);
    assert_eq!(variant.verdict, "correct_but_not_saved");
    assert_eq!(
        variant.sample.result,
        "STAGE=normalize_text_with_stage, FILE=src/tests.rs, KIND=function"
    );
}

#[test]
fn compare_e2e_report_marks_wrong_variant_when_expected_fields_fail() {
    let base = temp_test_dir("e2e-report-wrong");
    fs::create_dir_all(base.join(".tmp-codex-e2e")).expect("mkdir");
    let raw = base.join(".tmp-codex-e2e/rgcase.raw.jsonl");
    let wrong = base.join(".tmp-codex-e2e/rgcase.rtk-direct.jsonl");
    fs::write(
        &raw,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": repeated_lines("10: long raw line", 40)
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text, FILE=src/tests.rs, KIND=test-focus"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write raw");
    fs::write(
        &wrong,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "1 result in src/shim.rs"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text_with_stage, FILE=src/shim.rs, KIND=function"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write wrong");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-codex-e2e")],
        Some("codex"),
        &Config::default(),
    )
    .expect("report");
    let case = &report.cases[0];
    let variant = &case.variants[0];
    assert_eq!(variant.mode, "rtk-direct");
    assert!(!variant.expected_result_match);
    assert!(!variant.semantic_result_match);
    assert_eq!(variant.verdict, "saved_but_wrong");
    assert!(
        variant
            .sample
            .correctness
            .notes
            .iter()
            .any(|note| note.contains("src/tests.rs"))
    );
}

#[test]
fn compare_e2e_report_marks_gateway_timeout_from_json_value() {
    let base = temp_test_dir("e2e-report-gateway");
    fs::create_dir_all(base.join(".tmp-codex-e2e")).expect("mkdir");
    let raw = base.join(".tmp-codex-e2e/buildcase.raw.jsonl");
    let wrapped = base.join(".tmp-codex-e2e/buildcase.tke.jsonl");
    fs::write(
        &raw,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": repeated_lines("Compiling crate", 40)
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=build, FILE=src/tests.rs, KIND=test-focus"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write raw");
    fs::write(
        &wrapped,
        [
            serde_json::json!({
                "type": "response.failed",
                "error": {
                    "message": "API Error: 504 origin_gateway_timeout"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=build, FILE=src/tests.rs, KIND=test-focus"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write wrapped");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-codex-e2e")],
        Some("codex"),
        &Config::default(),
    )
    .expect("report");
    let case = &report.cases[0];
    let variant = case
        .variants
        .iter()
        .find(|variant| variant.mode == "tke")
        .expect("tke variant");
    assert_eq!(variant.sample.correctness.status, "gateway_error");
}

#[test]
fn compare_e2e_report_grades_findcase_and_buildcase_expectations() {
    let base = temp_test_dir("e2e-report-extra-cases");
    fs::create_dir_all(base.join(".tmp-codex-e2e")).expect("mkdir");

    let find_raw = base.join(".tmp-codex-e2e/findcase.raw.jsonl");
    let find_tke = base.join(".tmp-codex-e2e/findcase.tke.jsonl");
    fs::write(
        &find_raw,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": repeated_lines("src/tests.rs", 17)
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=find, FILE=src/tests.rs, COUNT=17"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write find raw");
    fs::write(
        &find_tke,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "__TKE__{\"v\":1,\"cmd\":\"head\",\"sc\":\"find\",\"sr\":\"search\",\"p\":\"pathlist\",\"c\":17,\"pl\":{}}"
                }
            }).to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=find, FILE=src/tests.rs, COUNT=17"
                }
            }).to_string(),
        ].join("\n"),
    ).expect("write find tke");

    let build_raw = base.join(".tmp-codex-e2e/buildcase.raw.jsonl");
    let build_tke = base.join(".tmp-codex-e2e/buildcase.tke.jsonl");
    fs::write(
        &build_raw,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "test result: ok. 100 passed; 0 failed; 0 ignored"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=cargo, FILE=src/lib.rs, COUNT=0"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write build raw");
    fs::write(
        &build_tke,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "__TKE__{\"v\":1,\"cmd\":\"tail\",\"sc\":\"cargo\",\"sr\":\"build\",\"p\":\"log\",\"lg\":{\"fail\":0}}"
                }
            }).to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=cargo, FILE=src/lib.rs, COUNT=0"
                }
            }).to_string(),
        ].join("\n"),
    ).expect("write build tke");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-codex-e2e")],
        Some("codex"),
        &Config::default(),
    )
    .expect("report");

    let find_case = report
        .cases
        .iter()
        .find(|case| case.name == "findcase")
        .expect("find case");
    assert_eq!(find_case.baseline.correctness.status, "pass");
    assert!(find_case.variants[0].expected_result_match);

    let build_case = report
        .cases
        .iter()
        .find(|case| case.name == "buildcase")
        .expect("build case");
    assert_eq!(build_case.baseline.correctness.status, "pass");
    assert!(build_case.variants[0].expected_result_match);
}

#[test]
fn compare_e2e_report_shows_positive_tool_savings_for_large_tke_payload() {
    let base = temp_test_dir("e2e-report-large");
    fs::create_dir_all(base.join(".tmp-claude-e2e")).expect("mkdir");
    let raw = base.join(".tmp-claude-e2e/large.raw.stream.jsonl");
    let wrapped = base.join(".tmp-claude-e2e/large.tke.stream.jsonl");
    let raw_tool = repeated_lines("123: very long benchmark match line", 120);
    let wrapped_tool = "__TKE__{\"v\":1,\"cmd\":\"rg\",\"p\":\"search\",\"h\":[\"123: very long benchmark match line\"],\"o\":[[1,120]],\"st\":{\"tb\":4096}}";
    fs::write(
        &raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_1",
                            "content": raw_tool
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "OK"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write raw");
    fs::write(
        &wrapped,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_1",
                            "content": wrapped_tool
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "OK"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write wrapped");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-claude-e2e")],
        Some("claude"),
        &Config::default(),
    )
    .expect("report");
    let case = report
        .cases
        .iter()
        .find(|case| case.name == "large")
        .expect("large case");
    assert_eq!(case.variants.len(), 1);
    let variant = &case.variants[0];
    assert!(variant.tool_bytes_saved > 0);
    assert!(variant.tool_tokens_saved > 0);
}

#[test]
fn compare_e2e_report_grades_claude_live_cases_using_builtin_expectations() {
    let base = temp_test_dir("claude-live-e2e-cases");
    fs::create_dir_all(base.join(".tmp-claude-e2e")).expect("mkdir");

    let find_raw = base.join(".tmp-claude-e2e/findcase.raw.stream.jsonl");
    fs::write(
        &find_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_find_raw",
                            "content": repeated_lines("src/tests.rs", 17)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write find raw");

    let livefind = base.join(".tmp-claude-e2e/findcase.tke.stream.jsonl");
    fs::write(
        &livefind,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_find",
                            "content": "__TKE__{\"v\":1,\"cmd\":\"find\",\"sc\":\"find\",\"sr\":\"search\",\"p\":\"pathlist\",\"c\":17}"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write livefind");

    let build_raw = base.join(".tmp-claude-e2e/buildcase.raw.stream.jsonl");
    fs::write(
        &build_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_build_raw",
                            "content": "test result: ok. 104 passed; 0 failed; 0 ignored; 0 measured"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=cargo test --lib -- --nocapture\nFILE=src/lib.rs\nCOUNT=0"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write build raw");

    let livebuild = base.join(".tmp-claude-e2e/buildcase.tke.stream.jsonl");
    fs::write(
        &livebuild,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_build",
                            "content": "__TKE__{\"v\":1,\"cmd\":\"cargo\",\"sc\":\"cargo\",\"sr\":\"build\",\"p\":\"log\",\"lg\":{\"fail\":0}}"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=cargo test --lib -- --nocapture\nFILE=src/lib.rs\nCOUNT=0"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write livebuild");

    let rg_raw = base.join(".tmp-claude-e2e/rgcase.raw.stream.jsonl");
    fs::write(
        &rg_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_rg_raw",
                            "content": repeated_lines("src/tests.rs:10:assert benchmark normalize claude", 8)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg -n \"assert|benchmark|normalize|claude\" src/tests.rs\nFILE=src/tests.rs\nKIND=search"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write rg raw");

    let liverg = base.join(".tmp-claude-e2e/rgcase.tke.stream.jsonl");
    fs::write(
        &liverg,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_rg",
                            "content": "__TKE__{\"v\":1,\"cmd\":\"rg\",\"sc\":\"rg\",\"sr\":\"search\",\"p\":\"search\"}"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg -n \"assert|benchmark|normalize|claude\" src/tests.rs\nFILE=src/tests.rs\nKIND=search"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write liverg");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-claude-e2e")],
        Some("claude"),
        &Config::default(),
    )
    .expect("report");

    let find_case = report
        .cases
        .iter()
        .find(|case| case.name == "findcase")
        .expect("findcase case");
    assert_eq!(find_case.variants[0].sample.correctness.status, "pass");

    let build_case = report
        .cases
        .iter()
        .find(|case| case.name == "buildcase")
        .expect("buildcase case");
    assert_eq!(build_case.variants[0].sample.correctness.status, "pass");

    let rg_case = report
        .cases
        .iter()
        .find(|case| case.name == "rgcase")
        .expect("rgcase case");
    assert_eq!(rg_case.variants[0].sample.correctness.status, "pass");
}

#[test]
fn compare_e2e_report_grades_claude_fair_cases_using_builtin_expectations() {
    let base = temp_test_dir("claude-fair-e2e-cases");
    fs::create_dir_all(base.join(".tmp-claude-e2e-fair")).expect("mkdir");

    let fairfind_raw = base.join(".tmp-claude-e2e-fair/fairfind.raw.stream.jsonl");
    fs::write(
        &fairfind_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairfind_raw",
                            "content": repeated_lines("src/rollout_stats.rs", 17)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg --files src | head -n 40\nFILE=src/rollout_stats.rs\nCOUNT=15"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairfind raw");

    let fairfind_hook = base.join(".tmp-claude-e2e-fair/fairfind.rtk-hook.stream.jsonl");
    fs::write(
        &fairfind_hook,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairfind_hook",
                            "content": repeated_lines("src/rollout_stats.rs", 17)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg --files src | head -n 40\nFILE=src/rollout_stats.rs\nCOUNT=15"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairfind hook");

    let fairbuild_raw = base.join(".tmp-claude-e2e-fair/fairbuild.raw.stream.jsonl");
    fs::write(
        &fairbuild_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairbuild_raw",
                            "content": "test result: ok. 105 passed; 0 failed; 0 ignored; 0 measured"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=cargo test --lib -- --nocapture | tail -n 80\nFILE=src/lib.rs\nCOUNT=0"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairbuild raw");

    let fairbuild_hook = base.join(".tmp-claude-e2e-fair/fairbuild.rtk-hook.stream.jsonl");
    fs::write(
        &fairbuild_hook,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairbuild_hook",
                            "content": "test result: ok. 105 passed; 0 failed; 0 ignored; 0 measured"
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=cargo test --lib -- --nocapture | tail -n 80\nFILE=src/lib.rs\nCOUNT=0"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairbuild hook");

    let fairrg_raw = base.join(".tmp-claude-e2e-fair/fairrg.raw.stream.jsonl");
    fs::write(
        &fairrg_raw,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairrg_raw",
                            "content": repeated_lines("src/tests.rs:10:normalize_text compare-e2e benchmark-commands", 8)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/tests.rs\nKIND=search"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairrg raw");

    let fairrg_hook = base.join(".tmp-claude-e2e-fair/fairrg.rtk-hook.stream.jsonl");
    fs::write(
        &fairrg_hook,
        [
            serde_json::json!({
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "call_fairrg_hook",
                            "content": repeated_lines("src/tests.rs:10:normalize_text compare-e2e benchmark-commands", 8)
                        }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "result",
                "result": "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/tests.rs\nKIND=search"
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write fairrg hook");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-claude-e2e-fair")],
        Some("claude"),
        &Config::default(),
    )
    .expect("report");

    let fairfind_case = report
        .cases
        .iter()
        .find(|case| case.name == "fairfind")
        .expect("fairfind case");
    assert_eq!(fairfind_case.variants[0].sample.correctness.status, "pass");

    let fairbuild_case = report
        .cases
        .iter()
        .find(|case| case.name == "fairbuild")
        .expect("fairbuild case");
    assert_eq!(fairbuild_case.variants[0].sample.correctness.status, "pass");

    let fairrg_case = report
        .cases
        .iter()
        .find(|case| case.name == "fairrg")
        .expect("fairrg case");
    assert_eq!(fairrg_case.variants[0].sample.correctness.status, "pass");
}

#[test]
fn compare_e2e_report_supports_multiple_variants_including_rtk() {
    let base = temp_test_dir("e2e-report-rtk");
    fs::create_dir_all(base.join(".tmp-codex-e2e")).expect("mkdir");
    let raw = base.join(".tmp-codex-e2e/rgcase.raw.jsonl");
    let tke = base.join(".tmp-codex-e2e/rgcase.tke.jsonl");
    let rtk = base.join(".tmp-codex-e2e/rgcase.rtk-direct.jsonl");
    fs::write(
        &raw,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": repeated_lines("10: long raw line", 80)
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text, FILE=src/tests.rs, KIND=test-focus"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write raw");
    fs::write(
        &tke,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "__TKE__{\"v\":1,\"cmd\":\"rg\",\"p\":\"search\",\"h\":[\"10: long raw line\"],\"o\":[[1,80]]}"
                }
            }).to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text_with_stage, FILE=src/tests.rs, KIND=function"
                }
            }).to_string(),
        ].join("\n"),
    ).expect("write tke");
    fs::write(
        &rtk,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "1 result in src/tests.rs"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text_with_stage, FILE=src/shim.rs, KIND=function"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write rtk");
    let rtk_rules = base.join(".tmp-codex-e2e/rgcase.rtk-codex-rules.jsonl");
    fs::write(
        &rtk_rules,
        [
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "aggregated_output": "1 result in src/tests.rs"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "item.completed",
                "item": {
                    "type": "agent_message",
                    "text": "STAGE=normalize_text, FILE=src/tests.rs, KIND=test-focus"
                }
            })
            .to_string(),
        ]
        .join("\n"),
    )
    .expect("write rtk rules");

    let report = build_e2e_compare_report(
        vec![base.join(".tmp-codex-e2e")],
        Some("codex"),
        &Config::default(),
    )
    .expect("report");
    assert_eq!(report.cases.len(), 1);
    assert_eq!(report.summary.len(), 1);
    let summary = report
        .summary
        .iter()
        .find(|summary| summary.agent == "codex")
        .expect("codex summary");
    assert_eq!(summary.cases, 1);
    assert_eq!(summary.variants, 3);
    assert_eq!(summary.saved_and_correct, 2);
    assert_eq!(summary.correct_but_not_saved, 0);
    assert_eq!(summary.saved_but_wrong, 1);
    assert_eq!(summary.wrong_and_not_saved, 0);
    assert!(summary.total_tool_tokens_saved > 0);
    let case = &report.cases[0];
    assert_eq!(case.variants.len(), 3);
    assert!(case.variants.iter().any(|variant| variant.mode == "tke"));
    assert!(
        case.variants
            .iter()
            .any(|variant| variant.mode == "rtk-direct")
    );
    assert!(
        case.variants
            .iter()
            .any(|variant| variant.mode == "rtk-codex-rules")
    );
    assert!(
        case.variants
            .iter()
            .find(|variant| variant.mode == "tke")
            .expect("tke")
            .expected_result_match
    );
    assert!(
        !case
            .variants
            .iter()
            .find(|variant| variant.mode == "rtk-direct")
            .expect("rtk")
            .expected_result_match
    );
}

#[test]
fn benchmark_report_contains_expected_cases() {
    let cfg = Config::default();
    let report = build_benchmark_report(&cfg).expect("benchmark");
    for name in [
        "cat_code",
        "sed_code",
        "bat_code",
        "nl_code",
        "awk_code",
        "cut_code",
        "head_code",
        "tail_code",
        "rg_code",
        "grep_code",
        "git_grep_code",
        "find_paths",
        "git_ls_files",
        "fd_paths",
        "tree_paths",
        "sort_paths",
        "uniq_paths",
        "ls_long",
        "ls_names",
        "wc_summary",
        "git_diff",
        "cargo_build",
        "pytest_run",
        "npm_test",
        "pnpm_test",
        "yarn_test",
        "bun_test",
        "dotnet_test",
        "go_test",
        "cmake_build",
        "ctest_run",
        "make_build",
        "ninja_build",
        "node_test",
        "pip_install",
        "uv_install",
        "poetry_install",
        "mvn_test",
        "gradle_test",
        "gradlew_build",
        "javac_build",
        "java_run",
        "bundle_test",
        "composer_test",
        "python_log",
        "python3_log",
        "python_unittest",
        "ps_table",
        "ss_table",
        "netstat_table",
        "systemctl_table",
        "docker_ps_table",
        "du_table",
        "df_table",
        "jq_json",
        "curl_json",
        "tr_code",
        "perl_code",
        "xargs_cat",
    ] {
        assert!(report.cases.iter().any(|case| case.name == name), "{name}");
    }
    for name in [
        "codex_api_trace_rollout_savings",
        "codex_api_trace_default_tool_coverage",
        "codex_interactive_trace_selected_search_stage",
        "codex_interactive_trace_selected_find_stage",
        "codex_interactive_trace_selected_build_stage",
        "claude_bash_trace_selected_search_stage",
        "claude_bash_trace_selected_find_stage",
        "claude_bash_trace_selected_diff_stage",
        "claude_bash_trace_selected_build_stage",
        "claude_bash_trace_complex_triage_task",
        "claude_bash_trace_complex_code_trace_task",
        "claude_bash_trace_complex_stacktrace_task",
        "claude_bash_trace_complex_stacktrace_diff_task",
        "claude_bash_trace_complex_root_cause_task",
        "claude_bash_trace_answer_consistency_task",
        "claude_bash_trace_candidate_root_cause_task",
        "claude_bash_trace_misleading_signal_task",
        "claude_bash_trace_cross_file_causality_task",
        "claude_bash_trace_negative_evidence_task",
        "claude_bash_trace_temporal_causality_task",
        "claude_bash_trace_symbol_collision_task",
        "claude_bash_trace_reversal_task",
        "claude_rtk_hook_trace_selected_find_stage",
        "claude_rtk_hook_trace_selected_search_stage",
        "claude_rtk_hook_trace_selected_diff_stage",
        "claude_rtk_hook_trace_selected_build_stage",
        "claude_rtk_hook_trace_complex_triage_task",
        "claude_rtk_hook_trace_complex_code_trace_task",
        "claude_rtk_hook_trace_complex_stacktrace_task",
        "claude_rtk_hook_trace_complex_stacktrace_diff_task",
        "claude_rtk_hook_trace_complex_root_cause_task",
        "claude_rtk_hook_trace_answer_consistency_task",
        "claude_rtk_hook_trace_candidate_root_cause_task",
        "claude_rtk_hook_trace_misleading_signal_task",
        "claude_rtk_hook_trace_cross_file_causality_task",
        "claude_rtk_hook_trace_negative_evidence_task",
        "claude_rtk_hook_trace_temporal_causality_task",
        "claude_rtk_hook_trace_symbol_collision_task",
        "claude_rtk_hook_trace_reversal_task",
    ] {
        assert!(report.tasks.iter().any(|task| task.name == name), "{name}");
    }
}

#[test]
fn benchmark_report_shows_positive_savings_for_core_cases() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    for case in &report.cases {
        match case.expected.as_str() {
            "compress" => {
                assert!(case.tokens_saved > 0, "{}", case.name);
                assert!(case.bytes_saved > 0, "{}", case.name);
            }
            "pass_through" => {
                assert_eq!(case.tokens_saved, 0, "{}", case.name);
                assert_eq!(case.bytes_saved, 0, "{}", case.name);
            }
            other => panic!("unexpected expectation {other}"),
        }
    }
}

#[test]
fn log_profiles_preserve_failure_and_warning_semantics() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    for (name, args, command, output, required) in [
        (
            "cargo",
            vec!["test".to_owned()],
            "cargo test -- --nocapture",
            format!(
                "{}\nwarning: deprecated item used\nerror: test failed, to rerun pass --lib\n",
                repeated_lines("test module::case ... ok", 120)
            ),
            vec!["warning: deprecated item used", "error: test failed"],
        ),
        (
            "dotnet",
            vec!["test".to_owned()],
            "dotnet test",
            format!(
                "{}\nFailed!  - Failed:     3, Passed:   117, Skipped:     0, Total:   120\nerror CS1002: ; expected\n",
                repeated_lines("Passed TestCase.Subcase", 120)
            ),
            vec!["Failed!  - Failed:     3", "error CS1002: ; expected"],
        ),
        (
            "go",
            vec!["test".to_owned(), "./...".to_owned()],
            "go test ./...",
            format!(
                "{}\n--- FAIL: TestParser (0.00s)\npanic: runtime error: index out of range\n",
                repeated_lines("ok   github.com/acme/mod/pkg 0.013s", 120)
            ),
            vec!["--- FAIL: TestParser", "panic: runtime error"],
        ),
        (
            "pytest",
            vec!["-q".to_owned()],
            "pytest -q",
            format!(
                "{}\nFAILED tests/test_parser.py::test_invalid_input - AssertionError: expected boom\nwarning: deprecated fixture used\n",
                repeated_lines(".", 120)
            ),
            vec![
                "FAILED tests/test_parser.py::test_invalid_input",
                "warning: deprecated fixture used",
            ],
        ),
        (
            "npm",
            vec!["test".to_owned()],
            "npm test",
            format!(
                "{}\nnpm ERR! Test failed.  See above for more details.\nerror: Expected status 200 but got 500\n",
                repeated_lines("PASS src/parser.test.ts", 120)
            ),
            vec![
                "npm ERR! Test failed.",
                "error: Expected status 200 but got 500",
            ],
        ),
        (
            "pnpm",
            vec!["test".to_owned()],
            "pnpm test",
            format!(
                "{}\nFAIL src/parser.test.ts\nwarning: obsolete lockfile entry\n",
                repeated_lines("✓ parser handles literals", 120)
            ),
            vec![
                "FAIL src/parser.test.ts",
                "warning: obsolete lockfile entry",
            ],
        ),
        (
            "yarn",
            vec!["test".to_owned()],
            "yarn test",
            format!(
                "{}\nerror Command failed with exit code 1.\nFAIL src/parser.test.ts\n",
                repeated_lines("PASS src/lexer.test.ts", 120)
            ),
            vec![
                "error Command failed with exit code 1.",
                "FAIL src/parser.test.ts",
            ],
        ),
        (
            "cmake",
            vec!["--build".to_owned(), "build".to_owned()],
            "cmake --build build",
            format!(
                "{}\nCMake Error at src/CMakeLists.txt:17 (add_executable): target sources missing\nFAILED: app\n",
                repeated_lines("[42/200] Building CXX object src/app.o", 120)
            ),
            vec!["CMake Error", "FAILED: app"],
        ),
        (
            "ctest",
            vec!["--output-on-failure".to_owned()],
            "ctest --output-on-failure",
            format!(
                "{}\n99% tests passed, 1 tests failed out of 120\nThe following tests FAILED:\n 42 - parser_test (Failed)\n",
                repeated_lines("Test #12: io_test ... Passed", 120)
            ),
            vec!["1 tests failed out of 120", "42 - parser_test (Failed)"],
        ),
        (
            "make",
            vec!["test".to_owned()],
            "make test",
            format!(
                "{}\nmake: *** [Makefile:42: test] Error 2\nwarning: stale generated file\n",
                repeated_lines("[ 84%] Built target parser", 120)
            ),
            vec![
                "make: *** [Makefile:42: test] Error 2",
                "warning: stale generated file",
            ],
        ),
        (
            "ninja",
            vec!["-C".to_owned(), "build".to_owned(), "test".to_owned()],
            "ninja -C build test",
            format!(
                "{}\nninja: build stopped: subcommand failed.\nFAILED: build/tests/parser_test\n",
                repeated_lines("[10/120] Building CXX object core.o", 120)
            ),
            vec![
                "ninja: build stopped: subcommand failed.",
                "FAILED: build/tests/parser_test",
            ],
        ),
        (
            "node",
            vec!["--test".to_owned()],
            "node --test",
            format!(
                "{}\nnot ok 12 - parser handles invalid input\nerror: Expected value to be truthy\n",
                repeated_lines("ok 1 - should parse simple expression", 120)
            ),
            vec![
                "not ok 12 - parser handles invalid input",
                "error: Expected value to be truthy",
            ],
        ),
        (
            "mvn",
            vec!["test".to_owned()],
            "mvn test",
            format!(
                "{}\n[INFO] BUILD FAILURE\n[ERROR] Tests run: 120, Failures: 1, Errors: 0, Skipped: 0\n",
                repeated_lines("[INFO] Building parser 1.0.0", 120)
            ),
            vec![
                "[INFO] BUILD FAILURE",
                "[ERROR] Tests run: 120, Failures: 1, Errors: 0, Skipped: 0",
            ],
        ),
        (
            "gradle",
            vec!["test".to_owned()],
            "gradle test",
            format!(
                "{}\nBUILD FAILED in 12s\n1 test completed, 1 failed\n",
                repeated_lines("> Task :compileJava", 120)
            ),
            vec!["BUILD FAILED in 12s", "1 test completed, 1 failed"],
        ),
        (
            "pip",
            vec![
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
            "pip install -r requirements.txt",
            format!(
                "{}\nSuccessfully installed demo-1.0 helper-2.0\nwarning: Retrying (Retry(total=4, connect=None))\n",
                repeated_lines("Collecting demo-package", 120)
            ),
            vec![
                "Successfully installed demo-1.0 helper-2.0",
                "warning: Retrying",
            ],
        ),
        (
            "bun",
            vec!["test".to_owned()],
            "bun test",
            format!(
                "{}\n1 fail\nerror: script \"test\" exited with code 1\n",
                repeated_lines("pass parser handles literals", 120)
            ),
            vec!["1 fail", "error: script \"test\" exited with code 1"],
        ),
    ] {
        let normalized = normalize_text(name, &args, "stdout", CommandKind::Log, &output, &cfg)
            .expect("normalize");
        let value = value_from_json(&normalized);
        assert_eq!(value["p"], "log", "{command}");
        assert_eq!(value["bd"]["n"], name, "{command}");
        let haystack = rollout_string_haystack(&normalized);
        for fragment in required {
            assert!(
                haystack.contains(fragment),
                "{command} missing `{fragment}`"
            );
        }
    }
}

#[test]
fn build_summary_serializes_family_specific_counts() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;

    for (name, args, output, expected) in [
        (
            "cargo",
            vec!["test".to_owned()],
            [
                "Compiling demo v0.1.0",
                "Running unittests src/lib.rs (target/debug/deps/demo)",
                "test result: FAILED. 117 passed; 3 failed; 0 ignored; 0 measured",
                "error: test failed, to rerun pass --lib",
            ]
            .join("\n"),
            serde_json::json!({"ok": 117, "fl": 3, "tt": 120, "cp": 1, "rn": 1}),
        ),
        (
            "pytest",
            vec!["-q".to_owned()],
            [
                "FAILED tests/test_parser.py::test_invalid_input - AssertionError: expected boom",
                "2 passed, 1 failed, 1 skipped, 4 total",
            ]
            .join("\n"),
            serde_json::json!({"ok": 2, "fl": 1, "sk": 1, "tt": 4}),
        ),
        (
            "dotnet",
            vec!["test".to_owned()],
            [
                "Passed TestCase.Parser",
                "Failed!  - Failed:     3, Passed:   117, Skipped:     2, Total:   122",
                "error CS1002: ; expected",
            ]
            .join("\n"),
            serde_json::json!({"ok": 117, "fl": 3, "sk": 2, "tt": 122}),
        ),
        (
            "pip",
            vec![
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
            [
                "Collecting demo-package",
                "Successfully installed demo-1.0 helper-2.0 toolkit-3.1",
                "warning: Retrying (Retry(total=4, connect=None))",
            ]
            .join("\n"),
            serde_json::json!({"ip": 3, "tt": 3}),
        ),
    ] {
        let normalized = normalize_text(name, &args, "stdout", CommandKind::Log, &output, &cfg)
            .expect("normalize");
        let value = value_from_json(&normalized);
        let bd = value["bd"].as_object().expect("build summary");
        let expected = expected.as_object().expect("expected object");
        for (key, expected_value) in expected {
            assert_eq!(bd.get(key), Some(expected_value), "{name} key={key}");
        }
    }
}

#[test]
fn compressed_search_and_file_outputs_preserve_semantic_answer_fragments() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;

    let search_output = (0..80)
        .map(|idx| format!("src/lib.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx))
        .chain((0..40).map(|idx| format!("src/main.rs:{}:struct Gamma{};", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n");
    let search = normalize_text(
        "rg",
        &["fn|struct".to_owned(), "src".to_owned()],
        "stdout",
        CommandKind::Search,
        &search_output,
        &cfg,
    )
    .expect("normalize");
    let search_haystack = rollout_string_haystack(&search);
    for fragment in ["src/lib.rs", "pub fn beta_0() {}", "struct Gamma0;"] {
        assert!(
            search_haystack.contains(fragment),
            "search missing `{fragment}`"
        );
    }

    let file_output = [
        "use std::io;",
        "",
        "pub struct Config {",
        "    field: usize,",
        "}",
        "",
        "impl Config {",
        "    pub fn load() -> Self {",
        "        Self { field: 1 }",
        "    }",
        "}",
        "",
        "pub fn helper() {",
        "    println!(\"hi\");",
        "}",
    ]
    .join("\n");
    let file = normalize_text(
        "cat",
        &["src/lib.rs".to_owned()],
        "stdout",
        CommandKind::File,
        &file_output,
        &cfg,
    )
    .expect("normalize");
    let file_haystack = rollout_string_haystack(&file);
    for fragment in ["pub struct Config", "pub fn load()", "pub fn helper()"] {
        assert!(
            file_haystack.contains(fragment),
            "file missing `{fragment}`"
        );
    }
}

#[test]
fn benchmark_report_covers_default_tool_families() {
    let report = build_benchmark_report(&Config::default()).expect("benchmark");
    let commands = report
        .cases
        .iter()
        .map(|case| case.command.as_str())
        .collect::<Vec<_>>();
    for needle in [
        "cat ",
        "sed ",
        "bat ",
        "nl ",
        "awk ",
        "cut ",
        "head ",
        "tail ",
        "tr ",
        "perl ",
        "rg ",
        "grep ",
        "git ",
        "cargo ",
        "pytest ",
        "npm ",
        "pnpm ",
        "yarn ",
        "dotnet ",
        "go ",
        "cmake ",
        "ctest ",
        "make ",
        "ninja ",
        "node ",
        "find ",
        "fd ",
        "tree ",
        "sort",
        "uniq",
        "wc ",
        "ls ",
        "jq ",
        "curl ",
        "python ",
        "python3 ",
        "docker ",
        "ps ",
        "ss ",
        "netstat ",
        "systemctl ",
        "du ",
        "df ",
        "xargs ",
    ] {
        assert!(commands.iter().any(|cmd| cmd.contains(needle)), "{needle}");
    }
}

#[test]
fn benchmark_specs_cover_default_tool_commands() {
    let specs = benchmark_specs();
    let commands = specs
        .iter()
        .map(|spec| spec.command.as_str())
        .collect::<Vec<_>>();
    for tool in default_tool_commands() {
        match canonical_command_name(tool) {
            "cat" => assert!(
                commands.iter().any(|cmd| cmd == &"cat src/lib.rs"),
                "{tool}"
            ),
            "sed" => assert!(
                commands
                    .iter()
                    .any(|cmd| cmd.starts_with("sed -n ") && cmd.contains("src/lib.rs")),
                "{tool}"
            ),
            "rg" => assert!(commands.iter().any(|cmd| cmd.starts_with("rg ")), "{tool}"),
            "grep" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("grep ")),
                "{tool}"
            ),
            "git" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("git diff")),
                "{tool}"
            ),
            "which" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("which ")),
                "{tool}"
            ),
            "cargo" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("cargo ")),
                "{tool}"
            ),
            "pytest" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("pytest ")),
                "{tool}"
            ),
            "npm" => assert!(commands.iter().any(|cmd| cmd.starts_with("npm ")), "{tool}"),
            "pnpm" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("pnpm ")),
                "{tool}"
            ),
            "yarn" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("yarn ")),
                "{tool}"
            ),
            "bun" => assert!(commands.iter().any(|cmd| cmd.starts_with("bun ")), "{tool}"),
            "pip" => assert!(commands.iter().any(|cmd| cmd.starts_with("pip ")), "{tool}"),
            "uv" => assert!(commands.iter().any(|cmd| cmd.starts_with("uv ")), "{tool}"),
            "poetry" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("poetry ")),
                "{tool}"
            ),
            "mvn" => assert!(commands.iter().any(|cmd| cmd.starts_with("mvn ")), "{tool}"),
            "gradle" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("gradle ")),
                "{tool}"
            ),
            "gradlew" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("./gradlew ")),
                "{tool}"
            ),
            "javac" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("javac ")),
                "{tool}"
            ),
            "java" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("java ")),
                "{tool}"
            ),
            "bundle" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("bundle ")),
                "{tool}"
            ),
            "composer" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("composer ")),
                "{tool}"
            ),
            "tail" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("tail ")),
                "{tool}"
            ),
            "head" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("head ")),
                "{tool}"
            ),
            "tr" => assert!(commands.iter().any(|cmd| cmd.starts_with("tr ")), "{tool}"),
            "perl" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("perl ")),
                "{tool}"
            ),
            "dotnet" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("dotnet ")),
                "{tool}"
            ),
            "go" => assert!(commands.iter().any(|cmd| cmd.starts_with("go ")), "{tool}"),
            "cmake" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("cmake ")),
                "{tool}"
            ),
            "ctest" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("ctest ")),
                "{tool}"
            ),
            "make" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("make ")),
                "{tool}"
            ),
            "ninja" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("ninja ")),
                "{tool}"
            ),
            "node" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("node ")),
                "{tool}"
            ),
            "ls" => assert!(commands.iter().any(|cmd| cmd.starts_with("ls ")), "{tool}"),
            "find" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("find ")),
                "{tool}"
            ),
            "fd" => assert!(commands.iter().any(|cmd| cmd.starts_with("fd ")), "{tool}"),
            "bat" => assert!(commands.iter().any(|cmd| cmd.starts_with("bat ")), "{tool}"),
            "nl" => assert!(commands.iter().any(|cmd| cmd.starts_with("nl ")), "{tool}"),
            "awk" => assert!(commands.iter().any(|cmd| cmd.starts_with("awk ")), "{tool}"),
            "cut" => assert!(commands.iter().any(|cmd| cmd.starts_with("cut ")), "{tool}"),
            "sort" => assert!(commands.iter().any(|cmd| cmd.contains("| sort")), "{tool}"),
            "uniq" => assert!(commands.iter().any(|cmd| cmd.contains("| uniq")), "{tool}"),
            "wc" => assert!(commands.iter().any(|cmd| cmd.starts_with("wc ")), "{tool}"),
            "jq" => assert!(commands.iter().any(|cmd| cmd.starts_with("jq ")), "{tool}"),
            "curl" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("curl ")),
                "{tool}"
            ),
            "python" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("python ")),
                "{tool}"
            ),
            "python3" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("python3 ")),
                "{tool}"
            ),
            "tree" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("tree ")),
                "{tool}"
            ),
            "docker" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("docker ")),
                "{tool}"
            ),
            "ps" => assert!(commands.iter().any(|cmd| cmd.starts_with("ps ")), "{tool}"),
            "ss" => assert!(commands.iter().any(|cmd| cmd.starts_with("ss ")), "{tool}"),
            "netstat" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("netstat ")),
                "{tool}"
            ),
            "systemctl" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("systemctl ")),
                "{tool}"
            ),
            "psql" => assert!(
                commands.iter().any(|cmd| cmd.starts_with("psql ")),
                "{tool}"
            ),
            "du" => assert!(commands.iter().any(|cmd| cmd.starts_with("du ")), "{tool}"),
            "df" => assert!(commands.iter().any(|cmd| cmd.starts_with("df ")), "{tool}"),
            "xargs" => assert!(
                commands.iter().any(|cmd| cmd.contains("xargs cat")),
                "{tool}"
            ),
            other => panic!("unexpected default tool command {tool} -> {other}"),
        }
    }
}

#[test]
fn default_tool_commands_have_expected_command_kinds() {
    for tool in default_tool_commands() {
        let args = match canonical_command_name(tool) {
            "git" => vec!["diff".to_owned()],
            "cargo" => vec!["build".to_owned()],
            "pytest" => vec!["-q".to_owned()],
            "npm" | "pnpm" | "yarn" | "bun" => vec!["test".to_owned()],
            "dotnet" => vec!["test".to_owned()],
            "go" => vec!["test".to_owned(), "./...".to_owned()],
            "cmake" => vec!["--build".to_owned(), "build".to_owned()],
            "ctest" => vec!["--output-on-failure".to_owned()],
            "make" => vec!["test".to_owned()],
            "ninja" => vec!["-C".to_owned(), "build".to_owned(), "test".to_owned()],
            "node" => vec!["--test".to_owned()],
            "pip" => vec![
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
            "uv" => vec![
                "pip".to_owned(),
                "install".to_owned(),
                "-r".to_owned(),
                "requirements.txt".to_owned(),
            ],
            "poetry" => vec!["install".to_owned()],
            "mvn" => vec!["test".to_owned()],
            "gradle" => vec!["test".to_owned()],
            "gradlew" => vec!["build".to_owned()],
            "javac" => vec!["Main.java".to_owned()],
            "java" => vec!["-jar".to_owned(), "app.jar".to_owned()],
            "bundle" => vec!["exec".to_owned(), "rspec".to_owned()],
            "composer" => vec!["test".to_owned()],
            "ls" => vec!["src".to_owned()],
            "find" => vec!["src".to_owned()],
            "fd" => vec![".".to_owned(), "src".to_owned()],
            "tree" => vec![
                "-a".to_owned(),
                "-L".to_owned(),
                "3".to_owned(),
                "src".to_owned(),
            ],
            "jq" => vec![".".to_owned(), "/tmp/demo.json".to_owned()],
            "curl" => vec!["-s".to_owned(), "http://127.0.0.1/demo".to_owned()],
            "python" | "python3" => vec!["script.py".to_owned()],
            "docker" => vec!["ps".to_owned()],
            "ps" => vec!["aux".to_owned()],
            "ss" => vec!["-ltnp".to_owned()],
            "netstat" => vec!["-ltnp".to_owned()],
            "systemctl" => vec!["list-units".to_owned()],
            "psql" => vec!["-c".to_owned(), "select 1".to_owned()],
            "du" => vec!["-sh".to_owned(), "/root/project".to_owned()],
            "df" => vec!["-h".to_owned()],
            "head" | "tail" => vec!["-n".to_owned(), "20".to_owned(), "src/lib.rs".to_owned()],
            "sed" => vec!["-n".to_owned(), "1,20p".to_owned(), "src/lib.rs".to_owned()],
            "bat" => vec!["--style=plain".to_owned(), "src/lib.rs".to_owned()],
            "nl" => vec!["-ba".to_owned(), "src/lib.rs".to_owned()],
            "awk" => vec!["{print}".to_owned(), "src/lib.rs".to_owned()],
            "cut" => vec!["-c1-120".to_owned(), "src/lib.rs".to_owned()],
            "tr" => vec!["-s".to_owned(), " ".to_owned()],
            "which" => vec!["cargo".to_owned()],
            "perl" => vec![
                "-ne".to_owned(),
                "print".to_owned(),
                "src/lib.rs".to_owned(),
            ],
            "sort" | "uniq" | "wc" | "xargs" => Vec::new(),
            _ => vec!["src/lib.rs".to_owned()],
        };

        let kind = classify_command(tool, &args);
        match canonical_command_name(tool) {
            "cat" | "sed" | "tail" | "head" | "bat" | "nl" | "awk" | "cut" | "tr" | "perl" => {
                assert!(matches!(kind, CommandKind::File), "{tool}");
            }
            "rg" | "grep" | "find" | "fd" | "tree" | "ls" | "which" => {
                assert!(matches!(kind, CommandKind::Search), "{tool}");
            }
            "git" => assert!(matches!(kind, CommandKind::Diff), "{tool}"),
            "cargo" | "pytest" | "npm" | "pnpm" | "yarn" | "bun" | "dotnet" | "go" | "cmake"
            | "ctest" | "make" | "ninja" | "node" | "pip" | "uv" | "poetry" | "mvn" | "gradle"
            | "gradlew" | "javac" | "java" | "bundle" | "composer" | "python" | "python3"
            | "docker" | "ps" | "ss" | "netstat" | "systemctl" | "psql" | "du" | "df" => {
                assert!(matches!(kind, CommandKind::Log), "{tool}");
            }
            "sort" | "uniq" | "wc" | "xargs" | "jq" => {
                assert!(matches!(kind, CommandKind::Generic), "{tool}");
            }
            "curl" | "wget" | "gh" | "glab" | "docker-compose" | "pip3" => {
                assert!(matches!(kind, CommandKind::Log), "{tool}");
            }
            other => panic!("unexpected default tool command {tool} -> {other}"),
        }
    }
}

#[test]
fn frequent_command_families_preserve_semantic_fragments() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;

    let code_sample = (0..80)
        .flat_map(|idx| {
            [
                format!("pub struct Config{idx} {{"),
                format!("    field_{idx}: usize,"),
                "}".to_owned(),
                format!("pub fn load_{idx}() -> usize {{"),
                format!("    field_{idx}"),
                "}".to_owned(),
            ]
        })
        .collect::<Vec<_>>()
        .join("\n");

    for (name, args, kind, text, fragments) in [
        (
            "sed",
            vec![
                "-n".to_owned(),
                "1,120p".to_owned(),
                "src/lib.rs".to_owned(),
            ],
            CommandKind::File,
            code_sample.clone(),
            vec!["pub struct Config0 {", "pub fn load_0() -> usize {"],
        ),
        (
            "bat",
            vec!["--style=plain".to_owned(), "src/lib.rs".to_owned()],
            CommandKind::File,
            code_sample.clone(),
            vec!["pub struct Config0 {", "pub fn load_0() -> usize {"],
        ),
        (
            "head",
            vec!["-n".to_owned(), "120".to_owned(), "src/lib.rs".to_owned()],
            CommandKind::File,
            code_sample.clone(),
            vec!["pub struct Config0 {", "pub fn load_0() -> usize {"],
        ),
        (
            "tail",
            vec!["-n".to_owned(), "120".to_owned(), "src/lib.rs".to_owned()],
            CommandKind::File,
            code_sample.clone(),
            vec!["pub struct Config0 {", "pub fn load_0() -> usize {"],
        ),
        (
            "grep",
            vec![
                "-n".to_owned(),
                "Config".to_owned(),
                "src/lib.rs".to_owned(),
            ],
            CommandKind::Search,
            (0..80)
                .map(|idx| format!("src/lib.rs:{}:pub struct Config{};", idx + 1, idx))
                .collect::<Vec<_>>()
                .join("\n"),
            vec!["src/lib.rs:1:pub struct Config0;", "Config79"],
        ),
        (
            "find",
            vec!["src".to_owned()],
            CommandKind::Search,
            (0..220)
                .map(|idx| format!("/root/project/src/module_{idx:03}.rs"))
                .collect::<Vec<_>>()
                .join("\n"),
            vec!["/root/project/src", "module_219.rs"],
        ),
        (
            "fd",
            vec![".".to_owned(), "src".to_owned()],
            CommandKind::Search,
            (0..220)
                .map(|idx| format!("/root/project/src/bin/tool_{idx:03}.rs"))
                .collect::<Vec<_>>()
                .join("\n"),
            vec!["/root/project/src/bin", "tool_219.rs"],
        ),
        (
            "tree",
            vec![
                "-a".to_owned(),
                "-L".to_owned(),
                "4".to_owned(),
                "src".to_owned(),
            ],
            CommandKind::Search,
            (0..120)
                .map(|idx| format!("src/module_{idx:03}/file_{idx:03}.rs"))
                .collect::<Vec<_>>()
                .join("\n"),
            vec!["module_000", "file_119.rs"],
        ),
        (
            "ls",
            vec!["src".to_owned()],
            CommandKind::Search,
            (0..160)
                .map(|idx| format!("module_{idx:03}.rs"))
                .collect::<Vec<_>>()
                .join("\n"),
            vec!["module_000.rs", "module_159.rs"],
        ),
        (
            "git",
            vec!["diff".to_owned(), "--".to_owned(), "src/lib.rs".to_owned()],
            CommandKind::Diff,
            [
                "diff --git a/src/lib.rs b/src/lib.rs",
                "index 123..456 100644",
                "--- a/src/lib.rs",
                "+++ b/src/lib.rs",
                "@@ -1,3 +1,4 @@",
                "-old_call();",
                "+new_call();",
            ]
            .join("\n"),
            vec!["diff --git a/src/lib.rs b/src/lib.rs", "+new_call();"],
        ),
    ] {
        let normalized =
            normalize_text(name, &args, "stdout", kind, &text, &cfg).expect("normalize");
        let haystack = rollout_string_haystack(&normalized);
        for fragment in fragments {
            assert!(haystack.contains(fragment), "{name} missing `{fragment}`");
        }
    }
}

#[test]
fn du_output_compresses_as_table_summary() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..24)
        .map(|idx| format!("{:>4}M\t/root/project/module_{idx:02}", 8 + idx))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "du",
        &["-sh".to_owned(), "/root/project/*".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "table");
    assert_eq!(value["tb"]["c"][0], "Size");
    assert_eq!(value["tb"]["c"][1], "Path");
    assert_eq!(value["tb"]["rc"], 24);
}

#[test]
fn large_json_uses_preview_body_lines() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = serde_json::json!({
        "success": true,
        "message": "生成市场技术分析师报告",
        "progress": 87,
        "current_step_name": "市场技术分析",
        "current_step_description": "生成市场技术分析师报告",
        "data": {
            "symbols": ["AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH"],
            "summary": "这是一个很长的说明字段，用来验证大 JSON 会走 preview 而不是整条紧凑串。"
        }
    })
    .to_string();
    let normalized = normalize_text(
        "jq",
        &[".".to_owned(), "/tmp/demo.json".to_owned()],
        "stdout",
        CommandKind::Generic,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
    assert!(value["b"].is_array());
    assert!(
        value["b"]
            .as_array()
            .map(|rows| rows.len())
            .unwrap_or_default()
            > 1
    );
}

#[test]
fn python_json_output_uses_json_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = serde_json::json!({
        "success": true,
        "message": "python emitted json",
        "data": {
            "items": ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"],
            "count": 6
        }
    })
    .to_string();
    let normalized = normalize_text(
        "python",
        &["script.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
}

#[test]
fn python_json_lines_output_uses_json_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "{\"task_id\":\"a1\",\"status\":\"running\",\"progress\":90}",
        "{\"task_id\":\"b2\",\"status\":\"running\",\"progress\":89}",
        "{\"task_id\":\"c3\",\"status\":\"done\",\"progress\":100}",
    ]
    .join("\n");
    let normalized = normalize_text(
        "python3",
        &["script.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "json");
}

#[test]
fn python_path_output_uses_pathlist_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = (0..24)
        .map(|idx| format!("/root/project/cache/item_{idx:03}.json"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = normalize_text(
        "python3",
        &["script.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 24);
}

#[test]
fn python_bare_identifier_list_does_not_use_pathlist_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "agent_job_items",
        "agent_jobs",
        "jobs",
        "logs",
        "stage1_outputs",
        "threads",
    ]
    .join("\n");
    let normalized = normalize_text(
        "python3",
        &["script.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_ne!(value["p"], "pathlist");
}

#[test]
fn python_table_output_uses_table_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "name        count   value",
        "alpha       10      ready",
        "beta        21      running",
        "gamma       34      complete",
        "delta       55      waiting",
        "epsilon     89      stopped",
    ]
    .join("\n");
    let normalized = normalize_text(
        "python",
        &["report.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "table");
    assert_eq!(value["tb"]["rc"], 5);
}

#[test]
fn psql_table_output_compresses() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        " schema |           table            | rows ",
        "--------+----------------------------+------",
        " public | users                      | 1200 ",
        " public | sessions                   | 8841 ",
        " public | audit_events               | 19342 ",
        " public | rollout_records            | 918 ",
        " public | command_execution_records  | 44120 ",
        " public | profile_stats              | 201 ",
    ]
    .join("\n");
    let normalized = normalize_text(
        "psql",
        &["-c".to_owned(), "select * from stats".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "table");
    assert_eq!(value["tb"]["hc"], 3);
    assert_eq!(value["tb"]["rc"], 6);
}

#[test]
fn df_table_prefers_capacity_columns_over_full_width() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "Filesystem      Size  Used Avail Use% Mounted on",
        "/dev/mapper/vg0  80G   20G   60G  25% /mnt/vol0",
        "/dev/mapper/vg1  81G   21G   60G  26% /mnt/vol1",
        "/dev/mapper/vg2  82G   22G   60G  27% /mnt/vol2",
        "/dev/mapper/vg3  83G   23G   60G  28% /mnt/vol3",
    ]
    .join("\n");
    let normalized = normalize_text(
        "df",
        &["-h".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "table");
    let cols = value["tb"]["c"].as_array().expect("cols");
    let cols = cols
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        cols,
        vec!["Filesystem", "Size", "Used", "Use%", "Mounted on"]
    );
}

#[test]
fn table_rows_omit_line_indexes_from_serialized_payload() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "name        count   value",
        "alpha       10      ready",
        "beta        21      running",
        "gamma       34      complete",
        "delta       55      waiting",
        "epsilon     89      stopped",
    ]
    .join("\n");
    let normalized = normalize_text(
        "python",
        &["report.py".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    let rows = value["tb"]["r"].as_array().expect("rows");
    assert!(rows.iter().all(|row| row.get("i").is_none()));
}

#[test]
fn which_path_output_uses_pathlist_profile() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let text = [
        "/usr/bin/cargo",
        "/usr/bin/rustc",
        "/usr/local/bin/tke",
        "/usr/local/bin/codex",
        "/usr/bin/rg",
        "/usr/bin/git",
    ]
    .join("\n");
    let normalized = normalize_text(
        "which",
        &["cargo".to_owned(), "rustc".to_owned(), "tke".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value = value_from_json(&normalized);
    assert_eq!(value["p"], "pathlist");
    assert_eq!(value["c"], 6);
    assert_eq!(value["pl"]["e"].as_array().map(|v| v.len()), Some(4));
}

#[test]
fn benchmark_report_shows_positive_savings_for_agent_tasks() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    assert!(!report.tasks.is_empty());
    for task in &report.tasks {
        assert!(task.changed, "{}", task.name);
        assert!(task.tokens_saved > 0, "{}", task.name);
        assert!(task.bytes_saved > 0, "{}", task.name);
        assert_eq!(
            task.required_fragments.len(),
            task.preserved_fragments.len(),
            "{}",
            task.name
        );
    }
}

#[test]
fn benchmark_agent_task_rollout_preserves_required_fragments() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    for task in benchmark_task_specs() {
        let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
            .expect("rewrite")
            .expect("changed");
        let haystack = rollout_string_haystack(&rewritten);
        for fragment in &task.required_fragments {
            assert!(
                haystack.contains(fragment),
                "{} missing {}",
                task.name,
                fragment
            );
        }
    }
}

#[test]
fn codex_search_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "codex_interactive_trace_selected_search_stage")
        .expect("codex search task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"rg\""));
    assert!(haystack.contains("\"sr\":\"search\""));
}

#[test]
fn codex_find_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "codex_interactive_trace_selected_find_stage")
        .expect("codex find task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"find\""));
    assert!(haystack.contains("\"sr\":\"search\""));
    assert!(haystack.contains("\"p\":\"pathlist\""));
}

#[test]
fn codex_build_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "codex_interactive_trace_selected_build_stage")
        .expect("codex build task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"cargo\""));
    assert!(haystack.contains("\"sr\":\"build\""));
    assert!(haystack.contains("\"p\":\"log\""));
    assert!(haystack.contains("error: test failed, to rerun pass --lib"));
}

#[test]
fn claude_benchmark_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_selected_search_stage")
        .expect("claude task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"rg\""));
    assert!(haystack.contains("\"sr\":\"search\""));
}

#[test]
fn claude_find_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_selected_find_stage")
        .expect("claude find task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"find\""));
    assert!(haystack.contains("\"sr\":\"search\""));
    assert!(haystack.contains("\"p\":\"pathlist\""));
}

#[test]
fn claude_build_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_selected_build_stage")
        .expect("claude build task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"cargo\""));
    assert!(haystack.contains("\"sr\":\"build\""));
    assert!(haystack.contains("\"p\":\"log\""));
    assert!(haystack.contains("error: test failed, to rerun pass --lib"));
}

#[test]
fn claude_diff_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_selected_diff_stage")
        .expect("claude diff task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"git\""));
    assert!(haystack.contains("\"p\":\"diff\""));
    assert!(haystack.contains("\"df\":"));
    assert!(haystack.contains("\"add\":3"));
    assert!(haystack.contains("\"del\":1"));
}

#[test]
fn claude_rtk_hook_search_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_selected_search_stage")
        .expect("claude rtk hook search task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"rg\""));
    assert!(haystack.contains("\"sr\":\"search\""));
    assert!(haystack.contains("src/tests.rs"));
}

#[test]
fn claude_rtk_hook_find_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_selected_find_stage")
        .expect("claude rtk hook find task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"find\""));
    assert!(haystack.contains("\"sr\":\"search\""));
    assert!(haystack.contains("\"p\":\"pathlist\""));
    assert!(haystack.contains("\"d\":\"/root/project/target/debug/incremental/tke\""));
    assert!(haystack.contains("\"f\":\"build-artifact-0000.o\""));
}

#[test]
fn claude_rtk_hook_diff_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_selected_diff_stage")
        .expect("claude rtk hook diff task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"git\""));
    assert!(haystack.contains("\"p\":\"diff\""));
    assert!(haystack.contains("\"df\":"));
    assert!(haystack.contains("\"add\":3"));
    assert!(haystack.contains("\"del\":1"));
}

#[test]
fn claude_rtk_hook_build_pipeline_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_selected_build_stage")
        .expect("claude rtk hook build task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    assert!(haystack.contains("\"sc\":\"cargo\""));
    assert!(haystack.contains("\"sr\":\"build\""));
    assert!(haystack.contains("\"p\":\"log\""));
    assert!(haystack.contains("error: test failed, to rerun pass --lib"));
}

#[test]
fn claude_complex_triage_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_complex_triage_task")
        .expect("claude complex triage task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_complex_triage_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_complex_triage_task")
        .expect("claude rtk hook complex triage task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_complex_code_trace_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_complex_code_trace_task")
        .expect("claude complex code trace task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_complex_code_trace_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_complex_code_trace_task")
        .expect("claude rtk hook complex code trace task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_complex_stacktrace_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_complex_stacktrace_task")
        .expect("claude complex stacktrace task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"p\":\"stacktrace\"",
        "\"k\":\"summary\"",
        "\"k\":\"frame\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_complex_stacktrace_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_complex_stacktrace_task")
        .expect("claude rtk hook complex stacktrace task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"p\":\"stacktrace\"",
        "\"k\":\"summary\"",
        "\"k\":\"frame\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_complex_stacktrace_diff_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_complex_stacktrace_diff_task")
        .expect("claude complex stacktrace diff task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"p\":\"stacktrace\"",
        "\"k\":\"summary\"",
        "\"k\":\"frame\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_complex_stacktrace_diff_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_complex_stacktrace_diff_task")
        .expect("claude rtk hook complex stacktrace diff task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"p\":\"stacktrace\"",
        "\"k\":\"summary\"",
        "\"k\":\"frame\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_complex_root_cause_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_complex_root_cause_task")
        .expect("claude complex root cause task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_complex_root_cause_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_complex_root_cause_task")
        .expect("claude rtk hook complex root cause task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_answer_consistency_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_answer_consistency_task")
        .expect("claude answer consistency task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "src/tests.rs",
        "normalize_text",
        "compare-e2e",
        "failing test signal",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_answer_consistency_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_answer_consistency_task")
        .expect("claude rtk hook answer consistency task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"df\":",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "src/tests.rs",
        "normalize_text",
        "compare-e2e",
        "root cause answer",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_candidate_root_cause_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_candidate_root_cause_task")
        .expect("claude candidate root cause task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_candidate_root_cause_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_candidate_root_cause_task")
        .expect("claude rtk hook candidate root cause task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_misleading_signal_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_misleading_signal_task")
        .expect("claude misleading signal task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "warning: verdict helper fell back to semantic_result_match",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_misleading_signal_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_misleading_signal_task")
        .expect("claude rtk hook misleading signal task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "warning: verdict helper fell back to semantic_result_match",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_cross_file_causality_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_cross_file_causality_task")
        .expect("claude cross file causality task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"p\":\"src/e2e_report.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_negative_evidence_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_negative_evidence_task")
        .expect("claude negative evidence task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "fn semantic_result_match(",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_cross_file_causality_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_cross_file_causality_task")
        .expect("claude rtk hook cross file causality task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"p\":\"src/e2e_report.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_negative_evidence_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_negative_evidence_task")
        .expect("claude rtk hook negative evidence task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn claude_answer_consistency_task_rollout_is_rewritten()",
        "fn semantic_result_match(",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_temporal_causality_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_temporal_causality_task")
        .expect("claude temporal causality task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"p\":\"src/e2e_report.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "newer src/tests.rs change",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_temporal_causality_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_temporal_causality_task")
        .expect("claude rtk hook temporal causality task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"p\":\"src/e2e_report.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "newer src/tests.rs change",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_symbol_collision_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_symbol_collision_task")
        .expect("claude symbol collision task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "assert_eq!(build_variant_verdict(",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::build_variant_verdict_prefers_saved_and_correct",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_symbol_collision_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_symbol_collision_task")
        .expect("claude rtk hook symbol collision task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "assert_eq!(build_variant_verdict(",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::build_variant_verdict_prefers_saved_and_correct",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_reversal_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_bash_trace_reversal_task")
        .expect("claude reversal task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn build_variant_verdict(",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "initial suspicion around src/e2e_report.rs is overturned",
        "not src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn claude_rtk_hook_reversal_task_rollout_is_rewritten() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let task = benchmark_task_specs()
        .into_iter()
        .find(|task| task.name == "claude_rtk_hook_trace_reversal_task")
        .expect("claude rtk hook reversal task");
    let rewritten = rewrite_agent_transcript(&task.rollout, &cfg)
        .expect("rewrite")
        .expect("changed");
    let haystack = rollout_string_haystack(&rewritten);
    for fragment in [
        "\"sc\":\"find\"",
        "\"p\":\"pathlist\"",
        "tests.rs",
        "e2e_report.rs",
        "\"sc\":\"rg\"",
        "\"sr\":\"search\"",
        "\"sc\":\"sed\"",
        "\"p\":\"file\"",
        "fn build_variant_verdict(",
        "\"sc\":\"git\"",
        "\"p\":\"diff\"",
        "\"p\":\"src/tests.rs\"",
        "\"sc\":\"cargo\"",
        "\"p\":\"log\"",
        "\"lg\":",
        "FAILED src/tests.rs::claude_answer_consistency_task_rollout_is_rewritten",
        "revise the first impression",
        "rather than src/e2e_report.rs",
    ] {
        assert!(haystack.contains(fragment), "missing {fragment}");
    }
}

#[test]
fn codex_benchmark_task_report_shows_positive_savings() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    for name in [
        "codex_interactive_trace_selected_search_stage",
        "codex_interactive_trace_selected_find_stage",
        "codex_interactive_trace_selected_build_stage",
    ] {
        let task = report
            .tasks
            .iter()
            .find(|task| task.name == name)
            .expect("codex report task");
        assert!(
            task.changed,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name, task.changed, task.tokens_saved, task.bytes_saved
        );
        assert!(
            task.tokens_saved > 0,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name,
            task.changed,
            task.tokens_saved,
            task.bytes_saved
        );
        assert!(
            task.bytes_saved > 0,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name,
            task.changed,
            task.tokens_saved,
            task.bytes_saved
        );
    }
}

#[test]
fn claude_benchmark_task_report_shows_positive_savings() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    for name in [
        "claude_bash_trace_selected_search_stage",
        "claude_bash_trace_selected_find_stage",
        "claude_bash_trace_selected_diff_stage",
        "claude_bash_trace_selected_build_stage",
        "claude_bash_trace_complex_triage_task",
        "claude_bash_trace_complex_code_trace_task",
        "claude_rtk_hook_trace_selected_find_stage",
        "claude_rtk_hook_trace_selected_search_stage",
        "claude_rtk_hook_trace_selected_diff_stage",
        "claude_rtk_hook_trace_selected_build_stage",
        "claude_rtk_hook_trace_complex_triage_task",
        "claude_rtk_hook_trace_complex_code_trace_task",
    ] {
        let task = report
            .tasks
            .iter()
            .find(|task| task.name == name)
            .expect("claude report task");
        assert!(
            task.changed,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name, task.changed, task.tokens_saved, task.bytes_saved
        );
        assert!(
            task.tokens_saved > 0,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name,
            task.changed,
            task.tokens_saved,
            task.bytes_saved
        );
        assert!(
            task.bytes_saved > 0,
            "name={} changed={} tokens_saved={} bytes_saved={}",
            task.name,
            task.changed,
            task.tokens_saved,
            task.bytes_saved
        );
    }
}

fn claude_synthetic_task_pairs() -> [(&'static str, &'static str, &'static str); 17] {
    [
        (
            "find/pathlist",
            "claude_bash_trace_selected_find_stage",
            "claude_rtk_hook_trace_selected_find_stage",
        ),
        (
            "search",
            "claude_bash_trace_selected_search_stage",
            "claude_rtk_hook_trace_selected_search_stage",
        ),
        (
            "diff",
            "claude_bash_trace_selected_diff_stage",
            "claude_rtk_hook_trace_selected_diff_stage",
        ),
        (
            "build/log",
            "claude_bash_trace_selected_build_stage",
            "claude_rtk_hook_trace_selected_build_stage",
        ),
        (
            "complex/triage",
            "claude_bash_trace_complex_triage_task",
            "claude_rtk_hook_trace_complex_triage_task",
        ),
        (
            "complex/code-trace",
            "claude_bash_trace_complex_code_trace_task",
            "claude_rtk_hook_trace_complex_code_trace_task",
        ),
        (
            "complex/stacktrace",
            "claude_bash_trace_complex_stacktrace_task",
            "claude_rtk_hook_trace_complex_stacktrace_task",
        ),
        (
            "complex/stacktrace-diff",
            "claude_bash_trace_complex_stacktrace_diff_task",
            "claude_rtk_hook_trace_complex_stacktrace_diff_task",
        ),
        (
            "complex/root-cause",
            "claude_bash_trace_complex_root_cause_task",
            "claude_rtk_hook_trace_complex_root_cause_task",
        ),
        (
            "answer-consistency",
            "claude_bash_trace_answer_consistency_task",
            "claude_rtk_hook_trace_answer_consistency_task",
        ),
        (
            "candidate-root-cause",
            "claude_bash_trace_candidate_root_cause_task",
            "claude_rtk_hook_trace_candidate_root_cause_task",
        ),
        (
            "misleading-signal",
            "claude_bash_trace_misleading_signal_task",
            "claude_rtk_hook_trace_misleading_signal_task",
        ),
        (
            "cross-file-causality",
            "claude_bash_trace_cross_file_causality_task",
            "claude_rtk_hook_trace_cross_file_causality_task",
        ),
        (
            "negative-evidence",
            "claude_bash_trace_negative_evidence_task",
            "claude_rtk_hook_trace_negative_evidence_task",
        ),
        (
            "temporal-causality",
            "claude_bash_trace_temporal_causality_task",
            "claude_rtk_hook_trace_temporal_causality_task",
        ),
        (
            "symbol-collision",
            "claude_bash_trace_symbol_collision_task",
            "claude_rtk_hook_trace_symbol_collision_task",
        ),
        (
            "reversal",
            "claude_bash_trace_reversal_task",
            "claude_rtk_hook_trace_reversal_task",
        ),
    ]
}

#[test]
fn benchmark_docs_match_claude_synthetic_totals_and_rows() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    let docs = fs::read_to_string("docs/benchmarks.md").expect("benchmarks doc");
    let vs_doc = fs::read_to_string("docs/rtk-vs-tke.md").expect("rtk vs tke doc");

    let tke_tasks = report
        .tasks
        .iter()
        .filter(|task| task.name.starts_with("claude_bash_trace_"))
        .collect::<Vec<_>>();
    let rtk_tasks = report
        .tasks
        .iter()
        .filter(|task| task.name.starts_with("claude_rtk_hook_trace_"))
        .collect::<Vec<_>>();

    assert_eq!(tke_tasks.len(), 17);
    assert_eq!(rtk_tasks.len(), 17);

    let summarize = |tasks: &[&crate::benchmark::BenchmarkTaskReport]| {
        let raw_tokens = tasks.iter().map(|task| task.raw_tokens).sum::<usize>();
        let rewritten_tokens = tasks
            .iter()
            .map(|task| task.rewritten_tokens)
            .sum::<usize>();
        let saved = tasks.iter().map(|task| task.tokens_saved).sum::<isize>();
        let required = tasks
            .iter()
            .map(|task| task.required_fragments.len())
            .sum::<usize>();
        let kept = tasks
            .iter()
            .map(|task| task.preserved_fragments.len())
            .sum::<usize>();
        let savings = if raw_tokens == 0 {
            0.0
        } else {
            saved as f64 / raw_tokens as f64 * 100.0
        };
        (raw_tokens, rewritten_tokens, saved, savings, kept, required)
    };

    let (tke_raw, tke_rewritten, tke_saved, tke_savings, tke_kept, tke_required) =
        summarize(&tke_tasks);
    let (rtk_raw, rtk_rewritten, rtk_saved, rtk_savings, rtk_kept, rtk_required) =
        summarize(&rtk_tasks);

    let tke_row = format!(
        "| `tke` | {tke_raw} | {tke_rewritten} | {tke_saved} | {:.1}% | `{tke_kept}/{tke_required}` |",
        tke_savings
    );
    let rtk_row = format!(
        "| `rtk-hook` | {rtk_raw} | {rtk_rewritten} | {rtk_saved} | {:.1}% | `{rtk_kept}/{rtk_required}` |",
        rtk_savings
    );
    assert!(docs.contains(&tke_row), "{tke_row}");
    assert!(docs.contains(&rtk_row), "{rtk_row}");

    let vs_sentence = format!(
        "`tke` saves `{tke_saved}` tokens total at `{:.1}%`, while `rtk-hook` saves `{rtk_saved}` at `{:.1}%`",
        tke_savings, rtk_savings
    );
    assert!(vs_doc.contains(&vs_sentence), "{vs_sentence}");
}

#[test]
fn benchmark_docs_match_claude_scenario_delta_rows() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let report = build_benchmark_report(&cfg).expect("benchmark");
    let docs = fs::read_to_string("docs/benchmarks.md").expect("benchmarks doc");

    for (label, tke_name, rtk_name) in claude_synthetic_task_pairs() {
        let tke = report
            .tasks
            .iter()
            .find(|task| task.name == tke_name)
            .expect("tke task");
        let rtk = report
            .tasks
            .iter()
            .find(|task| task.name == rtk_name)
            .expect("rtk task");
        let token_delta = rtk.tokens_saved - tke.tokens_saved;
        let ratio_delta = (rtk.tokens_saved_ratio - tke.tokens_saved_ratio) * 100.0;
        let full_tke = tke.preserved_fragments.len() == tke.required_fragments.len();
        let full_rtk = rtk.preserved_fragments.len() == rtk.required_fragments.len();
        let near_tie = full_tke
            && full_rtk
            && ((token_delta.abs() <= 250)
                || ((token_delta.abs() as f64)
                    <= 0.02 * (tke.tokens_saved.abs().max(rtk.tokens_saved.abs()) as f64)))
            && ratio_delta.abs() <= 0.5;
        let verdict = if near_tie { "near-tie" } else { "mixed" };
        let status = if full_tke && full_rtk {
            "both full"
        } else {
            "not-both-full"
        };
        let row = format!(
            "| {label} | `{:+}` | `{:+.1} pp` | `{status}` | `{verdict}` |",
            token_delta, ratio_delta
        );
        assert!(docs.contains(&row), "{row}");
    }
}

#[test]
fn rtk_docs_lock_fair_compare_aggregate_rows() {
    let benchmarks = fs::read_to_string("docs/benchmarks.md").expect("benchmarks doc");
    let vs_doc = fs::read_to_string("docs/rtk-vs-tke.md").expect("rtk vs tke doc");

    for row in [
        "| `rtk-codex-rules` | 2 | 1 | 1 | 0 | 0 | 11 |",
        "| `fairfind` | `rtk-codex-rules` | pass | 0 | `correct_but_not_saved` |",
        "| `fairbuild` | `rtk-hook` | no | no | no | - |",
        "| `fairfind` | `rtk-hook` | no | no | no | - |",
        "| `fairrg` | `rtk-hook` | no | no | no | - |",
    ] {
        assert!(benchmarks.contains(row), "{row}");
    }

    for fragment in [
        "| `rtk-codex-rules` | 2 fair cases | 1 | 1 | `11` token delta total |",
        "| `rtk-hook` | 4 | 3 | 0 | 1 | `-1` total delta |",
        "| `codex` | `fairfind` | pass | missing | pass |",
        "| `codex` | `fairrg` | fail | missing | fail |",
        "| `claude` | `fairbuild` | fail | fail | pass | `1167` saved | `saved_but_wrong` |",
        "| `claude` | `fairfind` | fail | fail | pass | `20` saved | `saved_but_wrong` |",
        "| `claude` | `fairrg` | pass | pass | pass | `4797` saved | `saved_and_correct` |",
    ] {
        assert!(vs_doc.contains(fragment), "{fragment}");
    }
}

#[test]
fn e2e_docs_match_generated_stable_case_rows_and_verdicts() {
    let base = temp_test_dir("e2e-doc-lock");
    let codex_root = base.join(".tmp-codex-e2e");
    let claude_root = base.join(".tmp-claude-e2e");
    let claude_fair_root = base.join(".tmp-claude-e2e-fair");
    fs::create_dir_all(&codex_root).expect("mkdir codex");
    fs::create_dir_all(&claude_root).expect("mkdir claude");
    fs::create_dir_all(&claude_fair_root).expect("mkdir claude fair");

    write_codex_e2e_sample(
        &codex_root.join("buildcase.raw.jsonl"),
        "test result: ok. 105 passed; 0 failed; 0 ignored; 0 measured",
        "STAGE=cargo test --lib -- --nocapture\nFILE=src/lib.rs\nCOUNT=0",
    );
    write_codex_e2e_sample(
        &codex_root.join("buildcase.tke.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"cargo\",\"sc\":\"cargo\",\"sr\":\"build\",\"p\":\"log\",\"bd\":{\"ok\":105,\"fl\":0,\"tt\":105}}",
        "STAGE=cargo test --lib -- --nocapture\nFILE=src/lib.rs\nCOUNT=0",
    );
    write_codex_e2e_sample(
        &codex_root.join("fairbuild.raw.jsonl"),
        "test result: ok. 105 passed; 0 failed; 0 ignored; 0 measured",
        "STAGE=cargo test --lib -- --nocapture | tail -n 80\nFILE=src/lib.rs\nCOUNT=0",
    );
    write_codex_e2e_sample(
        &codex_root.join("fairfind.raw.jsonl"),
        repeated_lines("src/rollout_stats.rs", 17).as_str(),
        "STAGE=rg --files src | head -n 40\nFILE=src/rollout_stats.rs\nCOUNT=12",
    );
    write_codex_e2e_sample(
        &codex_root.join("fairfind.rtk-codex-rules.jsonl"),
        repeated_lines("src/rollout_stats.rs", 17).as_str(),
        "STAGE=rg --files src | head -n 40\nFILE=src/rollout_stats.rs\nCOUNT=12",
    );
    write_codex_e2e_sample(
        &codex_root.join("fairrg.raw.jsonl"),
        repeated_lines(
            "src/tests.rs:10:normalize_text compare-e2e benchmark-commands",
            8,
        )
        .as_str(),
        "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/shim.rs\nKIND=search",
    );
    write_codex_e2e_sample(
        &codex_root.join("fairrg.rtk-codex-rules.jsonl"),
        "1 result in src/tests.rs",
        "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/shim.rs\nKIND=search",
    );
    write_codex_e2e_sample(
        &codex_root.join("findcase.raw.jsonl"),
        repeated_lines("src/tests.rs", 17).as_str(),
        "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17",
    );
    write_codex_e2e_sample(
        &codex_root.join("findcase.tke.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"find\",\"sc\":\"find\",\"sr\":\"search\",\"p\":\"pathlist\",\"c\":17}",
        "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17",
    );
    write_codex_e2e_sample(
        &codex_root.join("realtask.raw.jsonl"),
        repeated_lines("src/benchmark.rs:42:task compare row", 40).as_str(),
        "STAGE=sed -n '1,120p' src/benchmark.rs\nFILE=src/benchmark.rs\nCOUNT=40",
    );
    write_codex_e2e_sample(
        &codex_root.join("realtask.tke.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"sed\",\"sc\":\"sed\",\"sr\":\"file\",\"p\":\"file\",\"c\":40}",
        "STAGE=sed -n '1,120p' src/benchmark.rs\nFILE=src/benchmark.rs\nCOUNT=40",
    );
    write_codex_e2e_sample(
        &codex_root.join("rgcase.raw.jsonl"),
        repeated_lines("src/tests.rs:10:assert benchmark normalize claude", 8).as_str(),
        "STAGE=rg -n \"assert|benchmark|normalize|claude\" src/tests.rs\nFILE=src/tests.rs\nKIND=search",
    );
    write_codex_e2e_sample(
        &codex_root.join("rgcase.tke.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"rg\",\"sc\":\"rg\",\"sr\":\"search\",\"p\":\"search\"}",
        "STAGE=rg -n \"assert|benchmark|normalize|claude\" src/tests.rs\nFILE=src/tests.rs\nKIND=search",
    );

    write_claude_e2e_gateway_sample(
        &claude_root.join("findcase.raw.stream.jsonl"),
        repeated_lines("src/tests.rs", 17).as_str(),
        "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17",
    );
    write_claude_e2e_sample(
        &claude_root.join("findcase.tke.stream.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"find\",\"sc\":\"find\",\"sr\":\"search\",\"p\":\"pathlist\",\"c\":17}",
        "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=16",
    );
    write_claude_e2e_gateway_sample(
        &claude_root.join("findcase.rtk-hook.stream.jsonl"),
        repeated_lines("src/tests.rs", 17).as_str(),
        "STAGE=find src -type f | head -n 40\nFILE=src/tests.rs\nCOUNT=17",
    );

    write_claude_e2e_sample(
        &claude_fair_root.join("fairbuild.raw.stream.jsonl"),
        "test result: ok. 105 passed; 0 failed; 0 ignored; 0 measured",
        "STAGE=cargo test --lib\nFILE=src/tests.rs\nCOUNT=4",
    );
    write_claude_e2e_sample(
        &claude_fair_root.join("fairbuild.tke.stream.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"cargo\",\"sc\":\"cargo\",\"sr\":\"build\",\"p\":\"log\",\"bd\":{\"ok\":105,\"fl\":0,\"tt\":105}}",
        "STAGE=test\nFILE=src/tests.rs\nCOUNT=4",
    );
    write_claude_e2e_sample(
        &claude_fair_root.join("fairfind.raw.stream.jsonl"),
        repeated_lines("src/rollout_stats.rs", 17).as_str(),
        "STAGE=list_files\nFILE=src/rollout_stats.rs\nCOUNT=19",
    );
    write_claude_e2e_sample(
        &claude_fair_root.join("fairfind.tke.stream.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"rg\",\"sc\":\"rg\",\"sr\":\"search\",\"p\":\"pathlist\",\"c\":17}",
        "STAGE=preprocess\nFILE=rollout_stats.rs\nCOUNT=4",
    );
    write_claude_e2e_sample(
        &claude_fair_root.join("fairrg.raw.stream.jsonl"),
        repeated_lines(
            "src/tests.rs:10:normalize_text compare-e2e benchmark-commands",
            8,
        )
        .as_str(),
        "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/tests.rs\nKIND=search",
    );
    write_claude_e2e_sample(
        &claude_fair_root.join("fairrg.tke.stream.jsonl"),
        "__TKE__{\"v\":1,\"cmd\":\"rg\",\"sc\":\"rg\",\"sr\":\"search\",\"p\":\"search\"}",
        "STAGE=rg -n \"normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands\" src\nFILE=src/tests.rs\nKIND=search",
    );

    let report = build_e2e_compare_report(
        vec![codex_root, claude_root, claude_fair_root],
        None,
        &Config::default(),
    )
    .expect("e2e report");
    let docs = fs::read_to_string("docs/e2e.md").expect("e2e doc");

    let codex_cases = report
        .cases
        .iter()
        .filter(|case| case.agent == "codex")
        .collect::<Vec<_>>();
    for case in codex_cases {
        let notes = if e2e_case_status(case, "tke") == "pass" {
            "stable tke case"
        } else {
            "-"
        };
        let row = format!(
            "| `{}` | {} | {} | {} | {} |",
            case.name,
            case.baseline.correctness.status,
            e2e_case_status(case, "tke"),
            e2e_case_status(case, "rtk-codex-rules"),
            notes
        );
        assert!(docs.contains(&row), "{row}");
    }

    let claude_cases = report
        .cases
        .iter()
        .filter(|case| case.agent == "claude")
        .collect::<Vec<_>>();
    for case in claude_cases {
        let notes = match (case.name.as_str(), e2e_case_status(case, "tke").as_str()) {
            ("findcase", _) => "experimental live tke path, gateway noise on RTK hook path",
            (_, "pass") => "live tke correct",
            (_, "fail") => "experimental live tke path",
            _ => "-",
        };
        let row = format!(
            "| `{}` | {} | {} | {} | {} |",
            case.name,
            case.baseline.correctness.status,
            e2e_case_status(case, "tke"),
            e2e_case_status(case, "rtk-hook"),
            notes
        );
        assert!(docs.contains(&row), "{row}");
    }

    for line in [
        "- Codex vs RTK must use `rtk-codex-rules`.",
        "- Claude vs RTK must use `rtk-hook`.",
        "- `rtk-direct` is not the official fairness path for Codex.",
        "- Codex remains the primary validated live-compression path.",
        "- Claude tool compression works via PATH shims; agent output is passed through.",
        "- RTK results must be reported per agent integration mode, not as one universal number.",
    ] {
        assert!(docs.contains(line), "{line}");
    }
}

#[test]
fn render_posix_activate_script() {
    let script = render_activate_script(
        ShellKind::Posix,
        Path::new("/tmp/tke"),
        Path::new("/tmp/shims"),
        "/usr/bin:/bin",
        &["codex".to_owned()],
        &["cat".to_owned(), "rg".to_owned()],
    );
    assert!(script.contains("export TKE_BIN='/tmp/tke'"));
    assert!(script.contains("export PATH='/tmp/shims':$PATH"));
}

#[test]
fn render_powershell_activate_script() {
    let script = render_activate_script(
        ShellKind::PowerShell,
        Path::new("C:\\tke\\tke.exe"),
        Path::new("C:\\tke\\shims"),
        "C:\\Windows\\System32;C:\\Windows",
        &["codex".to_owned()],
        &["cat".to_owned(), "rg".to_owned()],
    );
    assert!(script.contains("$env:TKE_BIN = 'C:\\tke\\tke.exe'"));
    assert!(script.contains("$env:PATH = 'C:\\tke\\shims' + ';' + $env:PATH"));
}

#[test]
fn render_cmd_activate_script() {
    let script = render_activate_script(
        ShellKind::Cmd,
        Path::new("C:\\tke\\tke.exe"),
        Path::new("C:\\tke\\shims"),
        "C:\\Windows\\System32;C:\\Windows",
        &["codex".to_owned()],
        &["cat".to_owned()],
    );
    assert!(script.contains("set \"TKE_BIN=C:\\tke\\tke.exe\""));
    assert!(script.contains("set \"PATH=C:\\tke\\shims;%PATH%\""));
}

#[test]
fn render_powershell_deactivate_script_restores_real_path() {
    let script = render_deactivate_script(ShellKind::PowerShell);
    assert!(script.contains("$env:TKE_REAL_PATH"));
    assert!(script.contains("Remove-Item Env:TKE_BIN"));
}

#[test]
fn candidate_command_names_expand_windows_pathext() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    set_env_var("PATHEXT", ".EXE;.CMD");
    let names = candidate_command_names("rg");
    remove_env_var("PATHEXT");
    let rendered = names
        .iter()
        .map(|value| value.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if cfg!(windows) {
        assert!(rendered.contains(&"rg".to_owned()));
        assert!(rendered.contains(&"rg.EXE".to_owned()) || rendered.contains(&"rg.exe".to_owned()));
        assert!(rendered.contains(&"rg.CMD".to_owned()) || rendered.contains(&"rg.cmd".to_owned()));
    } else {
        assert_eq!(rendered, vec!["rg".to_owned()]);
    }
}

#[test]
fn create_windows_exe_shim_writes_executable_shim() {
    let base = temp_test_dir("windows-shim");
    fs::create_dir_all(&base).expect("base");
    let exe = base.join("tke.exe");
    fs::write(&exe, b"demo").expect("exe");
    create_windows_exe_shim(&base, &exe, "rg").expect("shim");
    let shim = base.join(if cfg!(windows) { "rg.exe" } else { "rg.exe" });
    assert!(shim.exists());
    assert!(fs::metadata(&shim).expect("shim metadata").len() > 0);
}

#[test]
fn shim_command_path_matches_platform_wrapper_shape() {
    let base = Path::new("C:\\tke\\shims");
    let shim = shim_command_path(base, "codex");
    if cfg!(windows) {
        assert_eq!(shim, base.join("codex.exe"));
    } else {
        assert_eq!(shim, base.join("codex"));
    }
}

#[test]
fn default_runtime_shim_dir_uses_temp_space_not_workspace_tke_dir() {
    let path = default_runtime_shim_dir();
    let rendered = path.to_string_lossy().replace('\\', "/");
    assert!(rendered.contains("/shims"));
    assert!(rendered.contains("tke-run-"));
    assert!(!rendered.contains("/.tke/shims"));
}

#[test]
fn default_activate_shim_dir_uses_temp_space_not_workspace_tke_dir() {
    let path = default_activate_shim_dir();
    let rendered = path.to_string_lossy().replace('\\', "/");
    assert!(rendered.contains("/tke/shims"));
    assert!(!rendered.contains("/.tke/shims"));
}

#[cfg(windows)]
#[test]
fn passthrough_runs_cmd_scripts_on_windows() {
    let base = temp_test_dir("passthrough-cmd");
    fs::create_dir_all(&base).expect("base");
    let script = base.join("echo.cmd");
    let output = base.join("result.txt");
    fs::write(
        &script,
        format!(
            "@echo off\r\necho %1>%~dp0{}\r\n",
            output.file_name().unwrap().to_string_lossy()
        ),
    )
    .expect("script");

    let code = crate::shim::passthrough(&script, &["hello".to_owned()], None, None, false)
        .expect("passthrough");

    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(output).expect("output").trim(), "hello");
}

#[test]
fn capture_interactive_rewrites_explicit_source_and_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let base = temp_test_dir("capture-explicit");
    let source = base.join("rollout.jsonl");
    let output_dir = base.join("out");
    fs::create_dir_all(&base).expect("base");
    let line = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_11",
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
            "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    fs::write(&source, format!("{line}\n")).expect("write source");

    capture_interactive(Some(source.clone()), Some(output_dir.clone()), &cfg).expect("capture");

    let mirrored = output_dir.join("rollout.jsonl");
    let raw = fs::read_to_string(mirrored).expect("mirrored");
    let value = value_from_json(raw.lines().next().expect("line"));
    let nested = value_from_json(
        value["item"]["aggregated_output"]
            .as_str()
            .expect("aggregated_output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
}

#[test]
fn capture_interactive_uses_codex_home_latest_rollout() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let base = temp_test_dir("capture-defaults");
    let codex_home = base.join("codex-home");
    let sessions = codex_home.join("sessions/2026/05/23");
    let project = base.join("project");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(&project).expect("project");
    let rollout = sessions.join("rollout-latest.jsonl");
    let line = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_12",
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
            "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    fs::write(&rollout, format!("{line}\n")).expect("write rollout");

    let original_cwd = std::env::current_dir().expect("cwd");
    let original_codex_home = std::env::var_os("CODEX_HOME");
    std::env::set_current_dir(&project).expect("chdir");
    set_env_var("CODEX_HOME", &codex_home);

    let result = capture_interactive(None, None, &cfg);

    if let Some(value) = original_codex_home {
        set_env_var("CODEX_HOME", value);
    } else {
        remove_env_var("CODEX_HOME");
    }
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    result.expect("capture");
    let mirrored = project.join(".tke/interactive/rollout-latest.jsonl");
    assert!(mirrored.exists());
}

#[test]
fn capture_interactive_errors_without_rollout() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let cfg = Config::default();
    let base = temp_test_dir("capture-missing");
    let codex_home = base.join("codex-home");
    let claude_home = base.join("claude-home");
    let project = base.join("project");
    fs::create_dir_all(codex_home.join("sessions")).expect("sessions");
    fs::create_dir_all(claude_home.join("sessions")).expect("sessions");
    fs::create_dir_all(&project).expect("project");

    let original_cwd = std::env::current_dir().expect("cwd");
    let original_codex_home = std::env::var_os("CODEX_HOME");
    let original_claude_home = std::env::var_os("CLAUDE_HOME");
    std::env::set_current_dir(&project).expect("chdir");
    set_env_var("CODEX_HOME", &codex_home);
    set_env_var("CLAUDE_HOME", &claude_home);

    let err = capture_interactive(None, None, &cfg).expect_err("missing rollout");

    if let Some(value) = original_codex_home {
        set_env_var("CODEX_HOME", value);
    } else {
        remove_env_var("CODEX_HOME");
    }
    if let Some(value) = original_claude_home {
        set_env_var("CLAUDE_HOME", value);
    } else {
        remove_env_var("CLAUDE_HOME");
    }
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    assert!(matches!(err, AppError::Usage(_)));
    assert!(
        err.to_string()
            .contains("could not find any agent rollout jsonl")
    );
}

#[test]
fn interactive_tracker_writes_rewritten_rollout_copy() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let base = temp_test_dir("tracker-write");
    let sessions = base.join("sessions/2026/05/23");
    let output_dir = base.join("project/.tke/interactive");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(base.join("project")).expect("project");

    let rollout = sessions.join("rollout-test.jsonl");
    let line = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item_1",
            "type": "command_execution",
            "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
            "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
            "exit_code": 0,
            "status": "completed"
        }
    })
    .to_string();
    fs::write(&rollout, format!("{line}\n")).expect("write rollout");

    let original_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(base.join("project")).expect("chdir");
    let tracker = InteractiveTracker {
        sessions_dir: base.join("sessions"),
        started_at_ms: 0,
        agent: "codex",
    };
    tracker.finish(&cfg).expect("finish");
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    let mirrored = output_dir.join("rollout-test.jsonl");
    let raw = fs::read_to_string(mirrored).expect("mirrored");
    let value = value_from_json(raw.lines().next().expect("line"));
    let nested = value_from_json(
        value["item"]["aggregated_output"]
            .as_str()
            .expect("aggregated_output")
            .trim_start_matches("__TKE__"),
    );
    assert_eq!(nested["sc"], "rg");
    assert_eq!(nested["sr"], "search");
}

#[test]
fn interactive_tracker_skips_unmodified_rollouts() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let cfg = Config::default();
    let base = temp_test_dir("tracker-skip");
    let sessions = base.join("sessions/2026/05/23");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(base.join("project")).expect("project");
    fs::write(
        sessions.join("rollout-test.jsonl"),
        "{\"type\":\"message\",\"payload\":{\"text\":\"hello\"}}\n",
    )
    .expect("write rollout");

    let original_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(base.join("project")).expect("chdir");
    let tracker = InteractiveTracker {
        sessions_dir: base.join("sessions"),
        started_at_ms: 0,
        agent: "codex",
    };
    tracker.finish(&cfg).expect("finish");
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    assert!(
        !base
            .join("project/.tke/interactive/rollout-test.jsonl")
            .exists()
    );
}

#[test]
fn claude_interactive_tracker_writes_rewritten_rollout_copy() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let base = temp_test_dir("claude-tracker");
    let claude_home = base.join("claude-home");
    let sessions_dir = claude_home.join("sessions");
    let projects_dir = claude_home.join("projects").join("-tmp-test");
    let output_dir = base.join("project/.tke/interactive");
    fs::create_dir_all(&sessions_dir).expect("sessions");
    fs::create_dir_all(&projects_dir).expect("projects");
    fs::create_dir_all(base.join("project")).expect("project");

    let session_id = "test-session-uuid-1234";
    let session_json = serde_json::json!({
        "pid": 12345,
        "sessionId": session_id,
        "cwd": "/tmp/test",
        "startedAt": 0u64,
        "kind": "interactive"
    });
    fs::write(
        sessions_dir.join("12345.json"),
        serde_json::to_string(&session_json).unwrap(),
    )
    .expect("write session json");

    let rollout = projects_dir.join(format!("{session_id}.jsonl"));
    let line = serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "tool_1",
                    "name": "Bash",
                    "input": { "command": "cat /tmp/demo.rs | rg -n fn | head" }
                }
            ]
        }
    })
    .to_string();
    let result_line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "tool_1",
                    "content": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160))
                }
            ]
        }
    })
    .to_string();
    fs::write(&rollout, format!("{line}\n{result_line}\n")).expect("write rollout");

    let original_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(base.join("project")).expect("chdir");
    let tracker = InteractiveTracker {
        sessions_dir: sessions_dir.clone(),
        started_at_ms: 0,
        agent: "claude",
    };
    tracker.finish(&cfg).expect("finish");
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    let mirrored = output_dir.join(format!("{session_id}.jsonl"));
    assert!(mirrored.exists(), "mirrored file should exist");
    let raw = fs::read_to_string(&mirrored).expect("mirrored");
    assert!(raw.contains("__TKE__"), "should contain rewritten output");
}

#[test]
fn capture_interactive_finds_claude_sessions_when_no_codex() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    let base = temp_test_dir("capture-claude");
    let codex_home = base.join("codex-home");
    let claude_home = base.join("claude-home");
    let sessions_dir = claude_home.join("sessions");
    let projects_dir = claude_home.join("projects").join("-tmp-test");
    let project = base.join("project");
    fs::create_dir_all(codex_home.join("sessions")).expect("codex sessions");
    fs::create_dir_all(&sessions_dir).expect("claude sessions");
    fs::create_dir_all(&projects_dir).expect("projects");
    fs::create_dir_all(&project).expect("project");

    let session_id = "capture-test-uuid-5678";
    let session_json = serde_json::json!({
        "pid": 99999,
        "sessionId": session_id,
        "cwd": "/tmp/test",
        "startedAt": 0u64,
        "kind": "interactive"
    });
    fs::write(
        sessions_dir.join("99999.json"),
        serde_json::to_string(&session_json).unwrap(),
    )
    .expect("write session json");

    let rollout = projects_dir.join(format!("{session_id}.jsonl"));
    let line = serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "tool_a",
                    "name": "Bash",
                    "input": { "command": "find src -name '*.rs' | head -n 40" }
                }
            ]
        }
    })
    .to_string();
    let result_line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "tool_a",
                    "content": format!("{}\n", repeated_lines("/tmp/project/src/lib.rs", 160))
                }
            ]
        }
    })
    .to_string();
    fs::write(&rollout, format!("{line}\n{result_line}\n")).expect("write rollout");

    let original_cwd = std::env::current_dir().expect("cwd");
    let original_codex_home = std::env::var_os("CODEX_HOME");
    let original_claude_home = std::env::var_os("CLAUDE_HOME");
    std::env::set_current_dir(&project).expect("chdir");
    set_env_var("CODEX_HOME", &codex_home);
    set_env_var("CLAUDE_HOME", &claude_home);

    let result = capture_interactive(None, None, &cfg);

    if let Some(value) = original_codex_home {
        set_env_var("CODEX_HOME", value);
    } else {
        remove_env_var("CODEX_HOME");
    }
    if let Some(value) = original_claude_home {
        set_env_var("CLAUDE_HOME", value);
    } else {
        remove_env_var("CLAUDE_HOME");
    }
    std::env::set_current_dir(original_cwd).expect("restore cwd");

    result.expect("capture");
    let mirrored = project.join(format!(".tke/interactive/{session_id}.jsonl"));
    assert!(mirrored.exists(), "should find and mirror Claude session");
}

#[test]
fn find_latest_claude_rollout_after_resolves_session_json_to_jsonl() {
    let base = temp_test_dir("claude-find");
    let claude_home = base.join("claude-home");
    let sessions_dir = claude_home.join("sessions");
    let projects_dir = claude_home.join("projects").join("-tmp-resolve");
    fs::create_dir_all(&sessions_dir).expect("sessions");
    fs::create_dir_all(&projects_dir).expect("projects");

    let session_json = serde_json::json!({
        "pid": 42,
        "sessionId": "resolve-uuid-abcd",
        "cwd": "/tmp/resolve",
        "startedAt": 1000u64,
        "kind": "interactive"
    });
    fs::write(
        sessions_dir.join("42.json"),
        serde_json::to_string(&session_json).unwrap(),
    )
    .expect("write session json");

    let jsonl = projects_dir.join("resolve-uuid-abcd.jsonl");
    fs::write(&jsonl, "{\"type\":\"message\"}\n").expect("write jsonl");

    let result =
        crate::rollout_io::find_latest_claude_rollout_after(&sessions_dir, 0).expect("find");
    assert!(result.is_some());
    assert_eq!(result.unwrap(), jsonl);
}

#[test]
fn find_latest_claude_rollout_after_filters_by_started_at() {
    let base = temp_test_dir("claude-filter");
    let claude_home = base.join("claude-home");
    let sessions_dir = claude_home.join("sessions");
    let projects_dir = claude_home.join("projects").join("-tmp-old");
    fs::create_dir_all(&sessions_dir).expect("sessions");
    fs::create_dir_all(&projects_dir).expect("projects");

    let session_json = serde_json::json!({
        "pid": 99,
        "sessionId": "old-uuid-xxxx",
        "cwd": "/tmp/old",
        "startedAt": 1000u64,
        "kind": "interactive"
    });
    fs::write(
        sessions_dir.join("99.json"),
        serde_json::to_string(&session_json).unwrap(),
    )
    .expect("write session json");

    let jsonl = projects_dir.join("old-uuid-xxxx.jsonl");
    fs::write(&jsonl, "{\"type\":\"message\"}\n").expect("write jsonl");

    let result =
        crate::rollout_io::find_latest_claude_rollout_after(&sessions_dir, 20000).expect("find");
    assert!(
        result.is_none(),
        "should filter out sessions before started_at"
    );
}

#[test]
fn claude_encode_project_path_handles_unix_paths() {
    assert_eq!(
        crate::rollout_io::claude_encode_project_path("/root/github/tke"),
        "-root-github-tke"
    );
    assert_eq!(
        crate::rollout_io::claude_encode_project_path("/tmp/test"),
        "-tmp-test"
    );
}

#[test]
fn claude_encode_project_path_handles_windows_paths() {
    // C:\Users\user\project → C:-Users-user-project → C--Users-user-project
    assert_eq!(
        crate::rollout_io::claude_encode_project_path("C:\\Users\\user\\project"),
        "C--Users-user-project"
    );
    assert_eq!(
        crate::rollout_io::claude_encode_project_path("D:\\dev\\tke"),
        "D--dev-tke"
    );
}

#[test]
fn log_lower_threshold_compresses_small_output() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 2048;
    cfg.max_body_lines = 100;
    // 25 lines of build-like output, ~650 bytes — below default 2048 but above new 512 threshold
    let lines: Vec<String> = (0..25)
        .map(|i| format!("Compiling crate-{i} v0.1.0"))
        .collect();
    let text = lines.join("\n");
    assert!(text.len() < 2048);
    assert!(text.len() >= 512);
    let result = maybe_normalize_text(
        "cargo",
        &["test".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &cfg,
        None,
    )
    .expect("no error");
    assert!(result.is_some(), "Log output >=512 bytes should compress");
}

#[test]
fn log_progress_lines_count_as_signals() {
    assert!(has_log_progress("Compiling foo v0.1.0"));
    assert!(has_log_progress("Downloading serde v1.0.0"));
    assert!(has_log_progress("Fetching crates.io index"));
    assert!(has_log_progress("Installing package foo"));
    assert!(has_log_progress("Building wheel for foo"));
    assert!(has_log_progress("Testing test_foo ... ok"));
    assert!(has_log_progress("Generated docs/html/index.html"));
    assert!(!has_log_progress("some random output line"));
}

#[test]
fn table_three_rows_detected() {
    let table = "NAME  STATUS  AGE\nfoo   Running 1d\nbar   Running 2d\nbaz   Failed  3d";
    let lines: Vec<&str> = table.lines().collect();
    assert!(
        looks_like_table(&lines),
        "3 headers + 3 rows should be detected as table"
    );
}

#[test]
fn table_common_headers_recognized() {
    let table = "Name   Tag    Repository\nlatest v1.0   myrepo\nstable v2.0   myrepo\n dev   v3.0   myrepo";
    let lines: Vec<&str> = table.lines().collect();
    assert!(
        looks_like_table(&lines),
        "Name/Tag/Repository should be recognized as table headers"
    );
}

#[test]
fn generic_lower_threshold_compresses() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 2048;
    cfg.max_body_lines = 100;
    // 50 lines of generic output, ~2500 bytes — above new 1024 threshold and large enough for token savings
    let lines: Vec<String> = (0..50)
        .map(|i| format!("processing item {i} with some extra padding bytes for size"))
        .collect();
    let text = lines.join("\n");
    assert!(text.len() >= 1024);
    let result = maybe_normalize_text(
        "somecmd",
        &[],
        "stdout",
        CommandKind::Generic,
        &text,
        &cfg,
        None,
    )
    .expect("no error");
    assert!(
        result.is_some(),
        "Generic output >=1024 bytes and >=24 lines should compress"
    );
}

#[test]
fn generic_repeated_lines_folded() {
    // Use a large enough text so that should_force_trim triggers
    let plain = "same line output here for fold detection test\n";
    let repeated = plain.repeat(25);
    let text = format!("{repeated}different line\n{repeated}end");
    let result = normalize_text(
        "somecmd",
        &[],
        "stdout",
        CommandKind::Generic,
        &text,
        &Config::default(),
    )
    .expect("no error");
    assert!(
        result.contains("fold"),
        "Generic profile should fold repeated lines"
    );
    assert!(result.contains("c:25"), "fold should report count of 25");
}

#[test]
fn generic_structural_template_folds() {
    // Lines sharing a canonical prefix ("Processing item #") should fold
    let lines: Vec<String> = (0..20)
        .map(|i| format!("Processing item {i} of 100 in the batch"))
        .collect();
    let text = lines.join("\n");
    let result = normalize_text(
        "somecmd",
        &[],
        "stdout",
        CommandKind::Generic,
        &text,
        &Config::default(),
    )
    .expect("no error");
    assert!(
        result.contains("fold"),
        "Structural template detection should fold lines with shared canonical prefix"
    );
    assert!(result.contains("c:20"), "fold should report count of 20");
}

#[test]
fn generic_short_output_compresses() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 2048;
    cfg.max_body_lines = 100;
    // 50 lines of generic output, ~2000 bytes — above 512 threshold and large enough for token savings
    let lines: Vec<String> = (0..50)
        .map(|i| format!("Processing item {i} of 100 in the batch"))
        .collect();
    let text = lines.join("\n");
    assert!(text.len() >= 512);
    let result = maybe_normalize_text(
        "somecmd",
        &[],
        "stdout",
        CommandKind::Generic,
        &text,
        &cfg,
        None,
    )
    .expect("no error");
    assert!(
        result.is_some(),
        "Generic output >=512 bytes and >=16 lines should compress"
    );
}

#[test]
fn search_profile_includes_context() {
    let lines: Vec<String> = vec![
        "line before match".to_owned(),
        "fn alpha_function() {}".to_owned(),
        "line after match".to_owned(),
        "unrelated line".to_owned(),
    ];
    let text = lines.join("\n");
    let result = normalize_text(
        "rg",
        &["alpha".to_owned()],
        "stdout",
        CommandKind::Search,
        &text,
        &Config::default(),
    )
    .expect("no error");
    // With match_context=1, the line before and after the match should be included
    assert!(
        result.contains("line before match") || result.contains("line after match"),
        "Search profile with match_context=1 should include context around matches"
    );
}

#[test]
fn log_structural_template_folds_build_output() {
    // Build-like output with interleaved progress lines should fold via structural templates
    let lines: Vec<String> = (0..20)
        .map(|i| format!("Compiling crate-{i} v0.1.0"))
        .collect();
    let text = lines.join("\n");
    let result = normalize_text(
        "cargo",
        &["build".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &Config::default(),
    )
    .expect("no error");
    assert!(
        result.contains("fold"),
        "Log profile should fold build output via structural templates"
    );
}

#[test]
fn log_summary_includes_progress_count() {
    let lines = vec![
        "Compiling serde v1.0.0",
        "Compiling tokio v1.0.0",
        "Downloading regex v1.0.0",
        "warning: unused variable `x`",
        "error: build failed",
    ];
    let summary = crate::log_profile::collect_log_summary(&lines);
    assert_eq!(summary.progress, 3, "should count 3 progress lines");
    assert_eq!(summary.warn, 1, "should count 1 warning");
    assert_eq!(summary.fail, 1, "should count 1 failure");
}

#[test]
fn log_summary_extracts_build_crates_and_elapsed() {
    let lines = vec![
        "   Compiling serde v1.0.0",
        "   Compiling tokio v1.28.0",
        "   Compiling hyper v0.14.27",
        "   Compiling reqwest v0.11.18",
        "   Compiling my-app v0.1.0 (/home/user/my-app)",
        "warning: unused variable `x` in src/main.rs",
        "    Finished release [optimized] in 42.5s",
    ];
    let summary = crate::log_profile::collect_log_summary(&lines);
    assert_eq!(summary.crates, 5, "should count 5 compiling lines");
    assert_eq!(
        summary.elapsed,
        Some("42.5s".to_owned()),
        "should extract elapsed time"
    );
    assert_eq!(summary.warn, 1);
    assert_eq!(
        summary.progress, 5,
        "compiling lines are also progress lines"
    );
}

#[test]
fn log_summary_handles_minutes_elapsed() {
    let lines = vec![
        "   Compiling libc v0.2.147",
        "   Compiling serde_derive v1.0.188",
        "    Finished dev [unoptimized] in 2m 15s",
    ];
    let summary = crate::log_profile::collect_log_summary(&lines);
    assert_eq!(summary.crates, 2);
    assert_eq!(summary.elapsed, Some("2m".to_owned()));
}

#[test]
fn log_summary_no_build_info() {
    let lines = vec!["Hello world", "Some output line", "Another line"];
    let summary = crate::log_profile::collect_log_summary(&lines);
    assert_eq!(summary.crates, 0);
    assert_eq!(summary.elapsed, None);
    assert_eq!(summary.progress, 0);
    assert_eq!(summary.warn, 0);
    assert_eq!(summary.fail, 0);
}

#[test]
fn log_profile_lower_threshold_triggers_on_small_build_output() {
    // Build output with 10 lines should trigger with the new lower threshold (8 lines OR 256 bytes)
    let lines: Vec<String> = (0..10)
        .map(|i| format!("   Compiling crate-{i} v0.1.0"))
        .collect();
    let text = lines.join("\n");
    let result = normalize_text(
        "cargo",
        &["build".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &Config::default(),
    )
    .expect("no error");
    let value: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(value["p"], "log", "should select log profile");
    assert_eq!(value["t"], true, "should be forced trim");
    assert!(value["bd"].is_object(), "should have build summary");
}

#[test]
fn file_profile_detects_typescript_outline() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    cfg.max_body_lines = 1;
    let text = [
        "import React from 'react';",
        "import { useState } from 'react';",
        "",
        "interface UserProps {",
        "  name: string;",
        "  age: number;",
        "}",
        "",
        "export function UserCard({ name, age }: UserProps) {",
        "  const [visible, setVisible] = useState(true);",
        "  return <div>{name}</div>;",
        "}",
        "",
        "export default function App() {",
        "  return <UserCard name='test' age={25} />;",
        "}",
    ]
    .join("\n");
    let json = normalize_text(
        "cat",
        &["src/App.tsx".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let matches = value["m"].as_array().expect("matches");
    let has_import = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("import"))
    });
    let has_interface = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("interface"))
    });
    let has_export_fn = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("export function"))
    });
    assert!(has_import, "should detect import statements as outline");
    assert!(has_interface, "should detect interface as outline");
    assert!(
        has_export_fn,
        "should detect export function as code boundary"
    );
}

#[test]
fn file_profile_detects_python_outline() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    cfg.max_body_lines = 1;
    let text = [
        "import os",
        "from pathlib import Path",
        "",
        "class Config:",
        "    def __init__(self, path):",
        "        self.path = path",
        "",
        "async def fetch_data(url):",
        "    return await request(url)",
        "",
        "def main():",
        "    config = Config('/tmp/config')",
        "    print(config.path)",
    ]
    .join("\n");
    let json = normalize_text(
        "cat",
        &["app.py".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let matches = value["m"].as_array().expect("matches");
    let has_import = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("import"))
    });
    let has_class = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("class"))
    });
    let has_async_def = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("async def"))
    });
    assert!(has_import, "should detect import as outline");
    assert!(has_class, "should detect class as code boundary");
    assert!(has_async_def, "should detect async def as code boundary");
}

#[test]
fn file_profile_detects_go_outline() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    cfg.max_body_lines = 1;
    let text = [
        "package main",
        "",
        "import \"fmt\"",
        "",
        "func main() {",
        "    fmt.Println(\"hello\")",
        "}",
        "",
        "func process(data []byte) error {",
        "    return nil",
        "}",
    ]
    .join("\n");
    let json = normalize_text(
        "cat",
        &["main.go".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let matches = value["m"].as_array().expect("matches");
    let has_import = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("import"))
    });
    let has_func = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("func"))
    });
    assert!(has_import, "should detect import as outline");
    assert!(has_func, "should detect func as code boundary");
}

#[test]
fn file_profile_detects_java_outline() {
    let mut cfg = Config::default();
    cfg.min_trim_bytes = 1;
    cfg.max_body_lines = 1;
    let text = [
        "package com.example;",
        "",
        "import java.util.List;",
        "",
        "public class UserService {",
        "    public static User findById(long id) {",
        "        return null;",
        "    }",
        "",
        "    private void validate(String input) {",
        "        // validation logic",
        "    }",
        "}",
    ]
    .join("\n");
    let json = normalize_text(
        "cat",
        &["UserService.java".to_owned()],
        "stdout",
        CommandKind::File,
        &text,
        &cfg,
    )
    .expect("normalize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let matches = value["m"].as_array().expect("matches");
    let has_import = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("import"))
    });
    let has_class = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("class"))
    });
    let has_public_static = matches.iter().any(|c| {
        c["l"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("public static"))
    });
    assert!(has_import, "should detect import as outline");
    assert!(has_class, "should detect class as code boundary");
    assert!(
        has_public_static,
        "should detect public static method as code boundary"
    );
}

#[test]
fn log_profile_catches_linking_and_checking_progress() {
    let lines = vec![
        "   Checking libc v0.2.147",
        "   Checking serde v1.0.188",
        "   Linking my-app",
        "   Running unittests src/lib.rs",
    ];
    for line in &lines {
        assert!(
            has_log_progress(line),
            "line should be detected as progress: {line}"
        );
    }
}

#[test]
fn real_world_npm_build_output_compresses() {
    let mut lines = Vec::new();
    for i in 0..30 {
        lines.push(format!(
            "Compiling module-{i} v1.0.0 (/src/modules/module-{i})"
        ));
    }
    lines.push("Creating optimized production build...".to_owned());
    lines.push("File sizes after gzip:".to_owned());
    lines.push("  45.2 kB  build/static/js/main.abc123.js".to_owned());
    lines.push("  1.2 kB   build/static/css/main.def456.css".to_owned());
    lines.push("The build folder is ready to be deployed.".to_owned());
    let text = lines.join("\n");
    let result = normalize_text(
        "npm",
        &["run".to_owned(), "build".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &Config::default(),
    )
    .expect("no error");
    let value: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(
        value["p"], "log",
        "npm build output should select log profile"
    );
    assert_eq!(value["t"], true, "npm build output should be forced");
}

#[test]
fn real_world_docker_build_output_compresses() {
    let mut lines = Vec::new();
    lines.push("Step 1/10 : FROM rust:1.70 as builder".to_owned());
    lines.push("    1234567890ab".to_owned());
    for i in 0..20 {
        lines.push(format!("Step {}/{} : RUN cargo build --release", i + 2, 21));
        lines.push("    Running in abcdef123456".to_owned());
        lines.push(format!("   Compiling crate-{i} v0.1.0 (/src/crate-{i})"));
        lines.push("    987654321fed".to_owned());
    }
    lines.push("Successfully built 1234567890ab".to_owned());
    lines.push("Successfully tagged myapp:latest".to_owned());
    let text = lines.join("\n");
    let result = normalize_text(
        "docker",
        &["build".to_owned(), ".".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &Config::default(),
    )
    .expect("no error");
    let value: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(
        value["p"], "log",
        "docker build output should select log profile"
    );
    assert_eq!(value["t"], true, "docker build output should be forced");
    assert!(
        value["bd"].is_object(),
        "docker build output should have build summary"
    );
}

#[test]
fn real_world_cargo_test_output_compresses() {
    let mut lines = Vec::new();
    lines.push("   Compiling my-app v0.1.0 (/home/user/my-app)".to_owned());
    for i in 0..20 {
        lines.push(format!("   Compiling dep-{i} v1.0.0 (/home/user/dep-{i})"));
    }
    lines.push("    Finished test [unoptimized + debuginfo] in 25.3s".to_owned());
    lines.push("     Running unittests src/lib.rs (target/debug/deps/my_app-abc123)".to_owned());
    for i in 0..25 {
        lines.push(format!("test tests::test_{i} ... ok"));
    }
    lines.push(
        "test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out".to_owned(),
    );
    let text = lines.join("\n");
    let result = normalize_text(
        "cargo",
        &["test".to_owned()],
        "stdout",
        CommandKind::Log,
        &text,
        &Config::default(),
    )
    .expect("no error");
    let value: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(
        value["p"], "log",
        "cargo test output should select log profile"
    );
    assert_eq!(value["t"], true, "cargo test output should be forced");
}
