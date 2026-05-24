use crate::adapter::rewrite_agent_transcript;
use crate::app::{AppError, Config};
use crate::benchmark::{RolloutCompareReport, estimate_text_tokens};
use crate::rollout_stats::collect_rollout_output_stats;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub(crate) struct E2eCompareReport {
    pub(crate) v: u8,
    pub(crate) summary: Vec<E2eAgentSummary>,
    pub(crate) cases: Vec<E2eCaseReport>,
}

#[derive(Serialize)]
pub(crate) struct E2eAgentSummary {
    pub(crate) agent: String,
    pub(crate) cases: usize,
    pub(crate) variants: usize,
    pub(crate) saved_and_correct: usize,
    pub(crate) correct_but_not_saved: usize,
    pub(crate) saved_but_wrong: usize,
    pub(crate) wrong_and_not_saved: usize,
    pub(crate) total_tool_tokens_saved: isize,
}

#[derive(Serialize)]
pub(crate) struct E2eCaseReport {
    pub(crate) agent: String,
    pub(crate) name: String,
    pub(crate) baseline: E2eSampleReport,
    pub(crate) variants: Vec<E2eVariantReport>,
}

#[derive(Serialize)]
pub(crate) struct E2eSampleReport {
    pub(crate) mode: String,
    pub(crate) source: String,
    pub(crate) tool_bytes: usize,
    pub(crate) tool_tokens: usize,
    pub(crate) tool_has_tke: bool,
    pub(crate) result: String,
    pub(crate) result_fields: BTreeMap<String, String>,
    pub(crate) correctness: E2eCorrectnessReport,
    pub(crate) rollout: RolloutCompareReport,
}

#[derive(Serialize)]
pub(crate) struct E2eVariantReport {
    pub(crate) mode: String,
    pub(crate) sample: E2eSampleReport,
    pub(crate) tool_bytes_saved: isize,
    pub(crate) tool_bytes_saved_ratio: f64,
    pub(crate) tool_tokens_saved: isize,
    pub(crate) tool_tokens_saved_ratio: f64,
    pub(crate) exact_result_match: bool,
    pub(crate) semantic_result_match: bool,
    pub(crate) expected_result_match: bool,
    pub(crate) verdict: String,
}

#[derive(Clone, Serialize)]
pub(crate) struct E2eCorrectnessReport {
    pub(crate) status: String,
    pub(crate) checked_fields: Vec<String>,
    pub(crate) notes: Vec<String>,
}

#[derive(Default)]
struct SampleModes {
    by_mode: BTreeMap<String, PathBuf>,
}

#[derive(Default)]
struct StreamFacts {
    tool_text: String,
    tool_has_tke: bool,
    result_text: String,
}

