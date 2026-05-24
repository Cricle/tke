use crate::trim::{MatchChunk, ProfileLimits, collect_term_chunks, push_chunk};
use std::collections::HashMap;

pub(crate) fn collect_search_chunks(
    lines: &[&str],
    terms: &[String],
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    let grouped = collect_grouped_search_chunks(lines, limits.max_matches);
    if !grouped.is_empty() {
        return grouped;
    }

    let mut out = collect_term_chunks(lines, terms, "result", 0, limits.max_matches);
    if !out.is_empty() {
        return out;
    }

    let mut used = Vec::<(usize, usize)>::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "result")
            && out.len() >= limits.max_matches
        {
            break;
        }
    }
    out
}

fn collect_grouped_search_chunks(lines: &[&str], max_matches: usize) -> Vec<MatchChunk> {
    let mut groups = HashMap::<String, Vec<(usize, String)>>::new();
    let mut order = Vec::<String>::new();

    for (idx, line) in lines.iter().enumerate() {
        let Some((path, rest)) = parse_search_result_line(line) else {
            continue;
        };
        if rest.trim().is_empty() {
            continue;
        }
        if !groups.contains_key(&path) {
            order.push(path.clone());
        }
        groups
            .entry(path)
            .or_default()
            .push((idx, (*line).to_owned()));
    }

    if groups.is_empty() {
        return Vec::new();
    }

    order.sort_by(|a, b| {
        let len_a = groups.get(a).map(|rows| rows.len()).unwrap_or(0);
        let len_b = groups.get(b).map(|rows| rows.len()).unwrap_or(0);
        len_b.cmp(&len_a).then_with(|| a.cmp(b))
    });

    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();
    for path in order.into_iter().take(max_matches) {
        let Some(rows) = groups.get(&path) else {
            continue;
        };
        let mut kept = rows
            .iter()
            .take(3)
            .map(|(_, line)| line.clone())
            .collect::<Vec<_>>();
        if rows.len() > 3
            && let Some((_, last)) = rows.last()
            && kept.last() != Some(last)
        {
            kept.push(last.clone());
        }
        let start = rows.first().map(|(idx, _)| *idx).unwrap_or(0);
        let end = rows.last().map(|(idx, _)| idx + 1).unwrap_or(start + 1);
        if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
            continue;
        }
        used.push((start, end));
        out.push(MatchChunk {
            k: "file".to_owned(),
            r: [start, end],
            l: kept,
        });
    }
    out
}

fn parse_search_result_line(line: &str) -> Option<(String, String)> {
    let (path, rest) = line.split_once(':')?;
    if path.is_empty() || !path.contains('.') {
        return None;
    }
    Some((path.to_owned(), rest.to_owned()))
}
