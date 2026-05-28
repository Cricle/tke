use crate::trim::{
    LogSummary, MatchChunk, ProfileLimits, RepeatedRun, detect_structural_templates,
    has_log_progress, is_failure_signal_line, is_log_signal, is_warning_signal, push_chunk,
    truncate_ellipsized,
};

pub(crate) fn collect_log_chunks(
    lines: &[&str],
    terms: &[String],
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();
    let folds = detect_repeated_runs(lines);

    for (idx, line) in lines.iter().enumerate() {
        if !is_log_signal(line, terms) {
            continue;
        }
        if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "signal")
            && out.len() >= limits.max_matches
        {
            break;
        }
    }

    for fold in folds {
        if push_fold_chunk(&mut out, &mut used, &fold) && out.len() >= limits.max_matches {
            break;
        }
    }

    // Structural template detection: group lines sharing a canonical prefix
    if out.len() < limits.max_matches {
        let templates = detect_structural_templates(lines, 3);
        for tmpl in templates {
            let [start, end] = tmpl.r;
            if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
                continue;
            }
            used.push((start, end));
            out.push(tmpl);
            if out.len() >= limits.max_matches {
                break;
            }
        }
    }

    if out.is_empty() {
        let start = lines.len().saturating_sub(limits.tail_lines);
        for idx in start..lines.len() {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "tail")
                && out.len() >= limits.max_matches
            {
                break;
            }
        }
    }
    out
}

pub(crate) fn collect_log_summary(lines: &[&str]) -> LogSummary {
    let mut fail = 0usize;
    let mut warn = 0usize;
    let mut prog = 0usize;
    let mut crates = 0usize;
    let mut elapsed = None;
    let mut first_fail = None;
    let mut first_warn = None;
    for line in lines {
        if is_warning_signal(line) {
            warn += 1;
            if first_warn.is_none() {
                first_warn = Some(truncate_for_sample(line));
            }
        }
        if is_failure_signal_line(line) {
            fail += 1;
            if first_fail.is_none() {
                first_fail = Some(truncate_for_sample(line));
            }
        }
        if has_log_progress(line) {
            prog += 1;
        }
        let trimmed = line.trim();
        if is_compiling_line(trimmed) {
            crates += 1;
        }
        if elapsed.is_none() {
            elapsed = extract_elapsed_time(trimmed);
        }
    }
    LogSummary {
        fail,
        warn,
        ff: first_fail,
        fw: first_warn,
        progress: prog,
        crates,
        elapsed,
    }
}

fn is_compiling_line(line: &str) -> bool {
    let tokens = crate::trim::ascii_word_tokens(line);
    crate::trim::has_ascii_token(&tokens, "compiling")
}

fn extract_elapsed_time(line: &str) -> Option<String> {
    if !line.contains("Finished") && !line.contains("finished") {
        return None;
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if (tok.ends_with('s') || tok.ends_with('m')) && i > 0 && tokens[i - 1].contains("in") {
            return Some((*tok).to_owned());
        }
    }
    None
}

fn push_fold_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    fold: &RepeatedRun,
) -> bool {
    let [start, end] = fold.range;
    if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(MatchChunk {
        k: "fold".to_owned(),
        r: fold.range,
        l: vec![format!(
            "rep:{} c:{} s:{}",
            end.saturating_sub(start),
            fold.count,
            fold.sample
        )],
    });
    true
}

pub(crate) fn detect_repeated_runs(lines: &[&str]) -> Vec<RepeatedRun> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let normalized = canonicalize_log_line(lines[idx]);
        let mut count = 1;
        let mut end = idx + 1;
        while end < lines.len() && canonicalize_log_line(lines[end]) == normalized {
            count += 1;
            end += 1;
        }
        if count >= 3 && !normalized.is_empty() {
            out.push(RepeatedRun {
                range: [idx, end],
                count,
                sample: truncate_for_sample(lines[idx]),
            });
        }
        idx = end;
    }
    out
}

fn canonicalize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_digit = false;
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            prev_digit = false;
            out.push(ch);
        }
    }
    out
}

pub(crate) fn truncate_for_sample(line: &str) -> String {
    const MAX: usize = 72;
    truncate_ellipsized(line, MAX)
}
