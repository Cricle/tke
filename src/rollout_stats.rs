use crate::app::Config;
use crate::rewrite::{
    extract_exec_command_output, parse_command_execution, parse_exec_command_args,
};
use crate::trim::{classify_command, select_profile};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Default, Clone, Copy)]
pub(crate) struct RolloutOutputStats {
    pub(crate) fields: usize,
    pub(crate) bytes: usize,
    pub(crate) approx_tokens: usize,
}

#[derive(Default, Clone)]
pub(crate) struct RolloutOutputBreakdown {
    pub(crate) by_profile: BTreeMap<String, RolloutOutputStats>,
    pub(crate) by_command: BTreeMap<String, RolloutOutputStats>,
}

#[derive(Default, Clone)]
pub(crate) struct RolloutOutputStatsDetailed {
    pub(crate) total: RolloutOutputStats,
    pub(crate) breakdown: RolloutOutputBreakdown,
    pub(crate) records: Vec<OutputRecord>,
}

#[derive(Clone)]
pub(crate) struct OutputRecord {
    pub(crate) profile: String,
    pub(crate) command: String,
    pub(crate) stats: RolloutOutputStats,
}

#[cfg(test)]
pub(crate) fn collect_rollout_output_stats(text: &str, config: &Config) -> RolloutOutputStats {
    collect_rollout_output_stats_detailed(text, config).total
}

pub(crate) fn collect_rollout_output_stats_detailed(
    text: &str,
    config: &Config,
) -> RolloutOutputStatsDetailed {
    let mut stats = RolloutOutputStatsDetailed::default();
    let mut exec_commands = HashMap::<String, String>::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some((call_id, command)) = extract_exec_command_call(&value) {
            exec_commands.insert(call_id, command);
        }
        if let Some(item) = value.get("item") {
            collect_value_output_stats(
                item,
                &["aggregated_output", "stdout", "stderr", "output"],
                item.get("command").and_then(Value::as_str),
                config,
                &mut stats,
            );
        }
        if let Some(payload) = value.get("payload") {
            let command = payload
                .get("call_id")
                .and_then(Value::as_str)
                .and_then(|call_id| exec_commands.get(call_id))
                .map(String::as_str);
            collect_value_output_stats(payload, &["output"], command, config, &mut stats);
        }
        if let Some(message) = value.get("message") {
            collect_message_output_stats(message, config, &mut stats);
        }
    }
    stats
}

pub(crate) fn rollout_string_haystack(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        collect_json_strings(&value, &mut out);
        out.push('\n');
    }
    out
}

fn collect_json_strings(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => {
            out.push_str(text);
            out.push('\n');
        }
        Value::Array(values) => {
            for value in values {
                collect_json_strings(value, out);
            }
        }
        Value::Object(map) => {
            collect_pathlist_expansions(map, out);
            for value in map.values() {
                collect_json_strings(value, out);
            }
        }
        _ => {}
    }
}

fn collect_pathlist_expansions(map: &Map<String, Value>, out: &mut String) {
    let Some(pathlist) = map.get("pl").and_then(|value| value.as_object()) else {
        return;
    };
    let Some(dir) = pathlist.get("d").and_then(|value| value.as_str()) else {
        return;
    };

    for key in ["f", "l"] {
        let Some(value) = pathlist.get(key).and_then(|value| value.as_str()) else {
            continue;
        };
        if value.chars().any(|ch| matches!(ch, '/' | '\\')) || dir == "." {
            continue;
        }
        out.push_str(dir);
        if !dir.ends_with('/') && !dir.ends_with('\\') {
            out.push('/');
        }
        out.push_str(value);
        out.push('\n');
    }
}

fn collect_value_output_stats(
    value: &Value,
    fields: &[&str],
    command: Option<&str>,
    config: &Config,
    stats: &mut RolloutOutputStatsDetailed,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for field in fields {
        let Some(text) = obj.get(*field).and_then(|v| v.as_str()) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let token_count = approx_token_count(text, config);
        stats.total.fields += 1;
        stats.total.bytes += text.len();
        stats.total.approx_tokens += token_count;
        if let Some(record) = infer_output_record(text, command, config) {
            add_breakdown_stats(&mut stats.breakdown, &record);
            stats.records.push(record);
        }
    }
}

