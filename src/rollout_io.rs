use crate::benchmark::RolloutCompareReport;
use crate::rollout_stats::collect_rollout_output_stats;
use crate::adapter::rewrite_agent_transcript;
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
        None => {
            let sessions_dir = codex_sessions_dir().ok_or_else(|| {
                AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
            })?;
            find_latest_rollout_after(&sessions_dir, 0)?.ok_or_else(|| {
                AppError::Usage("tke could not find a codex rollout jsonl".to_owned())
            })?
        }
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
        None => {
            let sessions_dir = codex_sessions_dir().ok_or_else(|| {
                AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
            })?;
            find_latest_rollout_after(&sessions_dir, 0)?.ok_or_else(|| {
                AppError::Usage("tke could not find a codex rollout jsonl".to_owned())
            })?
        }
    };

    let raw = fs::read_to_string(&source)?;
    let rewritten = rewrite_agent_transcript(&raw, config)?;
    let raw_stats = collect_rollout_output_stats(&raw, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    let report =
        RolloutCompareReport::from_stats(&source, rewritten.is_some(), raw_stats, rewritten_stats);
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

pub struct InteractiveTracker {
    pub(crate) sessions_dir: PathBuf,
    pub(crate) started_at_ms: u128,
}

impl InteractiveTracker {
    pub fn start() -> Result<Self, AppError> {
        let sessions_dir = codex_sessions_dir().ok_or_else(|| {
            AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
        })?;
        Ok(Self {
            sessions_dir,
            started_at_ms: now_millis(),
        })
    }

    pub fn finish(&self, config: &Config) -> Result<(), AppError> {
        let Some(latest) = find_latest_rollout_after(&self.sessions_dir, self.started_at_ms)?
        else {
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

fn resolve_codex_home() -> Option<PathBuf> {
    if let Ok(home) = env::var("CODEX_HOME") {
        return Some(PathBuf::from(home));
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