pub(crate) fn compare_e2e(
    sources: Vec<PathBuf>,
    agent: Option<String>,
    config: &Config,
) -> Result<(), AppError> {
    let report = build_e2e_compare_report(sources, agent.as_deref(), config)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

pub(crate) fn build_e2e_compare_report(
    sources: Vec<PathBuf>,
    agent: Option<&str>,
    config: &Config,
) -> Result<E2eCompareReport, AppError> {
    let mut cases = Vec::new();
    for ((agent_name, name), modes) in discover_e2e_cases(sources, agent)? {
        let Some(raw_path) = modes.by_mode.get("raw") else {
            continue;
        };
        let baseline = build_sample_report("raw", raw_path, config)?;
        let mut variants = Vec::new();
        for (mode, path) in &modes.by_mode {
            if mode == "raw" {
                continue;
            }
            let sample = build_sample_report(mode, path, config)?;
            let tool_bytes_saved = baseline.tool_bytes as isize - sample.tool_bytes as isize;
            let tool_tokens_saved = baseline.tool_tokens as isize - sample.tool_tokens as isize;
            let expected_result_match = sample.correctness.status == "pass";
            variants.push(E2eVariantReport {
                mode: mode.clone(),
                sample,
                tool_bytes_saved,
                tool_bytes_saved_ratio: ratio(tool_bytes_saved, baseline.tool_bytes),
                tool_tokens_saved,
                tool_tokens_saved_ratio: ratio(tool_tokens_saved, baseline.tool_tokens),
                exact_result_match: baseline.result
                    == variants_last_result(&variants).unwrap_or(""),
                semantic_result_match: false,
                expected_result_match,
                verdict: String::new(),
            });
            if let Some(last) = variants.last_mut() {
                last.exact_result_match = baseline.result == last.sample.result;
                last.semantic_result_match = semantic_result_match(
                    &baseline,
                    &last.sample,
                    case_expectation_for_name(&name).is_some(),
                );
                last.verdict =
                    build_variant_verdict(last.expected_result_match, last.tool_tokens_saved);
            }
        }
        variants.sort_by(|a, b| a.mode.cmp(&b.mode));
        cases.push(E2eCaseReport {
            agent: agent_name,
            name,
            baseline,
            variants,
        });
    }
    cases.sort_by(|a, b| a.agent.cmp(&b.agent).then(a.name.cmp(&b.name)));
    let summary = build_agent_summary(&cases);
    Ok(E2eCompareReport {
        v: 3,
        summary,
        cases,
    })
}

fn variants_last_result(variants: &[E2eVariantReport]) -> Option<&str> {
    variants
        .last()
        .map(|variant| variant.sample.result.as_str())
}

fn build_sample_report(
    mode: &str,
    path: &Path,
    config: &Config,
) -> Result<E2eSampleReport, AppError> {
    let facts = parse_stream_facts(path)?;
    let raw_text = fs::read_to_string(path)?;
    let rewritten = rewrite_agent_transcript(&raw_text, config)?;
    let rewritten_text = rewritten.as_deref().unwrap_or(&raw_text);
    let rollout =
        compare_rollout_pair(path, rewritten.is_some(), &raw_text, rewritten_text, config);
    let result_fields = parse_result_fields(&facts.result_text);
    let correctness = evaluate_result(path, &result_fields);
    Ok(E2eSampleReport {
        mode: mode.to_owned(),
        source: path.display().to_string(),
        tool_bytes: facts.tool_text.len(),
        tool_tokens: estimate_text_tokens(&facts.tool_text),
        tool_has_tke: is_tke_payload(&facts.tool_text),
        result: facts.result_text,
        result_fields,
        correctness,
        rollout,
    })
}

fn compare_rollout_pair(
    source: &Path,
    changed: bool,
    raw_text: &str,
    rewritten_text: &str,
    config: &Config,
) -> RolloutCompareReport {
    let raw_stats = collect_rollout_output_stats(raw_text, config);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    RolloutCompareReport::from_stats(source, changed, raw_stats, rewritten_stats)
}

fn ratio(saved: isize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        saved as f64 / total as f64
    }
}

fn discover_e2e_cases(
    roots: Vec<PathBuf>,
    agent: Option<&str>,
) -> Result<BTreeMap<(String, String), SampleModes>, AppError> {
    let search_roots = if roots.is_empty() {
        default_roots()
    } else {
        roots
    };
    let mut cases = BTreeMap::<(String, String), SampleModes>::new();
    for root in search_roots {
        if !root.exists() {
            continue;
        }
        let agent_name = detect_agent_name(&root);
        if let Some(filter) = agent
            && agent_name != filter
        {
            continue;
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.ends_with(".jsonl") {
                continue;
            }
            let Some((name, mode)) = parse_case_name_and_mode(file_name) else {
                continue;
            };
            let modes = cases.entry((agent_name.clone(), name)).or_default();
            let prefer_new = !file_name.contains(".failed.");
            match modes.by_mode.get(&mode) {
                None => {
                    modes.by_mode.insert(mode, path);
                }
                Some(existing) => {
                    let existing_name = existing
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    let existing_is_failed = existing_name.contains(".failed.");
                    if prefer_new && existing_is_failed {
                        modes.by_mode.insert(mode, path);
                    }
                }
            }
        }
    }
    Ok(cases)
}

fn parse_case_name_and_mode(file_name: &str) -> Option<(String, String)> {
    for mode in [
        "raw",
        "wrapped",
        "tke",
        "rtk",
        "rtk-hook",
        "rtk-direct",
        "rtk-codex-rules",
    ] {
        let marker = format!(".{mode}.");
        if let Some((head, _)) = file_name.split_once(&marker) {
            return Some((head.to_owned(), normalize_mode(mode)));
        }
    }
    None
}

fn normalize_mode(mode: &str) -> String {
    match mode {
        "wrapped" => "tke".to_owned(),
        other => other.to_owned(),
    }
}

fn default_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from(".tmp-claude-e2e"),
        PathBuf::from(".tmp-codex-e2e"),
    ]
}