fn collect_message_output_stats(
    value: &Value,
    config: &Config,
    stats: &mut RolloutOutputStatsDetailed,
) {
    let Some(content) = value.get("content").and_then(|v| v.as_array()) else {
        return;
    };
    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            let token_count = approx_token_count(text, config);
            stats.total.fields += 1;
            stats.total.bytes += text.len();
            stats.total.approx_tokens += token_count;
        }
        if let Some(nested) = block.get("content").and_then(|v| v.as_array()) {
            for item in nested {
                let Some(text) = item.get("text").and_then(|v| v.as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                let token_count = approx_token_count(text, config);
                stats.total.fields += 1;
                stats.total.bytes += text.len();
                stats.total.approx_tokens += token_count;
            }
        }
        if let Some(text) = block.get("content").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            let token_count = approx_token_count(text, config);
            stats.total.fields += 1;
            stats.total.bytes += text.len();
            stats.total.approx_tokens += token_count;
        }
    }
}

fn add_breakdown_stats(breakdown: &mut RolloutOutputBreakdown, record: &OutputRecord) {
    add_stat_row(
        breakdown
            .by_profile
            .entry(record.profile.clone())
            .or_default(),
        record.stats.bytes,
        record.stats.approx_tokens,
    );
    add_stat_row(
        breakdown
            .by_command
            .entry(record.command.clone())
            .or_default(),
        record.stats.bytes,
        record.stats.approx_tokens,
    );
}

fn add_stat_row(stats: &mut RolloutOutputStats, bytes: usize, approx_tokens: usize) {
    stats.fields += 1;
    stats.bytes += bytes;
    stats.approx_tokens += approx_tokens;
}

fn extract_exec_command_call(value: &Value) -> Option<(String, String)> {
    let payload = value.get("payload")?.as_object()?;
    if payload.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    if payload.get("name").and_then(Value::as_str) != Some("exec_command") {
        return None;
    }
    let call_id = payload.get("call_id").and_then(Value::as_str)?.to_owned();
    let command = parse_exec_command_args(payload.get("arguments")?.as_str()?)?;
    Some((call_id, command))
}

fn infer_output_record(text: &str, command: Option<&str>, config: &Config) -> Option<OutputRecord> {
    if let Some(raw) = text.strip_prefix(&config.json_prefix)
        && let Ok(value) = serde_json::from_str::<Value>(raw)
    {
        let profile = value
            .get("p")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown");
        let command = value
            .get("sc")
            .or_else(|| value.get("cmd"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown");
        return Some(OutputRecord {
            profile: profile.to_owned(),
            command: command.to_owned(),
            stats: RolloutOutputStats {
                fields: 1,
                bytes: text.len(),
                approx_tokens: count_json_tokens(&value),
            },
        });
    }

    let command = command?;
    let parsed = parse_command_execution(command);
    if !parsed.has_stages() {
        return None;
    }
    let stage = parsed.selected_stage();
    if stage.name.is_empty() {
        return None;
    }
    let analysis_text = extract_exec_command_output(text).unwrap_or(text);
    let lines = analysis_text.lines().collect::<Vec<_>>();
    let kind = classify_command(&stage.name, &stage.args);
    let profile = select_profile(&stage.name, &stage.args, kind, &lines);
    Some(OutputRecord {
        profile: profile.as_str().to_owned(),
        command: stage.name,
        stats: RolloutOutputStats {
            fields: 1,
            bytes: text.len(),
            approx_tokens: crate::benchmark::estimate_text_tokens(text),
        },
    })
}

fn approx_token_count(text: &str, config: &Config) -> usize {
    if let Some(raw) = text.strip_prefix(&config.json_prefix)
        && let Ok(value) = serde_json::from_str::<Value>(raw)
    {
        return count_json_tokens(&value);
    }
    crate::benchmark::estimate_text_tokens(text)
}

fn count_json_tokens(value: &Value) -> usize {
    match value {
        Value::Null => 1,
        Value::Bool(_) => 1,
        Value::Number(_) => 1,
        Value::String(s) => crate::benchmark::estimate_text_tokens(s),
        Value::Array(values) => values.iter().map(count_json_tokens).sum(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| crate::benchmark::estimate_text_tokens(k) + count_json_tokens(v))
            .sum(),
    }
}
