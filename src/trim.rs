use crate::app::{AppError, Config};
use serde::Serialize;
use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn collect_profile_chunks(
    lines: &[&str],
    terms: &[String],
    profile: TrimProfile,
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    match profile {
        TrimProfile::Diff => collect_diff_chunks(lines, terms, limits),
        TrimProfile::Search => crate::search_profile::collect_search_chunks(lines, terms, limits),
        TrimProfile::PathList => Vec::new(),
        TrimProfile::Log => crate::log_profile::collect_log_chunks(lines, terms, limits),
        TrimProfile::Table => Vec::new(),
        TrimProfile::Stacktrace => collect_stacktrace_chunks(lines, limits),
        TrimProfile::File => crate::file_profile::collect_file_chunks(lines, terms, limits),
        TrimProfile::Generic => collect_term_chunks(
            lines,
            terms,
            "hit",
            limits.match_context,
            limits.max_matches,
        ),
    }
}

pub(crate) fn collect_term_chunks(
    lines: &[&str],
    terms: &[String],
    label: &str,
    context: usize,
    max_matches: usize,
) -> Vec<MatchChunk> {
    if terms.is_empty() {
        return Vec::new();
    }
    let lower_terms = terms
        .iter()
        .map(|term| term.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut used = Vec::<(usize, usize)>::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        if !lower_terms.iter().any(|term| lower.contains(term)) {
            continue;
        }
        let start = idx.saturating_sub(context);
        let end = usize::min(lines.len(), idx + context + 1);
        if push_chunk(&mut out, &mut used, lines, start, end, label) && out.len() >= max_matches {
            break;
        }
    }
    out
}

pub(crate) fn compute_omitted_ranges(
    total_lines: usize,
    kept_ranges: &[(usize, usize)],
) -> Vec<[usize; 2]> {
    let mut omitted = Vec::new();
    let mut cursor = 0;
    for (start, end) in kept_ranges {
        if *start > cursor {
            omitted.push([cursor, *start]);
        }
        cursor = *end;
    }
    if cursor < total_lines {
        omitted.push([cursor, total_lines]);
    }
    omitted
}

fn collect_diff_chunks(lines: &[&str], terms: &[String], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("index ")
        {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "file") && out.len() >= 4 {
                break;
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("@@") {
            let start = idx;
            let end = usize::min(lines.len(), idx + 3);
            if push_chunk(&mut out, &mut used, lines, start, end, "hunk") && out.len() >= 6 {
                break;
            }
        }
    }

    for chunk in collect_term_chunks(
        lines,
        terms,
        "change",
        limits.match_context,
        limits.max_matches,
    ) {
        if push_existing_chunk(&mut out, &mut used, chunk) && out.len() >= limits.max_matches {
            break;
        }
    }
    out
}

fn collect_stacktrace_chunks(lines: &[&str], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for (idx, line) in lines.iter().enumerate() {
        if is_stack_summary(line) {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "summary") {
                break;
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        if !is_stack_frame(line) {
            continue;
        }
        if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "frame")
            && out.len() >= limits.max_matches
        {
            break;
        }
    }
    out
}

pub(crate) fn push_existing_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    chunk: MatchChunk,
) -> bool {
    let start = chunk.r[0];
    let end = chunk.r[1];
    if used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(chunk);
    true
}

pub(crate) fn push_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    lines: &[&str],
    start: usize,
    end: usize,
    label: &str,
) -> bool {
    if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(MatchChunk {
        k: label.to_owned(),
        r: [start, end],
        l: lines[start..end]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
    });
    true
}

pub(crate) fn collect_kept_ranges(
    total_lines: usize,
    head: &[String],
    tail: &[String],
    matches: &[MatchChunk],
) -> Vec<(usize, usize)> {
    let mut kept = Vec::new();
    if !head.is_empty() {
        kept.push((0, head.len()));
    }
    if !tail.is_empty() {
        kept.push((total_lines.saturating_sub(tail.len()), total_lines));
    }
    for chunk in matches {
        kept.push((chunk.r[0], chunk.r[1]));
    }
    kept
}

pub(crate) fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut merged = Vec::<(usize, usize)>::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

