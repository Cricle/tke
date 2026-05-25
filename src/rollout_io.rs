use crate::adapter::rewrite_agent_transcript;
use crate::benchmark::RolloutCompareReport;
use crate::rollout_stats::collect_rollout_output_stats_detailed;
use crate::trim::now_millis;
use crate::{AppError, Config};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub fn capture_interactive(
    source: Option<PathBuf>,
    output: Option<PathBuf>,
    config: &Config,
) -> Result<(), AppError> {
    let source = match source {
        Some(path) => path,
        None => {
            let sessions_dir = codex_sessions_dir().ok_or_else(|| {
                AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
            })?;
            find_latest_rollout_after(&sessions_dir, 0)?.ok_or_else(|| {
                AppError::Usage("tke could not find a codex rollout jsonl".to_owned())
            })?
        }
    };
    let output_dir = output.or_else(interactive_output_dir).ok_or_else(|| {
        AppError::Usage("tke could not resolve interactive output dir".to_owned())
    })?;
    rewrite_rollout_to_output(&source, &output_dir, config)?;
    println!("{}", output_dir.display());
    Ok(())
}

pub fn compare_rollout(source: Option<PathBuf>, config: &Config) -> Result<(), AppError> {
    let source = match source {
        Some(path) => path,
        None => {
            let sessions_dir = codex_sessions_dir().ok_or_else(|| {
                AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
            })?;
            find_latest_rollout_after(&sessions_dir, 0)?.ok_or_else(|| {
                AppError::Usage("tke could not find a codex rollout jsonl".to_owned())
            })?
        }
    };

    let raw = fs::read_to_string(&source)?;
    let rewritten = rewrite_agent_transcript(&raw, config)?;
    let raw_stats = collect_rollout_output_stats_detailed(&raw, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
    let rewritten_stats = collect_rollout_output_stats_detailed(rewritten_text, config);
    let report =
        RolloutCompareReport::from_stats(&source, rewritten.is_some(), raw_stats, rewritten_stats);
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

#[derive(Serialize)]
pub struct UsageStatsReport {
    v: u8,
    roots: Vec<String>,
    filters: UsageStatsFiltersReport,
    samples: usize,
    changed_samples: usize,
    raw_tokens: usize,
    rewritten_tokens: usize,
    tokens_saved: isize,
    tokens_saved_ratio: f64,
    raw_bytes: usize,
    rewritten_bytes: usize,
    bytes_saved: isize,
    bytes_saved_ratio: f64,
    days: Vec<UsageStatsDayReport>,
    groups: Vec<UsageStatsGroupReport>,
}

#[derive(Serialize)]
pub struct UsageStatsDayReport {
    day: String,
    samples: usize,
    changed_samples: usize,
    tokens_saved: isize,
    tokens_saved_ratio: f64,
    bytes_saved: isize,
    bytes_saved_ratio: f64,
}

#[derive(Serialize)]
pub struct UsageStatsGroupReport {
    key: String,
    samples: usize,
    changed_samples: usize,
    raw_tokens: usize,
    rewritten_tokens: usize,
    tokens_saved: isize,
    tokens_saved_ratio: f64,
    raw_bytes: usize,
    rewritten_bytes: usize,
    bytes_saved: isize,
    bytes_saved_ratio: f64,
}

#[derive(Serialize)]
pub struct UsageStatsFiltersReport {
    kind: String,
    value: Option<String>,
    trend: String,
}

#[derive(Debug, Clone, Copy)]
pub enum UsageStatsGroupBy {
    Day,
    Profile,
    Command,
}

impl UsageStatsGroupBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Profile => "profile",
            Self::Command => "command",
        }
    }
}

#[derive(Debug, Clone)]
pub enum UsageStatsFilter {
    None,
    Profile(String),
    Command(String),
}

impl UsageStatsFilter {
    fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Profile(_) => "profile",
            Self::Command(_) => "command",
        }
    }

    fn value(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::Profile(value) | Self::Command(value) => Some(value.clone()),
        }
    }
}

pub fn usage_stats(
    sources: Vec<PathBuf>,
    limit: Option<usize>,
    filter: UsageStatsFilter,
    group_by: UsageStatsGroupBy,
    json: bool,
    config: &Config,
) -> Result<(), AppError> {
    let report = build_usage_stats_report(sources, limit, &filter, group_by, config)?;
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print_usage_stats_report(&report)?;
    }
    Ok(())
}

