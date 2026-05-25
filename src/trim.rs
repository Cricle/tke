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
        TrimProfile::GitStatus => Vec::new(),
        TrimProfile::Json => Vec::new(),
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
    let term_sequences = terms
        .iter()
        .map(|term| ascii_word_tokens(term))
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    let mut used = Vec::<(usize, usize)>::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_tokens = ascii_word_tokens(line);
        if !term_sequences.iter().any(|sequence| {
            let refs = sequence.iter().map(String::as_str).collect::<Vec<_>>();
            has_token_sequence(&line_tokens, &refs)
        }) {
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
        let trimmed = line.trim_start();
        if is_diff_file_header(trimmed)
            || is_diff_path_marker(trimmed)
            || is_diff_index_marker(trimmed)
        {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "file") && out.len() >= 4 {
                break;
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if is_diff_hunk_header(trimmed) {
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
    if name == "du"
        && let Some(summary) = collect_du_summary(lines)
    {
        return Some(summary);
    }
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

pub(crate) fn collect_diff_summary(lines: &[&str]) -> Option<DiffSummary> {
    let mut files = Vec::new();
    let mut current: Option<DiffFileSummary> = None;

    for line in lines {
        if let Some(path) = line.strip_prefix("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            let path = path
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_start_matches("a/")
                .to_owned();
            current = Some(DiffFileSummary {
                p: path,
                add: 0,
                del: 0,
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if is_diff_path_marker(line) || is_diff_hunk_header(line) {
            continue;
        }
        if line.chars().next() == Some('+') {
            file.add += 1;
        } else if line.chars().next() == Some('-') {
            file.del += 1;
        }
    }

    if let Some(file) = current.take() {
        files.push(file);
    }
    if files.is_empty() {
        return collect_diff_stat_summary(lines);
    }
    files.truncate(6);
    Some(DiffSummary { f: files })
}

fn collect_diff_stat_summary(lines: &[&str]) -> Option<DiffSummary> {
    let mut files = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        let tokens = ascii_word_tokens(trimmed);
        if trimmed.is_empty() || is_diff_stat_totals_line(&tokens) {
            continue;
        }
        let Some((path, stat)) = trimmed.split_once('|') else {
            continue;
        };
        let stat = stat.trim();
        if stat.is_empty() {
            continue;
        }
        let mut add = 0usize;
        let mut del = 0usize;
        for ch in stat.chars() {
            if ch == '+' {
                add += 1;
            } else if ch == '-' {
                del += 1;
            }
        }
        files.push(DiffFileSummary {
            p: path.trim().to_owned(),
            add,
            del,
        });
    }
    if files.is_empty() {
        return None;
    }
    files.truncate(6);
    Some(DiffSummary { f: files })
}

pub(crate) fn collect_git_status_summary(lines: &[&str]) -> Option<GitStatusSummary> {
    let mut branch = None;
    let mut modified = 0usize;
    let mut added = 0usize;
    let mut deleted = 0usize;
    let mut renamed = 0usize;
    let mut untracked = 0usize;
    let mut examples = Vec::new();

    for line in lines {
        if let Some(value) = line.strip_prefix("## ") {
            if !value.trim().is_empty() {
                branch = Some(value.trim().to_owned());
            }
            continue;
        }

        let trimmed = line.trim_end();
        if trimmed.len() < 3 {
            continue;
        }
        let status = &trimmed[..2];
        let path = trimmed[3..].trim();
        if path.is_empty() {
            continue;
        }

        if status == "??" {
            untracked += 1;
            if examples.len() < 6 {
                examples.push(format!("?? {path}"));
            }
            continue;
        }

        let mut recognized = false;
        for ch in status.chars() {
            match ch {
                'M' => {
                    modified += 1;
                    recognized = true;
                }
                'A' => {
                    added += 1;
                    recognized = true;
                }
                'D' => {
                    deleted += 1;
                    recognized = true;
                }
                'R' => {
                    renamed += 1;
                    recognized = true;
                }
                ' ' | '.' => {}
                _ => {}
            }
        }
        if recognized && examples.len() < 6 {
            examples.push(format!("{status} {path}"));
        }
    }

    if branch.is_none()
        && modified == 0
        && added == 0
        && deleted == 0
        && renamed == 0
        && untracked == 0
    {
        return None;
    }

    Some(GitStatusSummary {
        br: branch,
        m: modified,
        a: added,
        d: deleted,
        r: renamed,
        u: untracked,
        e: examples,
    })
}

pub(crate) fn collect_build_summary(name: &str, lines: &[&str]) -> Option<BuildSummary> {
    let mut compiling = 0usize;
    let mut running = 0usize;
    let mut finished = None;
    let mut test_result = None;
    let mut counts = BuildCounters::default();
    let mut examples = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        let tokens = ascii_word_tokens(trimmed);
        if is_build_compile_line(trimmed, &tokens) {
            compiling += 1;
        } else if is_build_running_line(&tokens) {
            running += 1;
        } else if finished.is_none() && is_build_finished_line(&tokens) {
            finished = Some(trimmed.to_owned());
        } else if test_result.is_none() && is_build_test_result_line(&tokens) {
            test_result = Some(trimmed.to_owned());
        }

        if let Some(line_counts) = extract_build_counts(trimmed, &tokens) {
            merge_build_counters(&mut counts, line_counts);
        }
        if should_capture_build_example(trimmed, &tokens) {
            push_unique_sample(&mut examples, trimmed, 4, 96);
        }
    }

    if counts.tt == 0 {
        let computed_total = counts
            .ok
            .saturating_add(counts.fl)
            .saturating_add(counts.sk);
        if computed_total > 0 {
            counts.tt = computed_total;
        } else if counts.ip > 0 {
            counts.tt = counts.ip;
        }
    }

    if compiling == 0
        && running == 0
        && finished.is_none()
        && test_result.is_none()
        && counts.is_empty()
        && examples.is_empty()
    {
        return None;
    }

    Some(BuildSummary {
        n: name.to_owned(),
        cp: compiling,
        rn: running,
        ok: counts.ok,
        fl: counts.fl,
        sk: counts.sk,
        tt: counts.tt,
        ip: counts.ip,
        fi: finished.map(|value| truncate_ellipsized(&value, 96)),
        tr: test_result.map(|value| truncate_ellipsized(&value, 96)),
        e: examples,
    })
}

fn is_build_compile_line(line: &str, tokens: &[String]) -> bool {
    if has_ascii_token(tokens, "compiling")
        || has_ascii_token(tokens, "checking")
        || has_ascii_token(tokens, "building")
        || has_ascii_token(tokens, "collecting")
        || has_ascii_token(tokens, "installing")
    {
        return true;
    }
    line.as_bytes().first() == Some(&b'>') && has_ascii_token(tokens, "task")
}

fn is_build_running_line(tokens: &[String]) -> bool {
    has_ascii_token(tokens, "running")
        || has_ascii_token(tokens, "executing")
        || has_token_sequence(tokens, &["test", "run", "for"])
}

fn is_build_finished_line(tokens: &[String]) -> bool {
    (has_ascii_token(tokens, "finished")
        && (has_ascii_token(tokens, "test")
            || has_ascii_token(tokens, "profile")
            || has_ascii_token(tokens, "build")))
        || has_token_sequence(tokens, &["build", "success"])
        || has_token_sequence(tokens, &["build", "successful"])
        || has_token_sequence(tokens, &["build", "failure"])
        || has_token_sequence(tokens, &["build", "failed"])
        || has_token_sequence(tokens, &["successfully", "installed"])
        || has_token_sequence(tokens, &["successfully", "built"])
}

fn is_build_test_result_line(tokens: &[String]) -> bool {
    has_token_sequence(tokens, &["test", "result"])
        || has_token_sequence(tokens, &["tests", "run"])
        || has_ascii_token(tokens, "failures")
        || has_ascii_token(tokens, "failed")
        || has_token_sequence(tokens, &["not", "ok"])
        || has_ascii_token(tokens, "passed")
}

#[derive(Clone, Copy, Default)]
struct BuildCounters {
    ok: usize,
    fl: usize,
    sk: usize,
    tt: usize,
    ip: usize,
}

impl BuildCounters {
    fn is_empty(self) -> bool {
        self.ok == 0 && self.fl == 0 && self.sk == 0 && self.tt == 0 && self.ip == 0
    }
}

fn merge_build_counters(out: &mut BuildCounters, update: BuildCounters) {
    out.ok = out.ok.max(update.ok);
    out.fl = out.fl.max(update.fl);
    out.sk = out.sk.max(update.sk);
    out.tt = out.tt.max(update.tt);
    out.ip = out.ip.max(update.ip);
}

fn extract_build_counts(line: &str, tokens: &[String]) -> Option<BuildCounters> {
    let mut out = BuildCounters::default();
    let mut matched = false;

    if let Some(count) = colon_label_count(line, "passed") {
        out.ok = count;
        matched = true;
    }
    if let Some(count) =
        colon_label_count(line, "failed").or_else(|| colon_label_count(line, "failures"))
    {
        out.fl = count;
        matched = true;
    }
    if let Some(count) = colon_label_count(line, "skipped") {
        out.sk = count;
        matched = true;
    }
    if let Some(count) =
        colon_label_count(line, "total").or_else(|| colon_label_count(line, "tests run"))
    {
        out.tt = count;
        matched = true;
    }

    if out.ok == 0
        && let Some(count) = count_before_token(tokens, "passed")
            .or_else(|| count_before_sequence(tokens, &["tests", "passed"]))
    {
        out.ok = count;
        matched = true;
    }
    if out.fl == 0
        && let Some(count) = count_before_token(tokens, "failed")
            .or_else(|| count_before_token(tokens, "failures"))
            .or_else(|| count_before_sequence(tokens, &["tests", "failed"]))
    {
        out.fl = count;
        matched = true;
    }
    if out.sk == 0
        && let Some(count) = count_before_token(tokens, "skipped")
    {
        out.sk = count;
        matched = true;
    }
    if out.tt == 0
        && let Some(count) = count_after_sequence(tokens, &["tests", "run"])
            .or_else(|| count_before_sequence(tokens, &["tests", "completed"]))
            .or_else(|| count_before_sequence(tokens, &["test", "completed"]))
            .or_else(|| adjacent_count_for_token(tokens, "total"))
            .or_else(|| count_after_sequence(tokens, &["out", "of"]))
    {
        out.tt = count;
        matched = true;
    }
    if let Some(count) = count_packages_after_install(line, tokens) {
        out.ip = count;
        matched = true;
    }

    matched.then_some(out)
}

fn should_capture_build_example(line: &str, tokens: &[String]) -> bool {
    !line.is_empty()
        && (is_failure_signal_line(line)
            || has_token_sequence(tokens, &["successfully", "installed"])
            || has_token_sequence(tokens, &["tests", "run"])
            || has_token_sequence(tokens, &["test", "result"])
            || count_before_sequence(tokens, &["tests", "failed"]).is_some()
            || count_before_sequence(tokens, &["tests", "passed"]).is_some()
            || count_before_sequence(tokens, &["tests", "completed"]).is_some()
            || count_before_sequence(tokens, &["test", "completed"]).is_some())
}

fn push_unique_sample(out: &mut Vec<String>, line: &str, cap: usize, width: usize) {
    if out.len() >= cap {
        return;
    }
    let sample = truncate_ellipsized(line, width);
    if !out.iter().any(|existing| existing == &sample) {
        out.push(sample);
    }
}

fn adjacent_count_for_token(tokens: &[String], label: &str) -> Option<usize> {
    count_before_token(tokens, label).or_else(|| count_after_token(tokens, label))
}

fn count_before_token(tokens: &[String], label: &str) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(idx, token)| {
        if token == label && idx > 0 {
            parse_numeric_token(tokens.get(idx - 1)?)
        } else {
            None
        }
    })
}

fn count_after_token(tokens: &[String], label: &str) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(idx, token)| {
        if token == label {
            parse_numeric_token(tokens.get(idx + 1)?)
        } else {
            None
        }
    })
}

