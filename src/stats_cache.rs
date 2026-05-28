use crate::adapter::rewrite_agent_transcript;
use crate::benchmark::RolloutCompareReport;
use crate::rollout_io::{discover_rollout_paths_in_dir, rollout_modified_ms};
use crate::rollout_stats::{
    RolloutOutputStats, RolloutOutputStatsDetailed, collect_rollout_output_stats_detailed,
    rollout_has_relevant_tool_output,
};
use crate::stats::{UsageStatsSortBy, paired_record_rows_with_meta};
use crate::{AppError, Config};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UsageStatsCacheEntry {
    pub(crate) source: String,
    pub(crate) modified_ms: u128,
    pub(crate) size: u64,
    pub(crate) raw: RolloutOutputStatsDetailed,
    pub(crate) rewritten: RolloutOutputStatsDetailed,
    pub(crate) changed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UsageStatsDirCacheEntry {
    pub(crate) path: String,
    pub(crate) modified_ms: u128,
    pub(crate) files: Vec<String>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct UsageStatsCache {
    v: u8,
    pub(crate) files: Vec<UsageStatsCacheEntry>,
    pub(crate) dirs: Vec<UsageStatsDirCacheEntry>,
    pub(crate) days: Vec<UsageStatsDayCacheEntry>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct UsageStatsAggregateCounter {
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
pub(crate) struct UsageStatsDayCacheEntry {
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

fn usage_stats_cache_path() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".tke").join("stats-cache.json"))
}

pub(crate) fn load_usage_stats_cache() -> UsageStatsCache {
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

pub(crate) fn save_usage_stats_cache(cache: &UsageStatsCache) {
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

pub(crate) fn build_usage_stats_report_from_day_cache(
    roots: &[PathBuf],
    top: usize,
    sort_by: UsageStatsSortBy,
    cache: &mut UsageStatsCache,
    config: &Config,
) -> Result<Option<crate::stats::UsageStatsReport>, AppError> {
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
    Ok(Some(crate::stats::UsageStatsReport::from_parts(
        roots_rendered,
        top,
        total.samples,
        total.effective_samples,
        total.changed_samples,
        skipped_samples,
        raw_tokens,
        rewritten_tokens,
        tokens_saved,
        raw_bytes,
        rewritten_bytes,
        bytes_saved,
        day_rows,
        top_profiles,
        top_commands,
        bottom_profiles,
        bottom_commands,
    )))
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
    let mut raw_detail = RolloutOutputStatsDetailed::default();
    let mut rewritten_detail = RolloutOutputStatsDetailed::default();
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
    let mut raw_detail = RolloutOutputStatsDetailed::default();
    let mut rewritten_detail = RolloutOutputStatsDetailed::default();
    raw_detail.total.fields = entry.summary.effective_samples;
    raw_detail.total.bytes = entry.summary.raw_bytes;
    raw_detail.total.approx_tokens = entry.summary.raw_tokens;
    rewritten_detail.total.fields = entry.summary.effective_samples;
    rewritten_detail.total.bytes = entry.summary.rewritten_bytes;
    rewritten_detail.total.approx_tokens = entry.summary.rewritten_tokens;

    for (key, summary) in &entry.profiles {
        raw_detail.breakdown.by_profile.insert(
            key.clone(),
            RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.raw_bytes,
                approx_tokens: summary.raw_tokens,
            },
        );
        rewritten_detail.breakdown.by_profile.insert(
            key.clone(),
            RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.rewritten_bytes,
                approx_tokens: summary.rewritten_tokens,
            },
        );
    }
    for (key, summary) in &entry.commands {
        raw_detail.breakdown.by_command.insert(
            key.clone(),
            RolloutOutputStats {
                fields: summary.effective_samples,
                bytes: summary.raw_bytes,
                approx_tokens: summary.raw_tokens,
            },
        );
        rewritten_detail.breakdown.by_command.insert(
            key.clone(),
            RolloutOutputStats {
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
) -> Vec<crate::stats::UsageStatsGroupReport> {
    let mut rows = grouped
        .iter()
        .map(|(key, value)| {
            let tokens_saved = value.raw_tokens as isize - value.rewritten_tokens as isize;
            let bytes_saved = value.raw_bytes as isize - value.rewritten_bytes as isize;
            crate::stats::UsageStatsGroupReport::from_row(
                key.clone(),
                value.samples,
                value.effective_samples,
                value.changed_samples,
                value.raw_tokens,
                value.rewritten_tokens,
                tokens_saved,
                value.raw_bytes,
                value.rewritten_bytes,
                bytes_saved,
            )
        })
        .filter(|row| row.has_savings())
        .collect::<Vec<_>>();
    crate::stats::sort_usage_groups(&mut rows, sort_by);
    if rows.len() > top {
        rows.truncate(top);
    }
    rows
}

pub(crate) fn upsert_usage_stats_cache_entry(
    cache: &mut Vec<UsageStatsCacheEntry>,
    entry: UsageStatsCacheEntry,
) {
    if let Some(existing) = cache.iter_mut().find(|item| item.source == entry.source) {
        *existing = entry;
    } else {
        cache.push(entry);
    }
}

pub(crate) fn lookup_usage_stats_cache<'a>(
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

pub(crate) fn lookup_usage_stats_dir_cache<'a>(
    cache: &'a [UsageStatsDirCacheEntry],
    dir: &Path,
    modified_ms: u128,
) -> Option<&'a UsageStatsDirCacheEntry> {
    let dir = dir.display().to_string();
    cache
        .iter()
        .find(|entry| entry.path == dir && entry.modified_ms == modified_ms)
}

pub(crate) fn upsert_usage_stats_dir_cache(
    cache: &mut Vec<UsageStatsDirCacheEntry>,
    entry: UsageStatsDirCacheEntry,
) {
    if let Some(existing) = cache.iter_mut().find(|item| item.path == entry.path) {
        *existing = entry;
    } else {
        cache.push(entry);
    }
}

fn daily_usage_rows(reports: &[RolloutCompareReport]) -> Vec<crate::stats::UsageStatsDayReport> {
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
                crate::stats::UsageStatsDayReport::from_row(
                    day,
                    samples,
                    effective_samples,
                    changed_samples,
                    tokens_saved,
                    raw_tokens,
                    bytes_saved,
                    raw_bytes,
                )
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