pub fn build_usage_stats_report(
    sources: Vec<PathBuf>,
    limit: Option<usize>,
    filter: &UsageStatsFilter,
    group_by: UsageStatsGroupBy,
    config: &Config,
) -> Result<UsageStatsReport, AppError> {
    let roots = if sources.is_empty() {
        default_stats_roots()
    } else {
        sources
    };
    let mut rollouts = discover_rollout_paths(&roots)?;
    rollouts.sort_by(|a, b| rollout_modified_ms(b).cmp(&rollout_modified_ms(a)));
    if let Some(limit) = limit {
        rollouts.truncate(limit);
    }

    let mut reports = Vec::new();
    let mut changed_samples = 0usize;
    let mut raw_tokens = 0usize;
    let mut rewritten_tokens = 0usize;
    let mut raw_bytes = 0usize;
    let mut rewritten_bytes = 0usize;

    for source in &rollouts {
        let raw = fs::read_to_string(source)?;
        let rewritten = rewrite_agent_transcript(&raw, config)?;
        let raw_stats = collect_rollout_output_stats_detailed(&raw, config);
        let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
        let rewritten_stats = collect_rollout_output_stats_detailed(rewritten_text, config);
        let report = RolloutCompareReport::from_stats(
            source,
            rewritten.is_some(),
            raw_stats,
            rewritten_stats,
        );
        let report = filter_rollout_report(report, filter);
        if report.changed {
            changed_samples += 1;
        }
        raw_tokens += report.raw_tokens;
        rewritten_tokens += report.rewritten_tokens;
        raw_bytes += report.raw_bytes;
        rewritten_bytes += report.rewritten_bytes;
        reports.push(report);
    }

    let tokens_saved = raw_tokens as isize - rewritten_tokens as isize;
    let bytes_saved = raw_bytes as isize - rewritten_bytes as isize;
    Ok(UsageStatsReport {
        v: 1,
        roots: roots
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        filters: UsageStatsFiltersReport {
            kind: filter.kind().to_owned(),
            value: filter.value(),
            trend: group_by.as_str().to_owned(),
        },
        samples: reports.len(),
        changed_samples,
        raw_tokens,
        rewritten_tokens,
        tokens_saved,
        tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
        raw_bytes,
        rewritten_bytes,
        bytes_saved,
        bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
        days: daily_usage_rows(&reports),
        groups: usage_group_rows(&reports, &filter, group_by),
    })
}

pub struct InteractiveTracker {
    pub(crate) sessions_dir: PathBuf,
    pub(crate) started_at_ms: u128,
}

impl InteractiveTracker {
    pub fn start() -> Result<Self, AppError> {
        let sessions_dir = codex_sessions_dir().ok_or_else(|| {
            AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
        })?;
        Ok(Self {
            sessions_dir,
            started_at_ms: now_millis(),
        })
    }

    pub fn finish(&self, config: &Config) -> Result<(), AppError> {
        let Some(latest) = find_latest_rollout_after(&self.sessions_dir, self.started_at_ms)?
        else {
            return Ok(());
        };
        if let Some(dir) = interactive_output_dir() {
            rewrite_rollout_to_output(&latest, &dir, config)?;
        }
        Ok(())
    }
}

fn codex_sessions_dir() -> Option<PathBuf> {
    resolve_codex_home().map(|home| home.join("sessions"))
}

fn resolve_codex_home() -> Option<PathBuf> {
    if let Ok(home) = env::var("CODEX_HOME") {
        return Some(PathBuf::from(home));
    }
    env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".codex"))
}

fn interactive_output_dir() -> Option<PathBuf> {
    env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".tke").join("interactive"))
}

fn default_stats_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(sessions) = codex_sessions_dir() {
        roots.push(sessions);
    }
    if let Some(interactive) = interactive_output_dir() {
        roots.push(interactive);
    }
    roots
}

