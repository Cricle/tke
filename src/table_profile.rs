use serde::Serialize;

use crate::trim::{
    ascii_word_tokens, has_ascii_token, has_path_prefix, has_token_prefix, has_token_sequence,
    is_log_signal,
};

#[derive(Serialize)]
pub(crate) struct TableSummary {
    pub(crate) c: Vec<String>,
    pub(crate) r: Vec<TableRow>,
    pub(crate) rc: usize,
    pub(crate) hc: usize,
}

#[derive(Clone, Serialize)]
pub(crate) struct TableRow {
    #[serde(skip_serializing)]
    pub(crate) i: usize,
    pub(crate) v: Vec<String>,
}

pub(crate) struct TableLayout {
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<TableDataRow>,
    #[allow(dead_code)]
    pub(crate) header_index: usize,
}

pub(crate) struct TableDataRow {
    pub(crate) line_index: usize,
    pub(crate) fields: Vec<String>,
}

pub(crate) fn collect_table_summary(
    name: &str,
    args: &[String],
    lines: &[&str],
    terms: &[String],
) -> Option<TableSummary> {
    if name == "du"
        && let Some(summary) = collect_du_summary(lines)
    {
        return Some(summary);
    }
    let layout = detect_table_layout(lines)?;
    let selected_cols = select_table_columns(&layout.headers);
    if selected_cols.is_empty() {
        return None;
    }

    let selected_rows = select_table_rows(name, args, &layout, terms);
    if selected_rows.is_empty() {
        return None;
    }

    let cols = selected_cols
        .iter()
        .map(|idx| layout.headers[*idx].clone())
        .collect::<Vec<_>>();
    let rows = selected_rows
        .into_iter()
        .map(|row_idx| {
            let row = &layout.rows[row_idx];
            let values = selected_cols
                .iter()
                .map(|col_idx| row.fields.get(*col_idx).cloned().unwrap_or_default())
                .collect::<Vec<_>>();
            TableRow {
                i: row.line_index,
                v: values,
            }
        })
        .collect::<Vec<_>>();

    Some(TableSummary {
        c: cols,
        r: rows,
        rc: layout.rows.len(),
        hc: layout.headers.len(),
    })
}

pub(crate) fn collect_table_kept_ranges(table: &TableSummary) -> Vec<(usize, usize)> {
    table.r.iter().map(|row| (row.i, row.i + 1)).collect()
}

pub(crate) fn looks_like_table(lines: &[&str]) -> bool {
    detect_table_layout(lines).is_some() || looks_like_du_listing(lines)
}

pub(crate) fn detect_table_layout(lines: &[&str]) -> Option<TableLayout> {
    let search_limit = usize::min(lines.len(), 4);
    for header_index in 0..search_limit {
        let line = lines.get(header_index)?.trim();
        if line.is_empty() {
            continue;
        }
        let headers = split_table_fields(line, usize::MAX);
        if headers.len() < 3 || !looks_like_table_header(&headers) {
            continue;
        }
        if header_index + 1 < lines.len() {
            let next_fields = split_table_fields(lines[header_index + 1].trim(), headers.len());
            if next_fields
                .iter()
                .any(|field| looks_like_codeish_table_cell(field))
            {
                continue;
            }
        }

        let mut rows = Vec::new();
        for (offset, row_line) in lines.iter().enumerate().skip(header_index + 1).take(128) {
            let trimmed = row_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let fields = split_table_fields(trimmed, headers.len());
            if fields.len() + 1 < headers.len() || fields.len() < 3 {
                continue;
            }
            rows.push(TableDataRow {
                line_index: offset,
                fields,
            });
        }

        if rows.len() >= 3 {
            // Reject if all data rows are identical (repeated output, not a real table)
            let first_row = &rows[0].fields;
            let all_identical = rows[1..].iter().all(|r| r.fields == *first_row);
            if all_identical {
                continue;
            }
            return Some(TableLayout {
                headers,
                rows,
                header_index,
            });
        }
    }
    None
}

fn looks_like_du_listing(lines: &[&str]) -> bool {
    let mut matched = 0usize;
    for line in lines.iter().take(128) {
        if parse_du_row(line).is_some() {
            matched += 1;
        } else if !line.trim().is_empty() {
            return false;
        }
    }
    matched >= 4
}

fn collect_du_summary(lines: &[&str]) -> Option<TableSummary> {
    let rows = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let (size, path) = parse_du_row(line)?;
            Some(TableRow {
                i: idx,
                v: vec![size, path],
            })
        })
        .collect::<Vec<_>>();
    if rows.len() < 4 {
        return None;
    }

    let selected = select_du_rows(&rows);
    if selected.is_empty() {
        return None;
    }

    Some(TableSummary {
        c: vec!["Size".to_owned(), "Path".to_owned()],
        r: selected.into_iter().map(|idx| rows[idx].clone()).collect(),
        rc: rows.len(),
        hc: 2,
    })
}

