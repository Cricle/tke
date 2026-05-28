use crate::adapter::rewrite_agent_transcript;
use crate::benchmark::RolloutCompareReport;
use crate::rollout_stats::collect_rollout_output_stats_detailed;
use crate::trim::now_millis;
use crate::{AppError, Config};
use std::env;
use std::fs;
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

pub(crate) fn default_stats_roots() -> Vec<PathBuf> {
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

pub(crate) fn discover_rollout_paths(
    roots: &[PathBuf],
    mut cache: Option<&mut crate::stats_cache::UsageStatsCache>,
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

pub(crate) fn discover_rollout_paths_in_dir(
    dir: &Path,
    cache: &mut crate::stats_cache::UsageStatsCache,
) -> Result<Vec<PathBuf>, AppError> {
    use crate::stats_cache::{lookup_usage_stats_dir_cache, upsert_usage_stats_dir_cache};

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
        crate::stats_cache::UsageStatsDirCacheEntry {
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

pub(crate) fn rollout_modified_ms(path: &Path) -> u128 {
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

pub(crate) fn find_latest_claude_rollout_after(
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

pub(crate) fn claude_encode_project_path(cwd: &str) -> String {
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
