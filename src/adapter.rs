use crate::app::{AppError, Config};
use crate::rewrite::{
    ParsedCommand, extract_exec_command_output, looks_like_stderr_only_exec_output,
    parse_command_execution, parse_exec_command_args, rewrite_command_like_values,
};
use crate::trim::{classify_command, has_prefix};
use std::collections::HashMap;

pub(crate) fn rewrite_agent_transcript(
    text: &str,
    config: &Config,
) -> Result<Option<String>, AppError> {
    let mut changed = false;
    let mut out = Vec::new();
    let mut codex_calls = HashMap::new();
    let mut claude_calls = HashMap::new();

    for line in text.lines() {
        let mut value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        if rewrite_codex_event(&mut value, &mut codex_calls, config)?
            | rewrite_claude_event(&mut value, &mut claude_calls, config)?
            | rewrite_command_like_values(&mut value, config)?
        {
            changed = true;
        }
        out.push(serde_json::to_string(&value)?);
    }

    if !changed {
        return Ok(None);
    }
    Ok(Some(out.join("\n") + "\n"))
}

#[cfg(test)]
pub(crate) fn rewrite_codex_jsonl(text: &str, config: &Config) -> Result<Option<String>, AppError> {
    rewrite_agent_transcript(text, config)
}

#[cfg(test)]
pub(crate) fn rewrite_claude_jsonl(
    text: &str,
    config: &Config,
) -> Result<Option<String>, AppError> {
    rewrite_agent_transcript(text, config)
}

#[cfg(test)]
pub(crate) fn rewrite_generic_jsonl(
    text: &str,
    config: &Config,
) -> Result<Option<String>, AppError> {
    rewrite_agent_transcript(text, config)
}

struct PendingToolCall {
    tool_name: String,
    parsed: Option<ParsedCommand>,
}

fn rewrite_codex_event(
    value: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
    config: &Config,
) -> Result<bool, AppError> {
    let Some(response) = value.get_mut("payload") else {
        return Ok(false);
    };
    let Some(response_type) = response.get("type").and_then(|v| v.as_str()) else {
        return Ok(false);
    };

    match response_type {
        "function_call" => {
            record_codex_tool_call(response, tool_calls);
            Ok(false)
        }
        "function_call_output" => rewrite_codex_tool_call_output(response, tool_calls, config),
        _ => Ok(false),
    }
}

fn record_codex_tool_call(
    payload: &serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
) {
    let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) else {
        return;
    };
    let tool_name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let parsed = if tool_name == "exec_command" {
        payload
            .get("arguments")
            .and_then(|v| v.as_str())
            .and_then(parse_exec_command_args)
            .map(|cmd| parse_command_execution(&cmd))
    } else {
        None
    };
    tool_calls.insert(call_id.to_owned(), PendingToolCall { tool_name, parsed });
}

fn rewrite_codex_tool_call_output(
    payload: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
    config: &Config,
) -> Result<bool, AppError> {
    let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    let Some(pending) = tool_calls.get(call_id) else {
        return Ok(false);
    };
    if pending.tool_name != "exec_command" {
        return Ok(false);
    }
    let Some(parsed) = pending.parsed.as_ref() else {
        return Ok(false);
    };
    let Some(existing) = payload.get("output").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    if existing.is_empty() || has_prefix(existing, &config.json_prefix) {
        return Ok(false);
    }
    let Some(actual_output) = extract_exec_command_output(existing) else {
        return Ok(false);
    };
    if actual_output.is_empty() {
        return Ok(false);
    }

    let stream = if looks_like_stderr_only_exec_output(existing) {
        "stderr"
    } else {
        "stdout"
    };
    let Some(normalized) = normalize_with_parsed(parsed, stream, actual_output, config)? else {
        return Ok(false);
    };

    let Some(obj) = payload.as_object_mut() else {
        return Ok(false);
    };
    obj.insert(
        "output".to_owned(),
        serde_json::Value::String(format!("{}{}", config.json_prefix, normalized)),
    );
    Ok(true)
}

