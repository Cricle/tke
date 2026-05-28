use crate::app::{AppError, Config};
use crate::shim::maybe_normalize_text;
use crate::trim::{
    ascii_word_tokens, base_name, canonical_command_name, classify_command, has_ascii_token,
    has_prefix, has_token_prefix, has_token_sequence,
};
use std::fs;

pub(crate) fn parse_exec_command_args(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value.get("cmd")?.as_str().map(ToOwned::to_owned)
}

pub(crate) fn parse_command_like_args(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value
        .get("cmd")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("command").and_then(|v| v.as_str()))
        .map(ToOwned::to_owned)
}

struct ExecCommandEnvelope<'a> {
    exit_code: Option<i32>,
    output: &'a str,
}

fn parse_exec_command_envelope(raw: &str) -> Option<ExecCommandEnvelope<'_>> {
    let mut exit_code = None;
    let mut offset = 0;
    let mut saw_header = false;

    for chunk in raw.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        if line == "Output:" {
            return saw_header.then_some(ExecCommandEnvelope {
                exit_code,
                output: &raw[(offset + chunk.len())..],
            });
        }

        if let Some(code) = line
            .strip_prefix("Process exited with code ")
            .and_then(|value| value.parse::<i32>().ok())
        {
            exit_code = Some(code);
            saw_header = true;
        } else if let Some(code) = line
            .strip_prefix("Exit code:")
            .map(str::trim)
            .and_then(|value| value.parse::<i32>().ok())
        {
            exit_code = Some(code);
            saw_header = true;
        } else if matches_header_line(line) {
            saw_header = true;
        } else {
            return None;
        }

        offset += chunk.len();
    }

    if let Some(line) = raw[offset..].strip_suffix('\n')
        && line == "Output:"
    {
        return saw_header.then_some(ExecCommandEnvelope {
            exit_code,
            output: "",
        });
    }

    None
}

fn matches_header_line(line: &str) -> bool {
    let tokens = ascii_word_tokens(line);
    has_token_prefix(&tokens, &["chunk", "id"])
        || has_token_prefix(&tokens, &["wall", "time"])
        || has_token_prefix(&tokens, &["original", "token", "count"])
}

pub(crate) fn extract_exec_command_output(raw: &str) -> Option<&str> {
    parse_exec_command_envelope(raw).map(|envelope| envelope.output)
}

pub(crate) fn looks_like_stderr_only_exec_output(raw: &str) -> bool {
    let Some(envelope) = parse_exec_command_envelope(raw) else {
        return false;
    };
    if envelope.exit_code.unwrap_or_default() == 0 {
        return false;
    }

    envelope
        .output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
        .all(looks_like_diagnostic_line)
}

fn looks_like_diagnostic_line(line: &str) -> bool {
    let trimmed = line.trim();
    let tokens = ascii_word_tokens(trimmed);
    let has_error = has_ascii_token(&tokens, "error");
    let has_warning = has_ascii_token(&tokens, "warning") || has_ascii_token(&tokens, "warn");
    let has_traceback = has_ascii_token(&tokens, "traceback");
    let has_reading_error = has_token_sequence(&tokens, &["error", "reading"]);
    let has_fs_error = has_token_sequence(&tokens, &["is", "a", "directory"])
        || has_token_sequence(&tokens, &["no", "such", "file", "or", "directory"])
        || has_token_sequence(&tokens, &["permission", "denied"]);
    has_error || has_warning || has_traceback || has_reading_error || has_fs_error
}

pub(crate) fn rewrite_command_like_values(
    value: &mut serde_json::Value,
    config: &Config,
) -> Result<bool, AppError> {
    let Some(item) = value.get_mut("item") else {
        return Ok(false);
    };
    let Some(item_type) = item.get("type").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    if item_type != "command_execution" {
        return Ok(false);
    }

    let command = item
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let parsed = parse_command_execution(command);
    rewrite_command_item_fields(item, &parsed, config)
}