fn detect_agent_name(path: &Path) -> String {
    let joined = path.display().to_string().to_ascii_lowercase();
    if joined.contains("claude") {
        "claude".to_owned()
    } else if joined.contains("codex") {
        "codex".to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn parse_stream_facts(path: &Path) -> Result<StreamFacts, AppError> {
    let text = fs::read_to_string(path)?;
    let mut facts = StreamFacts::default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(item) = value.get("item")
            && item.get("type").and_then(|value| value.as_str()) == Some("agent_message")
            && let Some(text) = item.get("text").and_then(|value| value.as_str())
            && !text.is_empty()
        {
            facts.result_text = text.to_owned();
        }
        if let Some(result) = value.get("result").and_then(|value| value.as_str())
            && !result.is_empty()
        {
            facts.result_text = result.to_owned();
        }
        if facts.tool_text.is_empty()
            && let Some(item) = value.get("item")
            && item.get("type").and_then(|value| value.as_str()) == Some("command_execution")
            && let Some(output) = item
                .get("aggregated_output")
                .and_then(|value| value.as_str())
            && !output.is_empty()
        {
            facts.tool_has_tke = is_tke_payload(output);
            facts.tool_text = output.to_owned();
        }
        if !facts.tool_text.is_empty() {
            if let Some(message) = value.get("message")
                && let Some(content) = message.get("content").and_then(|value| value.as_array())
                && let Some(text) = last_text_block(content)
            {
                facts.result_text = text;
            }
            continue;
        }
        if let Some(message) = value.get("message")
            && let Some(content) = message.get("content").and_then(|value| value.as_array())
        {
            if let Some(text) = first_tool_result_text(content) {
                facts.tool_has_tke = is_tke_payload(&text);
                facts.tool_text = text;
                continue;
            }
            if let Some(text) = last_text_block(content) {
                facts.result_text = text;
            }
        }
        if let Some(tool_use_result) = value.get("tool_use_result")
            && let Some(stdout) = tool_use_result
                .get("stdout")
                .and_then(|value| value.as_str())
            && !stdout.is_empty()
        {
            facts.tool_has_tke = is_tke_payload(stdout);
            facts.tool_text = stdout.to_owned();
        }
    }
    Ok(facts)
}

fn first_tool_result_text(content: &[serde_json::Value]) -> Option<String> {
    for block in content {
        if block.get("type").and_then(|value| value.as_str()) != Some("tool_result") {
            continue;
        }
        if let Some(text) = block.get("content").and_then(|value| value.as_str()) {
            return Some(text.to_owned());
        }
        if let Some(nested) = block.get("content").and_then(|value| value.as_array()) {
            for item in nested {
                if let Some(text) = item.get("text").and_then(|value| value.as_str()) {
                    return Some(text.to_owned());
                }
            }
        }
    }
    None
}

fn last_text_block(content: &[serde_json::Value]) -> Option<String> {
    let mut last = None;
    for block in content {
        if block.get("type").and_then(|value| value.as_str()) != Some("text") {
            continue;
        }
        if let Some(text) = block.get("text").and_then(|value| value.as_str())
            && !text.is_empty()
        {
            last = Some(text.to_owned());
        }
    }
    last
}

fn is_tke_payload(text: &str) -> bool {
    text.trim_start().starts_with("__TKE__")
}

fn parse_result_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for part in text.lines().flat_map(|line| line.split(',')) {
        let piece = part.trim();
        if piece.is_empty() {
            continue;
        }
        let Some((key, value)) = piece.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_uppercase();
        let value = normalize_field_value(value);
        if !key.is_empty() && !value.is_empty() {
            fields.insert(key, value);
        }
    }
    fields
}