fn count_before_sequence(tokens: &[String], sequence: &[&str]) -> Option<usize> {
    if sequence.is_empty() || tokens.len() <= sequence.len() {
        return None;
    }
    tokens
        .windows(sequence.len())
        .enumerate()
        .find_map(|(idx, window)| {
            window
                .iter()
                .zip(sequence.iter())
                .all(|(token, expected)| token == expected)
                .then_some(idx)
                .and_then(|end| end.checked_sub(1))
                .and_then(|value_idx| parse_numeric_token(tokens.get(value_idx)?))
        })
}

fn count_after_sequence(tokens: &[String], sequence: &[&str]) -> Option<usize> {
    if sequence.is_empty() || tokens.len() <= sequence.len() {
        return None;
    }
    tokens
        .windows(sequence.len())
        .enumerate()
        .find_map(|(idx, window)| {
            window
                .iter()
                .zip(sequence.iter())
                .all(|(token, expected)| token == expected)
                .then_some(idx + sequence.len())
                .and_then(|value_idx| parse_numeric_token(tokens.get(value_idx)?))
        })
}

fn parse_numeric_token(token: &String) -> Option<usize> {
    token.chars().all(|ch| ch.is_ascii_digit()).then(|| ())?;
    token.parse().ok()
}

fn count_packages_after_install(line: &str, tokens: &[String]) -> Option<usize> {
    if !has_token_sequence(tokens, &["successfully", "installed"]) {
        return None;
    }
    let mut seen_installed = false;
    let mut count = 0usize;
    for word in line.split_whitespace() {
        let word_tokens = ascii_word_tokens(word);
        if word_tokens.len() == 1
            && word_tokens
                .first()
                .is_some_and(|token| token == "installed")
        {
            seen_installed = true;
            continue;
        }
        if seen_installed && !word_tokens.is_empty() {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

fn colon_label_count(line: &str, label: &str) -> Option<usize> {
    let lower = line.to_ascii_lowercase();
    let label_len = label.len();
    lower.char_indices().find_map(|(idx, _)| {
        let end = idx + label_len;
        let segment = lower.get(idx..end)?;
        if segment != label {
            return None;
        }
        let before_ok = idx == 0
            || lower[..idx]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        if !before_ok {
            return None;
        }
        let rest = lower.get(end..)?.trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let digits = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        (!digits.is_empty())
            .then(|| digits.parse::<usize>().ok())
            .flatten()
    })
}

pub(crate) fn compact_json_body(text: &str) -> Option<Vec<String>> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let compact = serde_json::to_string(&value).ok()?;
    if compact.len() <= 240 {
        return Some(vec![compact]);
    }
    Some(compact_json_preview(&value, 6))
}

pub(crate) fn compact_json_body_for_command(name: &str, text: &str) -> Option<Vec<String>> {
    let payload = json_payload_text_for_command(name, text)?;
    compact_json_body(payload)
}

pub(crate) fn has_prefix(raw: &str, prefix: &str) -> bool {
    raw.len() >= prefix.len() && raw.as_bytes().get(..prefix.len()) == Some(prefix.as_bytes())
}

pub(crate) fn match_terms(name: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for token in args
        .iter()
        .filter(|arg| !has_prefix(arg, "-"))
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
                truncate_ellipsized(arg, 80)
            } else {
                arg.clone()
            }
        })
        .collect()
}