pub(crate) fn take_head(lines: &[&str], count: usize) -> Vec<String> {
    lines
        .iter()
        .take(count)
        .map(|line| (*line).to_owned())
        .collect()
}

pub(crate) fn take_tail(lines: &[&str], count: usize) -> Vec<String> {
    lines
        .iter()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| (*line).to_owned())
        .collect()
}

pub(crate) fn collect_table_summary(
    name: &str,
    args: &[String],
    lines: &[&str],
    terms: &[String],
) -> Option<TableSummary> {
    let layout = detect_table_layout(lines)?;
    let selected_cols = select_table_columns(&layout.headers);
    if selected_cols.is_empty() {
        return None;
    }

    let selected_rows = select_table_rows(name, args, &layout, terms);
    if selected_rows.is_empty() {
        return None;
    }

    let cols = selected_cols
        .iter()
        .map(|idx| layout.headers[*idx].clone())
        .collect::<Vec<_>>();
    let rows = selected_rows
        .into_iter()
        .map(|row_idx| {
            let row = &layout.rows[row_idx];
            let values = selected_cols
                .iter()
                .map(|col_idx| row.fields.get(*col_idx).cloned().unwrap_or_default())
                .collect::<Vec<_>>();
            TableRow {
                i: row.line_index,
                v: values,
            }
        })
        .collect::<Vec<_>>();

    Some(TableSummary {
        c: cols,
        r: rows,
        rc: layout.rows.len(),
        hc: layout.headers.len(),
    })
}

pub(crate) fn collect_table_kept_ranges(table: &TableSummary) -> Vec<(usize, usize)> {
    table.r.iter().map(|row| (row.i, row.i + 1)).collect()
}

pub(crate) fn collect_path_list_summary(lines: &[&str]) -> Option<PathListSummary> {
    crate::path_profile::collect_path_list_summary(lines)
}

pub(crate) fn collect_path_list_kept_ranges(pathlist: &PathListSummary) -> Vec<(usize, usize)> {
    crate::path_profile::collect_path_list_kept_ranges(pathlist)
}

pub(crate) fn match_terms(name: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for token in args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .flat_map(|arg| split_token(arg))
        .chain(split_token(name))
        .chain(
            ["error", "failed", "panic", "warning", "exception", "todo"]
                .into_iter()
                .map(str::to_owned),
        )
    {
        let normalized = token.to_ascii_lowercase();
        if normalized.len() < 3 {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
        if out.len() >= 8 {
            break;
        }
    }

    out
}

fn split_token(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn compact_args(args: &[String]) -> Vec<String> {
    args.iter()
        .take(6)
        .map(|arg| {
            if arg.len() > 80 {
                format!("{}...", &arg[..80])
            } else {
                arg.clone()
            }
        })
        .collect()
}

pub(crate) fn read_stream_payload<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, AppError> {
    let mut buf = Vec::new();
    match reader.read_to_end(&mut buf) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            if buf.is_empty() {
                return Ok(None);
            }
        }
        Err(err) => return Err(err.into()),
    }
    Ok(Some(buf))
}

pub(crate) fn read_stdin_if_piped() -> Result<Option<Vec<u8>>, AppError> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut stdin = io::stdin();
    read_stream_payload(&mut stdin)
}

