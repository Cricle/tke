use crate::trim::{
    MatchChunk, ProfileLimits, collect_term_chunks, push_chunk, push_existing_chunk,
};

const MAX_DECL_CHUNKS: usize = 6;
const MAX_BLOCK_CHUNKS: usize = 2;

pub(crate) fn collect_file_chunks(
    lines: &[&str],
    terms: &[String],
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
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

fn normalized_code_line<'a>(line: &'a str) -> &'a str {
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
    line.starts_with("use ")
        || line.starts_with("mod ")
        || line.starts_with("pub mod ")
        || line.starts_with("const ")
        || line.starts_with("pub const ")
        || line.starts_with("type ")
        || line.starts_with("pub type ")
        || line.starts_with("trait ")
        || line.starts_with("pub trait ")
        || line.starts_with("impl ")
        || line.starts_with("#[")
        || is_code_boundary(line)
}

fn is_code_boundary(line: &str) -> bool {
    line.starts_with("fn ")
        || line.starts_with("pub fn ")
        || line.starts_with("pub(crate) fn ")
        || line.starts_with("async fn ")
        || line.starts_with("pub async fn ")
        || line.starts_with("pub(crate) async fn ")
        || line.starts_with("struct ")
        || line.starts_with("pub struct ")
        || line.starts_with("pub(crate) struct ")
        || line.starts_with("enum ")
        || line.starts_with("pub enum ")
        || line.starts_with("pub(crate) enum ")
        || line.starts_with("class ")
        || line.starts_with("def ")
        || line.starts_with("function ")
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
