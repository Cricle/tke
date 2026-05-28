use crate::trim::{
    ascii_word_tokens, has_ascii_token, has_token_sequence, is_failure_signal_line,
    truncate_ellipsized,
};
use serde::Serialize;

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
    let install_line = has_token_sequence(tokens, &["successfully", "installed"]);

    let explicit_ok = (!install_line)
        .then(|| label_value_count(line, "passed"))
        .flatten();
    let explicit_failures = (!install_line)
        .then(|| label_value_count(line, "failed").or_else(|| label_value_count(line, "failures")))
        .flatten();
    let explicit_errors = (!install_line)
        .then(|| label_value_count(line, "errors"))
        .flatten();
    let explicit_skipped = (!install_line)
        .then(|| label_value_count(line, "skipped").or_else(|| label_value_count(line, "ignored")))
        .flatten();
    let explicit_total = (!install_line && looks_like_test_count_line(tokens))
        .then(|| label_value_count(line, "total").or_else(|| label_value_count(line, "tests run")))
        .flatten();

    if let Some(count) = explicit_ok {
        out.ok = count;
        matched = true;
    }
    if explicit_failures.is_some() || explicit_errors.is_some() {
        out.fl = explicit_failures.unwrap_or(0) + explicit_errors.unwrap_or(0);
        matched = true;
    }
    if let Some(count) = explicit_skipped {
        out.sk = count;
        matched = true;
    }
    if let Some(count) = explicit_total {
        out.tt = count;
        matched = true;
    }

    if explicit_ok.is_none()
        && let Some(count) = count_before_token(tokens, "passed")
            .or_else(|| count_before_sequence(tokens, &["tests", "passed"]))
    {
        out.ok = count;
        matched = true;
    }
    if explicit_failures.is_none()
        && explicit_errors.is_none()
        && let Some(count) = count_before_token(tokens, "failed")
            .or_else(|| count_before_token(tokens, "failures"))
            .or_else(|| count_before_sequence(tokens, &["tests", "failed"]))
    {
        out.fl = count;
        matched = true;
    }
    if explicit_skipped.is_none()
        && let Some(count) =
            count_before_token(tokens, "skipped").or_else(|| count_before_token(tokens, "ignored"))
    {
        out.sk = count;
        matched = true;
    }
    if explicit_total.is_none()
        && let Some(count) = count_after_sequence(tokens, &["tests", "run"])
            .or_else(|| count_after_sequence(tokens, &["ran"]))
            .or_else(|| count_before_sequence(tokens, &["tests", "completed"]))
            .or_else(|| count_before_sequence(tokens, &["test", "completed"]))
            .or_else(|| {
                looks_like_test_count_line(tokens)
                    .then(|| adjacent_count_for_token(tokens, "total"))
                    .flatten()
            })
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

fn looks_like_test_count_line(tokens: &[String]) -> bool {
    has_ascii_token(tokens, "test")
        || has_ascii_token(tokens, "tests")
        || has_ascii_token(tokens, "passed")
        || has_ascii_token(tokens, "failed")
        || has_ascii_token(tokens, "failures")
        || has_ascii_token(tokens, "skipped")
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

fn parse_numeric_token(token: &str) -> Option<usize> {
    token.chars().all(|ch| ch.is_ascii_digit()).then_some(())?;
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

fn label_value_count(line: &str, label: &str) -> Option<usize> {
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
        let rest = rest
            .strip_prefix(':')
            .or_else(|| rest.strip_prefix('='))?
            .trim_start();
        let digits = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        Some(digits.parse::<usize>().ok().unwrap_or(0)).filter(|_| !digits.is_empty())
    })
}
