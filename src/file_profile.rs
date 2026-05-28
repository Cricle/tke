use crate::trim::{
    MatchChunk, ProfileLimits, ascii_word_tokens, collect_term_chunks, has_token_prefix,
    push_chunk, push_existing_chunk,
};
use std::collections::HashMap;

const MAX_DECL_CHUNKS: usize = 6;
const MAX_BLOCK_CHUNKS: usize = 2;

pub(crate) fn collect_file_chunks(
    lines: &[&str],
    terms: &[String],
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    // Multi-file output detection (e.g., find | xargs sed producing "===== file =====" sections)
    let multi = collect_multi_file_chunks(lines, limits.max_matches);
    if !multi.is_empty() {
        return multi;
    }

    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for idx in detect_outline_lines(lines)
        .into_iter()
        .take(usize::max(MAX_DECL_CHUNKS, limits.max_matches))
    {
        if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "decl")
            && out.len() >= MAX_DECL_CHUNKS
        {
            break;
        }
    }

    for chunk in collect_term_chunks(
        lines,
        terms,
        "snippet",
        usize::min(limits.match_context, 1),
        limits.max_matches,
    ) {
        if push_existing_chunk(&mut out, &mut used, chunk) && out.len() >= limits.max_matches {
            break;
        }
    }

    let has_block = out.iter().any(|chunk| chunk.k == "block");
    if out.len() < 4 || !has_block {
        for (start, end) in detect_code_blocks(lines).into_iter().take(MAX_BLOCK_CHUNKS) {
            if push_chunk(&mut out, &mut used, lines, start, end, "block") && out.len() >= 4 {
                break;
            }
        }
    }

    if out.is_empty() {
        let midpoint = lines.len() / 2;
        let start = midpoint.saturating_sub(usize::min(3, midpoint));
        let end = usize::min(lines.len(), start + 6);
        push_chunk(&mut out, &mut used, lines, start, end, "block");
    }

    out
}

fn detect_outline_lines(lines: &[&str]) -> Vec<usize> {
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = normalized_code_line(line);
        if trimmed.is_empty() {
            continue;
        }
        if is_outline_line(trimmed) {
            out.push(idx);
        }
    }
    out
}

fn detect_code_blocks(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_code_boundary(normalized_code_line(line)) {
            let end = find_block_end(lines, idx + 1);
            out.push((idx, end));
            if out.len() >= 4 {
                break;
            }
        }
    }
    out
}

fn normalized_code_line(line: &str) -> &str {
    let trimmed = line.trim_start();
    let digit_prefix = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || ch.is_ascii_whitespace())
        .count();
    let candidate = &trimmed[digit_prefix..];
    candidate
        .strip_prefix('|')
        .or_else(|| candidate.strip_prefix(':'))
        .unwrap_or(candidate)
        .trim_start()
}

fn is_outline_line(line: &str) -> bool {
    let tokens = ascii_word_tokens(line);
    line.starts_with('#')
        || has_token_prefix(&tokens, &["use"])
        || has_token_prefix(&tokens, &["mod"])
        || has_token_prefix(&tokens, &["pub", "mod"])
        || has_token_prefix(&tokens, &["const"])
        || has_token_prefix(&tokens, &["pub", "const"])
        || has_token_prefix(&tokens, &["type"])
        || has_token_prefix(&tokens, &["pub", "type"])
        || has_token_prefix(&tokens, &["trait"])
        || has_token_prefix(&tokens, &["pub", "trait"])
        || has_token_prefix(&tokens, &["impl"])
        || has_token_prefix(&tokens, &["extern"])
        || has_token_prefix(&tokens, &["static"])
        || has_token_prefix(&tokens, &["pub", "static"])
        || has_token_prefix(&tokens, &["let"])
        || has_token_prefix(&tokens, &["var"])
        || has_token_prefix(&tokens, &["import"])
        || has_token_prefix(&tokens, &["from"])
        || has_token_prefix(&tokens, &["require"])
        || has_token_prefix(&tokens, &["interface"])
        || has_token_prefix(&tokens, &["export"])
        || has_token_prefix(&tokens, &["public"])
        || has_token_prefix(&tokens, &["private"])
        || has_token_prefix(&tokens, &["protected"])
        || line.starts_with('@')
        || is_code_boundary(line)
}

