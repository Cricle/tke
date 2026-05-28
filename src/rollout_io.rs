use crate::adapter::rewrite_agent_transcript;
use crate::benchmark::RolloutCompareReport;
use crate::rollout_stats::{
    collect_rollout_output_stats_detailed, rollout_has_relevant_tool_output,
};
use crate::trim::now_millis;
use crate::{AppError, Config};
use serde::{Deserialize, Serialize};
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
        None => find_latest_any_rollout()?.ok_or_else(|| {
            AppError::Usage("tke could not find any agent rollout jsonl".to_owned())
        })?,
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
        None => find_latest_any_rollout()?.ok_or_else(|| {
            AppError::Usage("tke could not find any agent rollout jsonl".to_owned())
        })?,
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

#[derive(Clone, Serialize, Deserialize)]
struct UsageStatsCacheEntry {
    source: String,
    modified_ms: u128,
    size: u64,
    raw: crate::rollout_stats::RolloutOutputStatsDetailed,
    rewritten: crate::rollout_stats::RolloutOutputStatsDetailed,
    changed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct UsageStatsDirCacheEntry {
    path: String,
    modified_ms: u128,
    files: Vec<String>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct UsageStatsCache {
    v: u8,
    files: Vec<UsageStatsCacheEntry>,
    dirs: Vec<UsageStatsDirCacheEntry>,
    days: Vec<UsageStatsDayCacheEntry>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct UsageStatsAggregateCounter {
    samples: usize,
    effective_samples: usize,
    changed_samples: usize,
    raw_tokens: usize,
    rewritten_tokens: usize,
    raw_bytes: usize,
    rewritten_bytes: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct UsageStatsDayCacheEntry {
    path: String,
    day: String,
    modified_ms: u128,
    file_count: usize,
    total_size: u64,
    max_file_modified_ms: u128,
    summary: UsageStatsAggregateCounter,
    profiles: BTreeMap<String, UsageStatsAggregateCounter>,
    commands: BTreeMap<String, UsageStatsAggregateCounter>,
}

#[derive(Clone, Copy, Default)]
struct UsageStatsDaySignature {
    file_count: usize,
    total_size: u64,
    max_file_modified_ms: u128,
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
        let rewritten = rewrite_agent_transcript(&raw, config)?;
        let rewritten_stats = if let Some(text) = rewritten.as_deref() {
            collect_rollout_output_stats_detailed(text, config)
        } else {
            raw_stats.clone()
        };
        upsert_usage_stats_cache_entry(
            &mut cache.files,
            UsageStatsCacheEntry {
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

fn usage_stats_cache_path() -> Option<PathBuf> {
    env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".tke").join("stats-cache.json"))
}

fn load_usage_stats_cache() -> UsageStatsCache {
    let Some(path) = usage_stats_cache_path() else {
        return UsageStatsCache::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return UsageStatsCache::default();
    };
    if let Ok(cache) = serde_json::from_str::<UsageStatsCache>(&raw) {
        return cache;
    }
    if let Ok(files) = serde_json::from_str::<Vec<UsageStatsCacheEntry>>(&raw) {
        return UsageStatsCache {
            v: 2,
            files,
            dirs: Vec::new(),
            days: Vec::new(),
        };
    }
    UsageStatsCache::default()
}

fn save_usage_stats_cache(cache: &UsageStatsCache) {
    let Some(path) = usage_stats_cache_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(raw) = serde_json::to_string(cache) else {
        return;
    };
    let _ = fs::write(path, raw);
}

fn build_usage_stats_report_from_day_cache(
    roots: &[PathBuf],
    top: usize,
    sort_by: UsageStatsSortBy,
    cache: &mut UsageStatsCache,
    config: &Config,
) -> Result<Option<UsageStatsReport>, AppError> {
    let mut reports = Vec::<RolloutCompareReport>::new();
    let mut total = UsageStatsAggregateCounter::default();
    let mut total_profiles = BTreeMap::<String, UsageStatsAggregateCounter>::new();
    let mut total_commands = BTreeMap::<String, UsageStatsAggregateCounter>::new();
    let mut roots_rendered = Vec::new();
    let mut any_day = false;

    for root in roots {
        roots_rendered.push(root.display().to_string());
        if !root.exists() {
            continue;
        }
        for day_dir in discover_day_dirs(root)? {
            any_day = true;
            let day_files = discover_rollout_paths_in_dir(&day_dir, cache)?;
            let signature = usage_stats_day_signature(&day_files)?;
            if let Some(entry) = lookup_usage_stats_day_cache(&cache.days, &day_dir, signature) {
                merge_counter(&mut total, &entry.summary);
                merge_group_maps(&mut total_profiles, &entry.profiles);
                merge_group_maps(&mut total_commands, &entry.commands);
                reports.push(day_cache_entry_to_report(entry));
                continue;
            }

            let day = rollout_day(&day_dir.display().to_string());
            let mut day_summary = UsageStatsAggregateCounter::default();
            let mut profile_map = BTreeMap::<String, UsageStatsAggregateCounter>::new();
            let mut command_map = BTreeMap::<String, UsageStatsAggregateCounter>::new();
            let mut day_reports = Vec::new();
            for source in day_files {
                day_summary.samples += 1;
                let meta = fs::metadata(&source)?;
                let source_modified_ms = rollout_modified_ms(&source);
                let size = meta.len();
                let report = if let Some(entry) =
                    lookup_usage_stats_cache(&cache.files, &source, source_modified_ms, size)
                {
                    RolloutCompareReport::from_stats(
                        &source,
                        entry.changed,
                        entry.raw.clone(),
                        entry.rewritten.clone(),
                    )
                } else {
                    let raw = fs::read_to_string(&source)?;
                    if !rollout_has_relevant_tool_output(&raw) {
                        continue;
                    }
                    let raw_stats = collect_rollout_output_stats_detailed(&raw, config);
                    if raw_stats.total.fields == 0
                        || raw_stats.total.bytes == 0
                        || raw_stats.total.approx_tokens == 0
                    {
                        continue;
                    }
                    let rewritten = rewrite_agent_transcript(&raw, config)?;
                    let rewritten_stats = if let Some(text) = rewritten.as_deref() {
                        collect_rollout_output_stats_detailed(text, config)
                    } else {
                        raw_stats.clone()
                    };
                    upsert_usage_stats_cache_entry(
                        &mut cache.files,
                        UsageStatsCacheEntry {
                            source: source.display().to_string(),
                            modified_ms: source_modified_ms,
                            size,
                            raw: raw_stats.clone(),
                            rewritten: rewritten_stats.clone(),
                            changed: rewritten.is_some(),
                        },
                    );
                    RolloutCompareReport::from_stats(
                        &source,
                        rewritten.is_some(),
                        raw_stats,
                        rewritten_stats,
                    )
                };

                if report.raw_fields == 0 || report.raw_bytes == 0 || report.raw_tokens == 0 {
                    continue;
                }
                merge_counter(&mut day_summary, &counter_from_report(&report));
                merge_breakdown_maps(&mut profile_map, &mut command_map, &report);
                day_reports.push(report);
            }

            merge_counter(&mut total, &day_summary);
            merge_group_maps(&mut total_profiles, &profile_map);
            merge_group_maps(&mut total_commands, &command_map);
            upsert_usage_stats_day_cache(
                &mut cache.days,
                UsageStatsDayCacheEntry {
                    path: day_dir.display().to_string(),
                    day,
                    modified_ms: rollout_modified_ms(&day_dir),
                    file_count: signature.file_count,
                    total_size: signature.total_size,
                    max_file_modified_ms: signature.max_file_modified_ms,
                    summary: day_summary,
                    profiles: profile_map,
                    commands: command_map,
                },
            );
            reports.push(day_reports_to_day_report(&day_reports));
        }
    }

    if !any_day {
        return Ok(None);
    }

    let raw_tokens = total.raw_tokens;
    let rewritten_tokens = total.rewritten_tokens;
    let raw_bytes = total.raw_bytes;
    let rewritten_bytes = total.rewritten_bytes;
    let tokens_saved = raw_tokens as isize - rewritten_tokens as isize;
    let bytes_saved = raw_bytes as isize - rewritten_bytes as isize;
    let skipped_samples = total.samples.saturating_sub(total.effective_samples);
    let day_rows = daily_usage_rows(&reports);
    let top_profiles = aggregate_group_rows(&total_profiles, top, sort_by);
    let top_commands = aggregate_group_rows(&total_commands, top, sort_by);
    let bottom_profiles = aggregate_group_rows(&total_profiles, top, UsageStatsSortBy::LowRatio);
    let bottom_commands = aggregate_group_rows(&total_commands, top, UsageStatsSortBy::LowRatio);
    Ok(Some(UsageStatsReport {
        v: 1,
        roots: roots_rendered,
        filters: UsageStatsFiltersReport {
            kind: "none".to_owned(),
            value: None,
            trend: "day".to_owned(),
            changed_only: false,
            sort: sort_by.as_str().to_owned(),
        },
        top,
        samples: total.samples,
        effective_samples: total.effective_samples,
        changed_samples: total.changed_samples,
        skipped_samples,
        raw_tokens,
        rewritten_tokens,
        tokens_saved,
        tokens_saved_ratio: ratio(tokens_saved, raw_tokens),
        raw_bytes,
        rewritten_bytes,
        bytes_saved,
        bytes_saved_ratio: ratio(bytes_saved, raw_bytes),
        days: day_rows,
        groups: Vec::new(),
        top_profiles,
        top_commands,
        bottom_profiles,
        bottom_commands,
    }))
}

fn discover_day_dirs(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if looks_like_session_day_dir(&path) {
            out.push(path);
            continue;
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn looks_like_session_day_dir(path: &Path) -> bool {
    let Some(day) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    let Some(month) = path
        .parent()
        .and_then(|v| v.file_name())
        .and_then(|v| v.to_str())
    else {
        return false;
    };
    let Some(year) = path
        .parent()
        .and_then(|v| v.parent())
        .and_then(|v| v.file_name())
        .and_then(|v| v.to_str())
    else {
        return false;
    };
    year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
        && day.chars().all(|ch| ch.is_ascii_digit())
}

fn counter_from_report(report: &RolloutCompareReport) -> UsageStatsAggregateCounter {
    UsageStatsAggregateCounter {
        samples: 0,
        effective_samples: 1,
        changed_samples: usize::from(report.changed),
        raw_tokens: report.raw_tokens,
        rewritten_tokens: report.rewritten_tokens,
        raw_bytes: report.raw_bytes,
        rewritten_bytes: report.rewritten_bytes,
    }
}

fn merge_counter(dst: &mut UsageStatsAggregateCounter, src: &UsageStatsAggregateCounter) {
    dst.samples += src.samples;
    dst.effective_samples += src.effective_samples;
    dst.changed_samples += src.changed_samples;
    dst.raw_tokens += src.raw_tokens;
    dst.rewritten_tokens += src.rewritten_tokens;
    dst.raw_bytes += src.raw_bytes;
    dst.rewritten_bytes += src.rewritten_bytes;
}

fn merge_group_maps(
    dst: &mut BTreeMap<String, UsageStatsAggregateCounter>,
    src: &BTreeMap<String, UsageStatsAggregateCounter>,
) {
    for (key, value) in src {
        let entry = dst.entry(key.clone()).or_default();
        merge_counter(entry, value);
    }
}

fn merge_breakdown_maps(
    profiles: &mut BTreeMap<String, UsageStatsAggregateCounter>,
    commands: &mut BTreeMap<String, UsageStatsAggregateCounter>,
    report: &RolloutCompareReport,
) {
    for (profile, command, raw, rewritten) in paired_record_rows_with_meta(report) {
        let profile_entry = profiles.entry(profile).or_default();
        profile_entry.samples += 1;
        profile_entry.effective_samples += 1;
        profile_entry.changed_samples += usize::from(
            raw.approx_tokens != rewritten.approx_tokens || raw.bytes != rewritten.bytes,
        );
        profile_entry.raw_tokens += raw.approx_tokens;
        profile_entry.rewritten_tokens += rewritten.approx_tokens;
        profile_entry.raw_bytes += raw.bytes;
        profile_entry.rewritten_bytes += rewritten.bytes;

        let command_entry = commands.entry(command).or_default();
        command_entry.samples += 1;
        command_entry.effective_samples += 1;
        command_entry.changed_samples += usize::from(
            raw.approx_tokens != rewritten.approx_tokens || raw.bytes != rewritten.bytes,
        );
        command_entry.raw_tokens += raw.approx_tokens;
        command_entry.rewritten_tokens += rewritten.approx_tokens;
        command_entry.raw_bytes += raw.bytes;
        command_entry.rewritten_bytes += rewritten.bytes;
    }
}

fn day_reports_to_day_report(reports: &[RolloutCompareReport]) -> RolloutCompareReport {
    let source = reports
        .first()
        .map(|report| report.source.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let mut changed = false;
    let mut raw_detail = crate::rollout_stats::RolloutOutputStatsDetailed::default();
    let mut rewritten_detail = crate::rollout_stats::RolloutOutputStatsDetailed::default();
    for report in reports {
        changed |= report.changed;
        raw_detail.total.fields += report.raw_fields;
        raw_detail.total.bytes += report.raw_bytes;
        raw_detail.total.approx_tokens += report.raw_tokens;
        rewritten_detail.total.fields += report.rewritten_fields;
        rewritten_detail.total.bytes += report.rewritten_bytes;
        rewritten_detail.total.approx_tokens += report.rewritten_tokens;
        raw_detail.records.extend(report.raw_detail.records.clone());
        rewritten_detail
            .records
            .extend(report.rewritten_detail.records.clone());
    }
    RolloutCompareReport::from_stats(Path::new(&source), changed, raw_detail, rewritten_detail)
}

fn day_cache_entry_to_report(entry: &UsageStatsDayCacheEntry) -> RolloutCompareReport {
    let mut raw_detail = crate::rollout_stats::RolloutOutputStatsDetailed::default();
    let mut rewritten_detail = crate::rollout_stats::RolloutOutputStatsDetailed::default();
    raw_detail.total.fields = entry.summary.effective_samples;
    raw_detail.total.bytes = entry.summary.raw_bytes;
    raw_detail.total.approx_tokens = entry.summary.raw_tokens;
    rewritten_detail.total.fields = entry.summary.effective_samples;
    rewritten_detail.total.bytes = entry.summary.rewritten_bytes;
    rewritten_detail.total.approx_tokens = entry.summary.rewritten_tokens;

    for (key, summary) in &entry.profiles {
        raw_detail.breakdown.by_profile.insert(
            key.clone(),
            crate::rollout_stats::RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.raw_bytes,
                approx_tokens: summary.raw_tokens,
            },
        );
        rewritten_detail.breakdown.by_profile.insert(
            key.clone(),
            crate::rollout_stats::RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.rewritten_bytes,
                approx_tokens: summary.rewritten_tokens,
            },
        );
    }
    for (key, summary) in &entry.commands {
        raw_detail.breakdown.by_command.insert(
            key.clone(),
            crate::rollout_stats::RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.raw_bytes,
                approx_tokens: summary.raw_tokens,
            },
        );
        rewritten_detail.breakdown.by_command.insert(
            key.clone(),
            crate::rollout_stats::RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.rewritten_bytes,
                approx_tokens: summary.rewritten_tokens,
            },
        );
    }

    RolloutCompareReport::from_stats(
        Path::new(&entry.path),
        entry.summary.changed_samples > 0,
        raw_detail,
        rewritten_detail,
    )
}

fn lookup_usage_stats_day_cache<'a>(
    cache: &'a [UsageStatsDayCacheEntry],
    dir: &Path,
    signature: UsageStatsDaySignature,
) -> Option<&'a UsageStatsDayCacheEntry> {
    let dir = dir.display().to_string();
    cache.iter().find(|entry| {
        entry.path == dir
            && entry.file_count == signature.file_count
            && entry.total_size == signature.total_size
            && entry.max_file_modified_ms == signature.max_file_modified_ms
    })
}

fn upsert_usage_stats_day_cache(
    cache: &mut Vec<UsageStatsDayCacheEntry>,
    entry: UsageStatsDayCacheEntry,
) {
    if let Some(existing) = cache.iter_mut().find(|item| item.path == entry.path) {
        *existing = entry;
    } else {
        cache.push(entry);
    }
}

fn usage_stats_day_signature(files: &[PathBuf]) -> Result<UsageStatsDaySignature, AppError> {
    let mut signature = UsageStatsDaySignature {
        file_count: files.len(),
        ..UsageStatsDaySignature::default()
    };
    for path in files {
        let meta = fs::metadata(path)?;
        signature.total_size += meta.len();
        signature.max_file_modified_ms = signature
            .max_file_modified_ms
            .max(rollout_modified_ms(path));
    }
    Ok(signature)
}

fn aggregate_group_rows(
    grouped: &BTreeMap<String, UsageStatsAggregateCounter>,
    top: usize,
    sort_by: UsageStatsSortBy,
) -> Vec<UsageStatsGroupReport> {
    let mut rows = grouped
        .iter()
        .map(|(key, value)| {
            let tokens_saved = value.raw_tokens as isize - value.rewritten_tokens as isize;
            let bytes_saved = value.raw_bytes as isize - value.rewritten_bytes as isize;
            UsageStatsGroupReport {
                key: key.clone(),
                samples: value.samples,
                effective_samples: value.effective_samples,
                changed_samples: value.changed_samples,
                raw_tokens: value.raw_tokens,
                rewritten_tokens: value.rewritten_tokens,
                tokens_saved,
                tokens_saved_ratio: ratio(tokens_saved, value.raw_tokens),
                raw_bytes: value.raw_bytes,
                rewritten_bytes: value.rewritten_bytes,
                bytes_saved,
                bytes_saved_ratio: ratio(bytes_saved, value.raw_bytes),
            }
        })
        .filter(|row| row.tokens_saved != 0 || row.bytes_saved != 0)
        .collect::<Vec<_>>();
    sort_usage_groups(&mut rows, sort_by);
    if rows.len() > top {
        rows.truncate(top);
    }
    rows
}

fn upsert_usage_stats_cache_entry(
    cache: &mut Vec<UsageStatsCacheEntry>,
    entry: UsageStatsCacheEntry,
) {
    if let Some(existing) = cache.iter_mut().find(|item| item.source == entry.source) {
        *existing = entry;
    } else {
        cache.push(entry);
    }
}

fn lookup_usage_stats_cache<'a>(
    cache: &'a [UsageStatsCacheEntry],
    source: &Path,
    modified_ms: u128,
    size: u64,
) -> Option<&'a UsageStatsCacheEntry> {
    let source = source.display().to_string();
    cache.iter().find(|entry| {
        entry.source == source && entry.modified_ms == modified_ms && entry.size == size
    })
}