fn normalize_field_value(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_cmp(value: &str) -> String {
    normalize_field_value(value).to_ascii_lowercase()
}

fn evaluate_result(path: &Path, fields: &BTreeMap<String, String>) -> E2eCorrectnessReport {
    let raw_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if let Ok(text) = fs::read_to_string(path)
        && (text.contains("API Error: 504") || text.contains("origin_gateway_timeout"))
    {
        return E2eCorrectnessReport {
            status: "gateway_error".to_owned(),
            checked_fields: Vec::new(),
            notes: vec![format!("{raw_name} hit transient gateway timeout")],
        };
    }

    let Some(expectation) = case_expectation(path) else {
        return E2eCorrectnessReport {
            status: "ungraded".to_owned(),
            checked_fields: Vec::new(),
            notes: vec!["no built-in expectation".to_owned()],
        };
    };

    let mut checked_fields = Vec::new();
    let mut notes = Vec::new();

    for key in expectation.required_non_empty {
        checked_fields.push((*key).to_owned());
        match fields.get(*key) {
            Some(value) if !value.is_empty() => {}
            _ => notes.push(format!("{key} missing or empty")),
        }
    }

    for (key, expected) in expectation.required_equal {
        checked_fields.push((*key).to_owned());
        match fields.get(*key) {
            Some(value) if normalize_cmp(value) == normalize_cmp(expected) => {}
            Some(value) => notes.push(format!("{key} expected `{expected}` got `{value}`")),
            None => notes.push(format!("{key} missing; expected `{expected}`")),
        }
    }

    E2eCorrectnessReport {
        status: if notes.is_empty() {
            "pass".to_owned()
        } else {
            "fail".to_owned()
        },
        checked_fields,
        notes,
    }
}

fn semantic_result_match(
    baseline: &E2eSampleReport,
    sample: &E2eSampleReport,
    has_expectation: bool,
) -> bool {
    if sample.correctness.status == "pass" {
        return true;
    }
    if has_expectation {
        return sample.correctness.status == "pass";
    }
    if normalize_cmp(&baseline.result) == normalize_cmp(&sample.result) {
        return true;
    }
    !baseline.result_fields.is_empty() && baseline.result_fields == sample.result_fields
}

fn build_variant_verdict(expected_result_match: bool, tool_tokens_saved: isize) -> String {
    match (expected_result_match, tool_tokens_saved > 0) {
        (true, true) => "saved_and_correct".to_owned(),
        (true, false) => "correct_but_not_saved".to_owned(),
        (false, true) => "saved_but_wrong".to_owned(),
        (false, false) => "wrong_and_not_saved".to_owned(),
    }
}

fn build_agent_summary(cases: &[E2eCaseReport]) -> Vec<E2eAgentSummary> {
    let mut by_agent = BTreeMap::<String, E2eAgentSummary>::new();
    for case in cases {
        let entry = by_agent
            .entry(case.agent.clone())
            .or_insert_with(|| E2eAgentSummary {
                agent: case.agent.clone(),
                cases: 0,
                variants: 0,
                saved_and_correct: 0,
                correct_but_not_saved: 0,
                saved_but_wrong: 0,
                wrong_and_not_saved: 0,
                total_tool_tokens_saved: 0,
            });
        entry.cases += 1;
        for variant in &case.variants {
            entry.variants += 1;
            entry.total_tool_tokens_saved += variant.tool_tokens_saved;
            match variant.verdict.as_str() {
                "saved_and_correct" => entry.saved_and_correct += 1,
                "correct_but_not_saved" => entry.correct_but_not_saved += 1,
                "saved_but_wrong" => entry.saved_but_wrong += 1,
                "wrong_and_not_saved" => entry.wrong_and_not_saved += 1,
                _ => {}
            }
        }
    }
    by_agent.into_values().collect()
}

struct CaseExpectation {
    required_non_empty: &'static [&'static str],
    required_equal: &'static [(&'static str, &'static str)],
}

fn case_expectation(path: &Path) -> Option<CaseExpectation> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    case_expectation_for_name(name)
}

fn case_expectation_for_name(name: &str) -> Option<CaseExpectation> {
    if name.starts_with("fairrg.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/tests.rs"), ("KIND", "search")],
        });
    }
    if name.starts_with("fairfind.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/rollout_stats.rs"), ("COUNT", "15")],
        });
    }
    if name.starts_with("fairbuild.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/lib.rs"), ("COUNT", "0")],
        });
    }
    if name.starts_with("rgcase.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE", "KIND"],
            required_equal: &[("FILE", "src/tests.rs")],
        });
    }
    if name.starts_with("findcase.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/tests.rs"), ("COUNT", "17")],
        });
    }
    if name.starts_with("buildcase.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/lib.rs"), ("COUNT", "0")],
        });
    }
    if name.starts_with("pipelinefix.") || name.starts_with("realtask.") {
        return Some(CaseExpectation {
            required_non_empty: &["STAGE"],
            required_equal: &[("FILE", "src/benchmark.rs"), ("COUNT", "40")],
        });
    }
    None
}