pub(crate) fn rewrite_command_item_fields(
    item: &mut serde_json::Value,
    parsed: &ParsedCommand,
    config: &Config,
) -> Result<bool, AppError> {
    let mut changed = false;
    let Some(obj) = item.as_object_mut() else {
        return Ok(false);
    };

    for field in ["aggregated_output", "stdout", "stderr", "output"] {
        let Some(existing) = obj.get(field).and_then(|value| value.as_str()) else {
            continue;
        };
        if existing.is_empty() || has_prefix(existing, &config.json_prefix) {
            continue;
        }

        let stream = if field == "stderr" {
            "stderr"
        } else {
            "stdout"
        };
        let selected = parsed.selected_stage();
        let kind = classify_command(&selected.name, &selected.args);
        let Some(normalized) = maybe_normalize_text(
            &selected.name,
            &selected.args,
            stream,
            kind,
            existing,
            config,
            Some((&selected.name, selected.role.as_str())),
        )?
        else {
            continue;
        };
        let wrapped = format!("{}{}", config.json_prefix, normalized);
        obj.insert(field.to_owned(), serde_json::Value::String(wrapped));
        changed = true;
    }

    Ok(changed)
}

pub(crate) fn parse_command_execution(command: &str) -> ParsedCommand {
    parse_command_execution_inner(command, 0)
}

pub(crate) fn parse_live_shell_pipeline(command: &str) -> ParsedCommand {
    parse_command_execution(command)
}

pub(crate) fn detect_linux_parent_pipeline() -> Option<ParsedCommand> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let parent = current_linux_ppid()?;
    let parent_cmdline = read_proc_cmdline(parent)?;
    let parent_parsed = parse_live_shell_pipeline(&parent_cmdline);
    if parent_parsed.stage_count() > 1 {
        return Some(parent_parsed);
    }

    let grandparent = read_proc_ppid(parent)?;
    let grandparent_cmdline = read_proc_cmdline(grandparent)?;
    let grandparent_parsed = parse_live_shell_pipeline(&grandparent_cmdline);
    if grandparent_parsed.stage_count() > 1 {
        return Some(grandparent_parsed);
    }
    None
}

#[derive(Clone)]
pub(crate) enum LivePipelineDecision {
    NotPipeline,
    PassThrough,
    Normalize(ParsedStage),
}

pub(crate) fn live_pipeline_decision(
    parsed: &ParsedCommand,
    current_name: &str,
) -> LivePipelineDecision {
    if parsed.stage_count() <= 1 {
        return LivePipelineDecision::NotPipeline;
    }
    if !parsed.has_unique_stage_match(current_name) {
        return LivePipelineDecision::PassThrough;
    }
    if !stage_name_matches_current_command(&parsed.last_stage().name, current_name) {
        return LivePipelineDecision::PassThrough;
    }
    let selected = parsed.selected_stage();
    if selected.name.is_empty() {
        return LivePipelineDecision::PassThrough;
    }
    LivePipelineDecision::Normalize(selected)
}

#[cfg(test)]
pub(crate) fn live_pipeline_should_passthrough(parsed: &ParsedCommand, current_name: &str) -> bool {
    matches!(
        live_pipeline_decision(parsed, current_name),
        LivePipelineDecision::PassThrough
    )
}

fn parse_command_execution_inner(command: &str, depth: usize) -> ParsedCommand {
    if depth > 3 {
        return ParsedCommand::default();
    }

    let extracted_shell_body = extract_shell_body(command);
    let shell_body = extracted_shell_body.as_deref().unwrap_or(command).trim();
    let cleaned = shell_body.trim_matches(|c| c == '\'' || c == '"');
    if let Some(loop_body) = extract_shell_loop_body(cleaned) {
        let parsed = parse_command_execution_inner(&loop_body, depth + 1);
        if !parsed.selected_stage().name.is_empty() {
            return parsed;
        }
    }
    let tokens = shell_like_split(cleaned);
    if tokens.is_empty() {
        return ParsedCommand::default();
    }

    if let Some((cmd_idx, cmd_token)) = first_real_token_with_index(&tokens) {
        let base = base_name(cmd_token);
        if matches!(base.as_str(), "sh" | "bash" | "zsh" | "fish")
            && let Some(nested) = extract_nested_shell_invocation(&tokens[cmd_idx..])
        {
            let parsed = parse_command_execution_inner(&nested, depth + 1);
            if !parsed.selected_stage().name.is_empty() {
                return parsed;
            }
        }
    }

    let stages = split_command_segments(&tokens)
        .into_iter()
        .enumerate()
        .map(|(index, segment)| ParsedStage::from_tokens(segment, index))
        .collect::<Vec<_>>();
    ParsedCommand::new(stages, Some(cleaned.to_owned()))
}

