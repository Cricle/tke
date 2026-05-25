use crate::trim::{PathBucket, PathEntry, PathListSummary, PathRow};
use std::collections::HashMap;
use std::path::Path;

const MIN_PATH_ENTRIES: usize = 4;

pub(crate) fn looks_like_path_list(lines: &[&str]) -> bool {
    detect_path_entries(lines).is_some()
}

pub(crate) fn collect_path_list_summary(lines: &[&str]) -> Option<PathListSummary> {
    let entries = detect_path_entries(lines)?;
    let same_parent = dominant_parent(&entries);
    let first_full = entries.first().map(|entry| entry.value.clone());
    let last_full = entries.last().map(|entry| entry.value.clone());
    let first_compact = entries
        .first()
        .map(|entry| summarize_entry(entry, same_parent.as_deref()));
    let last_compact = entries
        .last()
        .map(|entry| summarize_entry(entry, same_parent.as_deref()));
    let mut dirs = HashMap::<String, Vec<&PathEntry>>::new();
    for entry in &entries {
        dirs.entry(entry.parent.clone()).or_default().push(entry);
    }

    let mut buckets = dirs
        .into_iter()
        .map(|(dir, rows)| {
            let count = rows.len();
            let mut examples = rows
                .iter()
                .take(2)
                .map(|entry| summarize_entry(entry, same_parent.as_deref()))
                .collect::<Vec<_>>();
            if count > 2
                && let Some(last) = rows.last()
            {
                let last_value = summarize_entry(last, same_parent.as_deref());
                if !examples.contains(&last_value) {
                    examples.push(last_value);
                }
            }
            PathBucket {
                d: dir,
                c: count,
                e: examples,
            }
        })
        .collect::<Vec<_>>();
    buckets.sort_by(|a, b| b.c.cmp(&a.c).then_with(|| a.d.cmp(&b.d)));
    buckets.truncate(8);

    let mut rows = entries
        .iter()
        .take(2)
        .map(|entry| PathRow {
            i: entry.line_index,
            v: summarize_entry(entry, same_parent.as_deref()),
        })
        .collect::<Vec<_>>();
    if entries.len() > 2 {
        for entry in entries.iter().rev().take(2).rev() {
            if rows.iter().all(|row| row.i != entry.line_index) {
                rows.push(PathRow {
                    i: entry.line_index,
                    v: summarize_entry(entry, same_parent.as_deref()),
                });
            }
        }
    }

    let compact_examples = rows.iter().map(|row| row.v.clone()).collect::<Vec<_>>();
    let summary_text = build_summary_text(
        entries.len(),
        same_parent.as_deref(),
        first_compact.as_deref(),
        last_compact.as_deref(),
    );
    if let Some(parent) = same_parent {
        return Some(PathListSummary {
            rc: entries.len(),
            s: Some(summary_text),
            d: Some(parent),
            f: first_compact,
            l: last_compact,
            e: compact_examples,
            b: Vec::new(),
            r: Vec::new(),
        });
    }

    Some(PathListSummary {
        rc: entries.len(),
        s: Some(summary_text),
        d: None,
        f: first_full,
        l: last_full,
        e: compact_examples,
        b: buckets,
        r: rows,
    })
}

pub(crate) fn collect_path_list_kept_ranges(pathlist: &PathListSummary) -> Vec<(usize, usize)> {
    pathlist.r.iter().map(|row| (row.i, row.i + 1)).collect()
}

fn detect_path_entries(lines: &[&str]) -> Option<Vec<PathEntry>> {
    let mut entries = Vec::new();
    let mut bare_name_count = 0usize;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if has_pathlist_spacing_noise(trimmed) {
            return None;
        }
        if has_non_path_colon(trimmed) || !looks_like_path(trimmed) {
            return None;
        }
        if is_bare_name(trimmed) {
            bare_name_count += 1;
        }
        entries.push(PathEntry {
            line_index: idx,
            parent: path_parent(trimmed),
            value: trimmed.to_owned(),
        });
    }
    if entries.len() >= MIN_PATH_ENTRIES
        && (bare_name_count == 0 || bare_name_count == entries.len())
    {
        Some(entries)
    } else {
        None
    }
}

fn has_pathlist_spacing_noise(line: &str) -> bool {
    let mut prev_space = false;
    for ch in line.chars() {
        if ch == '\t' {
            return true;
        }
        if ch == ' ' {
            if prev_space {
                return true;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
    }
    false
}

fn dominant_parent(entries: &[PathEntry]) -> Option<String> {
    let first = entries.first()?.parent.clone();
    if entries.iter().all(|entry| entry.parent == first) {
        Some(first)
    } else {
        None
    }
}

fn summarize_entry(entry: &PathEntry, shared_parent: Option<&str>) -> String {
    if let Some(parent) = shared_parent
        && entry.parent == parent
        && let Some(name) = Path::new(&entry.value)
            .file_name()
            .and_then(|name| name.to_str())
    {
        return name.to_owned();
    }
    if let Some(name) = Path::new(&entry.value)
        .file_name()
        .and_then(|name| name.to_str())
        && name.len() + 8 < entry.value.len()
    {
        return format!(".../{name}");
    }
    entry.value.clone()
}

fn has_non_path_colon(line: &str) -> bool {
    for (idx, ch) in line.char_indices() {
        if ch != ':' {
            continue;
        }
        let is_drive = idx == 1
            && line
                .chars()
                .next()
                .map(|head| head.is_ascii_alphabetic())
                .unwrap_or(false)
            && line
                .as_bytes()
                .get(2)
                .map(|next| *next == b'/' || *next == b'\\')
                .unwrap_or(false);
        if !is_drive {
            return true;
        }
    }
    false
}

fn looks_like_path(line: &str) -> bool {
    let bytes = line.as_bytes();
    let windows_drive = bytes.len() > 2
        && bytes[1] == b':'
        && bytes[0].is_ascii_alphabetic()
        && (bytes[2] == b'/' || bytes[2] == b'\\');
    let has_separator = line.chars().any(|ch| ch == '/' || ch == '\\');
    (has_path_prefix(line) || has_separator || is_bare_name(line) || windows_drive)
        && !line.ends_with(':')
        && !contains_arrow_mapping(line)
}

fn has_path_prefix(line: &str) -> bool {
    let chars = line.chars().collect::<Vec<_>>();
    matches!(
        chars.as_slice(),
        ['/', ..] | ['.', '/', ..] | ['.', '.', '/', ..] | ['.', '\\', ..] | ['.', '.', '\\', ..]
    )
}

fn is_bare_name(line: &str) -> bool {
    !line.is_empty()
        && !line
            .chars()
            .any(|ch| matches!(ch, ':' | '/' | '\\' | ' ' | '\t'))
        && line
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@'))
}

fn contains_arrow_mapping(line: &str) -> bool {
    let chars = line.chars().collect::<Vec<_>>();
    chars
        .windows(4)
        .any(|window| window == [' ', '-', '>', ' '])
}

fn path_parent(value: &str) -> String {
    Path::new(value)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| ".".to_owned())
}

fn build_summary_text(
    count: usize,
    dir: Option<&str>,
    first: Option<&str>,
    last: Option<&str>,
) -> String {
    let mut parts = vec![format!("C={count}")];
    if let Some(first) = first {
        parts.push(format!("F={first}"));
    }
    if let Some(last) = last {
        parts.push(format!("L={last}"));
    }
    if let Some(dir) = dir {
        parts.push(format!("D={dir}"));
    }
    parts.join(", ")
}