fn parse_du_row(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let size = parts.next()?.trim();
    let path = parts.next()?.trim();
    if size.is_empty() || path.is_empty() || !looks_like_du_size(size) {
        return None;
    }
    Some((size.to_owned(), path.to_owned()))
}

fn looks_like_du_size(raw: &str) -> bool {
    let mut chars = raw.chars();
    let mut saw_digit = false;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() || ch == '.' {
            saw_digit = true;
            continue;
        }
        if !saw_digit {
            return false;
        }
        return chars.next().is_none() && matches!(ch, 'B' | 'K' | 'M' | 'G' | 'T' | 'P');
    }
    saw_digit
}

fn select_du_rows(rows: &[TableRow]) -> Vec<usize> {
    let mut selected = Vec::new();
    let cap = 6usize;
    for idx in 0..usize::min(rows.len(), 3) {
        push_unique_index(&mut selected, idx, cap);
    }
    if rows.len() > 3 {
        push_unique_index(&mut selected, rows.len() - 1, cap);
    }

    let mut ranked = rows
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            row.v
                .first()
                .and_then(|value| parse_human_size(value))
                .map(|score| (idx, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (idx, _) in ranked.into_iter().take(2) {
        push_unique_index(&mut selected, idx, cap);
    }

    selected.sort_unstable();
    selected
}

fn parse_human_size(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    let unit = trimmed.chars().last()?;
    let scale = match unit {
        'B' => 1.0,
        'K' => 1024.0,
        'M' => 1024.0 * 1024.0,
        'G' => 1024.0 * 1024.0 * 1024.0,
        'T' => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        'P' => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return trimmed.parse::<f64>().ok(),
    };
    let number = &trimmed[..trimmed.len().saturating_sub(1)];
    Some(number.parse::<f64>().ok()? * scale)
}

fn split_table_fields(line: &str, max_fields: usize) -> Vec<String> {
    if looks_like_df_header_line(line) {
        return split_df_header_fields(line, max_fields);
    }
    if looks_like_df_data_line(line) {
        return split_on_any_whitespace(line, max_fields);
    }
    let aligned = split_on_wide_whitespace(line, max_fields);
    if aligned.len() >= 3 {
        return aligned;
    }
    split_on_any_whitespace(line, max_fields)
}

fn looks_like_df_header_line(line: &str) -> bool {
    let normalized = normalize_header_name(line);
    normalized.contains("filesystem")
        && normalized.contains("mountedon")
        && normalized.contains("use%")
}

fn looks_like_df_data_line(line: &str) -> bool {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    fields.len() >= 6
        && fields
            .get(4)
            .is_some_and(|value| value.ends_with('%') && parse_numeric_cell(value).is_some())
        && fields.get(5).is_some_and(|value| has_path_prefix(value))
}

fn split_df_header_fields(line: &str, max_fields: usize) -> Vec<String> {
    let fields = split_on_any_whitespace(line, usize::MAX);
    if fields.len() < 6 {
        return split_on_any_whitespace(line, max_fields);
    }

    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < fields.len() {
        if idx + 1 < fields.len() && fields[idx] == "Mounted" && fields[idx + 1] == "on" {
            out.push("Mounted on".to_owned());
            idx += 2;
            continue;
        }
        out.push(fields[idx].to_owned());
        idx += 1;
    }

    if max_fields != usize::MAX && out.len() > max_fields {
        let mut limited = out
            .iter()
            .take(max_fields.saturating_sub(1))
            .cloned()
            .collect::<Vec<_>>();
        limited.push(out[max_fields - 1..].join(" "));
        return limited;
    }
    out
}

fn split_on_wide_whitespace(line: &str, max_fields: usize) -> Vec<String> {
    if max_fields == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut idx = 0usize;
    let chars = line.char_indices().collect::<Vec<_>>();

    while idx < chars.len() {
        let (byte_idx, ch) = chars[idx];
        if !ch.is_whitespace() {
            idx += 1;
            continue;
        }

        let mut run_end = idx + 1;
        while run_end < chars.len() && chars[run_end].1.is_whitespace() {
            run_end += 1;
        }
        let run_len = run_end - idx;
        if run_len >= 2 && out.len() + 1 < max_fields {
            let end_byte = byte_idx;
            let next_byte = chars
                .get(run_end)
                .map(|(pos, _)| *pos)
                .unwrap_or_else(|| line.len());
            let field = line[start..end_byte].trim();
            if !field.is_empty() {
                out.push(field.to_owned());
            }
            start = next_byte;
        }
        idx = run_end;
    }

    let tail = line[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_owned());
    }
    out
}

fn split_on_any_whitespace(line: &str, max_fields: usize) -> Vec<String> {
    if max_fields == 0 {
        return Vec::new();
    }
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return Vec::new();
    }
    if max_fields == usize::MAX || parts.len() <= max_fields {
        return parts.into_iter().map(str::to_owned).collect();
    }

    let mut out = parts
        .iter()
        .take(max_fields.saturating_sub(1))
        .map(|part| (*part).to_owned())
        .collect::<Vec<_>>();
    out.push(parts[max_fields - 1..].join(" "));
    out
}

