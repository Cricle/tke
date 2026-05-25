use crate::app::Config;
use crate::rewrite::{
    extract_exec_command_output, parse_command_execution, parse_exec_command_args,
};
use crate::trim::{classify_command, select_profile};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Default, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct RolloutOutputStats {
    pub(crate) fields: usize,
    pub(crate) bytes: usize,
    pub(crate) approx_tokens: usize,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct RolloutOutputBreakdown {
    pub(crate) by_profile: BTreeMap<String, RolloutOutputStats>,
    pub(crate) by_command: BTreeMap<String, RolloutOutputStats>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct RolloutOutputStatsDetailed {
    pub(crate) total: RolloutOutputStats,
    pub(crate) breakdown: RolloutOutputBreakdown,
    pub(crate) records: Vec<OutputRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
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
    let mut claude_commands = HashMap::<String, String>::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some((call_id, command)) = extract_exec_command_call(&value) {
            exec_commands.insert(call_id, command);
        }
        record_claude_tool_calls(&value, &mut claude_commands);
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
        collect_claude_tool_result_stats(&value, &claude_commands, config, &mut stats);
    }
    stats
}

pub(crate) fn rollout_has_relevant_tool_output(text: &str) -> bool {
    let mut claude_commands = HashMap::<String, ()>::new();
    let mut codex_commands = HashMap::<String, ()>::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if remember_relevant_tool_call(&value, &mut codex_commands, &mut claude_commands) {
            continue;
        }
        if has_relevant_tool_output_value(&value, &codex_commands, &claude_commands) {
            return true;
        }
    }
    false
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
        collect_text_output_stats(text, command, config, stats);
    }
}

fn collect_text_output_stats(
    text: &str,
    command: Option<&str>,
    config: &Config,
    stats: &mut RolloutOutputStatsDetailed,
) {
    if text.is_empty() {
        return;
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

fn remember_relevant_tool_call(
    value: &Value,
    codex_commands: &mut HashMap<String, ()>,
    claude_commands: &mut HashMap<String, ()>,
) -> bool {
    if let Some(payload) = value.get("payload").and_then(Value::as_object) {
        let payload_type = payload.get("type").and_then(Value::as_str);
        let payload_name = payload.get("name").and_then(Value::as_str);
        if payload_type == Some("function_call") && payload_name == Some("exec_command") {
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                codex_commands.insert(call_id.to_owned(), ());
            }
            return true;
        }
    }

    let line_type = value.get("type").and_then(Value::as_str);
    if line_type != Some("assistant") {
        return false;
    }
    let Some(content) = value
        .get("message")
        .and_then(|v| v.get("content"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let mut recorded = false;
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(name, "Bash" | "bash" | "Shell" | "shell") {
            continue;
        }
        if let Some(tool_id) = block.get("id").and_then(Value::as_str) {
            claude_commands.insert(tool_id.to_owned(), ());
            recorded = true;
        }
    }
    recorded
}

fn has_relevant_tool_output_value(
    value: &Value,
    codex_commands: &HashMap<String, ()>,
    claude_commands: &HashMap<String, ()>,
) -> bool {
    if let Some(item) = value.get("item").and_then(Value::as_object) {
        if item.get("type").and_then(Value::as_str) == Some("command_execution")
            && has_nonempty_output_field(item, &["aggregated_output", "stdout", "stderr", "output"])
        {
            return true;
        }
    }

    if let Some(payload) = value.get("payload").and_then(Value::as_object) {
        let payload_type = payload.get("type").and_then(Value::as_str);
        if payload_type == Some("function_call_output")
            && payload
                .get("call_id")
                .and_then(Value::as_str)
                .is_some_and(|call_id| codex_commands.contains_key(call_id))
            && payload
                .get("output")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
        {
            return true;
        }
    }

    let line_type = value.get("type").and_then(Value::as_str);
    if line_type != Some("user") {
        return false;
    }
    let Some(content) = value
        .get("message")
        .and_then(|v| v.get("content"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let known_tool = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .or_else(|| block.get("id").and_then(Value::as_str))
            .is_some_and(|tool_id| claude_commands.contains_key(tool_id));
        if !known_tool {
            continue;
        }
        if block
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        {
            return true;
        }
        let Some(items) = block.get("content").and_then(Value::as_array) else {
            continue;
        };
        if items.iter().any(|item| {
            item.get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
        }) {
            return true;
        }
    }
    false
}

fn has_nonempty_output_field(obj: &serde_json::Map<String, Value>, fields: &[&str]) -> bool {
    fields.iter().any(|field| {
        obj.get(*field)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
    })
}

fn record_claude_tool_calls(value: &Value, tool_calls: &mut HashMap<String, String>) {
    let Some(line_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    if line_type != "assistant" {
        return;
    }
    let Some(content) = value
        .get("message")
        .and_then(|v| v.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(name, "Bash" | "bash" | "Shell" | "shell") {
            continue;
        }
        let Some(tool_id) = block.get("id").and_then(Value::as_str) else {
            continue;
        };
        let input = block.get("input").unwrap_or(&Value::Null);
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| input.get("cmd").and_then(Value::as_str));
        let Some(command) = command else {
            continue;
        };
        tool_calls.insert(tool_id.to_owned(), command.to_owned());
    }
}

fn collect_claude_tool_result_stats(
    value: &Value,
    tool_calls: &HashMap<String, String>,
    config: &Config,
    stats: &mut RolloutOutputStatsDetailed,
) {
    let Some(line_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    if line_type != "user" {
        return;
    }
    let Some(content) = value
        .get("message")
        .and_then(|v| v.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(tool_id) = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .or_else(|| block.get("id").and_then(Value::as_str))
        else {
            continue;
        };
        let command = tool_calls.get(tool_id).map(String::as_str);
        if let Some(text) = block.get("content").and_then(Value::as_str) {
            collect_text_output_stats(text, command, config, stats);
            continue;
        }
        let Some(items) = block.get("content").and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(text) = item.get("text").and_then(Value::as_str) else {
                continue;
            };
            collect_text_output_stats(text, command, config, stats);
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