pub(crate) fn truncate_ellipsized(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let cutoff = text
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= max_bytes)
        .last()
        .unwrap_or(0);
    format!("{}...", &text[..cutoff])
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
    if buf.is_empty() {
        return Ok(None);
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
        "ps" | "ss" | "netstat" | "systemctl" | "docker" | "du" | "df" | "psql" | "redis-cli" => {
            CommandKind::Log
        }
        "cat" | "sed" | "head" | "tail" | "bat" | "nl" | "awk" | "cut" | "tr" | "perl" | "file" => {
            CommandKind::File
        }
        "python" | "python3" => CommandKind::Log,
        "ls" if args
            .iter()
            .any(|arg| arg == "-l" || arg == "-la" || arg == "-lh" || arg == "-al") =>
        {
            CommandKind::Log
        }
        "rg" | "grep" | "find" | "fd" | "tree" | "ls" | "which" => CommandKind::Search,
        "readlink" | "stat" if args.iter().any(|arg| has_leading_char(arg, '/')) => {
            CommandKind::File
        }
        "sort" | "uniq" | "wc" | "xargs" | "jq" | "curl" | "readlink" | "stat" => {
            CommandKind::Generic
        }
        "git" if args.first().map(String::as_str) == Some("diff") => CommandKind::Diff,
        "git" if args.first().map(String::as_str) == Some("show") => CommandKind::File,
        "git" if args.first().map(String::as_str) == Some("status") => CommandKind::Log,
        "git"
            if matches!(
                args.first().map(String::as_str),
                Some("rev-parse" | "remote" | "branch" | "log" | "ls-remote")
            ) =>
        {
            CommandKind::Generic
        }
        "cargo" | "pytest" | "npm" | "pnpm" | "yarn" | "bun" | "dotnet" | "go" | "cmake"
        | "ctest" | "make" | "ninja" | "node" | "pip" | "uv" | "poetry" | "mvn" | "gradle"
        | "gradlew" | "javac" | "java" | "bundle" | "composer" => CommandKind::Log,
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
    GitStatus,
    Json,
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
            Self::GitStatus => "gitstatus",
            Self::Json => "json",
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

pub(crate) fn select_profile(
    name: &str,
    args: &[String],
    kind: CommandKind,
    lines: &[&str],
) -> TrimProfile {
    let is_python_command = matches!(name, "python" | "python3");
    if name == "git"
        && args.first().map(String::as_str) == Some("status")
        && collect_git_status_summary(lines).is_some()
    {
        return TrimProfile::GitStatus;
    }
    if looks_like_json_document(name, lines) {
        return TrimProfile::Json;
    }
    if is_python_command {
        if looks_like_path_list(lines)
            && lines
                .iter()
                .any(|line| has_explicit_path_signal(line.trim()))
        {
            return TrimProfile::PathList;
        }
        if looks_like_table(lines) {
            return TrimProfile::Table;
        }
    }
    if looks_like_diff(lines) {
        return TrimProfile::Diff;
    }
    if looks_like_stacktrace(lines) {
        return TrimProfile::Stacktrace;
    }
    if !is_python_command && looks_like_path_list(lines) {
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
        TrimProfile::GitStatus => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
        },
        TrimProfile::Json => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
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
        TrimProfile::GitStatus => {
            total_bytes >= usize::min(config.min_trim_bytes, 160)
                || total_lines >= usize::min(config.max_body_lines, 4)
        }
        TrimProfile::Json => {
            total_bytes >= usize::min(config.min_trim_bytes, 256)
                || total_lines >= usize::min(config.max_body_lines, 8)
        }
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
            let trimmed = line.trim_start();
            is_diff_file_header(trimmed)
                || is_diff_hunk_header(trimmed)
                || is_diff_path_marker(trimmed)
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
    detect_table_layout(lines).is_some() || looks_like_du_listing(lines)
}

fn looks_like_path_list(lines: &[&str]) -> bool {
    crate::path_profile::looks_like_path_list(lines)
}

fn looks_like_json_document(name: &str, lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }
    let text = lines.join("\n");
    json_payload_text_for_command(name, &text).is_some() || looks_like_json_lines(lines)
}

