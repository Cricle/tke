use crate::trim::{
    LogSummary, MatchChunk, ProfileLimits, RepeatedRun, is_failure_signal_line, is_log_signal,
    is_warning_signal, push_chunk, truncate_ellipsized,
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
    }
    LogSummary {
        fail,
        warn,
        ff: first_fail,
        fw: first_warn,
    }
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

fn detect_repeated_runs(lines: &[&str]) -> Vec<RepeatedRun> {
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

fn truncate_for_sample(line: &str) -> String {
    const MAX: usize = 72;
    truncate_ellipsized(line, MAX)
}