pub(crate) fn resolve_real_command(name: &str) -> Result<PathBuf, AppError> {
    let shim_dir = env::var("TKE_SHIM_DIR").unwrap_or_default();
    let real_path = real_path_string();
    let shim_dir = PathBuf::from(shim_dir);

    for dir in env::split_paths(&real_path) {
        if !shim_dir.as_os_str().is_empty() && dir == shim_dir {
            continue;
        }
        for candidate_name in candidate_command_names(name) {
            let candidate = dir.join(candidate_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(AppError::MissingRealCommand(name.to_owned()))
}

pub(crate) fn real_path_string() -> String {
    env::var("TKE_REAL_PATH").unwrap_or_else(|_| env::var("PATH").unwrap_or_default())
}

pub(crate) fn classify_command(name: &str, args: &[String]) -> CommandKind {
    match name {
        "ps" | "ss" | "netstat" | "systemctl" => CommandKind::Log,
        "cat" | "sed" | "head" | "tail" | "bat" | "nl" | "awk" | "cut" => CommandKind::File,
        "ls" if args
            .iter()
            .any(|arg| arg == "-l" || arg == "-la" || arg == "-lh" || arg == "-al") =>
        {
            CommandKind::Log
        }
        "rg" | "grep" | "find" | "fd" | "tree" | "ls" => CommandKind::Search,
        "sort" | "uniq" | "wc" | "xargs" => CommandKind::Generic,
        "git" if args.first().map(String::as_str) == Some("diff") => CommandKind::Diff,
        "cargo" | "pytest" | "npm" | "pnpm" | "yarn" | "dotnet" | "go" | "cmake" | "ctest"
        | "make" | "ninja" | "node" => CommandKind::Log,
        _ => CommandKind::Generic,
    }
}

pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

pub(crate) fn base_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned()
}

pub(crate) fn candidate_config_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("TKE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".tke").join("config.json"))
}

pub(crate) fn parse_usize(raw: &str, fallback: usize) -> usize {
    raw.parse().unwrap_or(fallback)
}

pub(crate) fn csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn shell_escape(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

pub(crate) fn powershell_escape(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

pub(crate) fn cmd_escape(raw: &str) -> String {
    raw.replace('"', "\"\"")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

impl ShellKind {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "sh" | "bash" | "zsh" | "fish" | "posix" => Some(Self::Posix),
            "powershell" | "pwsh" | "ps" => Some(Self::PowerShell),
            "cmd" | "cmd.exe" => Some(Self::Cmd),
            _ => None,
        }
    }
}

pub(crate) fn detect_shell_kind() -> ShellKind {
    if let Ok(value) = env::var("TKE_SHELL")
        && let Some(shell) = ShellKind::parse(&value)
    {
        return shell;
    }
    if cfg!(windows) {
        if env::var_os("PSModulePath").is_some() {
            return ShellKind::PowerShell;
        }
        return ShellKind::Cmd;
    }
    ShellKind::Posix
}

pub(crate) fn render_activate_script(
    shell: ShellKind,
    exe: &Path,
    shim_dir: &Path,
    real_path: &str,
    agents: &[String],
    tools: &[String],
) -> String {
    let exe = exe.to_string_lossy();
    let shim_dir = shim_dir.to_string_lossy();
    let agent_csv = agents.join(",");
    let tool_csv = tools.join(",");
    let sep = shell_path_sep(shell);
    match shell {
        ShellKind::Posix => {
            [
                format!("export TKE_BIN={}", shell_escape(&exe)),
                format!("export TKE_SHIM_DIR={}", shell_escape(&shim_dir)),
                format!("export TKE_REAL_PATH={}", shell_escape(real_path)),
                format!("export TKE_AGENT_CMDS={}", shell_escape(&agent_csv)),
                format!("export TKE_TOOL_CMDS={}", shell_escape(&tool_csv)),
                format!("export PATH={}:$PATH", shell_escape(&shim_dir)),
            ]
            .join("\n")
                + "\n"
        }
        ShellKind::PowerShell => {
            [
                format!("$env:TKE_BIN = {}", powershell_escape(&exe)),
                format!("$env:TKE_SHIM_DIR = {}", powershell_escape(&shim_dir)),
                format!("$env:TKE_REAL_PATH = {}", powershell_escape(real_path)),
                format!("$env:TKE_AGENT_CMDS = {}", powershell_escape(&agent_csv)),
                format!("$env:TKE_TOOL_CMDS = {}", powershell_escape(&tool_csv)),
                format!(
                    "$env:PATH = {} + '{}' + $env:PATH",
                    powershell_escape(&shim_dir),
                    sep
                ),
            ]
            .join("\n")
                + "\n"
        }
        ShellKind::Cmd => {
            [
                format!("set \"TKE_BIN={}\"", cmd_escape(&exe)),
                format!("set \"TKE_SHIM_DIR={}\"", cmd_escape(&shim_dir)),
                format!("set \"TKE_REAL_PATH={}\"", cmd_escape(real_path)),
                format!("set \"TKE_AGENT_CMDS={}\"", cmd_escape(&agent_csv)),
                format!("set \"TKE_TOOL_CMDS={}\"", cmd_escape(&tool_csv)),
                format!("set \"PATH={shim_dir}{sep}%PATH%\""),
            ]
            .join("\r\n")
                + "\r\n"
        }
    }
}

pub(crate) fn render_deactivate_script(shell: ShellKind) -> String {
    match shell {
        ShellKind::Posix => [
            "if [ -n \"${TKE_REAL_PATH:-}\" ]; then",
            "  export PATH=\"$TKE_REAL_PATH\"",
            "fi",
            "unset TKE_BIN TKE_SHIM_DIR TKE_REAL_PATH TKE_AGENT_CMDS TKE_TOOL_CMDS",
        ]
        .join("\n")
            + "\n",
        ShellKind::PowerShell => [
            "if ($env:TKE_REAL_PATH) { $env:PATH = $env:TKE_REAL_PATH }".to_owned(),
            "Remove-Item Env:TKE_BIN,Env:TKE_SHIM_DIR,Env:TKE_REAL_PATH,Env:TKE_AGENT_CMDS,Env:TKE_TOOL_CMDS -ErrorAction SilentlyContinue".to_owned(),
        ]
        .join("\n")
            + "\n",
        ShellKind::Cmd => [
            "if defined TKE_REAL_PATH set \"PATH=%TKE_REAL_PATH%\"".to_owned(),
            "set TKE_BIN=".to_owned(),
            "set TKE_SHIM_DIR=".to_owned(),
            "set TKE_REAL_PATH=".to_owned(),
            "set TKE_AGENT_CMDS=".to_owned(),
            "set TKE_TOOL_CMDS=".to_owned(),
        ]
        .join("\r\n")
            + "\r\n",
    }
}

pub(crate) fn shell_path_sep(shell: ShellKind) -> char {
    match shell {
        ShellKind::Posix => ':',
        ShellKind::PowerShell | ShellKind::Cmd => ';',
    }
}

pub(crate) fn create_single_shim(shim_dir: &Path, exe: &Path, name: &str) -> Result<(), AppError> {
    if cfg!(windows) {
        create_windows_cmd_shim(shim_dir, exe, name)
    } else {
        let link = shim_dir.join(name);
        if link.exists() {
            fs::remove_file(&link)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(exe, &link)?;
        Ok(())
    }
}

pub(crate) fn create_windows_cmd_shim(
    shim_dir: &Path,
    exe: &Path,
    name: &str,
) -> Result<(), AppError> {
    let wrapper = shim_dir.join(format!("{name}.cmd"));
    if wrapper.exists() {
        fs::remove_file(&wrapper)?;
    }
    let exe = exe.to_string_lossy().replace('"', "\"\"");
    let body = format!("@echo off\r\n\"{exe}\" shim \"%~n0\" %*\r\nexit /b %ERRORLEVEL%\r\n");
    fs::write(wrapper, body)?;
    Ok(())
}

pub(crate) fn candidate_command_names(name: &str) -> Vec<OsString> {
    if !cfg!(windows) {
        return vec![OsString::from(name)];
    }
    let raw = OsStr::new(name);
    let has_ext = Path::new(raw).extension().is_some();
    let mut names = Vec::new();
    names.push(raw.to_os_string());
    if has_ext {
        return names;
    }
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    for ext in pathext.split(';').filter(|ext| !ext.is_empty()) {
        names.push(OsString::from(format!("{name}{ext}")));
        names.push(OsString::from(format!(
            "{name}{}",
            ext.to_ascii_lowercase()
        )));
    }
    names
}

pub(crate) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub(crate) fn default_min_trim_bytes() -> usize {
    2048
}

pub(crate) fn default_max_body_lines() -> usize {
    120
}

pub(crate) fn default_head_lines() -> usize {
    16
}

pub(crate) fn default_tail_lines() -> usize {
    16
}

pub(crate) fn default_match_context() -> usize {
    2
}

pub(crate) fn default_max_matches() -> usize {
    6
}

pub(crate) fn default_show_stats() -> bool {
    true
}

pub(crate) fn default_output_trim() -> bool {
    true
}

pub(crate) fn default_json_prefix() -> String {
    "__TKE__".to_owned()
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CommandKind {
    File,
    Search,
    Diff,
    Log,
    Generic,
}

impl CommandKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Search => "search",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrimProfile {
    File,
    Search,
    Diff,
    PathList,
    Log,
    Table,
    Stacktrace,
    Generic,
}

impl TrimProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Search => "search",
            Self::Diff => "diff",
            Self::PathList => "pathlist",
            Self::Log => "log",
            Self::Table => "table",
            Self::Stacktrace => "stacktrace",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProfileLimits {
    pub(crate) head_lines: usize,
    pub(crate) tail_lines: usize,
    pub(crate) match_context: usize,
    pub(crate) max_matches: usize,
}

pub(crate) fn select_profile(kind: CommandKind, lines: &[&str]) -> TrimProfile {
    if looks_like_diff(lines) {
        return TrimProfile::Diff;
    }
    if looks_like_stacktrace(lines) {
        return TrimProfile::Stacktrace;
    }
    if looks_like_path_list(lines) {
        return TrimProfile::PathList;
    }
    if looks_like_table(lines) {
        return TrimProfile::Table;
    }
    match kind {
        CommandKind::Search => TrimProfile::Search,
        CommandKind::Diff => TrimProfile::Diff,
        CommandKind::Log => TrimProfile::Log,
        CommandKind::File => TrimProfile::File,
        CommandKind::Generic => {
            if lines.iter().any(|line| is_log_signal(line, &[])) {
                TrimProfile::Log
            } else {
                TrimProfile::Generic
            }
        }
    }
}

pub(crate) fn profile_limits(profile: TrimProfile, config: &Config) -> ProfileLimits {
    match profile {
        TrimProfile::Diff => ProfileLimits {
            head_lines: usize::min(config.head_lines, 8),
            tail_lines: usize::min(config.tail_lines, 8),
            match_context: 1,
            max_matches: usize::max(config.max_matches, 8),
        },
        TrimProfile::Search => ProfileLimits {
            head_lines: usize::min(config.head_lines, 6),
            tail_lines: usize::min(config.tail_lines, 6),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 12),
        },
        TrimProfile::PathList => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
        },
        TrimProfile::Stacktrace => ProfileLimits {
            head_lines: usize::min(config.head_lines, 6),
            tail_lines: usize::min(config.tail_lines, 6),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 10),
        },
        TrimProfile::Log => ProfileLimits {
            head_lines: usize::min(config.head_lines, 4),
            tail_lines: usize::min(config.tail_lines, 4),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 6),
        },
        TrimProfile::Table => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
        },
        TrimProfile::File => ProfileLimits {
            head_lines: usize::min(config.head_lines, 6),
            tail_lines: usize::min(config.tail_lines, 6),
            match_context: usize::min(config.match_context, 1),
            max_matches: usize::max(config.max_matches, 8),
        },
        TrimProfile::Generic => ProfileLimits {
            head_lines: config.head_lines,
            tail_lines: config.tail_lines,
            match_context: config.match_context,
            max_matches: config.max_matches,
        },
    }
}