fn has_explicit_path_signal(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let trimmed = line.trim_start();
    has_path_prefix(trimmed) || line.chars().any(|ch| matches!(ch, '/' | '\\'))
}

fn looks_like_json_lines(lines: &[&str]) -> bool {
    let non_empty = lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() < 2 {
        return false;
    }
    non_empty.iter().all(|line| {
        has_json_delimiters(line) && serde_json::from_str::<serde_json::Value>(line).is_ok()
    })
}

fn json_payload_text_for_command<'a>(name: &str, text: &'a str) -> Option<&'a str> {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return None;
    }
    if !has_json_delimiters(trimmed) {
        if name == "curl" {
            return extract_http_json_body(text);
        }
        return None;
    }
    serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .map(|_| trimmed)
}

fn extract_http_json_body(text: &str) -> Option<&str> {
    let (header, body) = if let Some((header, body)) = text.split_once("\r\n\r\n") {
        (header, body)
    } else if let Some((header, body)) = text.split_once("\n\n") {
        (header, body)
    } else {
        return None;
    };

    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let mut lines = header.lines();
    let status_line = lines.next()?.trim_end_matches('\r').trim();
    if !is_http_status_line(status_line) {
        return None;
    }
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())?;
    if !(200..300).contains(&status_code) {
        return None;
    }

    let mut saw_json_content_type = false;
    for line in lines.take(64) {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return None;
        };
        if name.eq_ignore_ascii_case("content-type")
            && has_ascii_token(&ascii_word_tokens(value), "json")
        {
            saw_json_content_type = true;
        }
    }
    if !saw_json_content_type {
        return None;
    }

    serde_json::from_str::<serde_json::Value>(body).ok()?;
    Some(body)
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
        if header_index + 1 < lines.len() {
            let next_fields = split_table_fields(lines[header_index + 1].trim(), headers.len());
            if next_fields
                .iter()
                .any(|field| looks_like_codeish_table_cell(field))
            {
                continue;
            }
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

fn looks_like_du_listing(lines: &[&str]) -> bool {
    let mut matched = 0usize;
    for line in lines.iter().take(128) {
        if parse_du_row(line).is_some() {
            matched += 1;
        } else if !line.trim().is_empty() {
            return false;
        }
    }
    matched >= 4
}

fn collect_du_summary(lines: &[&str]) -> Option<TableSummary> {
    let rows = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let (size, path) = parse_du_row(line)?;
            Some(TableRow {
                i: idx,
                v: vec![size, path],
            })
        })
        .collect::<Vec<_>>();
    if rows.len() < 4 {
        return None;
    }

    let selected = select_du_rows(&rows);
    if selected.is_empty() {
        return None;
    }

    Some(TableSummary {
        c: vec!["Size".to_owned(), "Path".to_owned()],
        r: selected.into_iter().map(|idx| rows[idx].clone()).collect(),
        rc: rows.len(),
        hc: 2,
    })
}