fn rewrite_claude_event(
    value: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, ParsedCommand>,
    config: &Config,
) -> Result<bool, AppError> {
    let line_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match line_type {
        "assistant" => {
            record_claude_tool_uses(value, tool_calls);
            Ok(false)
        }
        "user" => rewrite_claude_tool_results(value, tool_calls, config),
        _ => Ok(false),
    }
}

fn record_claude_tool_uses(
    value: &serde_json::Value,
    tool_calls: &mut HashMap<String, ParsedCommand>,
) {
    let Some(content) = value
        .get("message")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_array())
    else {
        return;
    };

    for block in content {
        let kind = block
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if kind != "tool_use" {
            continue;
        }
        let name = block
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !matches!(name, "Bash" | "bash" | "Shell" | "shell") {
            continue;
        }
        let Some(tool_id) = block.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let input = block.get("input").unwrap_or(&serde_json::Value::Null);
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .or_else(|| input.get("cmd").and_then(|v| v.as_str()));
        let Some(command) = command else {
            continue;
        };
        tool_calls.insert(tool_id.to_owned(), parse_command_execution(command));
    }
}

fn rewrite_claude_tool_results(
    value: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, ParsedCommand>,
    config: &Config,
) -> Result<bool, AppError> {
    let Some(content) = value
        .get_mut("message")
        .and_then(|v| v.get_mut("content"))
        .and_then(|v| v.as_array_mut())
    else {
        return Ok(false);
    };

    let mut changed = false;
    for block in content {
        let kind = block
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if kind != "tool_result" {
            continue;
        }
        let Some(tool_id) = block
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .or_else(|| block.get("id").and_then(|v| v.as_str()))
        else {
            continue;
        };
        let Some(parsed) = tool_calls.get(tool_id) else {
            continue;
        };
        changed |= rewrite_claude_result_block(block, parsed, config)?;
    }
    Ok(changed)
}

fn rewrite_claude_result_block(
    block: &mut serde_json::Value,
    parsed: &ParsedCommand,
    config: &Config,
) -> Result<bool, AppError> {
    if let Some(raw) = block.get("content").and_then(|v| v.as_str()) {
        if raw.is_empty() || has_prefix(raw, &config.json_prefix) {
            return Ok(false);
        }
        let Some(normalized) = normalize_with_parsed(parsed, "stdout", raw, config)? else {
            return Ok(false);
        };
        let Some(obj) = block.as_object_mut() else {
            return Ok(false);
        };
        obj.insert(
            "content".to_owned(),
            serde_json::Value::String(format!("{}{}", config.json_prefix, normalized)),
        );
        return Ok(true);
    }

    let Some(items) = block.get_mut("content").and_then(|v| v.as_array_mut()) else {
        return Ok(false);
    };
    let mut changed = false;
    for item in items {
        let text = item
            .get("text")
            .and_then(|v| v.as_str())
            .filter(|text| !text.is_empty() && !has_prefix(text, &config.json_prefix));
        let Some(text) = text else {
            continue;
        };
        let Some(normalized) = normalize_with_parsed(parsed, "stdout", text, config)? else {
            continue;
        };
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        obj.insert(
            "text".to_owned(),
            serde_json::Value::String(format!("{}{}", config.json_prefix, normalized)),
        );
        changed = true;
    }
    Ok(changed)
}

fn normalize_with_parsed(
    parsed: &ParsedCommand,
    stream: &str,
    text: &str,
    config: &Config,
) -> Result<Option<String>, AppError> {
    let selected = parsed.selected_stage();
    if selected.name.is_empty() {
        return Ok(None);
    }
    let kind = classify_command(&selected.name, &selected.args);
    crate::shim::maybe_normalize_text(
        &selected.name,
        &selected.args,
        stream,
        kind,
        text,
        config,
        Some((&selected.name, selected.role.as_str())),
    )
}
