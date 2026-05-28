use crate::benchmark::RolloutCompareReport;
use crate::rollout_io::{default_stats_roots, discover_rollout_paths, rollout_modified_ms};
use crate::rollout_stats::{
    collect_rollout_output_stats_detailed, rollout_has_relevant_tool_output,
};
use crate::stats_cache::{
    UsageStatsCache, build_usage_stats_report_from_day_cache, load_usage_stats_cache,
    lookup_usage_stats_cache, save_usage_stats_cache, upsert_usage_stats_cache_entry,
};
use crate::trim::ratio;
use crate::{AppError, Config};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct UsageStatsReport {
    v: u8,
    roots: Vec<String>,
    filters: UsageStatsFiltersReport,
    top: usize,
    samples: usize,
    effective_samples: usize,
    changed_samples: usize,
    skipped_samples: usize,
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
    top_profiles: Vec<UsageStatsGroupReport>,
    top_commands: Vec<UsageStatsGroupReport>,
    bottom_profiles: Vec<UsageStatsGroupReport>,
    bottom_commands: Vec<UsageStatsGroupReport>,
}

#[derive(Serialize)]
pub struct UsageStatsDayReport {
    day: String,
    samples: usize,
    effective_samples: usize,
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
    effective_samples: usize,
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
    changed_only: bool,
    sort: String,
}

#[derive(Debug, Clone, Copy)]
pub enum UsageStatsGroupBy {
    Day,
    Profile,
    Command,
    Agent,
}

impl UsageStatsReport {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        roots: Vec<String>,
        top: usize,
        samples: usize,
        effective_samples: usize,
        changed_samples: usize,
        skipped_samples: usize,
        raw_tokens: usize,
        rewritten_tokens: usize,
        tokens_saved: isize,
        raw_bytes: usize,
        rewritten_bytes: usize,
        bytes_saved: isize,
        days: Vec<UsageStatsDayReport>,
        top_profiles: Vec<UsageStatsGroupReport>,
        top_commands: Vec<UsageStatsGroupReport>,
        bottom_profiles: Vec<UsageStatsGroupReport>,
        bottom_commands: Vec<UsageStatsGroupReport>,
    ) -> Self {
        Self {
            v: 1,
            roots,
            filters: UsageStatsFiltersReport {
                kind: "none".to_owned(),
                value: None,
                trend: "day".to_owned(),
                changed_only: false,
                sort: "saved".to_owned(),
            },
            top,
            samples,
            effective_samples,
            changed_samples,
            skipped_samples,
            raw_tokens,
            rewritten_tokens,
            tokens_saved,
            tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
            raw_bytes,
            rewritten_bytes,
            bytes_saved,
            bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
            days,
            groups: Vec::new(),
            top_profiles,
            top_commands,
            bottom_profiles,
            bottom_commands,
        }
    }
}

impl UsageStatsDayReport {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_row(
        day: String,
        samples: usize,
        effective_samples: usize,
        changed_samples: usize,
        tokens_saved: isize,
        raw_tokens: usize,
        bytes_saved: isize,
        raw_bytes: usize,
    ) -> Self {
        Self {
            day,
            samples,
            effective_samples,
            changed_samples,
            tokens_saved,
            tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
            bytes_saved,
            bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
        }
    }
}

impl UsageStatsGroupReport {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_row(
        key: String,
        samples: usize,
        effective_samples: usize,
        changed_samples: usize,
        raw_tokens: usize,
        rewritten_tokens: usize,
        tokens_saved: isize,
        raw_bytes: usize,
        rewritten_bytes: usize,
        bytes_saved: isize,
    ) -> Self {
        Self {
            key,
            samples,
            effective_samples,
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
    }

    pub(crate) fn has_savings(&self) -> bool {
        self.tokens_saved != 0 || self.bytes_saved != 0
    }
}

impl UsageStatsGroupBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Profile => "profile",
            Self::Command => "command",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UsageStatsSortBy {
    Saved,
    Ratio,
    LowRatio,
    Samples,
}

impl UsageStatsSortBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Ratio => "ratio",
            Self::LowRatio => "low-ratio",
            Self::Samples => "samples",
        }
    }
}