fn parse_du_row(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let size = parts.next()?.trim();
    let path = parts.next()?.trim();
    if size.is_empty() || path.is_empty() || !looks_like_du_size(size) {
        return None;
    }
    Some((size.to_owned(), path.to_owned()))
}

fn looks_like_du_size(raw: &str) -> bool {
    let mut chars = raw.chars();
    let mut saw_digit = false;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() || ch == '.' {
            saw_digit = true;
            continue;
        }
        if !saw_digit {
            return false;
        }
        return chars.next().is_none() && matches!(ch, 'B' | 'K' | 'M' | 'G' | 'T' | 'P');
    }
    saw_digit
}

fn select_du_rows(rows: &[TableRow]) -> Vec<usize> {
    let mut selected = Vec::new();
    let cap = 6usize;
    for idx in 0..usize::min(rows.len(), 3) {
        push_unique_index(&mut selected, idx, cap);
    }
    if rows.len() > 3 {
        push_unique_index(&mut selected, rows.len() - 1, cap);
    }

    let mut ranked = rows
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            row.v
                .first()
                .and_then(|value| parse_human_size(value))
                .map(|score| (idx, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (idx, _) in ranked.into_iter().take(2) {
        push_unique_index(&mut selected, idx, cap);
    }

    selected.sort_unstable();
    selected
}

fn parse_human_size(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    let unit = trimmed.chars().last()?;
    let scale = match unit {
        'B' => 1.0,
        'K' => 1024.0,
        'M' => 1024.0 * 1024.0,
        'G' => 1024.0 * 1024.0 * 1024.0,
        'T' => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        'P' => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return trimmed.parse::<f64>().ok(),
    };
    let number = &trimmed[..trimmed.len().saturating_sub(1)];
    Some(number.parse::<f64>().ok()? * scale)
}

fn compact_json_preview(value: &serde_json::Value, limit: usize) -> Vec<String> {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .take(limit)
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    key,
                    truncate_ellipsized(&compact_json_scalar(value), 72)
                )
            })
            .collect(),
        serde_json::Value::Array(items) => items
            .iter()
            .take(limit)
            .map(|value| truncate_ellipsized(&compact_json_scalar(value), 72))
            .collect(),
        _ => vec![truncate_ellipsized(&compact_json_scalar(value), 120)],
    }
}

