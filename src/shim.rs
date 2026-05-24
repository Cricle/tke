use crate::adapter::rewrite_agent_transcript;
use crate::app::{AppError, Config};
use crate::benchmark::estimate_text_tokens;
use crate::rewrite::{LivePipelineDecision, detect_linux_parent_pipeline, live_pipeline_decision};
use crate::rollout_io::InteractiveTracker;
use crate::trim::{
    CommandKind, TrimEnvelope, TrimProfile, TrimStats, classify_command, collect_kept_ranges,
    collect_path_list_kept_ranges, collect_path_list_summary, collect_profile_chunks,
    collect_table_kept_ranges, collect_table_summary, compact_args, compute_omitted_ranges,
    create_single_shim, exit_code, match_terms, merge_ranges, profile_limits, read_stdin_if_piped,
    real_path_string, select_profile, should_force_trim, take_head, take_tail,
};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn write_all_resilient<W: Write>(writer: &mut W, mut bytes: &[u8]) -> Result<(), AppError> {
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "short write").into());
            }
            Ok(written) => bytes = &bytes[written..],
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                thread::sleep(Duration::from_millis(1));
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

pub(crate) fn create_shims(
    shim_dir: &Path,
    agents: &[String],
    tools: &[String],
) -> Result<(), AppError> {
    let exe = env::current_exe()?;
    let mut names = BTreeSet::new();
    if !cfg!(windows) {
        names.insert("tke".to_owned());
    }
    for name in agents {
        names.insert(name.clone());
    }
    for name in tools {
        names.insert(name.clone());
    }

    for name in names {
        create_single_shim(shim_dir, &exe, &name)?;
    }
    Ok(())
}

pub(crate) fn run_agent_command(
    name: &str,
    real_cmd: &Path,
    args: &[String],
    config: &Config,
) -> Result<i32, AppError> {
    let stdout_is_tty = io::stdout().is_terminal();
    let stderr_is_tty = io::stderr().is_terminal();
    let stdin_payload = read_stdin_if_piped()?;

    let mut envs = vec![
        ("TKE_AGENT_CONTEXT".to_owned(), Some(OsString::from("1"))),
        ("TKE_ACTIVE_AGENT".to_owned(), Some(OsString::from(name))),
        (
            "TKE_REAL_PATH".to_owned(),
            Some(OsString::from(real_path_string())),
        ),
    ];
    let force_passthrough = should_passthrough_agent_output(name);

    if stdout_is_tty && stderr_is_tty {
        let tracker = if name == "codex" {
            Some(InteractiveTracker::start()?)
        } else {
            None
        };
        let code = passthrough(real_cmd, args, Some(envs), stdin_payload, true)?;
        if let Some(tracker) = tracker {
            tracker.finish(config)?;
        }
        return Ok(code);
    }

    if force_passthrough {
        return passthrough(real_cmd, args, Some(envs), stdin_payload, true);
    }

    let output = capture_process(real_cmd, args, Some(envs.split_off(0)), stdin_payload, true)?;
    emit_stream(
        io::stdout(),
        &output.stdout,
        name,
        args,
        "stdout",
        classify_command(name, args),
        config.trim_agent_output,
        None,
        config,
    )?;
    emit_stream(
        io::stderr(),
        &output.stderr,
        name,
        args,
        "stderr",
        classify_command(name, args),
        config.trim_agent_output,
        None,
        config,
    )?;
    Ok(exit_code(output.status))
}