pub struct InteractiveTracker {
    pub(crate) sessions_dir: PathBuf,
    pub(crate) started_at_ms: u128,
    pub(crate) agent: &'static str,
}

impl InteractiveTracker {
    pub fn start_for_agent(name: &str) -> Option<Self> {
        match name {
            "codex" => {
                let sessions_dir = codex_sessions_dir()?;
                Some(Self {
                    sessions_dir,
                    started_at_ms: now_millis(),
                    agent: "codex",
                })
            }
            "claude" => {
                let sessions_dir = claude_sessions_dir()?;
                Some(Self {
                    sessions_dir,
                    started_at_ms: now_millis(),
                    agent: "claude",
                })
            }
            _ => None,
        }
    }

    pub fn finish(&self, config: &Config) -> Result<(), AppError> {
        let latest = if self.agent == "claude" {
            find_latest_claude_rollout_after(&self.sessions_dir, self.started_at_ms)?
        } else {
            find_latest_rollout_after(&self.sessions_dir, self.started_at_ms)?
        };
        let Some(latest) = latest else {
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

fn claude_sessions_dir() -> Option<PathBuf> {
    resolve_claude_home().map(|home| home.join("sessions"))
}

fn resolve_claude_home() -> Option<PathBuf> {
    if let Ok(home) = env::var("CLAUDE_HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        if let Ok(home) = env::var("USERPROFILE") {
            return Some(PathBuf::from(home).join(".claude"));
        }
        if let (Ok(drive), Ok(path)) = (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
            return Some(PathBuf::from(format!("{drive}{path}")).join(".claude"));
        }
    }
    env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".claude"))
}