fn compact_json_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_owned(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
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
    if headers
        .iter()
        .any(|header| looks_like_codeish_table_cell(header))
    {
        return false;
    }
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

fn looks_like_codeish_table_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return false;
    }
    let tokens = ascii_word_tokens(trimmed);
    has_token_prefix(&tokens, &["fn"])
        || has_token_prefix(&tokens, &["struct"])
        || has_token_prefix(&tokens, &["impl"])
        || has_token_prefix(&tokens, &["let"])
        || has_token_sequence(&tokens, &["println"])
        || trimmed.chars().any(|ch| matches!(ch, '{' | '}'))
        || has_repeated_symbol_pair(trimmed, ':')
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
            | "column"
            | "type"
            | "value"
            | "count"
            | "database"
            | "schema"
            | "table"
            | "rows"
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
        "filesystem",
        "user",
        "pid",
        "%cpu",
        "%mem",
        "size",
        "used",
        "avail",
        "use%",
        "mountedon",
        "stat",
        "command",
        "schema",
        "table",
        "rows",
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
            if selected.len() >= 5 {
                break;
            }
        }
        if selected.len() >= 5 {
            break;
        }
    }
    for idx in 0..headers.len() {
        if selected.len() >= 5 {
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
        "ls" | "df" | "du" | "psql" => 5,
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
        let joined = row.fields.join(" ");
        let tokens = ascii_word_tokens(&joined);
        if is_log_signal(&joined, terms)
            || has_ascii_token(&tokens, "codex")
            || has_ascii_token(&tokens, "listen")
            || has_ascii_token(&tokens, "estab")
            || has_ascii_token(&tokens, "exited")
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
    has_token_prefix(&ascii_word_tokens(trimmed), &["at"])
        || trimmed.chars().next() == Some('#')
        || has_source_location(trimmed)
}

fn is_stack_summary(line: &str) -> bool {
    let class = classify_log_line(line);
    class.stack_summary
        || class.failure && has_token_sequence(&ascii_word_tokens(line), &["stack", "trace"])
}

#[derive(Clone, Copy, Default)]
struct LogLineClass {
    warning: bool,
    failure: bool,
    stack_summary: bool,
}

fn classify_log_line(line: &str) -> LogLineClass {
    let trimmed = line.trim();
    let tokens = ascii_word_tokens(trimmed);
    let warning = has_ascii_token(&tokens, "warning") || has_ascii_token(&tokens, "warn");
    let stack_summary = has_ascii_token(&tokens, "traceback")
        || has_ascii_token(&tokens, "panic")
        || has_ascii_token(&tokens, "exception")
        || has_token_sequence(&tokens, &["stack", "trace"]);
    let failure = !is_zero_failed_summary(&tokens)
        && (has_ascii_token(&tokens, "error")
            || has_ascii_token(&tokens, "panic")
            || has_ascii_token(&tokens, "exception")
            || has_ascii_token(&tokens, "fail")
            || has_ascii_token(&tokens, "failed")
            || has_token_sequence(&tokens, &["not", "ok"]));
    LogLineClass {
        warning,
        failure,
        stack_summary,
    }
}

fn has_source_location(line: &str) -> bool {
    has_source_extension_location(line, ".rs")
        || has_source_extension_location(line, ".js")
        || has_python_trace_location(line)
}

fn has_source_extension_location(line: &str, extension: &str) -> bool {
    line.char_indices().any(|(idx, ch)| {
        ch == ':'
            && line
                .get(..idx)
                .is_some_and(|prefix| prefix.ends_with(extension))
    })
}

fn has_python_trace_location(line: &str) -> bool {
    let segments = line.split('"').collect::<Vec<_>>();
    segments
        .windows(2)
        .any(|window| window[0].ends_with("File ") && window[1].ends_with(".py"))
        && has_ascii_token(&ascii_word_tokens(line), "line")
}

pub(crate) fn ascii_word_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub(crate) fn has_ascii_token(tokens: &[String], needle: &str) -> bool {
    tokens.iter().any(|token| token == needle)
}

pub(crate) fn has_token_sequence(tokens: &[String], sequence: &[&str]) -> bool {
    if sequence.is_empty() || tokens.len() < sequence.len() {
        return false;
    }
    tokens.windows(sequence.len()).any(|window| {
        window
            .iter()
            .zip(sequence)
            .all(|(token, expected)| token == expected)
    })
}

pub(crate) fn has_token_prefix(tokens: &[String], prefix: &[&str]) -> bool {
    if prefix.is_empty() || tokens.len() < prefix.len() {
        return false;
    }
    prefix
        .iter()
        .enumerate()
        .all(|(idx, expected)| tokens.get(idx).is_some_and(|token| token == expected))
}

fn is_zero_failed_summary(tokens: &[String]) -> bool {
    has_token_sequence(tokens, &["0", "failed"])
        || has_token_sequence(tokens, &["0", "tests", "failed"])
}

pub(crate) fn is_warning_signal(line: &str) -> bool {
    classify_log_line(line).warning
}

pub(crate) fn is_failure_signal_line(line: &str) -> bool {
    classify_log_line(line).failure
}

pub(crate) fn is_log_signal(line: &str, terms: &[String]) -> bool {
    let class = classify_log_line(line);
    if class.warning || class.failure {
        return true;
    }
    let line_tokens = ascii_word_tokens(line);
    let term_sequences = terms
        .iter()
        .map(|term| ascii_word_tokens(term))
        .filter(|tokens| !tokens.is_empty())
        .collect::<Vec<_>>();
    term_sequences.iter().any(|sequence| {
        let refs = sequence.iter().map(String::as_str).collect::<Vec<_>>();
        has_token_sequence(&line_tokens, &refs)
    })
}

fn has_leading_char(raw: &str, ch: char) -> bool {
    raw.chars().next() == Some(ch)
}

fn has_repeated_symbol_pair(raw: &str, symbol: char) -> bool {
    let mut prev = None;
    for ch in raw.chars() {
        if prev == Some(symbol) && ch == symbol {
            return true;
        }
        prev = Some(ch);
    }
    false
}

fn has_json_delimiters(raw: &str) -> bool {
    matches!(
        (raw.chars().next(), raw.chars().next_back()),
        (Some('{'), Some('}')) | (Some('['), Some(']'))
    )
}

fn has_path_prefix(raw: &str) -> bool {
    let chars = raw.chars().collect::<Vec<_>>();
    matches!(
        chars.as_slice(),
        ['/', ..] | ['.', '/', ..] | ['.', '.', '/', ..] | ['.', '\\', ..] | ['.', '.', '\\', ..]
    )
}

fn is_http_status_line(line: &str) -> bool {
    line.split_whitespace()
        .next()
        .and_then(|segment| segment.split('/').next())
        == Some("HTTP")
}

fn is_diff_file_header(line: &str) -> bool {
    let tokens = ascii_word_tokens(line);
    has_token_prefix(&tokens, &["diff", "git"])
}

fn is_diff_index_marker(line: &str) -> bool {
    has_token_prefix(&ascii_word_tokens(line), &["index"])
}

fn is_diff_path_marker(line: &str) -> bool {
    let chars = line.chars().collect::<Vec<_>>();
    matches!(chars.as_slice(), ['+', '+', '+', ..] | ['-', '-', '-', ..])
}

fn is_diff_hunk_header(line: &str) -> bool {
    let chars = line.chars().collect::<Vec<_>>();
    matches!(chars.as_slice(), ['@', '@', ..])
}

fn is_diff_stat_totals_line(tokens: &[String]) -> bool {
    has_leading_numeric_token(tokens)
        && (has_ascii_token(tokens, "changed")
            || has_ascii_token(tokens, "insertion")
            || has_ascii_token(tokens, "insertions")
            || has_ascii_token(tokens, "deletion")
            || has_ascii_token(tokens, "deletions"))
}

fn has_leading_numeric_token(tokens: &[String]) -> bool {
    tokens
        .first()
        .is_some_and(|token| token.chars().all(|ch| ch.is_ascii_digit()))
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
    pub(crate) df: Option<DiffSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) gs: Option<GitStatusSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bd: Option<BuildSummary>,
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

#[derive(Clone, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fw: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct DiffSummary {
    pub(crate) f: Vec<DiffFileSummary>,
}

#[derive(Serialize)]
pub(crate) struct DiffFileSummary {
    pub(crate) p: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) add: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) del: usize,
}

#[derive(Serialize)]
pub(crate) struct GitStatusSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) br: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) m: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) a: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) d: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) r: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) u: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) e: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct BuildSummary {
    pub(crate) n: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) cp: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) rn: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) ok: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) fl: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) sk: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) tt: usize,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) ip: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tr: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) e: Vec<String>,
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