fn extract_shell_body(command: &str) -> Option<String> {
    let tokens = shell_like_split(command);
    let (cmd_idx, cmd_token) = first_real_token_with_index(&tokens)?;
    let shell = base_name(cmd_token);
    if !matches!(
        shell.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell" | "cmd" | "cmd.exe"
    ) {
        return None;
    }

    for window in tokens[cmd_idx + 1..].windows(2) {
        if matches!(
            window[0].as_str(),
            "-lc" | "-c" | "-Command" | "-command" | "/c"
        ) {
            return Some(window[1].clone());
        }
    }
    None
}

fn split_command_segments(tokens: &[String]) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for token in tokens {
        if matches!(
            token.as_str(),
            "|" | "||" | "&&" | ";" | "2>&1" | "1>" | "2>"
        ) {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(token.clone());
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn looks_like_env_assignment(token: &str) -> bool {
    let Some((key, _)) = token.split_once('=') else {
        return false;
    };
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn extract_nested_shell_invocation(tokens: &[String]) -> Option<String> {
    for window in tokens.windows(2) {
        if matches!(window[0].as_str(), "-c" | "-lc") {
            return Some(window[1].clone());
        }
    }
    None
}

fn extract_shell_loop_body(command: &str) -> Option<String> {
    let trimmed = command.trim_start();
    let tokens = ascii_word_tokens(trimmed);
    if !(has_token_prefix(&tokens, &["for"])
        || has_token_prefix(&tokens, &["while"])
        || has_token_prefix(&tokens, &["until"]))
    {
        return None;
    }

    let body_start = find_shell_keyword_boundary(trimmed, "do")?;
    let body_and_tail = trimmed.get(body_start..)?.trim_start();
    let body_end =
        find_shell_keyword_boundary(body_and_tail, "done").unwrap_or(body_and_tail.len());
    let body = body_and_tail[..body_end]
        .trim()
        .trim_end_matches(';')
        .trim();
    (!body.is_empty()).then_some(body.to_owned())
}

fn shell_like_split(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in input.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => {
                escape = true;
            }
            '\'' | '"' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[derive(Clone, Default)]
pub(crate) struct ParsedCommand {
    stages: Vec<ParsedStage>,
    #[allow(dead_code)]
    shell: Option<String>,
}

impl ParsedCommand {
    fn new(stages: Vec<ParsedStage>, shell: Option<String>) -> Self {
        Self { stages, shell }
    }

    pub(crate) fn stage_count(&self) -> usize {
        self.stages.len()
    }

    pub(crate) fn selected_stage(&self) -> ParsedStage {
        self.stages
            .iter()
            .max_by_key(|stage| stage.role_score())
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn last_stage(&self) -> ParsedStage {
        self.stages.last().cloned().unwrap_or_default()
    }

    pub(crate) fn has_stages(&self) -> bool {
        !self.stages.is_empty()
    }

    fn has_unique_stage_match(&self, current_name: &str) -> bool {
        self.stages
            .iter()
            .filter(|stage| stage_name_matches_current_command(&stage.name, current_name))
            .count()
            == 1
    }
}

fn first_real_token_with_index(tokens: &[String]) -> Option<(usize, &str)> {
    let mut idx = 0;
    while idx < tokens.len() {
        let token = tokens[idx].as_str();
        if token == "env" {
            idx += 1;
            while idx < tokens.len()
                && matches!(tokens[idx].as_str(), "-i" | "--ignore-environment" | "--")
            {
                idx += 1;
            }
            while idx < tokens.len() && looks_like_env_assignment(tokens[idx].as_str()) {
                idx += 1;
            }
            continue;
        }
        if matches!(
            token,
            "command" | "builtin" | "nohup" | "time" | "timeout" | "stdbuf" | "nice"
        ) {
            idx += 1;
            while idx < tokens.len()
                && (token_starts_with_char(tokens[idx].as_str(), '-')
                    || token == "timeout" && is_timeout_duration_token(tokens[idx].as_str())
                    || token == "nice" && is_nice_priority_token(tokens[idx].as_str()))
            {
                idx += 1;
            }
            continue;
        }
        if looks_like_env_assignment(token) {
            idx += 1;
            continue;
        }
        return Some((idx, token));
    }
    None
}

fn parse_xargs_payload(tokens: &[String]) -> Option<(String, Vec<String>)> {
    let (idx, token) = first_real_token_with_index(tokens)?;
    let name = base_name(token);
    let args = tokens.iter().skip(idx + 1).cloned().collect();
    Some((name, args))
}

fn is_timeout_duration_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let numeric_len = token
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .count();
    if numeric_len == 0 {
        return false;
    }
    let suffix = &token[numeric_len..];
    suffix.is_empty() || suffix.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn is_nice_priority_token(token: &str) -> bool {
    token
        .strip_prefix('-')
        .unwrap_or(token)
        .chars()
        .all(|ch| ch.is_ascii_digit())
}

#[derive(Clone, Default)]
pub(crate) struct ParsedStage {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
    pub(crate) role: StageRole,
    pub(crate) index: usize,
}

impl ParsedStage {
    fn from_tokens(tokens: Vec<String>, index: usize) -> Self {
        let Some((cmd_idx, cmd_token)) = first_real_token_with_index(&tokens) else {
            return Self {
                name: String::new(),
                args: Vec::new(),
                role: StageRole::Unknown,
                index,
            };
        };
        let base = canonical_command_name(&base_name(cmd_token)).to_owned();
        if base == "xargs"
            && let Some((payload_name, payload_args)) = parse_xargs_payload(&tokens[cmd_idx + 1..])
        {
            let role = classify_stage_role_with_args(&payload_name, &payload_args);
            return Self {
                name: payload_name,
                args: payload_args,
                role,
                index,
            };
        }

        let raw_args = tokens.iter().skip(cmd_idx + 1).cloned().collect::<Vec<_>>();
        let canonical_stage = canonical_stage_with_args(&base, &raw_args);
        let role = classify_stage_role_with_args(&canonical_stage, &raw_args);
        let args = if matches!(canonical_stage.as_str(), "head" | "tail") {
            trim_head_tail_args(raw_args)
        } else {
            raw_args
        };
        Self {
            name: canonical_stage,
            args,
            role,
            index,
        }
    }

    fn role_score(&self) -> (i32, i32, i32) {
        let role_weight = self.role.weight();
        let arg_weight = self
            .args
            .iter()
            .filter(|arg| arg.starts_with('/') || arg.chars().any(|ch| ch == '.'))
            .count() as i32;
        let position_weight = -(self.index as i32);
        (role_weight, arg_weight, position_weight)
    }
}

fn canonical_stage_with_args(name: &str, args: &[String]) -> String {
    if name == "head" {
        if args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-Last" | "-last"))
        {
            return "tail".to_owned();
        }
        if args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-Skip" | "-skip"))
        {
            return "awk".to_owned();
        }
    }
    name.to_owned()
}

fn stage_name_matches_current_command(stage_name: &str, current_name: &str) -> bool {
    let canonical_stage = canonical_command_name(stage_name);
    let canonical_current = canonical_command_name(current_name);
    if canonical_stage == canonical_current {
        return true;
    }

    matches!(
        base_name(current_name).as_str(),
        "Select-Object" | "select-object"
    ) && matches!(canonical_stage, "head" | "tail" | "awk")
}

fn trim_head_tail_args(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if matches!(arg, "-n" | "-c") {
            idx += 2;
            continue;
        }
        if token_starts_with_char(arg, '-') {
            idx += 1;
            continue;
        }
        out.extend(args.into_iter().skip(idx));
        return out;
    }
    Vec::new()
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StageRole {
    Source,
    Search,
    Filter,
    Summarize,
    Build,
    #[default]
    Unknown,
}

impl StageRole {
    fn weight(self) -> i32 {
        match self {
            Self::Search => 500,
            Self::Source => 400,
            Self::Build => 320,
            Self::Filter => 250,
            Self::Summarize => 150,
            Self::Unknown => 0,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Search => "search",
            Self::Filter => "filter",
            Self::Summarize => "summarize",
            Self::Build => "build",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn classify_stage_role(name: &str) -> StageRole {
    let name = canonical_command_name(name);
    match name {
        "rg" | "grep" | "find" | "fd" | "tree" => StageRole::Search,
        "cat" | "git" | "bat" | "nl" | "ls" | "curl" | "docker" | "which" | "readlink" | "file"
        | "stat" => StageRole::Source,
        "sed" | "awk" | "perl" | "cut" | "sort" | "uniq" | "tr" | "jq" => StageRole::Filter,
        "head" | "tail" | "wc" | "du" | "df" => StageRole::Summarize,
        "cargo" | "pytest" | "npm" | "pnpm" | "yarn" | "bun" | "dotnet" | "go" | "cmake"
        | "ctest" | "make" | "ninja" | "node" | "python" | "python3" | "pip" | "uv" | "poetry"
        | "mvn" | "gradle" | "gradlew" | "javac" | "java" | "bundle" | "composer" | "ps" | "ss"
        | "netstat" | "systemctl" | "psql" | "redis-cli" => StageRole::Build,
        _ => StageRole::Unknown,
    }
}

fn classify_stage_role_with_args(name: &str, args: &[String]) -> StageRole {
    let name = canonical_command_name(name);
    if name == "git" {
        match args
            .iter()
            .find(|arg| !has_prefix(arg, "-"))
            .map(String::as_str)
        {
            Some("grep" | "ls-files") => return StageRole::Search,
            Some("diff") => return StageRole::Source,
            Some("show" | "blame") => return StageRole::Source,
            Some("status" | "log" | "branch" | "remote" | "ls-remote" | "rev-parse") => {
                return StageRole::Build;
            }
            _ => {}
        }
    }
    classify_stage_role(name)
}

fn read_proc_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    let raw = fs::read(path).ok()?;
    if raw.is_empty() {
        return None;
    }
    let parts = raw
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn read_proc_ppid(pid: u32) -> Option<u32> {
    let path = format!("/proc/{pid}/stat");
    let stat = fs::read_to_string(path).ok()?;
    let tail = stat.rsplit_once(") ")?.1;
    let mut fields = tail.split_whitespace();
    let _state = fields.next()?;
    fields.next()?.parse().ok()
}

fn current_linux_ppid() -> Option<u32> {
    read_proc_ppid(std::process::id())
}

fn token_starts_with_char(raw: &str, ch: char) -> bool {
    raw.starts_with(ch)
}

fn find_shell_keyword_boundary(text: &str, keyword: &str) -> Option<usize> {
    let keyword_len = keyword.len();
    text.char_indices().find_map(|(idx, _)| {
        let end = idx + keyword_len;
        let segment = text.get(idx..end)?;
        if segment != keyword {
            return None;
        }
        let before_ok = idx == 0
            || text[..idx]
                .chars()
                .next_back()
                .is_none_or(|ch| ch.is_whitespace() || matches!(ch, ';' | '\n' | '\t'));
        let after_ok = end == text.len()
            || text[end..]
                .chars()
                .next()
                .is_none_or(|ch| ch.is_whitespace() || matches!(ch, ';' | '\n' | '\t'));
        (before_ok && after_ok).then_some(end)
    })
}