pub(crate) fn should_force_trim(
    profile: TrimProfile,
    total_bytes: usize,
    total_lines: usize,
    config: &Config,
) -> bool {
    match profile {
        TrimProfile::Table => {
            total_bytes >= usize::min(config.min_trim_bytes, 1024)
                || total_lines >= usize::min(config.max_body_lines, 12)
        }
        TrimProfile::PathList => {
            total_bytes >= usize::min(config.min_trim_bytes, 160)
                || total_lines >= usize::min(config.max_body_lines, 4)
        }
        _ => total_bytes >= config.min_trim_bytes || total_lines > config.max_body_lines,
    }
}

fn looks_like_diff(lines: &[&str]) -> bool {
    let score = lines
        .iter()
        .take(48)
        .filter(|line| {
            line.starts_with("diff --git ")
                || line.starts_with("@@")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
        })
        .count();
    score >= 2
}

fn looks_like_stacktrace(lines: &[&str]) -> bool {
    let frames = lines.iter().filter(|line| is_stack_frame(line)).count();
    let summary = lines.iter().any(|line| is_stack_summary(line));
    summary && frames >= 2
}

fn looks_like_table(lines: &[&str]) -> bool {
    detect_table_layout(lines).is_some()
}

fn looks_like_path_list(lines: &[&str]) -> bool {
    crate::path_profile::looks_like_path_list(lines)
}