fn looks_like_table_header(headers: &[String]) -> bool {
    if headers
        .iter()
        .any(|header| looks_like_codeish_table_cell(header))
    {
        return false;
    }
    let mut known = 0usize;
    let mut score = 0usize;
    for header in headers {
        let normalized = normalize_header_name(header);
        if normalized.is_empty() {
            continue;
        }
        if is_known_table_header(&normalized) {
            known += 1;
            score += 2;
            continue;
        }
        if header
            .chars()
            .any(|ch| matches!(ch, ':' | '/' | '(' | ')' | '{' | '}' | '[' | ']'))
        {
            continue;
        }
        let alpha_count = normalized
            .chars()
            .filter(|ch| ch.is_ascii_alphabetic())
            .count();
        if alpha_count >= 2
            && normalized.len() <= 24
            && header.chars().all(|ch| {
                ch.is_ascii_alphanumeric()
                    || ch.is_ascii_whitespace()
                    || ch == '%'
                    || ch == '-'
                    || ch == '_'
            })
        {
            score += 1;
        }
    }
    known >= 1 || score >= usize::max(4, headers.len())
}

fn looks_like_codeish_table_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    if trimmed.is_empty() {
        return false;
    }
    let tokens = ascii_word_tokens(trimmed);
    has_token_prefix(&tokens, &["fn"])
        || has_token_prefix(&tokens, &["struct"])
        || has_token_prefix(&tokens, &["impl"])
        || has_token_prefix(&tokens, &["let"])
        || has_token_sequence(&tokens, &["println"])
        || trimmed.chars().any(|ch| matches!(ch, '{' | '}'))
        || has_repeated_symbol_pair(trimmed, ':')
}

fn has_repeated_symbol_pair(raw: &str, symbol: char) -> bool {
    let mut prev = None;
    for ch in raw.chars() {
        if prev == Some(symbol) && ch == symbol {
            return true;
        }
        prev = Some(ch);
    }
    false
}

fn normalize_header_name(header: &str) -> String {
    header
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '%')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn is_known_table_header(header: &str) -> bool {
    matches!(
        header,
        "user"
            | "pid"
            | "%cpu"
            | "%mem"
            | "vsz"
            | "rss"
            | "tty"
            | "stat"
            | "start"
            | "time"
            | "command"
            | "unit"
            | "load"
            | "active"
            | "sub"
            | "description"
            | "netid"
            | "state"
            | "recvq"
            | "sendq"
            | "localaddressport"
            | "peeraddressport"
            | "process"
            | "containerid"
            | "image"
            | "created"
            | "status"
            | "ports"
            | "names"
            | "size"
            | "avail"
            | "use%"
            | "mountedon"
            | "column"
            | "type"
            | "value"
            | "count"
            | "database"
            | "schema"
            | "table"
            | "rows"
            | "name"
            | "tag"
            | "repository"
            | "version"
            | "id"
            | "port"
            | "host"
            | "address"
            | "path"
            | "file"
            | "profile"
            | "mode"
            | "age"
            | "ip"
            | "node"
            | "namespace"
            | "ready"
            | "restarts"
    )
}

fn select_table_columns(headers: &[String]) -> Vec<usize> {
    select_named_table_columns(headers).unwrap_or_else(|| select_generic_table_columns(headers))
}

fn select_named_table_columns(headers: &[String]) -> Option<Vec<usize>> {
    let normalized = headers
        .iter()
        .map(|header| normalize_header_name(header))
        .collect::<Vec<_>>();

    if normalized == ["filesystem", "size", "used", "avail", "use%", "mountedon"] {
        return Some(select_columns_by_name(
            &normalized,
            &["filesystem", "size", "used", "use%", "mountedon"],
            5,
        ));
    }

    if normalized == ["schema", "table", "rows"] {
        return Some(vec![0, 1, 2]);
    }

    let has = |name: &str| normalized.iter().any(|h| h == name);

    if has("command") && has("pid") && has("%cpu") {
        return Some(select_columns_by_name(
            &normalized,
            &["user", "pid", "%cpu", "%mem", "command"],
            5,
        ));
    }

    if has("containerid") && has("names") {
        return Some(select_columns_by_name(
            &normalized,
            &["containerid", "image", "status", "ports", "names"],
            5,
        ));
    }

    if has("unit") && has("description") {
        return Some(select_columns_by_name(
            &normalized,
            &["unit", "active", "sub", "description"],
            4,
        ));
    }

    None
}