fn resolve_codex_home() -> Option<PathBuf> {
    if let Ok(home) = env::var("CODEX_HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        if let Ok(home) = env::var("USERPROFILE") {
            return Some(PathBuf::from(home).join(".codex"));
        }
        if let (Ok(drive), Ok(path)) = (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
            return Some(PathBuf::from(format!("{drive}{path}")).join(".codex"));
        }
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
    if let Some(claude_home) = resolve_claude_home() {
        roots.push(claude_home.join("projects"));
    }
    if let Some(interactive) = interactive_output_dir() {
        roots.push(interactive);
    }
    roots
}

fn find_latest_any_rollout() -> Result<Option<PathBuf>, AppError> {
    let mut best: Option<(u128, PathBuf)> = None;

    if let Some(sessions_dir) = codex_sessions_dir()
        && let Some(path) = find_latest_rollout_after(&sessions_dir, 0)?
    {
        let ms = rollout_modified_ms(&path);
        match &best {
            Some((best_ms, _)) if ms <= *best_ms => {}
            _ => best = Some((ms, path)),
        }
    }

    if let Some(sessions_dir) = claude_sessions_dir()
        && let Some(path) = find_latest_claude_rollout_after(&sessions_dir, 0)?
    {
        let ms = rollout_modified_ms(&path);
        match &best {
            Some((best_ms, _)) if ms <= *best_ms => {}
            _ => best = Some((ms, path)),
        }
    }

    Ok(best.map(|(_, path)| path))
}