fn detect_table_layout(lines: &[&str]) -> Option<TableLayout> {
    let search_limit = usize::min(lines.len(), 4);
    for header_index in 0..search_limit {
        let line = lines.get(header_index)?.trim();
        if line.is_empty() {
            continue;
        }
        let headers = split_table_fields(line, usize::MAX);
        if headers.len() < 3 || !looks_like_table_header(&headers) {
            continue;
        }

        let mut rows = Vec::new();
        for (offset, row_line) in lines.iter().enumerate().skip(header_index + 1).take(128) {
            let trimmed = row_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let fields = split_table_fields(trimmed, headers.len());
            if fields.len() + 1 < headers.len() || fields.len() < 3 {
                continue;
            }
            rows.push(TableDataRow {
                line_index: offset,
                fields,
            });
        }

        if rows.len() >= 4 {
            return Some(TableLayout {
                headers,
                rows,
                header_index,
            });
        }
    }
    None
}

fn split_table_fields(line: &str, max_fields: usize) -> Vec<String> {
    let aligned = split_on_wide_whitespace(line, max_fields);
    if aligned.len() >= 3 {
        return aligned;
    }
    split_on_any_whitespace(line, max_fields)
}

fn split_on_wide_whitespace(line: &str, max_fields: usize) -> Vec<String> {
    if max_fields == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut idx = 0usize;
    let chars = line.char_indices().collect::<Vec<_>>();

    while idx < chars.len() {
        let (byte_idx, ch) = chars[idx];
        if !ch.is_whitespace() {
            idx += 1;
            continue;
        }

        let mut run_end = idx + 1;
        while run_end < chars.len() && chars[run_end].1.is_whitespace() {
            run_end += 1;
        }
        let run_len = run_end - idx;
        if run_len >= 2 && out.len() + 1 < max_fields {
            let end_byte = byte_idx;
            let next_byte = chars
                .get(run_end)
                .map(|(pos, _)| *pos)
                .unwrap_or_else(|| line.len());
            let field = line[start..end_byte].trim();
            if !field.is_empty() {
                out.push(field.to_owned());
            }
            start = next_byte;
        }
        idx = run_end;
    }

    let tail = line[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_owned());
    }
    out
}