#[derive(Debug, Clone)]
pub enum UsageStatsFilter {
    None,
    Profile(String),
    Command(String),
    Agent(String),
}

impl UsageStatsFilter {
    fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Profile(_) => "profile",
            Self::Command(_) => "command",
            Self::Agent(_) => "agent",
        }
    }

    fn value(&self) -> Option<String> {
        match self {
            Self::None => None,
            Self::Profile(value) | Self::Command(value) | Self::Agent(value) => Some(value.clone()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn usage_stats(
    sources: Vec<PathBuf>,
    limit: Option<usize>,
    filter: UsageStatsFilter,
    group_by: UsageStatsGroupBy,
    changed_only: bool,
    refresh: bool,
    top: usize,
    sort_by: UsageStatsSortBy,
    json: bool,
    config: &Config,
) -> Result<(), AppError> {
    let report = build_usage_stats_report(
        sources,
        limit,
        &filter,
        group_by,
        changed_only,
        refresh,
        top,
        sort_by,
        config,
    )?;
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print_usage_stats_report(&report)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn build_usage_stats_report(
    sources: Vec<PathBuf>,
    limit: Option<usize>,
    filter: &UsageStatsFilter,
    group_by: UsageStatsGroupBy,
    changed_only: bool,
    refresh: bool,
    top: usize,
    sort_by: UsageStatsSortBy,
    config: &Config,
) -> Result<UsageStatsReport, AppError> {
    let use_default_roots = sources.is_empty();
    let use_fast_day_cache = use_default_roots
        && !refresh
        && limit.is_none()
        && matches!(filter, UsageStatsFilter::None)
        && matches!(group_by, UsageStatsGroupBy::Day)
        && !changed_only;
    let roots = if use_default_roots {
        default_stats_roots()
    } else {
        sources
    };
    let mut cache = if use_default_roots {
        load_usage_stats_cache()
    } else {
        UsageStatsCache::default()
    };
    if use_fast_day_cache
        && let Some(report) =
            build_usage_stats_report_from_day_cache(&roots, top, sort_by, &mut cache, config)?
    {
        save_usage_stats_cache(&cache);
        return Ok(report);
    }
    let mut rollouts = discover_rollout_paths(
        &roots,
        if use_default_roots {
            Some(&mut cache)
        } else {
            None
        },
    )?;
    rollouts.sort_by_key(|b| std::cmp::Reverse(rollout_modified_ms(b)));
    if let Some(limit) = limit {
        rollouts.truncate(limit);
    }

    let mut reports = Vec::new();
    let mut effective_samples = 0usize;
    let mut changed_samples = 0usize;
    let mut skipped_samples = 0usize;
    let mut raw_tokens = 0usize;
    let mut rewritten_tokens = 0usize;
    let mut raw_bytes = 0usize;
    let mut rewritten_bytes = 0usize;

    for source in &rollouts {
        let meta = fs::metadata(source)?;
        let modified_ms = rollout_modified_ms(source);
        let size = meta.len();
        if let Some(entry) = lookup_usage_stats_cache(&cache.files, source, modified_ms, size) {
            let report = RolloutCompareReport::from_stats(
                source,
                entry.changed,
                entry.raw.clone(),
                entry.rewritten.clone(),
            );
            let report = filter_rollout_report(report, filter);
            if report.raw_fields == 0 || report.raw_bytes == 0 || report.raw_tokens == 0 {
                skipped_samples += 1;
                continue;
            }
            effective_samples += 1;
            if changed_only && !report.changed {
                continue;
            }
            if report.changed {
                changed_samples += 1;
            }
            raw_tokens += report.raw_tokens;
            rewritten_tokens += report.rewritten_tokens;
            raw_bytes += report.raw_bytes;
            rewritten_bytes += report.rewritten_bytes;
            reports.push(report);
            continue;
        }

        let raw = fs::read_to_string(source)?;
        if !rollout_has_relevant_tool_output(&raw) {
            skipped_samples += 1;
            continue;
        }
        let raw_stats = collect_rollout_output_stats_detailed(&raw, config);
        let raw_total = raw_stats.total;
        if raw_total.fields == 0 || raw_total.bytes == 0 || raw_total.approx_tokens == 0 {
            skipped_samples += 1;
            continue;
        }
        let rewritten = crate::adapter::rewrite_agent_transcript(&raw, config)?;
        let rewritten_stats = if let Some(text) = rewritten.as_deref() {
            collect_rollout_output_stats_detailed(text, config)
        } else {
            raw_stats.clone()
        };
        upsert_usage_stats_cache_entry(
            &mut cache.files,
            crate::stats_cache::UsageStatsCacheEntry {
                source: source.display().to_string(),
                modified_ms,
                size,
                raw: raw_stats.clone(),
                rewritten: rewritten_stats.clone(),
                changed: rewritten.is_some(),
            },
        );
        let report = RolloutCompareReport::from_stats(
            source,
            rewritten.is_some(),
            raw_stats,
            rewritten_stats,
        );
        let report = filter_rollout_report(report, filter);
        if report.raw_fields == 0 || report.raw_bytes == 0 || report.raw_tokens == 0 {
            continue;
        }
        effective_samples += 1;
        if changed_only && !report.changed {
            continue;
        }
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
    if use_default_roots {
        save_usage_stats_cache(&cache);
    }
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
            changed_only,
            sort: sort_by.as_str().to_owned(),
        },
        top,
        samples: rollouts.len(),
        effective_samples,
        changed_samples,
        skipped_samples,
        raw_tokens,
        rewritten_tokens,
        tokens_saved,
        tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
        raw_bytes,
        rewritten_bytes,
        bytes_saved,
        bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
        days: daily_usage_rows(&reports),
        groups: usage_group_rows(&reports, filter, group_by, top, sort_by),
        top_profiles: usage_group_rows(&reports, filter, UsageStatsGroupBy::Profile, top, sort_by),
        top_commands: usage_group_rows(&reports, filter, UsageStatsGroupBy::Command, top, sort_by),
        bottom_profiles: usage_group_rows(
            &reports,
            filter,
            UsageStatsGroupBy::Profile,
            top,
            UsageStatsSortBy::LowRatio,
        ),
        bottom_commands: usage_group_rows(
            &reports,
            filter,
            UsageStatsGroupBy::Command,
            top,
            UsageStatsSortBy::LowRatio,
        ),
    })
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

fn format_saved(saved: isize, ratio: f64) -> String {
    format!(
        "{} ({:.1}%)",
        format_number(saved.unsigned_abs()),
        ratio * 100.0
    )
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
            "Filter: kind={} value={} trend={} changed_only={} sort={} top={}",
            report.filters.kind,
            report.filters.value.as_deref().unwrap_or("-"),
            report.filters.trend,
            report.filters.changed_only,
            report.filters.sort,
            report.top
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "Samples: {} total, {} effective, {} changed, {} skipped",
        format_number(report.samples),
        format_number(report.effective_samples),
        format_number(report.changed_samples),
        format_number(report.skipped_samples)
    )?;
    writeln!(out, "Scope: tool-output savings only")?;
    writeln!(out)?;
    writeln!(out, "Summary")?;
    writeln!(
        out,
        "  Tokens: {} -> {}  saved {}",
        format_number(report.raw_tokens),
        format_number(report.rewritten_tokens),
        format_saved(report.tokens_saved, report.tokens_saved_ratio)
    )?;
    writeln!(
        out,
        "  Bytes:  {} -> {}  saved {}",
        format_number(report.raw_bytes),
        format_number(report.rewritten_bytes),
        format_saved(report.bytes_saved, report.bytes_saved_ratio)
    )?;

    if !report.days.is_empty() {
        writeln!(out)?;
        writeln!(
            out,
            "By day:\n  {:<12} {:>8} {:>9} {:>8} {:>14} {:>7} {:>14} {:>7}",
            "Date",
            "Samples",
            "Effective",
            "Changed",
            "Tokens Saved",
            "Ratio",
            "Bytes Saved",
            "Ratio"
        )?;
        for row in &report.days {
            writeln!(
                out,
                "  {:<12} {:>8} {:>9} {:>8} {:>14} {:>6.1}% {:>14} {:>6.1}%",
                row.day,
                row.samples,
                row.effective_samples,
                row.changed_samples,
                format_number(row.tokens_saved.unsigned_abs()),
                row.tokens_saved_ratio * 100.0,
                format_number(row.bytes_saved.unsigned_abs()),
                row.bytes_saved_ratio * 100.0
            )?;
        }
    }
    if !report.groups.is_empty() && report.filters.trend != "day" {
        writeln!(out)?;
        writeln!(
            out,
            "By {}:\n  {:<20} {:>8} {:>9} {:>8} {:>14} {:>7} {:>14} {:>7}",
            report.filters.trend,
            "Key",
            "Samples",
            "Effective",
            "Changed",
            "Tokens Saved",
            "Ratio",
            "Bytes Saved",
            "Ratio"
        )?;
        for row in &report.groups {
            writeln!(
                out,
                "  {:<20} {:>8} {:>9} {:>8} {:>14} {:>6.1}% {:>14} {:>6.1}%",
                row.key,
                row.samples,
                row.effective_samples,
                row.changed_samples,
                format_number(row.tokens_saved.unsigned_abs()),
                row.tokens_saved_ratio * 100.0,
                format_number(row.bytes_saved.unsigned_abs()),
                row.bytes_saved_ratio * 100.0
            )?;
        }
    }
    if report.filters.trend == "day" {
        for (label, groups) in [
            ("Top profiles", &report.top_profiles),
            ("Top commands", &report.top_commands),
            ("Bottom profiles", &report.bottom_profiles),
            ("Bottom commands", &report.bottom_commands),
        ] {
            if groups.is_empty() {
                continue;
            }
            writeln!(out)?;
            writeln!(
                out,
                "{label}:\n  {:<20} {:>8} {:>9} {:>8} {:>14} {:>7}",
                "Name", "Samples", "Effective", "Changed", "Tokens Saved", "Ratio"
            )?;
            for row in groups {
                writeln!(
                    out,
                    "  {:<20} {:>8} {:>9} {:>8} {:>14} {:>6.1}%",
                    row.key,
                    row.samples,
                    row.effective_samples,
                    row.changed_samples,
                    format_number(row.tokens_saved.unsigned_abs()),
                    row.tokens_saved_ratio * 100.0
                )?;
            }
        }
    }
    Ok(())
}

fn daily_usage_rows(reports: &[RolloutCompareReport]) -> Vec<UsageStatsDayReport> {
    let mut grouped = BTreeMap::<String, (usize, usize, usize, usize, usize, usize, usize)>::new();
    for item in reports {
        let day = rollout_day(&item.source);
        let entry = grouped.entry(day).or_insert((0, 0, 0, 0, 0, 0, 0));
        entry.0 += 1;
        entry.1 += 1;
        if item.changed {
            entry.2 += 1;
        }
        entry.3 += item.raw_tokens;
        entry.4 += item.rewritten_tokens;
        entry.5 += item.raw_bytes;
        entry.6 += item.rewritten_bytes;
    }
    grouped
        .into_iter()
        .map(
            |(
                day,
                (
                    samples,
                    effective_samples,
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
                    effective_samples,
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
        UsageStatsFilter::Agent(value) => {
            if detect_agent_from_path(&report.source) == value.as_str() {
                Some((report.raw_detail.total, report.rewritten_detail.total))
            } else {
                let zero = crate::rollout_stats::RolloutOutputStats::default();
                Some((zero, zero))
            }
        }
    }
}

fn detect_agent_from_path(source: &str) -> &'static str {
    let normalized = source.replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').collect();
    let has_component = |name: &str| components.contains(&name);
    if has_component(".codex") || has_component("codex") || normalized.contains("codex-") {
        "codex"
    } else if has_component(".claude") || has_component("claude") || normalized.contains("claude-")
    {
        "claude"
    } else {
        "unknown"
    }
}

fn usage_group_rows(
    reports: &[RolloutCompareReport],
    filter: &UsageStatsFilter,
    group_by: UsageStatsGroupBy,
    top: usize,
    sort_by: UsageStatsSortBy,
) -> Vec<UsageStatsGroupReport> {
    let mut grouped = BTreeMap::<String, (usize, usize, usize, usize, usize, usize, usize)>::new();
    for item in reports {
        let rows = match group_by {
            UsageStatsGroupBy::Day => vec![(
                rollout_day(&item.source),
                item.raw_detail.total,
                item.rewritten_detail.total,
            )],
            UsageStatsGroupBy::Profile => report_record_rows(item, filter, true),
            UsageStatsGroupBy::Command => report_record_rows(item, filter, false),
            UsageStatsGroupBy::Agent => vec![(
                detect_agent_from_path(&item.source).to_owned(),
                item.raw_detail.total,
                item.rewritten_detail.total,
            )],
        };
        for (key, raw, rewritten) in rows {
            let entry = grouped.entry(key).or_insert((0, 0, 0, 0, 0, 0, 0));
            entry.0 += 1;
            if raw.fields > 0 || raw.bytes > 0 || raw.approx_tokens > 0 {
                entry.1 += 1;
            }
            if raw.approx_tokens != rewritten.approx_tokens || raw.bytes != rewritten.bytes {
                entry.2 += 1;
            }
            entry.3 += raw.approx_tokens;
            entry.4 += rewritten.approx_tokens;
            entry.5 += raw.bytes;
            entry.6 += rewritten.bytes;
        }
    }

    let mut rows = grouped
        .into_iter()
        .map(
            |(
                key,
                (
                    samples,
                    effective_samples,
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
                    effective_samples,
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
            if matches!(group_by, UsageStatsGroupBy::Day | UsageStatsGroupBy::Agent) {
                true
            } else {
                row.tokens_saved != 0 || row.bytes_saved != 0
            }
        })
        .collect::<Vec<_>>();
    sort_usage_groups(&mut rows, sort_by);
    if !matches!(group_by, UsageStatsGroupBy::Day | UsageStatsGroupBy::Agent) && rows.len() > top {
        rows.truncate(top);
    }
    rows
}

pub(crate) fn sort_usage_groups(rows: &mut [UsageStatsGroupReport], sort_by: UsageStatsSortBy) {
    rows.sort_by(|a, b| match sort_by {
        UsageStatsSortBy::Saved => b
            .tokens_saved
            .cmp(&a.tokens_saved)
            .then_with(|| b.bytes_saved.cmp(&a.bytes_saved))
            .then_with(|| a.key.cmp(&b.key)),
        UsageStatsSortBy::Ratio => b
            .tokens_saved_ratio
            .total_cmp(&a.tokens_saved_ratio)
            .then_with(|| b.tokens_saved.cmp(&a.tokens_saved))
            .then_with(|| a.key.cmp(&b.key)),
        UsageStatsSortBy::LowRatio => a
            .tokens_saved_ratio
            .total_cmp(&b.tokens_saved_ratio)
            .then_with(|| a.tokens_saved.cmp(&b.tokens_saved))
            .then_with(|| b.samples.cmp(&a.samples))
            .then_with(|| a.key.cmp(&b.key)),
        UsageStatsSortBy::Samples => b
            .samples
            .cmp(&a.samples)
            .then_with(|| b.tokens_saved.cmp(&a.tokens_saved))
            .then_with(|| a.key.cmp(&b.key)),
    });
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
            UsageStatsFilter::None | UsageStatsFilter::Agent(_) => true,
            UsageStatsFilter::Profile(value) => profile == value,
            UsageStatsFilter::Command(value) => command == value,
        })
        .map(|(profile, command, raw, rewritten)| {
            (if by_profile { profile } else { command }, raw, rewritten)
        })
        .collect()
}

pub(crate) fn paired_record_rows_with_meta(
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
