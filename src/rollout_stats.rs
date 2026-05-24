use crate::app::Config;
use serde_json::{Map, Value};

#[derive(Default, Clone, Copy)]
pub(crate) struct RolloutOutputStats {
    pub(crate) fields: usize,
    pub(crate) bytes: usize,
    pub(crate) approx_tokens: usize,
}

pub(crate) fn collect_rollout_output_stats(text: &str, config: &Config) -> RolloutOutputStats {
    let mut stats = RolloutOutputStats::default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(item) = value.get("item") {
            collect_value_output_stats(
                item,
                &["aggregated_output", "stdout", "stderr", "output"],
                config,
                &mut stats,
            );
        }
        if let Some(payload) = value.get("payload") {
            collect_value_output_stats(payload, &["output"], config, &mut stats);
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
        if value.contains('/') || value.contains('\\') || dir == "." {
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
    config: &Config,
    stats: &mut RolloutOutputStats,
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
        stats.fields += 1;
        stats.bytes += text.len();
        stats.approx_tokens += approx_token_count(text, config);
    }
}

fn collect_message_output_stats(value: &Value, config: &Config, stats: &mut RolloutOutputStats) {
    let Some(content) = value.get("content").and_then(|v| v.as_array()) else {
        return;
    };
    for block in content {
        if let Some(text) = block.get("text").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            stats.fields += 1;
            stats.bytes += text.len();
            stats.approx_tokens += approx_token_count(text, config);
        }
        if let Some(nested) = block.get("content").and_then(|v| v.as_array()) {
            for item in nested {
                let Some(text) = item.get("text").and_then(|v| v.as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                stats.fields += 1;
                stats.bytes += text.len();
                stats.approx_tokens += approx_token_count(text, config);
            }
        }
        if let Some(text) = block.get("content").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            stats.fields += 1;
            stats.bytes += text.len();
            stats.approx_tokens += approx_token_count(text, config);
        }
    }
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