fn discover_rollout_paths(
    roots: &[PathBuf],
    mut cache: Option<&mut UsageStatsCache>,
) -> Result<Vec<PathBuf>, AppError> {
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
        if let Some(cache) = cache.as_deref_mut() {
            out.extend(discover_rollout_paths_in_dir(root, cache)?);
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

fn discover_rollout_paths_in_dir(
    dir: &Path,
    cache: &mut UsageStatsCache,
) -> Result<Vec<PathBuf>, AppError> {
    let modified_ms = rollout_modified_ms(dir);
    if let Some(entry) = lookup_usage_stats_dir_cache(&cache.dirs, dir, modified_ms) {
        return Ok(entry.files.iter().map(PathBuf::from).collect());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(discover_rollout_paths_in_dir(&path, cache)?);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    upsert_usage_stats_dir_cache(
        &mut cache.dirs,
        UsageStatsDirCacheEntry {
            path: dir.display().to_string(),
            modified_ms,
            files: files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
        },
    );
    Ok(files)
}

fn lookup_usage_stats_dir_cache<'a>(
    cache: &'a [UsageStatsDirCacheEntry],
    dir: &Path,
    modified_ms: u128,
) -> Option<&'a UsageStatsDirCacheEntry> {
    let dir = dir.display().to_string();
    cache
        .iter()
        .find(|entry| entry.path == dir && entry.modified_ms == modified_ms)
}

fn upsert_usage_stats_dir_cache(
    cache: &mut Vec<UsageStatsDirCacheEntry>,
    entry: UsageStatsDirCacheEntry,
) {
    if let Some(existing) = cache.iter_mut().find(|item| item.path == entry.path) {
        *existing = entry;
    } else {
        cache.push(entry);
    }
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

fn find_latest_claude_rollout_after(
    sessions_dir: &Path,
    started_at_ms: u128,
) -> Result<Option<PathBuf>, AppError> {
    if !sessions_dir.exists() {
        return Ok(None);
    }

    let claude_home = sessions_dir
        .parent()
        .ok_or_else(|| AppError::Usage("cannot resolve claude home".to_owned()))?;
    let projects_dir = claude_home.join("projects");

    let mut best: Option<(u128, PathBuf)> = None;
    for entry in fs::read_dir(sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let meta: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let session_started = meta.get("startedAt").and_then(|v| v.as_u64()).unwrap_or(0) as u128;
        if session_started + 5000 < started_at_ms {
            continue;
        }
        let Some(session_id) = meta.get("sessionId").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(cwd) = meta.get("cwd").and_then(|v| v.as_str()) else {
            continue;
        };
        let encoded = claude_encode_project_path(cwd);
        let jsonl_path = projects_dir
            .join(&encoded)
            .join(format!("{session_id}.jsonl"));
        if !jsonl_path.exists() {
            continue;
        }
        let file_ms = rollout_modified_ms(&jsonl_path);
        match &best {
            Some((best_ms, _)) if file_ms <= *best_ms => {}
            _ => best = Some((file_ms, jsonl_path)),
        }
    }
    Ok(best.map(|(_, path)| path))
}

fn claude_encode_project_path(cwd: &str) -> String {
    cwd.replace(['/', '\\', ':'], "-")
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

use crate::trim::ratio;

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
        report.samples, report.effective_samples, report.changed_samples, report.skipped_samples
    )?;
    writeln!(
        out,
        "Scope: tool-output savings only, not total model/session token usage"
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
                "  {}  samples={} effective={} changed={} tokens_saved={} ({:.1}%) bytes_saved={} ({:.1}%)",
                row.day,
                row.samples,
                row.effective_samples,
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
                "  {}  samples={} effective={} changed={} tokens_saved={} ({:.1}%) bytes_saved={} ({:.1}%)",
                row.key,
                row.samples,
                row.effective_samples,
                row.changed_samples,
                row.tokens_saved,
                row.tokens_saved_ratio * 100.0,
                row.bytes_saved,
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
            writeln!(out, "{label}:")?;
            for row in groups {
                writeln!(
                    out,
                    "  {}  samples={} effective={} changed={} tokens_saved={} ({:.1}%) bytes_saved={} ({:.1}%)",
                    row.key,
                    row.samples,
                    row.effective_samples,
                    row.changed_samples,
                    row.tokens_saved,
                    row.tokens_saved_ratio * 100.0,
                    row.bytes_saved,
                    row.bytes_saved_ratio * 100.0
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

fn sort_usage_groups(rows: &mut [UsageStatsGroupReport], sort_by: UsageStatsSortBy) {
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

#[cfg(test)]
pub(crate) fn test_find_latest_claude_rollout_after(
    sessions_dir: &Path,
    started_at_ms: u128,
) -> Result<Option<PathBuf>, AppError> {
    find_latest_claude_rollout_after(sessions_dir, started_at_ms)
}

#[cfg(test)]
pub(crate) fn test_claude_encode_project_path(cwd: &str) -> String {
    claude_encode_project_path(cwd)
}