fn split_on_any_whitespace(line: &str, max_fields: usize) -> Vec<String> {
    if max_fields == 0 {
        return Vec::new();
    }
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return Vec::new();
    }
    if max_fields == usize::MAX || parts.len() <= max_fields {
        return parts.into_iter().map(str::to_owned).collect();
    }

    let mut out = parts
        .iter()
        .take(max_fields.saturating_sub(1))
        .map(|part| (*part).to_owned())
        .collect::<Vec<_>>();
    out.push(parts[max_fields - 1..].join(" "));
    out
}

fn looks_like_table_header(headers: &[String]) -> bool {
    let mut known = 0usize;
    let mut score = 0usize;
    for header in headers {
        let normalized = normalize_header_name(header);
        if normalized.is_empty() {
            continue;
        }
        if is_known_table_header(&normalized) {
            known += 1;
            score += 2;
            continue;
        }
        if header
            .chars()
            .any(|ch| matches!(ch, ':' | '/' | '(' | ')' | '{' | '}' | '[' | ']'))
        {
            continue;
        }
        let alpha_count = normalized
            .chars()
            .filter(|ch| ch.is_ascii_alphabetic())
            .count();
        if alpha_count >= 2
            && normalized.len() <= 24
            && header.chars().all(|ch| {
                ch.is_ascii_alphanumeric()
                    || ch.is_ascii_whitespace()
                    || ch == '%'
                    || ch == '-'
                    || ch == '_'
            })
        {
            score += 1;
        }
    }
    known >= 1 || score >= usize::max(4, headers.len().saturating_sub(1))
}

