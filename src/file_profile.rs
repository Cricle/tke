use crate::trim::{
    MatchChunk, ProfileLimits, ascii_word_tokens, collect_term_chunks, has_token_prefix,
    push_chunk, push_existing_chunk,
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
        || is_code_boundary(line)
}

fn is_code_boundary(line: &str) -> bool {
    let tokens = ascii_word_tokens(line);
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
        || has_token_prefix(&tokens, &["class"])
        || has_token_prefix(&tokens, &["def"])
        || has_token_prefix(&tokens, &["function"])
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