fn discover_rollout_paths(roots: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
    let mut out = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        if root.is_file() {
            if root.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(root.clone());
            }
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(path) = stack.pop() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn rollout_modified_ms(path: &Path) -> u128 {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn find_latest_rollout_after(dir: &Path, started_at_ms: u128) -> Result<Option<PathBuf>, AppError> {
    if !dir.exists() {
        return Ok(None);
    }

    let mut best: Option<(u128, PathBuf)> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = entry.metadata()?;
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            if modified_ms + 5000 < started_at_ms {
                continue;
            }
            match &best {
                Some((best_ms, _)) if modified_ms <= *best_ms => {}
                _ => best = Some((modified_ms, path)),
            }
        }
    }
    Ok(best.map(|(_, path)| path))
}

fn rewrite_rollout_to_output(
    source: &Path,
    output_dir: &Path,
    config: &Config,
) -> Result<(), AppError> {
    let raw = fs::read_to_string(source)?;
    let Some(rewritten) = rewrite_agent_transcript(&raw, config)? else {
        return Ok(());
    };
    fs::create_dir_all(output_dir)?;
    let output = output_dir.join(
        source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("interactive-rollout.jsonl"),
    );
    fs::write(output, rewritten)?;
    Ok(())
}

fn ratio(saved: isize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        saved as f64 / total as f64
    }
}

