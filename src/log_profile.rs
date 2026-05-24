use crate::trim::{LogSummary, MatchChunk, ProfileLimits, RepeatedRun, is_log_signal, push_chunk};

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
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.contains("warning") {
            warn += 1;
        }
        if lower.contains("error")
            || lower.contains("failed")
            || lower.contains("panic")
            || lower.contains("exception")
            || lower.contains("not ok")
        {
            fail += 1;
        }
    }
    LogSummary { fail, warn }
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
            "repeat:{} count:{} sample:{}",
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
    const MAX: usize = 96;
    if line.len() <= MAX {
        line.to_owned()
    } else {
        format!("{}...", &line[..MAX])
    }
}