fn is_code_boundary(line: &str) -> bool {
    let tokens = ascii_word_tokens(line);
    // Rust
    has_token_prefix(&tokens, &["fn"])
        || has_token_prefix(&tokens, &["pub", "fn"])
        || has_token_prefix(&tokens, &["pub", "crate", "fn"])
        || has_token_prefix(&tokens, &["async", "fn"])
        || has_token_prefix(&tokens, &["pub", "async", "fn"])
        || has_token_prefix(&tokens, &["pub", "crate", "async", "fn"])
        || has_token_prefix(&tokens, &["struct"])
        || has_token_prefix(&tokens, &["pub", "struct"])
        || has_token_prefix(&tokens, &["pub", "crate", "struct"])
        || has_token_prefix(&tokens, &["enum"])
        || has_token_prefix(&tokens, &["pub", "enum"])
        || has_token_prefix(&tokens, &["pub", "crate", "enum"])
        // Python
        || has_token_prefix(&tokens, &["class"])
        || has_token_prefix(&tokens, &["def"])
        || has_token_prefix(&tokens, &["async", "def"])
        || has_token_prefix(&tokens, &["function"])
        // Go
        || has_token_prefix(&tokens, &["func"])
        // Java/C# methods
        || has_token_prefix(&tokens, &["public", "static"])
        || has_token_prefix(&tokens, &["private", "static"])
        || has_token_prefix(&tokens, &["protected", "static"])
        || has_token_prefix(&tokens, &["public", "abstract"])
        || has_token_prefix(&tokens, &["private", "abstract"])
        || has_token_prefix(&tokens, &["protected", "abstract"])
        // JavaScript/TypeScript
        || has_token_prefix(&tokens, &["export", "function"])
        || has_token_prefix(&tokens, &["export", "async", "function"])
        || has_token_prefix(&tokens, &["export", "class"])
        || has_token_prefix(&tokens, &["export", "default", "function"])
        || has_token_prefix(&tokens, &["export", "default", "class"])
        || has_token_prefix(&tokens, &["export", "default", "async", "function"])
}

fn find_block_end(lines: &[&str], start: usize) -> usize {
    let mut end = usize::min(lines.len(), start + 1);
    while end < lines.len() {
        let trimmed = normalized_code_line(lines[end]).trim();
        if trimmed.is_empty() {
            break;
        }
        if is_code_boundary(trimmed) && end > start {
            break;
        }
        if end.saturating_sub(start) >= 7 {
            break;
        }
        end += 1;
    }
    end
}

/// Detect multi-file output patterns like:
///   ===== /path/to/file =====
///   ==> /path/to/file <==
///   --- /path/to/file ---
///   ### /path/to/file ###
fn is_multi_file_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 5 {
        return false;
    }
    // Pattern: "==> path <=="
    if trimmed.starts_with("==> ") && trimmed.contains(" <==") {
        return true;
    }
    // Pattern: repeated chars + path + repeated chars
    // e.g., "===== /etc/profile =====" or "--- /etc/profile ---"
    let starts_special = trimmed.starts_with('=')
        || trimmed.starts_with('-')
        || trimmed.starts_with('#')
        || trimmed.starts_with('*');
    if !starts_special {
        return false;
    }
    // Must contain a path-like segment (has / or \)
    let has_path = trimmed.contains('/') || trimmed.contains('\\');
    if !has_path {
        return false;
    }
    // Must have matching closing markers (>= 3 chars)
    let first_char = trimmed.chars().next().unwrap();
    let trailing: String = trimmed
        .chars()
        .rev()
        .take_while(|&c| c == first_char)
        .collect();
    trailing.len() >= 3
}

fn collect_multi_file_chunks(lines: &[&str], max_matches: usize) -> Vec<MatchChunk> {
    // Find separator lines
    let separators: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_multi_file_separator(line))
        .map(|(idx, _)| idx)
        .collect();

    if separators.len() < 2 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();
    let mut file_groups = HashMap::<String, Vec<usize>>::new();
    let mut order = Vec::<String>::new();

    for window in separators.windows(2) {
        let sep_idx = window[0];
        let path = lines[sep_idx].trim().to_owned();
        if !file_groups.contains_key(&path) {
            order.push(path.clone());
        }
        file_groups.entry(path).or_default().push(sep_idx);
    }

    // Sort by number of occurrences (most frequent first)
    order.sort_by(|a, b| {
        let ca = file_groups.get(a).map(|v| v.len()).unwrap_or(0);
        let cb = file_groups.get(b).map(|v| v.len()).unwrap_or(0);
        cb.cmp(&ca).then_with(|| a.cmp(b))
    });

    for path in order.iter().take(max_matches) {
        let Some(indices) = file_groups.get(path) else {
            continue;
        };
        // Take first occurrence as representative
        let &sep_idx = indices.first().unwrap();
        let next_sep = separators
            .iter()
            .find(|&&i| i > sep_idx)
            .copied()
            .unwrap_or(lines.len());
        // Keep separator + first few content lines
        let content_end = usize::min(next_sep, sep_idx + 4);
        if push_chunk(&mut out, &mut used, lines, sep_idx, content_end, "file")
            && out.len() >= max_matches
        {
            break;
        }
    }

    out
}