fn should_passthrough_agent_output(name: &str) -> bool {
    if name != "claude" {
        return false;
    }
    !matches!(
        env::var("TKE_CLAUDE_LIVE_TOOLS").ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

pub(crate) fn run_tool_command(
    name: &str,
    real_cmd: &Path,
    args: &[String],
    config: &Config,
) -> Result<i32, AppError> {
    let live_pipeline_stage = detect_live_pipeline_stage(name);
    if matches!(live_pipeline_stage, LivePipelineStage::PassThrough) {
        return passthrough(real_cmd, args, None, None, false);
    }
    let fallback_kind = classify_command(name, args);
    let normalize_view = live_pipeline_stage.normalization_view(name, args, fallback_kind);
    let stdin_payload = read_stdin_if_piped()?;
    let mut envs = vec![
        ("PATH".to_owned(), Some(OsString::from(real_path_string()))),
        ("TKE_TOOL_SOURCE".to_owned(), Some(OsString::from(name))),
    ];
    let output = capture_process(
        real_cmd,
        args,
        Some(envs.split_off(0)),
        stdin_payload,
        false,
    )?;
    emit_stream(
        io::stdout(),
        &output.stdout,
        name,
        args,
        "stdout",
        fallback_kind,
        true,
        Some(normalize_view),
        config,
    )?;
    emit_stream(
        io::stderr(),
        &output.stderr,
        name,
        args,
        "stderr",
        fallback_kind,
        true,
        Some(normalize_view),
        config,
    )?;
    Ok(exit_code(output.status))
}

#[derive(Clone)]
enum LivePipelineStage {
    None,
    PassThrough,
    Normalize {
        selected_name: String,
        selected_args: Vec<String>,
        selected_role: String,
        selected_kind: CommandKind,
    },
}

impl LivePipelineStage {
    fn normalization_view<'a>(
        &'a self,
        fallback_name: &'a str,
        fallback_args: &'a [String],
        fallback_kind: CommandKind,
    ) -> NormalizeView<'a> {
        match self {
            Self::Normalize {
                selected_name,
                selected_args,
                selected_role,
                selected_kind,
            } => NormalizeView {
                name: selected_name.as_str(),
                args: selected_args.as_slice(),
                kind: *selected_kind,
                selected_stage: Some((selected_name.as_str(), selected_role.as_str())),
            },
            _ => NormalizeView {
                name: fallback_name,
                args: fallback_args,
                kind: fallback_kind,
                selected_stage: None,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct NormalizeView<'a> {
    name: &'a str,
    args: &'a [String],
    kind: CommandKind,
    selected_stage: Option<(&'a str, &'a str)>,
}

fn detect_live_pipeline_stage(name: &str) -> LivePipelineStage {
    let Some(parsed) = detect_linux_parent_pipeline() else {
        return LivePipelineStage::None;
    };
    match live_pipeline_decision(&parsed, name) {
        LivePipelineDecision::NotPipeline => LivePipelineStage::None,
        LivePipelineDecision::PassThrough => LivePipelineStage::PassThrough,
        LivePipelineDecision::Normalize(selected) => LivePipelineStage::Normalize {
            selected_kind: classify_command(&selected.name, &selected.args),
            selected_name: selected.name,
            selected_args: selected.args,
            selected_role: selected.role.as_str().to_owned(),
        },
    }
}

pub(crate) fn passthrough(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    stdin_payload: Option<Vec<u8>>,
    keep_shim_path: bool,
) -> Result<i32, AppError> {
    let mut cmd = Command::new(real_cmd);
    cmd.args(args)
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if !keep_shim_path {
        cmd.env("PATH", real_path_string());
    }
    if let Some(envs) = extra_envs {
        for (key, value) in envs {
            match value {
                Some(v) => {
                    cmd.env(key, v);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }

    let mut child = cmd.spawn()?;
    if let Some(payload) = stdin_payload {
        if let Some(mut stdin) = child.stdin.take() {
            write_all_resilient(&mut stdin, &payload)?;
        }
    }
    let status = child.wait()?;
    Ok(exit_code(status))
}

pub(crate) fn capture_process(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    stdin_payload: Option<Vec<u8>>,
    keep_shim_path: bool,
) -> Result<std::process::Output, AppError> {
    let mut cmd = Command::new(real_cmd);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

    if !keep_shim_path {
        cmd.env("PATH", real_path_string());
    }
    if let Some(envs) = extra_envs {
        for (key, value) in envs {
            match value {
                Some(v) => {
                    cmd.env(key, v);
                }
                None => {
                    cmd.env_remove(key);
                }
            }
        }
    }

    let mut child = cmd.spawn()?;
    if let Some(payload) = stdin_payload {
        if let Some(mut stdin) = child.stdin.take() {
            write_all_resilient(&mut stdin, &payload)?;
        }
    }
    Ok(child.wait_with_output()?)
}

pub(crate) fn emit_stream<W: Write>(
    mut writer: W,
    bytes: &[u8],
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    normalize: bool,
    normalize_view: Option<NormalizeView<'_>>,
    config: &Config,
) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Ok(());
    }

    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            write_all_resilient(&mut writer, bytes)?;
            return Ok(());
        }
    };

    if !normalize {
        write_all_resilient(&mut writer, text.as_bytes())?;
        return Ok(());
    }

    if config.is_agent_command(name) && stream == "stdout" {
        if let Some(rewritten) = rewrite_agent_transcript(text, config)? {
            write_all_resilient(&mut writer, rewritten.as_bytes())?;
            return Ok(());
        }
    }

    let normalize_view = normalize_view.unwrap_or(NormalizeView {
        name,
        args,
        kind,
        selected_stage: None,
    });

    let Some(payload) = maybe_normalize_text(
        normalize_view.name,
        normalize_view.args,
        stream,
        normalize_view.kind,
        text,
        config,
        normalize_view.selected_stage,
    )?
    else {
        write_all_resilient(&mut writer, text.as_bytes())?;
        return Ok(());
    };
    write_all_resilient(&mut writer, config.json_prefix.as_bytes())?;
    write_all_resilient(&mut writer, payload.as_bytes())?;
    write_all_resilient(&mut writer, b"\n")?;
    Ok(())
}

pub(crate) fn maybe_normalize_text(
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    text: &str,
    config: &Config,
    selected_stage: Option<(&str, &str)>,
) -> Result<Option<String>, AppError> {
    let lines = text.lines().collect::<Vec<_>>();
    let profile = select_profile(kind, &lines);
    let forced = should_force_trim(profile, text.len(), lines.len(), config);
    if !forced {
        return Ok(None);
    }
    let normalized =
        normalize_text_with_stage(name, args, stream, kind, text, config, selected_stage)?;
    if estimate_text_tokens(text)
        <= estimate_text_tokens(&format!("{}{}", config.json_prefix, normalized))
    {
        return Ok(None);
    }
    Ok(Some(normalized))
}

#[cfg(test)]
pub(crate) fn normalize_text(
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    text: &str,
    config: &Config,
) -> Result<String, AppError> {
    normalize_text_with_stage(name, args, stream, kind, text, config, None)
}

pub(crate) fn normalize_text_with_stage(
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    text: &str,
    config: &Config,
    selected_stage: Option<(&str, &str)>,
) -> Result<String, AppError> {
    let lines: Vec<&str> = text.lines().collect();
    let total_bytes = text.len();
    let total_lines = lines.len();
    let profile = select_profile(kind, &lines);
    let forced = should_force_trim(profile, total_bytes, total_lines, config);
    let terms = match_terms(name, args);

    let body = if !forced {
        Some(
            lines
                .iter()
                .map(|line| (*line).to_owned())
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    let limits = profile_limits(profile, config);
    let table = if forced && profile == TrimProfile::Table {
        collect_table_summary(name, args, &lines, &terms)
    } else {
        None
    };
    let pathlist = if forced && profile == TrimProfile::PathList {
        collect_path_list_summary(&lines)
    } else {
        None
    };
    let log_summary = if forced && profile == TrimProfile::Log {
        Some(crate::log_profile::collect_log_summary(&lines))
    } else {
        None
    };
    let (head, tail, matches, kept_ranges, emitted_lines) = if let Some(table) = &table {
        let kept = merge_ranges(collect_table_kept_ranges(table));
        let emitted = kept.iter().map(|(start, end)| end - start).sum();
        (Vec::new(), Vec::new(), Vec::new(), kept, emitted)
    } else if let Some(pathlist) = &pathlist {
        let kept = merge_ranges(collect_path_list_kept_ranges(pathlist));
        let emitted = kept.iter().map(|(start, end)| end - start).sum();
        (Vec::new(), Vec::new(), Vec::new(), kept, emitted)
    } else {
        let head = take_head(&lines, limits.head_lines);
        let tail = take_tail(&lines, limits.tail_lines);
        let matches = collect_profile_chunks(&lines, &terms, profile, limits);
        let kept_ranges = if forced {
            merge_ranges(collect_kept_ranges(total_lines, &head, &tail, &matches))
        } else if total_lines == 0 {
            Vec::new()
        } else {
            vec![(0, total_lines)]
        };
        let emitted = kept_ranges.iter().map(|(start, end)| end - start).sum();
        (head, tail, matches, kept_ranges, emitted)
    };
    let omitted = if forced {
        if profile == TrimProfile::PathList {
            Vec::new()
        } else {
            compute_omitted_ranges(total_lines, &kept_ranges)
        }
    } else {
        Vec::new()
    };

    let envelope = TrimEnvelope {
        v: 1,
        cmd: if profile == TrimProfile::PathList {
            String::new()
        } else {
            name.to_owned()
        },
        a: if profile == TrimProfile::PathList {
            Vec::new()
        } else {
            compact_args(args)
        },
        k: if profile == TrimProfile::PathList {
            String::new()
        } else {
            kind.as_str().to_owned()
        },
        sc: selected_stage.map(|(name, _)| name.to_owned()),
        sr: selected_stage.map(|(_, role)| role.to_owned()),
        p: profile.as_str().to_owned(),
        c: pathlist.as_ref().map(|summary| summary.rc),
        s: if profile == TrimProfile::PathList {
            String::new()
        } else {
            stream.to_owned()
        },
        t: if forced { Some(true) } else { None },
        h: head,
        ta: tail,
        m: matches,
        o: omitted,
        st: if profile == TrimProfile::PathList {
            None
        } else {
            Some(TrimStats {
                tb: total_bytes,
                tl: total_lines,
                el: emitted_lines,
            })
        },
        tb: table,
        pl: pathlist,
        lg: log_summary,
        b: body,
    };
    Ok(serde_json::to_string(&envelope)?)
}