fn normalize_header_name(header: &str) -> String {
    header
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '%')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_known_table_header(header: &str) -> bool {
    matches!(
        header,
        "user"
            | "pid"
            | "%cpu"
            | "%mem"
            | "vsz"
            | "rss"
            | "tty"
            | "stat"
            | "start"
            | "time"
            | "command"
            | "unit"
            | "load"
            | "active"
            | "sub"
            | "description"
            | "netid"
            | "state"
            | "recvq"
            | "sendq"
            | "localaddressport"
            | "peeraddressport"
            | "process"
            | "containerid"
            | "image"
            | "created"
            | "status"
            | "ports"
            | "names"
            | "size"
            | "avail"
            | "use%"
            | "mountedon"
    )
}

fn select_table_columns(headers: &[String]) -> Vec<usize> {
    if headers.len() <= 6 {
        return (0..headers.len()).collect();
    }

    let normalized = headers
        .iter()
        .map(|header| normalize_header_name(header))
        .collect::<Vec<_>>();
    let wanted = [
        "user",
        "pid",
        "%cpu",
        "%mem",
        "stat",
        "command",
        "unit",
        "active",
        "sub",
        "description",
        "netid",
        "state",
        "localaddressport",
        "peeraddressport",
        "process",
        "containerid",
        "image",
        "status",
        "ports",
        "names",
    ];

    let mut selected = Vec::new();
    for idx in [0usize, 1usize, headers.len().saturating_sub(1)] {
        if idx < headers.len() && !selected.contains(&idx) {
            selected.push(idx);
        }
    }
    for name in wanted {
        for (idx, header) in normalized.iter().enumerate() {
            if header == name && !selected.contains(&idx) {
                selected.push(idx);
            }
            if selected.len() >= 6 {
                break;
            }
        }
        if selected.len() >= 6 {
            break;
        }
    }
    for idx in 0..headers.len() {
        if selected.len() >= 6 {
            break;
        }
        if !selected.contains(&idx) {
            selected.push(idx);
        }
    }
    selected.sort_unstable();
    selected
}

fn select_table_rows(
    name: &str,
    args: &[String],
    layout: &TableLayout,
    terms: &[String],
) -> Vec<usize> {
    let mut selected = Vec::new();
    let cap = match name {
        "ps" | "ss" | "netstat" | "systemctl" => 5,
        "docker" if args.first().map(String::as_str) == Some("ps") => 5,
        _ => 6,
    };

    for idx in 0..usize::min(layout.rows.len(), 3) {
        push_unique_index(&mut selected, idx, cap);
    }

    for idx in collect_table_signal_rows(layout, terms) {
        push_unique_index(&mut selected, idx, cap);
    }

    for idx in collect_top_numeric_rows(layout, "%cpu", 2) {
        push_unique_index(&mut selected, idx, cap);
    }
    for idx in collect_top_numeric_rows(layout, "%mem", 2) {
        push_unique_index(&mut selected, idx, cap);
    }

    if layout.rows.len() > 3 {
        push_unique_index(&mut selected, layout.rows.len() - 1, cap);
    }

    selected.sort_unstable();
    selected
}

fn collect_table_signal_rows(layout: &TableLayout, terms: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for (idx, row) in layout.rows.iter().enumerate() {
        let joined = row.fields.join(" ").to_ascii_lowercase();
        if is_log_signal(&joined, terms)
            || joined.contains("codex")
            || joined.contains("listen")
            || joined.contains("estab")
            || joined.contains("failed")
            || joined.contains("exited")
        {
            out.push(idx);
            if out.len() >= 3 {
                break;
            }
        }
    }
    out
}