fn print_usage_stats_report(report: &UsageStatsReport) -> Result<(), AppError> {
    let mut out = std::io::stdout();
    writeln!(out, "tke usage stats")?;
    writeln!(out)?;
    writeln!(out, "Roots:")?;
    for root in &report.roots {
        writeln!(out, "  - {root}")?;
    }
    if report.filters.kind != "none" || report.filters.trend != "day" {
        writeln!(out)?;
        writeln!(
            out,
            "Filter: kind={} value={} trend={}",
            report.filters.kind,
            report.filters.value.as_deref().unwrap_or("-"),
            report.filters.trend
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "Samples: {} total, {} changed",
        report.samples, report.changed_samples
    )?;
    writeln!(
        out,
        "Tokens: {} -> {}  saved {} ({:.1}%)",
        report.raw_tokens,
        report.rewritten_tokens,
        report.tokens_saved,
        report.tokens_saved_ratio * 100.0
    )?;
    writeln!(
        out,
        "Bytes:  {} -> {}  saved {} ({:.1}%)",
        report.raw_bytes,
        report.rewritten_bytes,
        report.bytes_saved,
        report.bytes_saved_ratio * 100.0
    )?;

    if !report.days.is_empty() {
        writeln!(out)?;
        writeln!(out, "By day:")?;
        for row in &report.days {
            writeln!(
                out,
                "  {}  samples={} changed={} tokens_saved={} ({:.1}%) bytes_saved={} ({:.1}%)",
                row.day,
                row.samples,
                row.changed_samples,
                row.tokens_saved,
                row.tokens_saved_ratio * 100.0,
                row.bytes_saved,
                row.bytes_saved_ratio * 100.0
            )?;
        }
    }
    if !report.groups.is_empty() && report.filters.trend != "day" {
        writeln!(out)?;
        writeln!(out, "By {}:", report.filters.trend)?;
        for row in &report.groups {
            writeln!(
                out,
                "  {}  samples={} changed={} tokens_saved={} ({:.1}%) bytes_saved={} ({:.1}%)",
                row.key,
                row.samples,
                row.changed_samples,
                row.tokens_saved,
                row.tokens_saved_ratio * 100.0,
                row.bytes_saved,
                row.bytes_saved_ratio * 100.0
            )?;
        }
    }
    Ok(())
}

fn daily_usage_rows(reports: &[RolloutCompareReport]) -> Vec<UsageStatsDayReport> {
    let mut grouped = BTreeMap::<String, (usize, usize, usize, usize, usize, usize)>::new();
    for item in reports {
        let day = rollout_day(&item.source);
        let entry = grouped.entry(day).or_insert((0, 0, 0, 0, 0, 0));
        entry.0 += 1;
        if item.changed {
            entry.1 += 1;
        }
        entry.2 += item.raw_tokens;
        entry.3 += item.rewritten_tokens;
        entry.4 += item.raw_bytes;
        entry.5 += item.rewritten_bytes;
    }
    grouped
        .into_iter()
        .map(
            |(
                day,
                (
                    samples,
                    changed_samples,
                    raw_tokens,
                    rewritten_tokens,
                    raw_bytes,
                    rewritten_bytes,
                ),
            )| {
                let tokens_saved = raw_tokens as isize - rewritten_tokens as isize;
                let bytes_saved = raw_bytes as isize - rewritten_bytes as isize;
                UsageStatsDayReport {
                    day,
                    samples,
                    changed_samples,
                    tokens_saved,
                    tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
                    bytes_saved,
                    bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
                }
            },
        )
        .collect::<Vec<_>>()
}

fn rollout_day(source: &str) -> String {
    let normalized = source.replace('\\', "/");
    let parts = normalized.split('/').collect::<Vec<_>>();
    for idx in 0..parts.len() {
        if parts[idx] == "sessions" && idx + 3 < parts.len() {
            return format!("{}-{}-{}", parts[idx + 1], parts[idx + 2], parts[idx + 3]);
        }
    }
    "unknown".to_owned()
}

fn filter_rollout_report(
    mut report: RolloutCompareReport,
    filter: &UsageStatsFilter,
) -> RolloutCompareReport {
    let Some((raw, rewritten)) = select_breakdown_pair(&report, filter) else {
        return report;
    };
    let bytes_saved = raw.bytes as isize - rewritten.bytes as isize;
    let tokens_saved = raw.approx_tokens as isize - rewritten.approx_tokens as isize;
    report.changed = tokens_saved != 0 || bytes_saved != 0;
    report.raw_fields = raw.fields;
    report.raw_bytes = raw.bytes;
    report.raw_tokens = raw.approx_tokens;
    report.rewritten_fields = rewritten.fields;
    report.rewritten_bytes = rewritten.bytes;
    report.rewritten_tokens = rewritten.approx_tokens;
    report.bytes_saved = bytes_saved;
    report.tokens_saved = tokens_saved;
    report.bytes_saved_ratio = ratio(bytes_saved, raw.bytes);
    report.tokens_saved_ratio = ratio(tokens_saved, raw.approx_tokens);
    report
}

fn select_breakdown_pair(
    report: &RolloutCompareReport,
    filter: &UsageStatsFilter,
) -> Option<(
    crate::rollout_stats::RolloutOutputStats,
    crate::rollout_stats::RolloutOutputStats,
)> {
    match filter {
        UsageStatsFilter::None => Some((report.raw_detail.total, report.rewritten_detail.total)),
        UsageStatsFilter::Profile(value) => {
            Some(filtered_pair_stats(report, |profile, _| profile == value))
        }
        UsageStatsFilter::Command(value) => {
            Some(filtered_pair_stats(report, |_, command| command == value))
        }
    }
}

fn usage_group_rows(
    reports: &[RolloutCompareReport],
    filter: &UsageStatsFilter,
    group_by: UsageStatsGroupBy,
) -> Vec<UsageStatsGroupReport> {
    let mut grouped = BTreeMap::<String, (usize, usize, usize, usize, usize, usize)>::new();
    for item in reports {
        let rows = match group_by {
            UsageStatsGroupBy::Day => vec![(
                rollout_day(&item.source),
                item.raw_detail.total,
                item.rewritten_detail.total,
            )],
            UsageStatsGroupBy::Profile => report_record_rows(item, filter, true),
            UsageStatsGroupBy::Command => report_record_rows(item, filter, false),
        };
        for (key, raw, rewritten) in rows {
            let entry = grouped.entry(key).or_insert((0, 0, 0, 0, 0, 0));
            entry.0 += 1;
            if raw.approx_tokens != rewritten.approx_tokens || raw.bytes != rewritten.bytes {
                entry.1 += 1;
            }
            entry.2 += raw.approx_tokens;
            entry.3 += rewritten.approx_tokens;
            entry.4 += raw.bytes;
            entry.5 += rewritten.bytes;
        }
    }

    grouped
        .into_iter()
        .map(
            |(
                key,
                (
                    samples,
                    changed_samples,
                    raw_tokens,
                    rewritten_tokens,
                    raw_bytes,
                    rewritten_bytes,
                ),
            )| {
                let tokens_saved = raw_tokens as isize - rewritten_tokens as isize;
                let bytes_saved = raw_bytes as isize - rewritten_bytes as isize;
                UsageStatsGroupReport {
                    key,
                    samples,
                    changed_samples,
                    raw_tokens,
                    rewritten_tokens,
                    tokens_saved,
                    tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
                    raw_bytes,
                    rewritten_bytes,
                    bytes_saved,
                    bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
                }
            },
        )
        .filter(|row| {
            if matches!(group_by, UsageStatsGroupBy::Day) {
                true
            } else {
                row.tokens_saved != 0 || row.bytes_saved != 0
            }
        })
        .collect()
}

fn report_record_rows(
    report: &RolloutCompareReport,
    filter: &UsageStatsFilter,
    by_profile: bool,
) -> Vec<(
    String,
    crate::rollout_stats::RolloutOutputStats,
    crate::rollout_stats::RolloutOutputStats,
)> {
    let mut raw = BTreeMap::<String, crate::rollout_stats::RolloutOutputStats>::new();
    let mut rewritten = BTreeMap::<String, crate::rollout_stats::RolloutOutputStats>::new();
    for (key, raw_stats, rewritten_stats) in paired_record_rows(report, filter, by_profile) {
        let raw_entry = raw.entry(key.clone()).or_default();
        raw_entry.fields += raw_stats.fields;
        raw_entry.bytes += raw_stats.bytes;
        raw_entry.approx_tokens += raw_stats.approx_tokens;

        let rewritten_entry = rewritten.entry(key).or_default();
        rewritten_entry.fields += rewritten_stats.fields;
        rewritten_entry.bytes += rewritten_stats.bytes;
        rewritten_entry.approx_tokens += rewritten_stats.approx_tokens;
    }
    let mut keys = raw.keys().cloned().collect::<Vec<_>>();
    for key in rewritten.keys() {
        if !keys.iter().any(|existing| existing == key) {
            keys.push(key.clone());
        }
    }
    if keys.is_empty() {
        keys.push("unknown".to_owned());
    }
    keys.into_iter()
        .map(|key: String| {
            (
                key.clone(),
                raw.get(&key).copied().unwrap_or_default(),
                rewritten.get(&key).copied().unwrap_or_default(),
            )
        })
        .collect()
}

fn filtered_pair_stats(
    report: &RolloutCompareReport,
    include: impl Fn(&str, &str) -> bool,
) -> (
    crate::rollout_stats::RolloutOutputStats,
    crate::rollout_stats::RolloutOutputStats,
) {
    let mut raw_total = crate::rollout_stats::RolloutOutputStats::default();
    let mut rewritten_total = crate::rollout_stats::RolloutOutputStats::default();
    for (profile, command, raw, rewritten) in paired_record_rows_with_meta(report) {
        if !include(&profile, &command) {
            continue;
        }
        raw_total.fields += raw.fields;
        raw_total.bytes += raw.bytes;
        raw_total.approx_tokens += raw.approx_tokens;
        rewritten_total.fields += rewritten.fields;
        rewritten_total.bytes += rewritten.bytes;
        rewritten_total.approx_tokens += rewritten.approx_tokens;
    }
    (raw_total, rewritten_total)
}

fn paired_record_rows(
    report: &RolloutCompareReport,
    filter: &UsageStatsFilter,
    by_profile: bool,
) -> Vec<(
    String,
    crate::rollout_stats::RolloutOutputStats,
    crate::rollout_stats::RolloutOutputStats,
)> {
    paired_record_rows_with_meta(report)
        .into_iter()
        .filter(|(profile, command, _, _)| match filter {
            UsageStatsFilter::None => true,
            UsageStatsFilter::Profile(value) => profile == value,
            UsageStatsFilter::Command(value) => command == value,
        })
        .map(|(profile, command, raw, rewritten)| {
            (if by_profile { profile } else { command }, raw, rewritten)
        })
        .collect()
}

fn paired_record_rows_with_meta(
    report: &RolloutCompareReport,
) -> Vec<(
    String,
    String,
    crate::rollout_stats::RolloutOutputStats,
    crate::rollout_stats::RolloutOutputStats,
)> {
    let mut rows = Vec::new();
    let count = usize::max(
        report.raw_detail.records.len(),
        report.rewritten_detail.records.len(),
    );
    for idx in 0..count {
        let raw = report.raw_detail.records.get(idx);
        let rewritten = report.rewritten_detail.records.get(idx);
        let selector = rewritten.or(raw);
        let Some(selector) = selector else {
            continue;
        };
        rows.push((
            selector.profile.clone(),
            selector.command.clone(),
            raw.map(|record| record.stats).unwrap_or_default(),
            rewritten.map(|record| record.stats).unwrap_or_default(),
        ));
    }
    rows
}