fn select_columns_by_name(normalized: &[String], wanted: &[&str], cap: usize) -> Vec<usize> {
    let mut selected = Vec::new();
    for name in wanted {
        if let Some(idx) = normalized.iter().position(|header| header == name)
            && !selected.contains(&idx)
        {
            selected.push(idx);
        }
        if selected.len() >= cap {
            break;
        }
    }
    selected.sort_unstable();
    selected
}

fn select_generic_table_columns(headers: &[String]) -> Vec<usize> {
    if headers.len() <= 6 {
        return (0..headers.len()).collect();
    }

    let normalized = headers
        .iter()
        .map(|header| normalize_header_name(header))
        .collect::<Vec<_>>();
    let wanted = [
        "filesystem",
        "user",
        "pid",
        "%cpu",
        "%mem",
        "size",
        "used",
        "avail",
        "use%",
        "mountedon",
        "stat",
        "command",
        "schema",
        "table",
        "rows",
        "unit",
        "active",
        "sub",
        "description",
        "netid",
        "state",
        "localaddressport",
        "peeraddressport",
        "process",
        "containerid",
        "image",
        "status",
        "ports",
        "names",
    ];

    let mut selected = Vec::new();
    for idx in [0usize, 1usize, headers.len().saturating_sub(1)] {
        if idx < headers.len() && !selected.contains(&idx) {
            selected.push(idx);
        }
    }
    for name in wanted {
        for (idx, header) in normalized.iter().enumerate() {
            if header == name && !selected.contains(&idx) {
                selected.push(idx);
            }
            if selected.len() >= 5 {
                break;
            }
        }
        if selected.len() >= 5 {
            break;
        }
    }
    for idx in 0..headers.len() {
        if selected.len() >= 5 {
            break;
        }
        if !selected.contains(&idx) {
            selected.push(idx);
        }
    }
    selected.sort_unstable();
    selected
}

fn select_table_rows(
    name: &str,
    args: &[String],
    layout: &TableLayout,
    terms: &[String],
) -> Vec<usize> {
    let mut selected = Vec::new();
    let cap = match name {
        "ps" | "ss" | "netstat" | "systemctl" => 5,
        "docker" if args.first().map(String::as_str) == Some("ps") => 5,
        "ls" | "df" | "du" | "psql" => 5,
        _ => 6,
    };

    for idx in 0..usize::min(layout.rows.len(), 3) {
        push_unique_index(&mut selected, idx, cap);
    }

    for idx in collect_table_signal_rows(layout, terms) {
        push_unique_index(&mut selected, idx, cap);
    }

    for idx in collect_top_numeric_rows(layout, "%cpu", 2) {
        push_unique_index(&mut selected, idx, cap);
    }
    for idx in collect_top_numeric_rows(layout, "%mem", 2) {
        push_unique_index(&mut selected, idx, cap);
    }

    if layout.rows.len() > 3 {
        push_unique_index(&mut selected, layout.rows.len() - 1, cap);
    }

    selected.sort_unstable();
    selected
}

fn collect_table_signal_rows(layout: &TableLayout, terms: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for (idx, row) in layout.rows.iter().enumerate() {
        let joined = row.fields.join(" ");
        let tokens = ascii_word_tokens(&joined);
        if is_log_signal(&joined, terms)
            || has_ascii_token(&tokens, "codex")
            || has_ascii_token(&tokens, "listen")
            || has_ascii_token(&tokens, "estab")
            || has_ascii_token(&tokens, "exited")
        {
            out.push(idx);
            if out.len() >= 3 {
                break;
            }
        }
    }
    out
}

fn collect_top_numeric_rows(layout: &TableLayout, header: &str, limit: usize) -> Vec<usize> {
    let normalized = layout
        .headers
        .iter()
        .map(|value| normalize_header_name(value))
        .collect::<Vec<_>>();
    let Some(column) = normalized.iter().position(|value| value == header) else {
        return Vec::new();
    };

    let mut ranked = layout
        .rows
        .iter()
        .enumerate()
        .filter_map(|(idx, row)| {
            row.fields
                .get(column)
                .and_then(|value| parse_numeric_cell(value))
                .map(|score| (idx, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(limit).map(|(idx, _)| idx).collect()
}

fn parse_numeric_cell(value: &str) -> Option<f64> {
    let cleaned = value.trim().trim_end_matches('%').replace(',', "");
    cleaned.parse::<f64>().ok()
}

fn push_unique_index(out: &mut Vec<usize>, idx: usize, cap: usize) {
    if out.len() >= cap || out.contains(&idx) {
        return;
    }
    out.push(idx);
}