fn collect_top_numeric_rows(layout: &TableLayout, header: &str, limit: usize) -> Vec<usize> {
    let normalized = layout
        .headers
        .iter()
        .map(|value| normalize_header_name(value))
        .collect::<Vec<_>>();
    let Some(column) = normalized.iter().position(|value| value == header) else {
        return Vec::new();
    };

    let mut ranked = layout
        .rows
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            row.fields
                .get(column)
                .and_then(|value| parse_numeric_cell(value))
                .map(|score| (idx, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(limit).map(|(idx, _)| idx).collect()
}

fn parse_numeric_cell(value: &str) -> Option<f64> {
    let cleaned = value.trim().trim_end_matches('%').replace(',', "");
    cleaned.parse::<f64>().ok()
}

fn push_unique_index(out: &mut Vec<usize>, idx: usize, cap: usize) {
    if out.len() >= cap || out.contains(&idx) {
        return;
    }
    out.push(idx);
}

fn is_stack_frame(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("at ")
        || trimmed.starts_with('#')
        || trimmed.contains(".rs:")
        || trimmed.contains(".py\", line ")
        || trimmed.contains(".js:")
}

fn is_stack_summary(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("traceback")
        || lower.contains("stack trace")
        || lower.contains("panic")
        || lower.contains("exception")
}

pub(crate) fn is_log_signal(line: &str, terms: &[String]) -> bool {
    let lower = line.to_ascii_lowercase();
    if [
        "error",
        "failed",
        "panic",
        "exception",
        "warning",
        "abort",
        "undefined",
        "mismatch",
    ]
    .iter()
    .any(|term| lower.contains(term))
    {
        return true;
    }
    terms
        .iter()
        .any(|term| !term.is_empty() && lower.contains(term))
}

#[derive(Serialize)]
pub(crate) struct TrimEnvelope {
    pub(crate) v: u8,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) cmd: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) a: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) k: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sr: Option<String>,
    pub(crate) p: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) c: Option<usize>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) s: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) t: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) h: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) ta: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) m: Vec<MatchChunk>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) o: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) st: Option<TrimStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tb: Option<TableSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pl: Option<PathListSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) lg: Option<LogSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) b: Option<Vec<String>>,
}

#[derive(Serialize)]
pub(crate) struct MatchChunk {
    pub(crate) k: String,
    pub(crate) r: [usize; 2],
    pub(crate) l: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct TrimStats {
    pub(crate) tb: usize,
    pub(crate) tl: usize,
    pub(crate) el: usize,
}

#[derive(Serialize)]
pub(crate) struct TableSummary {
    pub(crate) c: Vec<String>,
    pub(crate) r: Vec<TableRow>,
    pub(crate) rc: usize,
    pub(crate) hc: usize,
}

#[derive(Serialize)]
pub(crate) struct TableRow {
    pub(crate) i: usize,
    pub(crate) v: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct PathListSummary {
    #[serde(skip_serializing)]
    pub(crate) rc: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) s: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) d: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) f: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) l: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) e: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) b: Vec<PathBucket>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) r: Vec<PathRow>,
}

#[derive(Serialize)]
pub(crate) struct PathBucket {
    pub(crate) d: String,
    pub(crate) c: usize,
    pub(crate) e: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct PathRow {
    pub(crate) i: usize,
    pub(crate) v: String,
}

#[derive(Serialize)]
pub(crate) struct LogSummary {
    pub(crate) fail: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) warn: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

pub(crate) struct RepeatedRun {
    pub(crate) range: [usize; 2],
    pub(crate) count: usize,
    pub(crate) sample: String,
}

pub(crate) struct TableLayout {
    headers: Vec<String>,
    rows: Vec<TableDataRow>,
    #[allow(dead_code)]
    header_index: usize,
}

pub(crate) struct TableDataRow {
    pub(crate) line_index: usize,
    pub(crate) fields: Vec<String>,
}

pub(crate) struct PathEntry {
    pub(crate) line_index: usize,
    pub(crate) parent: String,
    pub(crate) value: String,
}

pub(crate) struct BenchmarkSpec {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) profile: String,
    pub(crate) expected: BenchmarkExpectation,
    pub(crate) call_id: String,
    pub(crate) sample: String,
}

pub(crate) struct BenchmarkTaskSpec {
    pub(crate) name: String,
    pub(crate) mode: String,
    pub(crate) objective: String,
    pub(crate) required_fragments: Vec<String>,
    pub(crate) rollout: String,
}

pub(crate) struct BenchmarkTaskStep {
    pub(crate) call_id: String,
    pub(crate) command: String,
    pub(crate) output: String,
}

#[derive(Clone, Copy)]
pub(crate) enum BenchmarkExpectation {
    Compress,
    PassThrough,
}

impl BenchmarkExpectation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Compress => "compress",
            Self::PassThrough => "pass_through",
        }
    }
}
