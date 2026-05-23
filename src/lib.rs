use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_AGENT_COMMANDS: &[&str] = &["codex", "claude", "gemini", "aider"];
const DEFAULT_TOOL_COMMANDS: &[&str] = &[
    "cat", "sed", "rg", "grep", "git", "cargo", "pytest", "npm", "pnpm", "yarn", "tail", "head",
    "ls", "find", "fd", "bat", "nl", "awk", "cut", "sort", "uniq", "wc", "tree", "xargs",
];

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Json(serde_json::Error),
    Usage(String),
    MissingRealCommand(String),
}

impl From<io::Error> for AppError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Json(err) => write!(f, "{err}"),
            Self::Usage(msg) => write!(f, "{msg}"),
            Self::MissingRealCommand(cmd) => {
                write!(
                    f,
                    "tke could not find the real command for `{cmd}` on TKE_REAL_PATH"
                )
            }
        }
    }
}

impl std::error::Error for AppError {}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_min_trim_bytes")]
    pub min_trim_bytes: usize,
    #[serde(default = "default_max_body_lines")]
    pub max_body_lines: usize,
    #[serde(default = "default_head_lines")]
    pub head_lines: usize,
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
    #[serde(default = "default_match_context")]
    pub match_context: usize,
    #[serde(default = "default_max_matches")]
    pub max_matches: usize,
    #[serde(default = "default_show_stats")]
    pub show_stats: bool,
    #[serde(default = "default_output_trim")]
    pub trim_agent_output: bool,
    #[serde(default = "default_json_prefix")]
    pub json_prefix: String,
    #[serde(default)]
    pub agent_commands: Vec<String>,
    #[serde(default)]
    pub tool_commands: Vec<String>,
    #[serde(default)]
    pub whitelist_commands: Vec<String>,
    #[serde(default)]
    pub whitelist_extensions: Vec<String>,
    #[serde(default)]
    pub whitelist_paths: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_trim_bytes: default_min_trim_bytes(),
            max_body_lines: default_max_body_lines(),
            head_lines: default_head_lines(),
            tail_lines: default_tail_lines(),
            match_context: default_match_context(),
            max_matches: default_max_matches(),
            show_stats: default_show_stats(),
            trim_agent_output: default_output_trim(),
            json_prefix: default_json_prefix(),
            agent_commands: DEFAULT_AGENT_COMMANDS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            tool_commands: DEFAULT_TOOL_COMMANDS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            whitelist_commands: Vec::new(),
            whitelist_extensions: vec![
                ".json".to_owned(),
                ".toml".to_owned(),
                ".yaml".to_owned(),
                ".yml".to_owned(),
                ".lock".to_owned(),
            ],
            whitelist_paths: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, AppError> {
        let mut cfg = if let Some(path) = candidate_config_path() {
            if path.is_file() {
                let raw = fs::read_to_string(path)?;
                serde_json::from_str(&raw)?
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        if let Ok(value) = env::var("TKE_MIN_TRIM_BYTES") {
            cfg.min_trim_bytes = parse_usize(&value, cfg.min_trim_bytes);
        }
        if let Ok(value) = env::var("TKE_MAX_BODY_LINES") {
            cfg.max_body_lines = parse_usize(&value, cfg.max_body_lines);
        }
        if let Ok(value) = env::var("TKE_HEAD_LINES") {
            cfg.head_lines = parse_usize(&value, cfg.head_lines);
        }
        if let Ok(value) = env::var("TKE_TAIL_LINES") {
            cfg.tail_lines = parse_usize(&value, cfg.tail_lines);
        }
        if let Ok(value) = env::var("TKE_MATCH_CONTEXT") {
            cfg.match_context = parse_usize(&value, cfg.match_context);
        }
        if let Ok(value) = env::var("TKE_MAX_MATCHES") {
            cfg.max_matches = parse_usize(&value, cfg.max_matches);
        }
        if let Ok(value) = env::var("TKE_SHOW_STATS") {
            cfg.show_stats = matches!(value.as_str(), "1" | "true" | "TRUE" | "yes");
        }
        if let Ok(value) = env::var("TKE_TRIM_AGENT_OUTPUT") {
            cfg.trim_agent_output = matches!(value.as_str(), "1" | "true" | "TRUE" | "yes");
        }
        if let Ok(value) = env::var("TKE_JSON_PREFIX") {
            cfg.json_prefix = value;
        }
        if let Ok(value) = env::var("TKE_AGENT_CMDS") {
            cfg.agent_commands = csv_list(&value);
        }
        if let Ok(value) = env::var("TKE_TOOL_CMDS") {
            cfg.tool_commands = csv_list(&value);
        }
        Ok(cfg)
    }

    pub fn is_agent_command(&self, name: &str) -> bool {
        self.agent_commands.iter().any(|cmd| cmd == name)
    }

    pub fn is_tool_command(&self, name: &str) -> bool {
        self.tool_commands.iter().any(|cmd| cmd == name)
    }

    pub fn is_whitelisted(&self, name: &str, args: &[String]) -> bool {
        if self.whitelist_commands.iter().any(|cmd| cmd == name) {
            return true;
        }

        for arg in args {
            if self
                .whitelist_paths
                .iter()
                .any(|pattern| !pattern.is_empty() && arg.contains(pattern))
            {
                return true;
            }
            if self
                .whitelist_extensions
                .iter()
                .any(|ext| !ext.is_empty() && arg.ends_with(ext))
            {
                return true;
            }
        }
        false
    }
}

#[derive(Debug)]
pub enum Dispatch {
    Help,
    Activate {
        agents: Vec<String>,
        shim_dir: Option<PathBuf>,
        shell: Option<ShellKind>,
    },
    Deactivate,
    CaptureInteractive {
        source: Option<PathBuf>,
        output: Option<PathBuf>,
    },
    CompareRollout {
        source: Option<PathBuf>,
    },
    BenchmarkCommands {
        check: bool,
    },
    PackageRelease,
    Shim {
        name: String,
        args: Vec<String>,
    },
    ShimExec {
        name: String,
        args: Vec<String>,
    },
}

pub fn parse_dispatch(argv0: &str, args: Vec<String>) -> Result<Dispatch, AppError> {
    let invoked = base_name(argv0);
    if invoked != "tke" {
        return Ok(Dispatch::Shim {
            name: invoked,
            args: args.into_iter().skip(1).collect(),
        });
    }

    let sub = args.get(1).map(String::as_str);
    match sub {
        None | Some("-h") | Some("--help") | Some("help") => Ok(Dispatch::Help),
        Some("activate") | Some("env") => parse_activate(args),
        Some("deactivate") => Ok(Dispatch::Deactivate),
        Some("capture-interactive") => parse_capture_interactive(args),
        Some("compare-rollout") => parse_compare_rollout(args),
        Some("benchmark-commands") => parse_benchmark_commands(args),
        Some("package-release") => Ok(Dispatch::PackageRelease),
        Some("shim") => parse_shim_exec(args),
        Some(other) => Err(AppError::Usage(format!(
            "unknown subcommand `{other}`\n\n{}",
            usage()
        ))),
    }
}

pub fn usage() -> String {
    [
        "tke - local token shaving shim",
        "",
        "Usage:",
        "  tke activate [--shim-dir PATH] [--shell SHELL] [agent ...]",
        "  tke env [--shim-dir PATH] [--shell SHELL] [agent ...]",
        "  tke deactivate",
        "  tke capture-interactive [--source PATH] [--output PATH]",
        "  tke compare-rollout [--source PATH]",
        "  tke benchmark-commands [--check]",
        "  tke package-release",
        "",
        "Examples:",
        "  eval \"$(tke activate codex claude)\"",
        "  eval \"$(tke activate --shim-dir ./.tke/shims codex)\"",
        "  tke capture-interactive",
        "  tke compare-rollout",
        "  tke benchmark-commands",
        "  tke package-release",
    ]
    .join("\n")
}

pub fn print_activate(
    agents: &[String],
    shim_dir: Option<PathBuf>,
    shell: Option<ShellKind>,
    config: &Config,
) -> Result<(), AppError> {
    let selected_agents = if agents.is_empty() {
        config.agent_commands.clone()
    } else {
        agents.to_vec()
    };

    let cwd = env::current_dir()?;
    let shim_dir = shim_dir.unwrap_or_else(|| cwd.join(".tke").join("shims"));
    fs::create_dir_all(&shim_dir)?;
    create_shims(&shim_dir, &selected_agents, &config.tool_commands)?;

    let exe = env::current_exe()?;
    let shim_dir_abs = fs::canonicalize(&shim_dir).unwrap_or(shim_dir);
    let current_path = env::var("PATH").unwrap_or_default();
    let real_path = env::var("TKE_REAL_PATH").unwrap_or(current_path.clone());
    let shell = shell.unwrap_or_else(detect_shell_kind);
    print!(
        "{}",
        render_activate_script(
            shell,
            &exe,
            &shim_dir_abs,
            &real_path,
            &selected_agents,
            &config.tool_commands,
        )
    );
    Ok(())
}

pub fn print_deactivate() {
    print!("{}", render_deactivate_script(detect_shell_kind()));
}

pub fn run_shim(name: &str, args: &[String], config: &Config) -> Result<i32, AppError> {
    let real_cmd = resolve_real_command(name)?;
    let in_agent = env::var_os("TKE_AGENT_CONTEXT").is_some();
    let is_agent = config.is_agent_command(name);
    let is_tool = config.is_tool_command(name);

    if is_agent {
        return run_agent_command(name, &real_cmd, args, config);
    }

    if in_agent && is_tool && !config.is_whitelisted(name, args) {
        return run_tool_command(name, &real_cmd, args, config);
    }

    passthrough(&real_cmd, args, None, None, false)
}

fn parse_activate(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut agents = Vec::new();
    let mut shim_dir = None;
    let mut shell = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        if arg == "--shim-dir" {
            let value = iter.next().ok_or_else(|| {
                AppError::Usage(format!("missing value for --shim-dir\n\n{}", usage()))
            })?;
            shim_dir = Some(PathBuf::from(value));
            continue;
        }
        if arg == "--shell" {
            let value = iter.next().ok_or_else(|| {
                AppError::Usage(format!("missing value for --shell\n\n{}", usage()))
            })?;
            shell = Some(ShellKind::parse(&value).ok_or_else(|| {
                AppError::Usage(format!("unsupported shell `{value}`\n\n{}", usage()))
            })?);
            continue;
        }
        agents.push(arg);
    }
    Ok(Dispatch::Activate {
        agents,
        shim_dir,
        shell,
    })
}

fn parse_capture_interactive(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut source = None;
    let mut output = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --source\n\n{}", usage()))
                })?;
                source = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --output\n\n{}", usage()))
                })?;
                output = Some(PathBuf::from(value));
            }
            other => {
                return Err(AppError::Usage(format!(
                    "unknown capture-interactive arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::CaptureInteractive { source, output })
}

fn parse_compare_rollout(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut source = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --source\n\n{}", usage()))
                })?;
                source = Some(PathBuf::from(value));
            }
            other => {
                return Err(AppError::Usage(format!(
                    "unknown compare-rollout arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::CompareRollout { source })
}

fn parse_shim_exec(args: Vec<String>) -> Result<Dispatch, AppError> {
    let name = args
        .get(2)
        .cloned()
        .ok_or_else(|| AppError::Usage("missing shim command name".to_owned()))?;
    Ok(Dispatch::ShimExec {
        name,
        args: args.into_iter().skip(3).collect(),
    })
}

fn parse_benchmark_commands(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut check = false;
    for arg in args.into_iter().skip(2) {
        match arg.as_str() {
            "--check" => check = true,
            other => {
                return Err(AppError::Usage(format!(
                    "unknown benchmark-commands arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::BenchmarkCommands { check })
}

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
    let rewritten = rewrite_codex_jsonl(&raw, config)?;
    let raw_stats = collect_rollout_output_stats(&raw, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    let report =
        RolloutCompareReport::from_stats(&source, rewritten.is_some(), raw_stats, rewritten_stats);
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

pub fn benchmark_commands(config: &Config, check: bool) -> Result<(), AppError> {
    let report = build_benchmark_report(config)?;
    if check {
        benchmark_report_check(&report)?;
    }
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn create_shims(shim_dir: &Path, agents: &[String], tools: &[String]) -> Result<(), AppError> {
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

fn run_agent_command(
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

    let output = capture_process(real_cmd, args, Some(envs.split_off(0)), stdin_payload, true)?;
    emit_stream(
        io::stdout(),
        &output.stdout,
        name,
        args,
        "stdout",
        classify_command(name, args),
        config.trim_agent_output,
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
        config,
    )?;
    Ok(exit_code(output.status))
}

fn run_tool_command(
    name: &str,
    real_cmd: &Path,
    args: &[String],
    config: &Config,
) -> Result<i32, AppError> {
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
        classify_command(name, args),
        true,
        config,
    )?;
    emit_stream(
        io::stderr(),
        &output.stderr,
        name,
        args,
        "stderr",
        classify_command(name, args),
        true,
        config,
    )?;
    Ok(exit_code(output.status))
}

fn passthrough(
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
            stdin.write_all(&payload)?;
        }
    }
    let status = child.wait()?;
    Ok(exit_code(status))
}

fn capture_process(
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
            stdin.write_all(&payload)?;
        }
    }
    Ok(child.wait_with_output()?)
}

fn emit_stream<W: Write>(
    mut writer: W,
    bytes: &[u8],
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    normalize: bool,
    config: &Config,
) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Ok(());
    }

    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            writer.write_all(bytes)?;
            return Ok(());
        }
    };

    if !normalize {
        writer.write_all(text.as_bytes())?;
        return Ok(());
    }

    if name == "codex" && stream == "stdout" {
        if let Some(rewritten) = rewrite_codex_jsonl(text, config)? {
            writer.write_all(rewritten.as_bytes())?;
            return Ok(());
        }
    }

    let Some(payload) = maybe_normalize_text(name, args, stream, kind, text, config, None)? else {
        writer.write_all(text.as_bytes())?;
        return Ok(());
    };
    writer.write_all(config.json_prefix.as_bytes())?;
    writer.write_all(payload.as_bytes())?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn maybe_normalize_text(
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
fn normalize_text(
    name: &str,
    args: &[String],
    stream: &str,
    kind: CommandKind,
    text: &str,
    config: &Config,
) -> Result<String, AppError> {
    normalize_text_with_stage(name, args, stream, kind, text, config, None)
}

fn normalize_text_with_stage(
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
        compute_omitted_ranges(total_lines, &kept_ranges)
    } else {
        Vec::new()
    };

    let envelope = TrimEnvelope {
        v: 1,
        cmd: name.to_owned(),
        a: compact_args(args),
        k: kind.as_str().to_owned(),
        sc: selected_stage.map(|(name, _)| name.to_owned()),
        sr: selected_stage.map(|(_, role)| role.to_owned()),
        p: profile.as_str().to_owned(),
        s: stream.to_owned(),
        t: forced,
        h: head,
        ta: tail,
        m: matches,
        o: omitted,
        st: TrimStats {
            tb: total_bytes,
            tl: total_lines,
            el: emitted_lines,
        },
        tb: table,
        pl: pathlist,
        b: body,
    };
    Ok(serde_json::to_string(&envelope)?)
}

fn rewrite_codex_jsonl(text: &str, config: &Config) -> Result<Option<String>, AppError> {
    let mut changed = false;
    let mut out = Vec::new();
    let mut tool_calls = HashMap::new();

    for line in text.lines() {
        let mut value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        if rewrite_codex_event(&mut value, &mut tool_calls, config)? {
            changed = true;
        }
        out.push(serde_json::to_string(&value)?);
    }

    if !changed {
        return Ok(None);
    }
    Ok(Some(out.join("\n") + "\n"))
}

fn rewrite_codex_event(
    value: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
    config: &Config,
) -> Result<bool, AppError> {
    if let Some(response) = value.get_mut("payload")
        && let Some(response_type) = response.get("type").and_then(|v| v.as_str())
    {
        return match response_type {
            "function_call" => {
                record_tool_call(response, tool_calls);
                Ok(false)
            }
            "function_call_output" => rewrite_tool_call_output(response, tool_calls, config),
            _ => Ok(false),
        };
    }

    let Some(item) = value.get_mut("item") else {
        return Ok(false);
    };
    let Some(item_type) = item.get("type").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    if item_type != "command_execution" {
        return Ok(false);
    }

    let command = item
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let parsed = parse_command_execution(command);
    rewrite_command_item_fields(item, &parsed, config)
}

struct PendingToolCall {
    tool_name: String,
    parsed: Option<ParsedCommand>,
}

fn record_tool_call(
    payload: &serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
) {
    let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) else {
        return;
    };
    let tool_name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let parsed = if tool_name == "exec_command" {
        payload
            .get("arguments")
            .and_then(|v| v.as_str())
            .and_then(parse_exec_command_args)
            .map(|cmd| parse_command_execution(&cmd))
    } else {
        None
    };
    tool_calls.insert(call_id.to_owned(), PendingToolCall { tool_name, parsed });
}

fn rewrite_tool_call_output(
    payload: &mut serde_json::Value,
    tool_calls: &mut HashMap<String, PendingToolCall>,
    config: &Config,
) -> Result<bool, AppError> {
    let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    let Some(pending) = tool_calls.get(call_id) else {
        return Ok(false);
    };
    if pending.tool_name != "exec_command" {
        return Ok(false);
    }
    let Some(parsed) = pending.parsed.as_ref() else {
        return Ok(false);
    };
    let Some(existing) = payload.get("output").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    if existing.is_empty() || existing.starts_with(&config.json_prefix) {
        return Ok(false);
    }
    let Some(actual_output) = extract_exec_command_output(existing) else {
        return Ok(false);
    };
    if actual_output.is_empty() {
        return Ok(false);
    }

    let selected = parsed.selected_stage();
    let kind = classify_command(&selected.name, &selected.args);
    let stream = if looks_like_stderr_only_exec_output(existing) {
        "stderr"
    } else {
        "stdout"
    };
    let Some(normalized) = maybe_normalize_text(
        &selected.name,
        &selected.args,
        stream,
        kind,
        actual_output,
        config,
        Some((&selected.name, selected.role.as_str())),
    )?
    else {
        return Ok(false);
    };
    let wrapped = format!("{}{}", config.json_prefix, normalized);
    let Some(obj) = payload.as_object_mut() else {
        return Ok(false);
    };
    obj.insert("output".to_owned(), serde_json::Value::String(wrapped));
    Ok(true)
}

fn parse_exec_command_args(arguments: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    value.get("cmd")?.as_str().map(ToOwned::to_owned)
}

fn extract_exec_command_output(raw: &str) -> Option<&str> {
    raw.split_once("\nOutput:\n").map(|(_, tail)| tail)
}

fn looks_like_stderr_only_exec_output(raw: &str) -> bool {
    raw.contains("\nProcess exited with code ")
        && raw.contains("\nOutput:\n")
        && (raw.contains("\nerror:")
            || raw.contains("\nwarning:")
            || raw.contains("Traceback ")
            || raw.contains("Is a directory"))
}

fn rewrite_command_item_fields(
    item: &mut serde_json::Value,
    parsed: &ParsedCommand,
    config: &Config,
) -> Result<bool, AppError> {
    let mut changed = false;
    let Some(obj) = item.as_object_mut() else {
        return Ok(false);
    };

    for field in ["aggregated_output", "stdout", "stderr", "output"] {
        let Some(existing) = obj.get(field).and_then(|value| value.as_str()) else {
            continue;
        };
        if existing.is_empty() || existing.starts_with(&config.json_prefix) {
            continue;
        }

        let stream = if field == "stderr" {
            "stderr"
        } else {
            "stdout"
        };
        let selected = parsed.selected_stage();
        let kind = classify_command(&selected.name, &selected.args);
        let Some(normalized) = maybe_normalize_text(
            &selected.name,
            &selected.args,
            stream,
            kind,
            existing,
            config,
            Some((&selected.name, selected.role.as_str())),
        )?
        else {
            continue;
        };
        let wrapped = format!("{}{}", config.json_prefix, normalized);
        obj.insert(field.to_owned(), serde_json::Value::String(wrapped));
        changed = true;
    }

    Ok(changed)
}

fn parse_command_execution(command: &str) -> ParsedCommand {
    parse_command_execution_inner(command, 0)
}

fn parse_command_execution_inner(command: &str, depth: usize) -> ParsedCommand {
    if depth > 3 {
        return ParsedCommand::default();
    }

    let shell_body = extract_shell_body(command).unwrap_or(command).trim();
    let cleaned = shell_body.trim_matches(|c| c == '\'' || c == '"');
    let tokens = shell_like_split(cleaned);
    if tokens.is_empty() {
        return ParsedCommand::default();
    }

    if let Some((cmd_idx, cmd_token)) = first_real_token_with_index(&tokens) {
        let base = base_name(cmd_token);
        if matches!(base.as_str(), "sh" | "bash" | "zsh" | "fish")
            && let Some(nested) = extract_nested_shell_invocation(&tokens[cmd_idx..])
        {
            let parsed = parse_command_execution_inner(&nested, depth + 1);
            if !parsed.selected_stage().name.is_empty() {
                return parsed;
            }
        }
    }

    let stages = split_command_segments(&tokens)
        .into_iter()
        .enumerate()
        .map(|(index, segment)| ParsedStage::from_tokens(segment, index))
        .collect::<Vec<_>>();
    ParsedCommand::new(stages, Some(cleaned.to_owned()))
}

fn extract_shell_body(command: &str) -> Option<&str> {
    for marker in [" -lc ", " -c "] {
        if let Some(pos) = command.find(marker) {
            return Some(&command[(pos + marker.len())..]);
        }
    }
    None
}

fn split_command_segments(tokens: &[String]) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for token in tokens {
        if matches!(
            token.as_str(),
            "|" | "||" | "&&" | ";" | "2>&1" | "1>" | "2>"
        ) {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(token.clone());
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn looks_like_env_assignment(token: &str) -> bool {
    let Some((key, _)) = token.split_once('=') else {
        return false;
    };
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn extract_nested_shell_invocation(tokens: &[String]) -> Option<String> {
    for window in tokens.windows(2) {
        if matches!(window[0].as_str(), "-c" | "-lc") {
            return Some(window[1].clone());
        }
    }
    None
}

fn shell_like_split(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in input.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => {
                escape = true;
            }
            '\'' | '"' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[derive(Clone)]
struct ParsedCommand {
    stages: Vec<ParsedStage>,
    #[allow(dead_code)]
    shell: Option<String>,
}

impl ParsedCommand {
    fn new(stages: Vec<ParsedStage>, shell: Option<String>) -> Self {
        Self { stages, shell }
    }

    fn selected_stage(&self) -> ParsedStage {
        self.stages
            .iter()
            .max_by_key(|stage| stage.role_score())
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for ParsedCommand {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            shell: None,
        }
    }
}

fn first_real_token_with_index(tokens: &[String]) -> Option<(usize, &str)> {
    let mut idx = 0;
    while idx < tokens.len() {
        let token = tokens[idx].as_str();
        if token == "env" {
            idx += 1;
            while idx < tokens.len() && looks_like_env_assignment(tokens[idx].as_str()) {
                idx += 1;
            }
            continue;
        }
        if matches!(token, "command" | "builtin" | "nohup" | "time") {
            idx += 1;
            continue;
        }
        if looks_like_env_assignment(token) {
            idx += 1;
            continue;
        }
        return Some((idx, token));
    }
    None
}

fn parse_xargs_payload(tokens: &[String]) -> Option<(String, Vec<String>)> {
    let (idx, token) = first_real_token_with_index(tokens)?;
    let name = base_name(token);
    let args = tokens.iter().skip(idx + 1).cloned().collect();
    Some((name, args))
}

#[derive(Clone, Default)]
struct ParsedStage {
    name: String,
    args: Vec<String>,
    role: StageRole,
    index: usize,
}

impl ParsedStage {
    fn from_tokens(tokens: Vec<String>, index: usize) -> Self {
        let Some((cmd_idx, cmd_token)) = first_real_token_with_index(&tokens) else {
            return Self {
                name: String::new(),
                args: Vec::new(),
                role: StageRole::Unknown,
                index,
            };
        };
        let base = base_name(cmd_token);
        if base == "xargs"
            && let Some((payload_name, payload_args)) = parse_xargs_payload(&tokens[cmd_idx + 1..])
        {
            let role = classify_stage_role(&payload_name);
            return Self {
                name: payload_name,
                args: payload_args,
                role,
                index,
            };
        }

        let args = tokens.iter().skip(cmd_idx + 1).cloned().collect::<Vec<_>>();
        let role = classify_stage_role(&base);
        Self {
            name: base,
            args,
            role,
            index,
        }
    }

    fn role_score(&self) -> (i32, i32, i32) {
        let role_weight = self.role.weight();
        let arg_weight = self
            .args
            .iter()
            .filter(|arg| arg.starts_with('/') || arg.contains('.'))
            .count() as i32;
        let position_weight = -(self.index as i32);
        (role_weight, arg_weight, position_weight)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum StageRole {
    Source,
    Search,
    Filter,
    Summarize,
    Build,
    #[default]
    Unknown,
}

impl StageRole {
    fn weight(self) -> i32 {
        match self {
            Self::Search => 500,
            Self::Source => 400,
            Self::Filter => 250,
            Self::Summarize => 150,
            Self::Build => 120,
            Self::Unknown => 0,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Search => "search",
            Self::Filter => "filter",
            Self::Summarize => "summarize",
            Self::Build => "build",
            Self::Unknown => "unknown",
        }
    }
}

fn classify_stage_role(name: &str) -> StageRole {
    match name {
        "rg" | "grep" | "find" | "fd" | "tree" => StageRole::Search,
        "cat" | "git" | "bat" | "nl" | "ls" => StageRole::Source,
        "sed" | "awk" | "perl" | "cut" | "sort" | "uniq" | "tr" => StageRole::Filter,
        "head" | "tail" | "wc" => StageRole::Summarize,
        "cargo" | "pytest" | "npm" | "pnpm" | "yarn" => StageRole::Build,
        _ => StageRole::Unknown,
    }
}

fn collect_profile_chunks(
    lines: &[&str],
    terms: &[String],
    profile: TrimProfile,
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    match profile {
        TrimProfile::Diff => collect_diff_chunks(lines, terms, limits),
        TrimProfile::Search => collect_search_chunks(lines, terms, limits),
        TrimProfile::PathList => Vec::new(),
        TrimProfile::Log => collect_log_chunks(lines, terms, limits),
        TrimProfile::Table => Vec::new(),
        TrimProfile::Stacktrace => collect_stacktrace_chunks(lines, limits),
        TrimProfile::File => collect_file_chunks(lines, terms, limits),
        TrimProfile::Generic => collect_term_chunks(
            lines,
            terms,
            "hit",
            limits.match_context,
            limits.max_matches,
        ),
    }
}

fn collect_term_chunks(
    lines: &[&str],
    terms: &[String],
    label: &str,
    context: usize,
    max_matches: usize,
) -> Vec<MatchChunk> {
    if terms.is_empty() {
        return Vec::new();
    }
    let lower_terms = terms
        .iter()
        .map(|term| term.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut used = Vec::<(usize, usize)>::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        if !lower_terms.iter().any(|term| lower.contains(term)) {
            continue;
        }
        let start = idx.saturating_sub(context);
        let end = usize::min(lines.len(), idx + context + 1);
        if push_chunk(&mut out, &mut used, lines, start, end, label) && out.len() >= max_matches {
            break;
        }
    }
    out
}

fn compute_omitted_ranges(total_lines: usize, kept_ranges: &[(usize, usize)]) -> Vec<[usize; 2]> {
    let mut omitted = Vec::new();
    let mut cursor = 0;
    for (start, end) in kept_ranges {
        if *start > cursor {
            omitted.push([cursor, *start]);
        }
        cursor = *end;
    }
    if cursor < total_lines {
        omitted.push([cursor, total_lines]);
    }
    omitted
}

fn collect_diff_chunks(lines: &[&str], terms: &[String], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("index ")
        {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "file") && out.len() >= 4 {
                break;
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("@@") {
            let start = idx;
            let end = usize::min(lines.len(), idx + 3);
            if push_chunk(&mut out, &mut used, lines, start, end, "hunk") && out.len() >= 6 {
                break;
            }
        }
    }

    for chunk in collect_term_chunks(
        lines,
        terms,
        "change",
        limits.match_context,
        limits.max_matches,
    ) {
        if push_existing_chunk(&mut out, &mut used, chunk) && out.len() >= limits.max_matches {
            break;
        }
    }
    out
}

fn collect_search_chunks(
    lines: &[&str],
    terms: &[String],
    limits: ProfileLimits,
) -> Vec<MatchChunk> {
    let grouped = collect_grouped_search_chunks(lines, terms, limits.max_matches);
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

fn collect_grouped_search_chunks(
    lines: &[&str],
    _terms: &[String],
    max_matches: usize,
) -> Vec<MatchChunk> {
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

fn collect_log_chunks(lines: &[&str], terms: &[String], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();
    let folds = detect_repeated_runs(lines);

    for (idx, line) in lines.iter().enumerate() {
        if !is_log_signal(line, terms) {
            continue;
        }
        let start = idx;
        let end = usize::min(lines.len(), idx + 1);
        if push_chunk(&mut out, &mut used, lines, start, end, "signal")
            && out.len() >= limits.max_matches
        {
            break;
        }
    }

    for fold in folds {
        if push_fold_chunk(&mut out, &mut used, &fold) && out.len() >= limits.max_matches {
            break;
        }
    }

    if out.is_empty() {
        let start = lines.len().saturating_sub(limits.tail_lines);
        for idx in start..lines.len() {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "tail")
                && out.len() >= limits.max_matches
            {
                break;
            }
        }
    }
    out
}

fn collect_file_chunks(lines: &[&str], terms: &[String], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for (start, end) in detect_code_blocks(lines) {
        if push_chunk(&mut out, &mut used, lines, start, end, "block") && out.len() >= 2 {
            break;
        }
    }

    for chunk in collect_term_chunks(
        lines,
        terms,
        "snippet",
        limits.match_context,
        limits.max_matches,
    ) {
        if push_existing_chunk(&mut out, &mut used, chunk) && out.len() >= limits.max_matches {
            break;
        }
    }

    if out.is_empty() {
        let midpoint = lines.len() / 2;
        let start = midpoint.saturating_sub(usize::min(4, midpoint));
        let end = usize::min(lines.len(), start + 8);
        push_chunk(&mut out, &mut used, lines, start, end, "block");
    }
    out
}

fn collect_stacktrace_chunks(lines: &[&str], limits: ProfileLimits) -> Vec<MatchChunk> {
    let mut out = Vec::new();
    let mut used = Vec::<(usize, usize)>::new();

    for (idx, line) in lines.iter().enumerate() {
        if is_stack_summary(line) {
            if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "summary") {
                break;
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        if !is_stack_frame(line) {
            continue;
        }
        if push_chunk(&mut out, &mut used, lines, idx, idx + 1, "frame")
            && out.len() >= limits.max_matches
        {
            break;
        }
    }
    out
}

fn push_existing_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    chunk: MatchChunk,
) -> bool {
    let start = chunk.r[0];
    let end = chunk.r[1];
    if used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(chunk);
    true
}

fn push_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    lines: &[&str],
    start: usize,
    end: usize,
    label: &str,
) -> bool {
    if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(MatchChunk {
        k: label.to_owned(),
        r: [start, end],
        l: lines[start..end]
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
    });
    true
}

fn push_fold_chunk(
    out: &mut Vec<MatchChunk>,
    used: &mut Vec<(usize, usize)>,
    fold: &RepeatedRun,
) -> bool {
    let [start, end] = fold.range;
    if start >= end || used.iter().any(|(s, e)| start < *e && end > *s) {
        return false;
    }
    used.push((start, end));
    out.push(MatchChunk {
        k: "fold".to_owned(),
        r: fold.range,
        l: vec![format!(
            "repeat:{} count:{} sample:{}",
            end.saturating_sub(start),
            fold.count,
            fold.sample
        )],
    });
    true
}

fn collect_kept_ranges(
    total_lines: usize,
    head: &[String],
    tail: &[String],
    matches: &[MatchChunk],
) -> Vec<(usize, usize)> {
    let mut kept = Vec::new();
    if !head.is_empty() {
        kept.push((0, head.len()));
    }
    if !tail.is_empty() {
        kept.push((total_lines.saturating_sub(tail.len()), total_lines));
    }
    for chunk in matches {
        kept.push((chunk.r[0], chunk.r[1]));
    }
    kept
}

fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut merged = Vec::<(usize, usize)>::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn take_head(lines: &[&str], count: usize) -> Vec<String> {
    lines
        .iter()
        .take(count)
        .map(|line| (*line).to_owned())
        .collect()
}

fn take_tail(lines: &[&str], count: usize) -> Vec<String> {
    lines
        .iter()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| (*line).to_owned())
        .collect()
}

fn collect_table_summary(
    name: &str,
    args: &[String],
    lines: &[&str],
    terms: &[String],
) -> Option<TableSummary> {
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

fn collect_table_kept_ranges(table: &TableSummary) -> Vec<(usize, usize)> {
    table.r.iter().map(|row| (row.i, row.i + 1)).collect()
}

fn collect_path_list_summary(lines: &[&str]) -> Option<PathListSummary> {
    let entries = detect_path_entries(lines)?;
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
                .map(|entry| entry.value.clone())
                .collect::<Vec<_>>();
            if count > 2
                && let Some(last) = rows.last()
            {
                let last_value = last.value.clone();
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
        .take(3)
        .map(|entry| PathRow {
            i: entry.line_index,
            v: entry.value.clone(),
        })
        .collect::<Vec<_>>();
    if entries.len() > 3 {
        for entry in entries.iter().rev().take(2).rev() {
            if rows.iter().all(|row| row.i != entry.line_index) {
                rows.push(PathRow {
                    i: entry.line_index,
                    v: entry.value.clone(),
                });
            }
        }
    }

    Some(PathListSummary {
        rc: entries.len(),
        b: buckets,
        r: rows,
    })
}

fn collect_path_list_kept_ranges(pathlist: &PathListSummary) -> Vec<(usize, usize)> {
    pathlist.r.iter().map(|row| (row.i, row.i + 1)).collect()
}

fn match_terms(name: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for token in args
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .flat_map(|arg| split_token(arg))
        .chain(split_token(name))
        .chain(
            ["error", "failed", "panic", "warning", "exception", "todo"]
                .into_iter()
                .map(str::to_owned),
        )
    {
        let normalized = token.to_ascii_lowercase();
        if normalized.len() < 3 {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
        if out.len() >= 8 {
            break;
        }
    }

    out
}

fn split_token(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn compact_args(args: &[String]) -> Vec<String> {
    args.iter()
        .take(6)
        .map(|arg| {
            if arg.len() > 80 {
                format!("{}...", &arg[..80])
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn read_stdin_if_piped() -> Result<Option<Vec<u8>>, AppError> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = Vec::new();
    io::stdin().read_to_end(&mut buf)?;
    Ok(Some(buf))
}

fn resolve_real_command(name: &str) -> Result<PathBuf, AppError> {
    let shim_dir = env::var("TKE_SHIM_DIR").unwrap_or_default();
    let real_path = real_path_string();
    let shim_dir = PathBuf::from(shim_dir);

    for dir in env::split_paths(&real_path) {
        if !shim_dir.as_os_str().is_empty() && dir == shim_dir {
            continue;
        }
        for candidate_name in candidate_command_names(name) {
            let candidate = dir.join(candidate_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(AppError::MissingRealCommand(name.to_owned()))
}

fn real_path_string() -> String {
    env::var("TKE_REAL_PATH").unwrap_or_else(|_| env::var("PATH").unwrap_or_default())
}

fn classify_command(name: &str, args: &[String]) -> CommandKind {
    match name {
        "ps" | "ss" | "netstat" | "systemctl" => CommandKind::Log,
        "cat" | "sed" | "head" | "tail" | "bat" | "nl" | "awk" | "cut" => CommandKind::File,
        "rg" | "grep" | "find" | "fd" | "tree" => CommandKind::Search,
        "ls" if args
            .iter()
            .any(|arg| arg == "-l" || arg == "-la" || arg == "-lh" || arg == "-al") =>
        {
            CommandKind::Log
        }
        "ls" | "sort" | "uniq" | "wc" | "xargs" => CommandKind::Generic,
        "git" if args.first().map(String::as_str) == Some("diff") => CommandKind::Diff,
        "cargo" | "pytest" | "npm" | "pnpm" | "yarn" => CommandKind::Log,
        _ => CommandKind::Generic,
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn base_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned()
}

fn candidate_config_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("TKE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".tke").join("config.json"))
}

fn parse_usize(raw: &str, fallback: usize) -> usize {
    raw.parse().unwrap_or(fallback)
}

fn csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn shell_escape(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn powershell_escape(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

fn cmd_escape(raw: &str) -> String {
    raw.replace('"', "\"\"")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

impl ShellKind {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "sh" | "bash" | "zsh" | "fish" | "posix" => Some(Self::Posix),
            "powershell" | "pwsh" | "ps" => Some(Self::PowerShell),
            "cmd" | "cmd.exe" => Some(Self::Cmd),
            _ => None,
        }
    }
}

fn detect_shell_kind() -> ShellKind {
    if let Ok(value) = env::var("TKE_SHELL")
        && let Some(shell) = ShellKind::parse(&value)
    {
        return shell;
    }
    if cfg!(windows) {
        if env::var_os("PSModulePath").is_some() {
            return ShellKind::PowerShell;
        }
        return ShellKind::Cmd;
    }
    ShellKind::Posix
}

fn render_activate_script(
    shell: ShellKind,
    exe: &Path,
    shim_dir: &Path,
    real_path: &str,
    agents: &[String],
    tools: &[String],
) -> String {
    let exe = exe.to_string_lossy();
    let shim_dir = shim_dir.to_string_lossy();
    let agent_csv = agents.join(",");
    let tool_csv = tools.join(",");
    let sep = shell_path_sep(shell);
    match shell {
        ShellKind::Posix => {
            [
                format!("export TKE_BIN={}", shell_escape(&exe)),
                format!("export TKE_SHIM_DIR={}", shell_escape(&shim_dir)),
                format!("export TKE_REAL_PATH={}", shell_escape(real_path)),
                format!("export TKE_AGENT_CMDS={}", shell_escape(&agent_csv)),
                format!("export TKE_TOOL_CMDS={}", shell_escape(&tool_csv)),
                format!("export PATH={}:$PATH", shell_escape(&shim_dir)),
            ]
            .join("\n")
                + "\n"
        }
        ShellKind::PowerShell => {
            [
                format!("$env:TKE_BIN = {}", powershell_escape(&exe)),
                format!("$env:TKE_SHIM_DIR = {}", powershell_escape(&shim_dir)),
                format!("$env:TKE_REAL_PATH = {}", powershell_escape(real_path)),
                format!("$env:TKE_AGENT_CMDS = {}", powershell_escape(&agent_csv)),
                format!("$env:TKE_TOOL_CMDS = {}", powershell_escape(&tool_csv)),
                format!(
                    "$env:PATH = {} + '{}' + $env:PATH",
                    powershell_escape(&shim_dir),
                    sep
                ),
            ]
            .join("\n")
                + "\n"
        }
        ShellKind::Cmd => {
            [
                format!("set \"TKE_BIN={}\"", cmd_escape(&exe)),
                format!("set \"TKE_SHIM_DIR={}\"", cmd_escape(&shim_dir)),
                format!("set \"TKE_REAL_PATH={}\"", cmd_escape(real_path)),
                format!("set \"TKE_AGENT_CMDS={}\"", cmd_escape(&agent_csv)),
                format!("set \"TKE_TOOL_CMDS={}\"", cmd_escape(&tool_csv)),
                format!("set \"PATH={shim_dir}{sep}%PATH%\""),
            ]
            .join("\r\n")
                + "\r\n"
        }
    }
}

fn render_deactivate_script(shell: ShellKind) -> String {
    match shell {
        ShellKind::Posix => [
            "if [ -n \"${TKE_REAL_PATH:-}\" ]; then",
            "  export PATH=\"$TKE_REAL_PATH\"",
            "fi",
            "unset TKE_BIN TKE_SHIM_DIR TKE_REAL_PATH TKE_AGENT_CMDS TKE_TOOL_CMDS",
        ]
        .join("\n")
            + "\n",
        ShellKind::PowerShell => [
            "if ($env:TKE_REAL_PATH) { $env:PATH = $env:TKE_REAL_PATH }".to_owned(),
            "Remove-Item Env:TKE_BIN,Env:TKE_SHIM_DIR,Env:TKE_REAL_PATH,Env:TKE_AGENT_CMDS,Env:TKE_TOOL_CMDS -ErrorAction SilentlyContinue".to_owned(),
        ]
        .join("\n")
            + "\n",
        ShellKind::Cmd => [
            "if defined TKE_REAL_PATH set \"PATH=%TKE_REAL_PATH%\"".to_owned(),
            "set TKE_BIN=".to_owned(),
            "set TKE_SHIM_DIR=".to_owned(),
            "set TKE_REAL_PATH=".to_owned(),
            "set TKE_AGENT_CMDS=".to_owned(),
            "set TKE_TOOL_CMDS=".to_owned(),
        ]
        .join("\r\n")
            + "\r\n",
    }
}

fn shell_path_sep(shell: ShellKind) -> char {
    match shell {
        ShellKind::Posix => ':',
        ShellKind::PowerShell | ShellKind::Cmd => ';',
    }
}

fn create_single_shim(shim_dir: &Path, exe: &Path, name: &str) -> Result<(), AppError> {
    if cfg!(windows) {
        create_windows_cmd_shim(shim_dir, exe, name)
    } else {
        let link = shim_dir.join(name);
        if link.exists() {
            fs::remove_file(&link)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(exe, &link)?;
        Ok(())
    }
}

fn create_windows_cmd_shim(shim_dir: &Path, exe: &Path, name: &str) -> Result<(), AppError> {
    let wrapper = shim_dir.join(format!("{name}.cmd"));
    if wrapper.exists() {
        fs::remove_file(&wrapper)?;
    }
    let exe = exe.to_string_lossy().replace('"', "\"\"");
    let body = format!("@echo off\r\n\"{exe}\" shim \"%~n0\" %*\r\nexit /b %ERRORLEVEL%\r\n");
    fs::write(wrapper, body)?;
    Ok(())
}

fn candidate_command_names(name: &str) -> Vec<OsString> {
    if !cfg!(windows) {
        return vec![OsString::from(name)];
    }
    let raw = OsStr::new(name);
    let has_ext = Path::new(raw).extension().is_some();
    let mut names = Vec::new();
    names.push(raw.to_os_string());
    if has_ext {
        return names;
    }
    let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    for ext in pathext.split(';').filter(|ext| !ext.is_empty()) {
        names.push(OsString::from(format!("{name}{ext}")));
        names.push(OsString::from(format!(
            "{name}{}",
            ext.to_ascii_lowercase()
        )));
    }
    names
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

struct InteractiveTracker {
    sessions_dir: PathBuf,
    started_at_ms: u128,
}

impl InteractiveTracker {
    fn start() -> Result<Self, AppError> {
        let sessions_dir = codex_sessions_dir().ok_or_else(|| {
            AppError::Usage("tke could not resolve CODEX_HOME sessions dir".to_owned())
        })?;
        Ok(Self {
            sessions_dir,
            started_at_ms: now_millis(),
        })
    }

    fn finish(&self, config: &Config) -> Result<(), AppError> {
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

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

fn rewrite_rollout_to_output(
    source: &Path,
    output_dir: &Path,
    config: &Config,
) -> Result<(), AppError> {
    let raw = fs::read_to_string(source)?;
    let Some(rewritten) = rewrite_codex_jsonl(&raw, config)? else {
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

#[derive(Default, Clone, Copy)]
struct RolloutOutputStats {
    fields: usize,
    bytes: usize,
    approx_tokens: usize,
}

#[derive(Serialize)]
struct RolloutCompareReport {
    v: u8,
    source: String,
    changed: bool,
    raw_fields: usize,
    raw_bytes: usize,
    raw_tokens: usize,
    rewritten_fields: usize,
    rewritten_bytes: usize,
    rewritten_tokens: usize,
    bytes_saved: isize,
    tokens_saved: isize,
    bytes_saved_ratio: f64,
    tokens_saved_ratio: f64,
}

impl RolloutCompareReport {
    fn from_stats(
        source: &Path,
        changed: bool,
        raw: RolloutOutputStats,
        rewritten: RolloutOutputStats,
    ) -> Self {
        let bytes_saved = raw.bytes as isize - rewritten.bytes as isize;
        let tokens_saved = raw.approx_tokens as isize - rewritten.approx_tokens as isize;
        let bytes_saved_ratio = ratio(bytes_saved, raw.bytes);
        let tokens_saved_ratio = ratio(tokens_saved, raw.approx_tokens);
        Self {
            v: 1,
            source: source.display().to_string(),
            changed,
            raw_fields: raw.fields,
            raw_bytes: raw.bytes,
            raw_tokens: raw.approx_tokens,
            rewritten_fields: rewritten.fields,
            rewritten_bytes: rewritten.bytes,
            rewritten_tokens: rewritten.approx_tokens,
            bytes_saved,
            tokens_saved,
            bytes_saved_ratio,
            tokens_saved_ratio,
        }
    }
}

#[derive(Serialize)]
struct BenchmarkReport {
    v: u8,
    cases: Vec<BenchmarkCaseReport>,
    corpus: Vec<CorpusCaseReport>,
}

#[derive(Serialize)]
struct BenchmarkCaseReport {
    name: String,
    command: String,
    profile: String,
    expected: String,
    changed: bool,
    raw_bytes: usize,
    raw_tokens: usize,
    rewritten_bytes: usize,
    rewritten_tokens: usize,
    bytes_saved: isize,
    tokens_saved: isize,
    bytes_saved_ratio: f64,
    tokens_saved_ratio: f64,
}

#[derive(Serialize)]
struct CorpusCaseReport {
    source: String,
    changed: bool,
    raw_bytes: usize,
    raw_tokens: usize,
    rewritten_bytes: usize,
    rewritten_tokens: usize,
    bytes_saved: isize,
    tokens_saved: isize,
    bytes_saved_ratio: f64,
    tokens_saved_ratio: f64,
}

fn ratio(saved: isize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        saved as f64 / total as f64
    }
}

fn build_benchmark_report(config: &Config) -> Result<BenchmarkReport, AppError> {
    let cases = benchmark_specs()
        .into_iter()
        .map(|spec| benchmark_case_report(spec, config))
        .collect::<Result<Vec<_>, _>>()?;
    let corpus = benchmark_corpus_reports(config)?;
    Ok(BenchmarkReport {
        v: 1,
        cases,
        corpus,
    })
}

fn benchmark_case_report(
    spec: BenchmarkSpec,
    config: &Config,
) -> Result<BenchmarkCaseReport, AppError> {
    let payload = spec.sample;
    let jsonl = build_exec_rollout(&spec.command, &payload, &spec.call_id);
    let rewritten = rewrite_codex_jsonl(&jsonl, config)?;
    let raw_stats = collect_rollout_output_stats(&jsonl, config);
    let rewritten_text = rewritten.as_deref().unwrap_or(&jsonl);
    let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
    let report = RolloutCompareReport::from_stats(
        Path::new(&format!("/tmp/{}.jsonl", spec.name)),
        rewritten.is_some(),
        raw_stats,
        rewritten_stats,
    );
    Ok(BenchmarkCaseReport {
        name: spec.name,
        command: spec.command,
        profile: spec.profile,
        expected: spec.expected.as_str().to_owned(),
        changed: report.changed,
        raw_bytes: report.raw_bytes,
        raw_tokens: report.raw_tokens,
        rewritten_bytes: report.rewritten_bytes,
        rewritten_tokens: report.rewritten_tokens,
        bytes_saved: report.bytes_saved,
        tokens_saved: report.tokens_saved,
        bytes_saved_ratio: report.bytes_saved_ratio,
        tokens_saved_ratio: report.tokens_saved_ratio,
    })
}

fn benchmark_corpus_reports(config: &Config) -> Result<Vec<CorpusCaseReport>, AppError> {
    let mut reports = Vec::new();
    for source in discover_benchmark_corpus_paths()? {
        if !source.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&source)?;
        let rewritten = rewrite_codex_jsonl(&raw, config)?;
        let raw_stats = collect_rollout_output_stats(&raw, config);
        let rewritten_text = rewritten.as_deref().unwrap_or(&raw);
        let rewritten_stats = collect_rollout_output_stats(rewritten_text, config);
        let report = RolloutCompareReport::from_stats(
            &source,
            rewritten.is_some(),
            raw_stats,
            rewritten_stats,
        );
        reports.push(CorpusCaseReport {
            source: source.display().to_string(),
            changed: report.changed,
            raw_bytes: report.raw_bytes,
            raw_tokens: report.raw_tokens,
            rewritten_bytes: report.rewritten_bytes,
            rewritten_tokens: report.rewritten_tokens,
            bytes_saved: report.bytes_saved,
            tokens_saved: report.tokens_saved,
            bytes_saved_ratio: report.bytes_saved_ratio,
            tokens_saved_ratio: report.tokens_saved_ratio,
        });
    }
    Ok(reports)
}

fn discover_benchmark_corpus_paths() -> Result<Vec<PathBuf>, AppError> {
    let mut out = Vec::new();
    for candidate in [
        PathBuf::from(".tmp-benchmark-rollout.jsonl"),
        PathBuf::from(".tmp-real-large-rollout.jsonl"),
    ] {
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    let cwd = env::current_dir()?;
    let interactive = cwd.join(".tke").join("interactive");
    if interactive.is_dir() {
        for entry in fs::read_dir(interactive)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                out.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn benchmark_report_check(report: &BenchmarkReport) -> Result<(), AppError> {
    for case in &report.cases {
        match case.expected.as_str() {
            "compress" => {
                if case.tokens_saved_ratio < 0.20 {
                    return Err(AppError::Usage(format!(
                        "benchmark check failed: case `{}` saved too little token ratio ({:.3})",
                        case.name, case.tokens_saved_ratio
                    )));
                }
            }
            "pass_through" => {
                if case.changed || case.tokens_saved != 0 {
                    return Err(AppError::Usage(format!(
                        "benchmark check failed: case `{}` should pass through unchanged",
                        case.name
                    )));
                }
            }
            _ => {}
        }
    }
    for case in &report.corpus {
        if case.changed && case.tokens_saved_ratio < 0.10 {
            return Err(AppError::Usage(format!(
                "benchmark check failed: corpus `{}` rewrote with too little gain ({:.3})",
                case.source, case.tokens_saved_ratio
            )));
        }
    }
    Ok(())
}

pub fn package_release(config: &Config) -> Result<(), AppError> {
    let _ = config;
    let cwd = env::current_dir()?;
    let dist = cwd.join("dist");
    fs::create_dir_all(&dist)?;

    let exe = env::current_exe()?;
    let release_bin =
        if exe.ends_with("target/release/tke") || exe.ends_with("target\\release\\tke.exe") {
            exe
        } else {
            cwd.join("target")
                .join("release")
                .join(if cfg!(windows) { "tke.exe" } else { "tke" })
        };
    if !release_bin.is_file() {
        return Err(AppError::Usage(format!(
            "release binary not found at {}",
            release_bin.display()
        )));
    }

    let package_root = dist.join("tke-package");
    if package_root.exists() {
        fs::remove_dir_all(&package_root)?;
    }
    fs::create_dir_all(&package_root)?;
    fs::copy(
        &release_bin,
        package_root.join(release_bin.file_name().unwrap_or_else(|| OsStr::new("tke"))),
    )?;
    fs::copy(cwd.join("README.md"), package_root.join("README.md"))?;

    let archive = dist.join(if cfg!(windows) {
        "tke-release.zip"
    } else {
        "tke-release.tar.gz"
    });
    if archive.exists() {
        fs::remove_file(&archive)?;
    }

    let status = if cfg!(windows) {
        Command::new("zip")
            .arg("-r")
            .arg(&archive)
            .arg(".")
            .current_dir(&package_root)
            .status()?
    } else {
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&dist)
            .arg("tke-package")
            .status()?
    };
    if !status.success() {
        return Err(AppError::Usage(
            "failed to create release archive".to_owned(),
        ));
    }

    let sha = sha256_file(&archive)?;
    let checksum_path = archive.with_extension(format!(
        "{}sha256",
        archive
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!("{ext}."))
            .unwrap_or_default()
    ));
    fs::write(
        &checksum_path,
        format!(
            "{sha}  {}\n",
            archive
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("archive")
        ),
    )?;

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "v": 1,
            "archive": archive.display().to_string(),
            "sha256_file": checksum_path.display().to_string(),
            "sha256": sha
        }))?
    );
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err(AppError::Usage(format!(
            "failed to compute sha256 for {}",
            path.display()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned())
}

fn build_exec_rollout(command: &str, output: &str, call_id: &str) -> String {
    [
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "exec_command",
                "arguments": serde_json::json!({
                    "cmd": command,
                    "yield_time_ms": 1000
                }).to_string(),
                "call_id": call_id
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": call_id,
                "output": format!(
                    "Chunk ID: bench\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 1000\nOutput:\n{output}\n"
                )
            }
        })
        .to_string(),
    ]
    .join("\n")
}

fn benchmark_specs() -> Vec<BenchmarkSpec> {
    vec![
        BenchmarkSpec {
            name: "cat_code".to_owned(),
            command: "cat src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cat_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "sed_code".to_owned(),
            command: "sed -n 1,180p src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_sed_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "bat_code".to_owned(),
            command: "bat --style=plain src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_bat_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "nl_code".to_owned(),
            command: "nl -ba src/lib.rs | sed -n 1,180p".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_nl_1".to_owned(),
            sample: repeated_benchmark_numbered_code(180),
        },
        BenchmarkSpec {
            name: "awk_code".to_owned(),
            command: "awk '{print}' src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_awk_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "cut_code".to_owned(),
            command: "cut -c1-120 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cut_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "head_code".to_owned(),
            command: "head -n 180 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_head_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "tail_code".to_owned(),
            command: "tail -n 180 src/lib.rs".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_tail_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
        BenchmarkSpec {
            name: "rg_code".to_owned(),
            command: "rg -n 'fn|struct|impl|enum' src".to_owned(),
            profile: "search".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_rg_1".to_owned(),
            sample: repeated_benchmark_search(),
        },
        BenchmarkSpec {
            name: "grep_code".to_owned(),
            command: "grep -n 'fn\\|struct\\|impl\\|enum' src/lib.rs".to_owned(),
            profile: "search".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_grep_1".to_owned(),
            sample: repeated_benchmark_search(),
        },
        BenchmarkSpec {
            name: "find_paths".to_owned(),
            command: "find /root/project -type f | head -n 500".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_find_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "fd_paths".to_owned(),
            command: "fd . /root/project | head -n 500".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_fd_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "tree_paths".to_owned(),
            command: "tree -a -L 6 /root/project".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_tree_1".to_owned(),
            sample: repeated_benchmark_paths(240),
        },
        BenchmarkSpec {
            name: "sort_paths".to_owned(),
            command: "find /root/project -type f | sort".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_sort_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "uniq_paths".to_owned(),
            command: "find /root/project -type f | uniq".to_owned(),
            profile: "pathlist".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_uniq_1".to_owned(),
            sample: repeated_benchmark_paths(500),
        },
        BenchmarkSpec {
            name: "ls_long".to_owned(),
            command: "ls -l /root/project/src".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ls_1".to_owned(),
            sample: repeated_benchmark_ls_long(),
        },
        BenchmarkSpec {
            name: "wc_summary".to_owned(),
            command: "wc -l src/lib.rs README.md".to_owned(),
            profile: "generic".to_owned(),
            expected: BenchmarkExpectation::PassThrough,
            call_id: "bench_wc_1".to_owned(),
            sample: repeated_benchmark_wc(),
        },
        BenchmarkSpec {
            name: "git_diff".to_owned(),
            command: "git diff -- src/lib.rs".to_owned(),
            profile: "diff".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_git_1".to_owned(),
            sample: repeated_benchmark_diff(),
        },
        BenchmarkSpec {
            name: "cargo_build".to_owned(),
            command: "cargo build".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_cargo_1".to_owned(),
            sample: repeated_benchmark_build_log("cargo"),
        },
        BenchmarkSpec {
            name: "pytest_run".to_owned(),
            command: "pytest -q".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_pytest_1".to_owned(),
            sample: repeated_benchmark_build_log("pytest"),
        },
        BenchmarkSpec {
            name: "npm_test".to_owned(),
            command: "npm test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_npm_1".to_owned(),
            sample: repeated_benchmark_build_log("npm"),
        },
        BenchmarkSpec {
            name: "pnpm_test".to_owned(),
            command: "pnpm test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_pnpm_1".to_owned(),
            sample: repeated_benchmark_build_log("pnpm"),
        },
        BenchmarkSpec {
            name: "yarn_test".to_owned(),
            command: "yarn test".to_owned(),
            profile: "log".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_yarn_1".to_owned(),
            sample: repeated_benchmark_build_log("yarn"),
        },
        BenchmarkSpec {
            name: "ps_table".to_owned(),
            command: "ps aux --sort=-%cpu | head -n 25".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_ps_1".to_owned(),
            sample: repeated_benchmark_ps(),
        },
        BenchmarkSpec {
            name: "systemctl_table".to_owned(),
            command: "systemctl list-units --type=service --all --no-pager | head -n 40".to_owned(),
            profile: "table".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_systemctl_1".to_owned(),
            sample: repeated_benchmark_systemctl(),
        },
        BenchmarkSpec {
            name: "xargs_cat".to_owned(),
            command: "printf '/root/project/src/lib.rs\\n' | xargs cat".to_owned(),
            profile: "file".to_owned(),
            expected: BenchmarkExpectation::Compress,
            call_id: "bench_xargs_1".to_owned(),
            sample: repeated_benchmark_code(180),
        },
    ]
}

fn repeated_benchmark_code(lines: usize) -> String {
    (0..lines)
        .map(|idx| match idx % 6 {
            0 => format!("pub struct Struct{idx} {{"),
            1 => format!("    field_{idx}: usize,"),
            2 => "}".to_owned(),
            3 => format!("pub fn function_{idx}() {{"),
            4 => format!("    println!(\"{{}}\", {idx});"),
            _ => "}".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_numbered_code(lines: usize) -> String {
    repeated_benchmark_code(lines)
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("{:>6}\t{line}", idx + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_search() -> String {
    (0..140)
        .map(|idx| format!("src/lib.rs:{}:pub fn alpha_{}() {{}}", idx + 1, idx))
        .chain((0..80).map(|idx| format!("src/main.rs:{}:pub struct Beta{};", idx + 1, idx)))
        .chain((0..40).map(|idx| format!("tests/lib.rs:{}:impl Gamma{} {{}}", idx + 1, idx)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_paths(count: usize) -> String {
    (0..count)
        .map(|idx| format!("/root/project/target/debug/incremental/tke/build-artifact-{idx:04}.o"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repeated_benchmark_ls_long() -> String {
    let mut rows = vec!["total 160".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "-rw-r--r-- 1 root root {:>5} May 23 17:{:02} module_{idx:02}.rs",
            1024 + idx * 13,
            idx % 60
        ));
    }
    rows.join("\n")
}

fn repeated_benchmark_wc() -> String {
    [
        "  3816 115848 src/lib.rs",
        "    82   3580 README.md",
        "  3898 119428 total",
    ]
    .join("\n")
}

fn repeated_benchmark_diff() -> String {
    let mut rows = vec![
        "diff --git a/src/lib.rs b/src/lib.rs".to_owned(),
        "index 1234567..89abcde 100644".to_owned(),
        "--- a/src/lib.rs".to_owned(),
        "+++ b/src/lib.rs".to_owned(),
    ];
    for idx in 0..120 {
        rows.push(format!(
            "@@ -{},3 +{},6 @@ pub fn function_{}() {{",
            idx * 10 + 1,
            idx * 10 + 1,
            idx
        ));
        rows.push("-    old_call();".to_owned());
        rows.push("+    new_call();".to_owned());
        rows.push(format!("+    extra_line_{}();", idx));
        rows.push("+    trace_call();".to_owned());
        rows.push(" }".to_owned());
    }
    rows.join("\n")
}

fn repeated_benchmark_build_log(kind: &str) -> String {
    let mut rows = Vec::new();
    for idx in 0..120 {
        rows.push(format!("{kind}: step {idx:03} finished"));
    }
    rows.push(format!("{kind}: warning: deprecated config key"));
    rows.push(format!("{kind}: error: build failed at target 007"));
    rows.join("\n")
}

fn repeated_benchmark_ps() -> String {
    let mut rows = vec![
        "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND".to_owned(),
    ];
    for idx in 0..24 {
        rows.push(format!(
            "root        {:>4}  {:>3}.{:1}  1.2 357624 101588 pts/1   Sl+  08:08   0:00 /usr/bin/process-{} --flag value",
            3000 + idx,
            9 - (idx / 3),
            idx % 10,
            idx
        ));
    }
    rows.join("\n")
}

fn repeated_benchmark_systemctl() -> String {
    let mut rows =
        vec!["UNIT                         LOAD   ACTIVE SUB     DESCRIPTION".to_owned()];
    for idx in 0..40 {
        rows.push(format!(
            "service-{idx:02}.service      loaded active running Sample Service {idx:02}"
        ));
    }
    rows.join("\n")
}

fn collect_rollout_output_stats(text: &str, config: &Config) -> RolloutOutputStats {
    let mut stats = RolloutOutputStats::default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(item) = value.get("item") {
            collect_value_output_stats(
                item,
                &["aggregated_output", "stdout", "stderr", "output"],
                config,
                &mut stats,
            );
        }
        if let Some(payload) = value.get("payload") {
            collect_value_output_stats(payload, &["output"], config, &mut stats);
        }
    }
    stats
}

fn collect_value_output_stats(
    value: &serde_json::Value,
    fields: &[&str],
    config: &Config,
    stats: &mut RolloutOutputStats,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    for field in fields {
        let Some(text) = obj.get(*field).and_then(|v| v.as_str()) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        stats.fields += 1;
        stats.bytes += text.len();
        stats.approx_tokens += approx_token_count(text, config);
    }
}

fn approx_token_count(text: &str, config: &Config) -> usize {
    if let Some(raw) = text.strip_prefix(&config.json_prefix)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(raw)
    {
        return count_json_tokens(&value);
    }
    estimate_text_tokens(text)
}

fn count_json_tokens(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 1,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 1,
        serde_json::Value::String(s) => estimate_text_tokens(s),
        serde_json::Value::Array(values) => values.iter().map(count_json_tokens).sum(),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| estimate_text_tokens(k) + count_json_tokens(v))
            .sum(),
    }
}

fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    usize::max(1, chars.div_ceil(4))
}

fn default_min_trim_bytes() -> usize {
    2048
}

fn default_max_body_lines() -> usize {
    120
}

fn default_head_lines() -> usize {
    16
}

fn default_tail_lines() -> usize {
    16
}

fn default_match_context() -> usize {
    2
}

fn default_max_matches() -> usize {
    6
}

fn default_show_stats() -> bool {
    true
}

fn default_output_trim() -> bool {
    true
}

fn default_json_prefix() -> String {
    "__TKE__".to_owned()
}

#[derive(Debug, Clone, Copy)]
enum CommandKind {
    File,
    Search,
    Diff,
    Log,
    Generic,
}

impl CommandKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Search => "search",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrimProfile {
    File,
    Search,
    Diff,
    PathList,
    Log,
    Table,
    Stacktrace,
    Generic,
}

impl TrimProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Search => "search",
            Self::Diff => "diff",
            Self::PathList => "pathlist",
            Self::Log => "log",
            Self::Table => "table",
            Self::Stacktrace => "stacktrace",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProfileLimits {
    head_lines: usize,
    tail_lines: usize,
    match_context: usize,
    max_matches: usize,
}

fn select_profile(kind: CommandKind, lines: &[&str]) -> TrimProfile {
    if looks_like_diff(lines) {
        return TrimProfile::Diff;
    }
    if looks_like_stacktrace(lines) {
        return TrimProfile::Stacktrace;
    }
    if looks_like_path_list(lines) {
        return TrimProfile::PathList;
    }
    if looks_like_table(lines) {
        return TrimProfile::Table;
    }
    match kind {
        CommandKind::Search => TrimProfile::Search,
        CommandKind::Diff => TrimProfile::Diff,
        CommandKind::Log => TrimProfile::Log,
        CommandKind::File => TrimProfile::File,
        CommandKind::Generic => {
            if lines.iter().any(|line| is_log_signal(line, &[])) {
                TrimProfile::Log
            } else {
                TrimProfile::Generic
            }
        }
    }
}

fn profile_limits(profile: TrimProfile, config: &Config) -> ProfileLimits {
    match profile {
        TrimProfile::Diff => ProfileLimits {
            head_lines: usize::min(config.head_lines, 8),
            tail_lines: usize::min(config.tail_lines, 8),
            match_context: 1,
            max_matches: usize::max(config.max_matches, 8),
        },
        TrimProfile::Search => ProfileLimits {
            head_lines: usize::min(config.head_lines, 6),
            tail_lines: usize::min(config.tail_lines, 6),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 12),
        },
        TrimProfile::PathList => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
        },
        TrimProfile::Stacktrace => ProfileLimits {
            head_lines: usize::min(config.head_lines, 6),
            tail_lines: usize::min(config.tail_lines, 6),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 10),
        },
        TrimProfile::Log => ProfileLimits {
            head_lines: usize::min(config.head_lines, 4),
            tail_lines: usize::min(config.tail_lines, 4),
            match_context: 0,
            max_matches: usize::max(config.max_matches, 6),
        },
        TrimProfile::Table => ProfileLimits {
            head_lines: 0,
            tail_lines: 0,
            match_context: 0,
            max_matches: 0,
        },
        TrimProfile::File | TrimProfile::Generic => ProfileLimits {
            head_lines: config.head_lines,
            tail_lines: config.tail_lines,
            match_context: config.match_context,
            max_matches: config.max_matches,
        },
    }
}

fn should_force_trim(
    profile: TrimProfile,
    total_bytes: usize,
    total_lines: usize,
    config: &Config,
) -> bool {
    match profile {
        TrimProfile::Table => {
            total_bytes >= usize::min(config.min_trim_bytes, 1024)
                || total_lines >= usize::min(config.max_body_lines, 12)
        }
        TrimProfile::PathList => {
            total_bytes >= usize::min(config.min_trim_bytes, 512)
                || total_lines >= usize::min(config.max_body_lines, 24)
        }
        _ => total_bytes >= config.min_trim_bytes || total_lines > config.max_body_lines,
    }
}

fn looks_like_diff(lines: &[&str]) -> bool {
    let score = lines
        .iter()
        .take(48)
        .filter(|line| {
            line.starts_with("diff --git ")
                || line.starts_with("@@")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
        })
        .count();
    score >= 2
}

fn looks_like_stacktrace(lines: &[&str]) -> bool {
    let frames = lines.iter().filter(|line| is_stack_frame(line)).count();
    let summary = lines.iter().any(|line| is_stack_summary(line));
    summary && frames >= 2
}

fn looks_like_table(lines: &[&str]) -> bool {
    detect_table_layout(lines).is_some()
}

fn looks_like_path_list(lines: &[&str]) -> bool {
    detect_path_entries(lines).is_some()
}

fn detect_path_entries(lines: &[&str]) -> Option<Vec<PathEntry>> {
    let mut entries = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains('\t') || trimmed.contains("  ") || trimmed.contains(':') {
            return None;
        }
        if !looks_like_path(trimmed) {
            return None;
        }
        entries.push(PathEntry {
            line_index: idx,
            parent: path_parent(trimmed),
            value: trimmed.to_owned(),
        });
    }
    if entries.len() >= 8 {
        Some(entries)
    } else {
        None
    }
}

fn looks_like_path(line: &str) -> bool {
    (line.starts_with('/')
        || line.starts_with("./")
        || line.starts_with("../")
        || line.contains('/'))
        && !line.ends_with(':')
        && !line.contains(" -> ")
}

fn path_parent(value: &str) -> String {
    Path::new(value)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| ".".to_owned())
}

fn detect_table_layout(lines: &[&str]) -> Option<TableLayout> {
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

        if rows.len() >= 4 {
            return Some(TableLayout {
                headers,
                rows,
                header_index,
            });
        }
    }
    None
}

fn split_table_fields(line: &str, max_fields: usize) -> Vec<String> {
    let aligned = split_on_wide_whitespace(line, max_fields);
    if aligned.len() >= 3 {
        return aligned;
    }
    split_on_any_whitespace(line, max_fields)
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
    known >= 1 || score >= usize::max(4, headers.len().saturating_sub(1))
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
    )
}

fn select_table_columns(headers: &[String]) -> Vec<usize> {
    if headers.len() <= 6 {
        return (0..headers.len()).collect();
    }

    let normalized = headers
        .iter()
        .map(|header| normalize_header_name(header))
        .collect::<Vec<_>>();
    let wanted = [
        "user",
        "pid",
        "%cpu",
        "%mem",
        "stat",
        "command",
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
            if selected.len() >= 6 {
                break;
            }
        }
        if selected.len() >= 6 {
            break;
        }
    }
    for idx in 0..headers.len() {
        if selected.len() >= 6 {
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
        let joined = row.fields.join(" ").to_ascii_lowercase();
        if is_log_signal(&joined, terms)
            || joined.contains("codex")
            || joined.contains("listen")
            || joined.contains("estab")
            || joined.contains("failed")
            || joined.contains("exited")
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

fn is_stack_frame(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("at ")
        || trimmed.starts_with('#')
        || trimmed.contains(".rs:")
        || trimmed.contains(".py\", line ")
        || trimmed.contains(".js:")
}

fn is_stack_summary(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("traceback")
        || lower.contains("stack trace")
        || lower.contains("panic")
        || lower.contains("exception")
}

fn is_log_signal(line: &str, terms: &[String]) -> bool {
    let lower = line.to_ascii_lowercase();
    if [
        "error",
        "failed",
        "panic",
        "exception",
        "warning",
        "abort",
        "undefined",
        "mismatch",
    ]
    .iter()
    .any(|term| lower.contains(term))
    {
        return true;
    }
    terms
        .iter()
        .any(|term| !term.is_empty() && lower.contains(term))
}

fn detect_repeated_runs(lines: &[&str]) -> Vec<RepeatedRun> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let normalized = canonicalize_log_line(lines[idx]);
        let mut count = 1;
        let mut end = idx + 1;
        while end < lines.len() && canonicalize_log_line(lines[end]) == normalized {
            count += 1;
            end += 1;
        }
        if count >= 3 && !normalized.is_empty() {
            out.push(RepeatedRun {
                range: [idx, end],
                count,
                sample: truncate_for_sample(lines[idx]),
            });
        }
        idx = end;
    }
    out
}

fn detect_code_blocks(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_code_boundary(line) {
            let end = find_block_end(lines, idx + 1);
            out.push((idx, end));
            if out.len() >= 4 {
                break;
            }
        }
    }
    out
}

fn is_code_boundary(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("fn ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("async fn ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("pub struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("function ")
}

fn find_block_end(lines: &[&str], start: usize) -> usize {
    let mut end = usize::min(lines.len(), start + 1);
    while end < lines.len() {
        let trimmed = lines[end].trim();
        if trimmed.is_empty() {
            break;
        }
        if is_code_boundary(lines[end]) && end > start {
            break;
        }
        if end.saturating_sub(start) >= 11 {
            break;
        }
        end += 1;
    }
    end
}

fn canonicalize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_digit = false;
    for ch in line.chars() {
        if ch.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            prev_digit = false;
            out.push(ch);
        }
    }
    out.trim().to_ascii_lowercase()
}

fn truncate_for_sample(line: &str) -> String {
    let sample = line.trim();
    if sample.len() > 96 {
        format!("{}...", &sample[..96])
    } else {
        sample.to_owned()
    }
}

#[derive(Serialize)]
struct TrimEnvelope {
    v: u8,
    cmd: String,
    a: Vec<String>,
    k: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sr: Option<String>,
    p: String,
    s: String,
    t: bool,
    h: Vec<String>,
    ta: Vec<String>,
    m: Vec<MatchChunk>,
    o: Vec<[usize; 2]>,
    st: TrimStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    tb: Option<TableSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pl: Option<PathListSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    b: Option<Vec<String>>,
}

#[derive(Serialize)]
struct MatchChunk {
    k: String,
    r: [usize; 2],
    l: Vec<String>,
}

#[derive(Serialize)]
struct TrimStats {
    tb: usize,
    tl: usize,
    el: usize,
}

#[derive(Serialize)]
struct TableSummary {
    c: Vec<String>,
    r: Vec<TableRow>,
    rc: usize,
    hc: usize,
}

#[derive(Serialize)]
struct TableRow {
    i: usize,
    v: Vec<String>,
}

#[derive(Serialize)]
struct PathListSummary {
    rc: usize,
    b: Vec<PathBucket>,
    r: Vec<PathRow>,
}

#[derive(Serialize)]
struct PathBucket {
    d: String,
    c: usize,
    e: Vec<String>,
}

#[derive(Serialize)]
struct PathRow {
    i: usize,
    v: String,
}

struct RepeatedRun {
    range: [usize; 2],
    count: usize,
    sample: String,
}

struct TableLayout {
    headers: Vec<String>,
    rows: Vec<TableDataRow>,
    #[allow(dead_code)]
    header_index: usize,
}

struct TableDataRow {
    line_index: usize,
    fields: Vec<String>,
}

struct PathEntry {
    line_index: usize,
    parent: String,
    value: String,
}

struct BenchmarkSpec {
    name: String,
    command: String,
    profile: String,
    expected: BenchmarkExpectation,
    call_id: String,
    sample: String,
}

#[derive(Clone, Copy)]
enum BenchmarkExpectation {
    Compress,
    PassThrough,
}

impl BenchmarkExpectation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Compress => "compress",
            Self::PassThrough => "pass_through",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn value_from_json(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("json")
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tke-{label}-{}", now_millis()))
    }

    fn set_env_var(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env_var(key: &str) {
        unsafe {
            std::env::remove_var(key);
        }
    }

    fn repeated_lines(prefix: &str, count: usize) -> String {
        (0..count)
            .map(|idx| format!("{prefix} {idx}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn small_payload_keeps_body() {
        let cfg = Config::default();
        let text = "line1\nline2\nline3\n";
        let json = normalize_text(
            "cat",
            &["foo.rs".to_owned()],
            "stdout",
            CommandKind::File,
            text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["t"], false);
        assert_eq!(value["b"][0], "line1");
    }

    #[test]
    fn large_payload_trims_ranges() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        cfg.head_lines = 2;
        cfg.tail_lines = 2;
        let text = (0..12)
            .map(|idx| format!("line-{idx} error"))
            .collect::<Vec<_>>()
            .join("\n");
        let json = normalize_text(
            "cargo",
            &["test".to_owned()],
            "stdout",
            CommandKind::Log,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["t"], true);
        assert!(value["o"].as_array().is_some());
        assert!(value["m"].as_array().expect("matches").len() >= 1);
    }

    #[test]
    fn match_terms_include_errors_and_args() {
        let terms = match_terms("cargo", &["test".to_owned(), "AuthFailure".to_owned()]);
        assert!(terms.contains(&"authfailure".to_owned()));
        assert!(terms.contains(&"error".to_owned()));
    }

    #[test]
    fn diff_profile_marks_hunks() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "diff --git a/src/lib.rs b/src/lib.rs",
            "index 123..456 100644",
            "--- a/src/lib.rs",
            "+++ b/src/lib.rs",
            "@@ -1,3 +1,4 @@",
            "-old",
            "+new",
            " unchanged",
        ]
        .join("\n");
        let json = normalize_text(
            "git",
            &["diff".to_owned()],
            "stdout",
            CommandKind::Diff,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["p"], "diff");
        assert!(
            value["m"]
                .as_array()
                .expect("matches")
                .iter()
                .any(|chunk| chunk["k"] == "hunk")
        );
    }

    #[test]
    fn stacktrace_profile_detected() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "Traceback (most recent call last):",
            "  File \"app.py\", line 10, in <module>",
            "  File \"svc.py\", line 20, in run",
            "ValueError: boom",
        ]
        .join("\n");
        let json = normalize_text(
            "python",
            &["script.py".to_owned()],
            "stderr",
            CommandKind::Generic,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["p"], "stacktrace");
        assert!(
            value["m"]
                .as_array()
                .expect("matches")
                .iter()
                .any(|chunk| chunk["k"] == "frame")
        );
    }

    #[test]
    fn search_profile_prefers_result_lines() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "src/lib.rs:10:fn normalize_text(",
            "src/lib.rs:20:fn collect_profile_chunks(",
            "src/main.rs:5:fn run()",
        ]
        .join("\n");
        let json = normalize_text(
            "rg",
            &["normalize".to_owned()],
            "stdout",
            CommandKind::Search,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["p"], "search");
        assert!(
            value["m"]
                .as_array()
                .expect("matches")
                .iter()
                .all(|chunk| chunk["k"] == "file" || chunk["k"] == "result")
        );
    }

    #[test]
    fn search_profile_groups_results_by_file() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = (0..40)
            .map(|idx| format!("src/lib.rs:{}:pub fn alpha_{}() {{}}", idx + 1, idx))
            .chain((0..20).map(|idx| format!("src/main.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx)))
            .collect::<Vec<_>>()
            .join("\n");
        let json = normalize_text(
            "rg",
            &["fn".to_owned()],
            "stdout",
            CommandKind::Search,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value = value_from_json(&json);
        let chunks = value["m"].as_array().expect("matches");
        assert!(chunks.iter().any(|chunk| chunk["k"] == "file"));
    }

    #[test]
    fn log_profile_folds_repeated_lines() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "Compiling crate_a v0.1.0",
            "Compiling crate_a v0.1.0",
            "Compiling crate_a v0.1.0",
            "Compiling crate_a v0.1.0",
            "error: build failed",
        ]
        .join("\n");
        let json = normalize_text(
            "cargo",
            &["build".to_owned()],
            "stdout",
            CommandKind::Log,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["p"], "log");
        assert!(
            value["m"]
                .as_array()
                .expect("matches")
                .iter()
                .any(|chunk| chunk["k"] == "fold")
        );
    }

    #[test]
    fn table_profile_detected_for_ps_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 proxima",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
        ]
        .join("\n");
        let json = normalize_text(
            "ps",
            &["aux".to_owned(), "--sort=-%cpu".to_owned()],
            "stdout",
            CommandKind::Log,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value = value_from_json(&json);
        assert_eq!(value["p"], "table");
        assert!(value["tb"].is_object());
        assert_eq!(value["tb"]["c"][0], "USER");
        assert!(
            value["tb"]["r"]
                .as_array()
                .expect("rows")
                .iter()
                .any(|row| row["v"].to_string().to_ascii_lowercase().contains("codex"))
        );
    }

    #[test]
    fn maybe_normalize_prefers_table_summary_when_cheaper() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 proxima",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
            "root        1063  0.0  1.6 1440068 66960 ?       Sl   07:56   0:00 cloud-monitor-agent",
            "root        2546  0.0  1.2 1403548 48180 pts/1   Sl+  08:08   0:00 node codex",
            "root         886  0.0  1.1 1313072 46252 ?       Ssl  07:56   0:00 mihomo",
            "root         710  0.0  1.4 1950568 57672 ?       Ssl  07:56   0:00 containerd",
            "root         964  0.0  2.2 2271304 91340 ?       Ssl  07:56   0:00 dockerd",
        ]
        .join("\n");
        let normalized = maybe_normalize_text(
            "ps",
            &["aux".to_owned(), "--sort=-%cpu".to_owned()],
            "stdout",
            CommandKind::Log,
            &text,
            &cfg,
            None,
        )
        .expect("normalize");
        assert!(normalized.is_some());
        let payload = normalized.expect("payload");
        let value = value_from_json(&payload);
        assert_eq!(value["p"], "table");
    }

    #[test]
    fn table_profile_can_trigger_below_default_trim_bytes() {
        let cfg = Config::default();
        let text = [
            "CONTAINER ID   IMAGE          STATUS       PORTS      NAMES",
            "abc123         redis:7        Up 2 hours   6379/tcp   redis-main",
            "def456         postgres:16    Up 2 hours   5432/tcp   pg-main",
            "ghi789         nginx:latest   Exited (0)              web-old",
            "jkl012         app:v1         Up 5 mins    8080/tcp   app-blue",
            "mno345         app:v2         Up 1 min     8081/tcp   app-green",
            "pqr678         worker:v2      Up 1 min                worker-green",
            "stu901         sidekiq:v2     Restarting              sidekiq-green",
            "vwx234         cron:v1        Exited (137)            cron-old",
            "yz5678         proxy:v3       Up 3 days    80/tcp     edge-proxy",
            "qwe111         admin:v1       Up 3 days    9000/tcp   admin-ui",
            "rty222         mail:v1        Up 1 day     25/tcp     smtp",
        ]
        .join("\n");
        assert!(text.len() < cfg.min_trim_bytes);
        let normalized = maybe_normalize_text(
            "docker",
            &["ps".to_owned(), "-a".to_owned(), "--no-trunc".to_owned()],
            "stdout",
            CommandKind::Generic,
            &text,
            &cfg,
            None,
        )
        .expect("normalize");
        assert!(normalized.is_some());
    }

    #[test]
    fn compare_rollout_reports_savings_for_realish_ps_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let output = [
            "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND",
            "root        3142  100  0.0   7324  3192 ?        Ss   08:09   0:00 /bin/bash -c ps aux --sort=-%cpu | head -n 25",
            "root        2553  2.2  2.5 357624 101588 pts/1   Sl+  08:08   0:00 /root/.nvm/.../codex",
            "root         674  0.3  1.1 1262168 45772 ?       Ssl  07:56   0:02 cloudflared --token secret",
            "root         683  0.2  0.7 1530044 30480 ?       Ssl  07:56   0:01 /opt/proxima/proxima",
            "root        2696  0.1  0.3 102660 15264 pts/1    S+   08:08   0:00 git-remote-https https://github.com/openai/plugins.git",
            "root         665  0.1  2.6 1304956 105920 ?      Ssl  07:56   0:01 /usr/bin/1panel-agent",
            "root           1  0.1  0.3 167764 12476 ?        Ss   07:56   0:00 /sbin/init nopti",
            "root        1063  0.0  1.6 1440068 66960 ?       Sl   07:56   0:00 cloud-monitor-agent worker",
            "root        2546  0.0  1.2 1403548 48180 pts/1   Sl+  08:08   0:00 node /root/.nvm/.../codex",
            "root         886  0.0  1.1 1313072 46252 ?       Ssl  07:56   0:00 mihomo",
            "root         710  0.0  1.4 1950568 57672 ?       Ssl  07:56   0:00 containerd",
            "root         964  0.0  2.2 2271304 91340 ?       Ssl  07:56   0:00 dockerd",
            "root        2408  0.0  0.1  11028  7764 pts/1    Ss   08:08   0:00 /bin/bash",
            "root          63  0.0  0.0      0     0 ?        I    07:56   0:00 [kworker/3:1-events]",
            "root         673  0.0  1.5 1366336 60272 ?       Ssl  07:56   0:00 cloud-monitor-agent start",
            "root         685  0.0  0.1 221780  4376 ?        Ssl  07:56   0:00 rsyslogd -n -iNONE",
            "root         266  0.0  0.3  33208 13396 ?        Ss   07:56   0:00 systemd-journald",
            "root          39  0.0  0.0      0     0 ?        I    07:56   0:00 [kworker/u10:1-writeback]",
            "root        2407  0.0  0.0   7016  2532 ?        Ss   08:08   0:00 SCREEN -S t",
            "earlyoom     679  0.0  0.0   2480   884 ?        SLs  07:56   0:00 earlyoom",
            "root         668  0.0  0.8 1308624 32080 ?       Ssl  07:56   0:00 assist-client start",
            "root        1843  0.0  0.0      0     0 ?        I    08:07   0:00 [kworker/u9:1-events_unbound]",
            "root         667  0.0  0.8 1292420 33452 ?       Ssl  07:56   0:00 /usr/bin/1panel-core",
            "70          1411  0.0  0.6 175360 26788 ?        Ss   07:56   0:00 postgres",
        ]
        .join("\n");
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"ps aux --sort=-%cpu | head -n 25\",\"yield_time_ms\":1000}",
                    "call_id": "call_7"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_7",
                    "output": format!(
                        "Chunk ID: eebb60\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 757\nOutput:\n{output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let raw_stats = collect_rollout_output_stats(&jsonl, &cfg);
        let rewritten_stats = collect_rollout_output_stats(&rewritten, &cfg);
        assert!(rewritten_stats.approx_tokens < raw_stats.approx_tokens);
        assert!(rewritten_stats.bytes < raw_stats.bytes);
    }

    #[test]
    fn file_profile_extracts_code_blocks() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = [
            "use std::io;",
            "",
            "pub fn normalize_text() {",
            "    let x = 1;",
            "    println!(\"{}\", x);",
            "}",
            "",
            "pub struct Config {",
            "    value: usize,",
            "}",
        ]
        .join("\n");
        let json = normalize_text(
            "cat",
            &["src/lib.rs".to_owned()],
            "stdout",
            CommandKind::File,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["p"], "file");
        assert!(
            value["m"]
                .as_array()
                .expect("matches")
                .iter()
                .any(|chunk| chunk["k"] == "block")
        );
    }

    #[test]
    fn rewrites_codex_command_execution_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
                "aggregated_output": format!("{}\n", repeated_lines("pub fn a() {}", 200)),
                "exit_code": 0,
                "status": "completed"
            }
        })
        .to_string();
        let rewritten = rewrite_codex_jsonl(&line, &cfg)
            .expect("rewrite")
            .expect("changed");
        let value: serde_json::Value =
            serde_json::from_str(rewritten.lines().next().expect("line")).expect("json");
        let output = value["item"]["aggregated_output"]
            .as_str()
            .expect("aggregated_output");
        assert!(output.starts_with("__TKE__"));
        let nested = value_from_json(output.trim_start_matches("__TKE__"));
        assert_eq!(nested["sc"], "cat");
        assert_eq!(nested["sr"], "source");
    }

    #[test]
    fn parses_shell_wrapped_command_execution() {
        let parsed =
            parse_command_execution("/bin/bash -lc \"env FOO=1 cat /tmp/demo.txt | sed -n '1p'\"");
        assert_eq!(parsed.selected_stage().name, "cat");
        assert_eq!(
            parsed.selected_stage().args.first().map(String::as_str),
            Some("/tmp/demo.txt")
        );
    }

    #[test]
    fn rewrites_multiple_command_output_fields() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let mut value = serde_json::json!({
            "item": {
                "type": "command_execution",
                "command": "/bin/bash -lc 'python tool.py'",
                "stdout": format!("Traceback (most recent call last):\n{}\nValueError: boom\n", repeated_lines("  File \"x.py\", line 1, in run", 80)),
                "stderr": format!("{}\n", repeated_lines("warning: noisy", 120)),
                "output": format!("{}\n", repeated_lines("pub fn alpha() {}", 180))
            }
        });
        let parsed = parse_command_execution("/bin/bash -lc 'python tool.py'");
        let changed =
            rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
        assert!(changed);
        assert!(
            value["item"]["stdout"]
                .as_str()
                .expect("stdout")
                .starts_with("__TKE__")
        );
        assert!(
            value["item"]["stderr"]
                .as_str()
                .expect("stderr")
                .starts_with("__TKE__")
        );
        assert!(
            value["item"]["output"]
                .as_str()
                .expect("output")
                .starts_with("__TKE__")
        );
    }

    #[test]
    fn rewrite_is_idempotent_for_prefixed_output() {
        let cfg = Config::default();
        let mut value = serde_json::json!({
            "item": {
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
                "aggregated_output": "__TKE__{\"v\":1}"
            }
        });
        let parsed = parse_command_execution("/bin/bash -lc 'cat /tmp/demo.rs'");
        let changed =
            rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
        assert!(!changed);
        assert_eq!(value["item"]["aggregated_output"], "__TKE__{\"v\":1}");
    }

    #[test]
    fn rewrite_skips_empty_fields() {
        let cfg = Config::default();
        let mut value = serde_json::json!({
            "item": {
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs'",
                "aggregated_output": ""
            }
        });
        let parsed = parse_command_execution("/bin/bash -lc 'cat /tmp/demo.rs'");
        let changed =
            rewrite_command_item_fields(&mut value["item"], &parsed, &cfg).expect("rewrite");
        assert!(!changed);
    }

    #[test]
    fn non_jsonl_input_is_not_rewritten() {
        let cfg = Config::default();
        let rewritten = rewrite_codex_jsonl("not-json\nstill-not-json\n", &cfg).expect("rewrite");
        assert!(rewritten.is_none());
    }

    #[test]
    fn parses_nested_shell_invocation() {
        let parsed = parse_command_execution(
            "/bin/bash -lc \"sh -c 'rg -n fn /tmp/tke-codex/big.rs | sed -n 1p'\"",
        );
        assert_eq!(parsed.selected_stage().name, "rg");
        assert!(
            parsed
                .selected_stage()
                .args
                .iter()
                .any(|arg| arg.contains("/tmp/tke-codex/big.rs"))
        );
    }

    #[test]
    fn parses_xargs_payload_command() {
        let parsed = parse_command_execution(
            "/bin/bash -lc \"printf '/tmp/tke-codex/big.rs\\n' | xargs cat\"",
        );
        assert_eq!(parsed.selected_stage().name, "cat");
    }

    #[test]
    fn stage_selection_prefers_search_over_filter() {
        let parsed = parse_command_execution(
            "env FOO=1 echo ignore | rg -n fn /tmp/tke-codex/big.rs | sed -n 1p",
        );
        assert_eq!(parsed.selected_stage().name, "rg");
    }

    #[test]
    fn stage_selection_prefers_search_over_source_in_mixed_pipeline() {
        let parsed = parse_command_execution(
            "cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1",
        );
        assert_eq!(parsed.selected_stage().name, "rg");
    }

    #[test]
    fn default_tool_commands_cover_common_reading_tools() {
        let cfg = Config::default();
        for name in [
            "ls", "find", "fd", "bat", "nl", "awk", "cut", "sort", "uniq", "wc", "tree", "xargs",
        ] {
            assert!(cfg.is_tool_command(name), "missing tool command {name}");
        }
    }

    #[test]
    fn default_tool_commands_cover_core_agent_workflows() {
        let cfg = Config::default();
        for name in [
            "cat", "sed", "rg", "grep", "git", "cargo", "pytest", "npm", "pnpm", "yarn", "tail",
            "head", "ls", "find", "fd", "bat", "nl", "awk", "cut", "sort", "uniq", "wc", "tree",
            "xargs",
        ] {
            assert!(cfg.is_tool_command(name), "missing tool command {name}");
        }
    }

    #[test]
    fn stage_selection_prefers_find_for_file_discovery_pipeline() {
        let parsed = parse_command_execution("find src -name '*.rs' | head -n 20");
        assert_eq!(parsed.selected_stage().name, "find");
        assert_eq!(parsed.selected_stage().role.as_str(), "search");
    }

    #[test]
    fn stage_selection_prefers_fd_over_filter() {
        let parsed = parse_command_execution("fd normalize src | sort | head -n 10");
        assert_eq!(parsed.selected_stage().name, "fd");
        assert_eq!(parsed.selected_stage().role.as_str(), "search");
    }

    #[test]
    fn classify_common_code_reading_commands() {
        assert!(matches!(
            classify_command("bat", &["src/lib.rs".to_owned()]),
            CommandKind::File
        ));
        assert!(matches!(
            classify_command("nl", &["-ba".to_owned(), "src/lib.rs".to_owned()]),
            CommandKind::File
        ));
        assert!(matches!(
            classify_command(
                "find",
                &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()]
            ),
            CommandKind::Search
        ));
        assert!(matches!(
            classify_command("ls", &["-l".to_owned(), "src".to_owned()]),
            CommandKind::Log
        ));
        assert!(matches!(
            classify_command("tree", &["-a".to_owned(), "src".to_owned()]),
            CommandKind::Search
        ));
        assert!(matches!(
            classify_command("awk", &["{print}".to_owned(), "src/lib.rs".to_owned()]),
            CommandKind::File
        ));
        assert!(matches!(
            classify_command("wc", &["-l".to_owned(), "src/lib.rs".to_owned()]),
            CommandKind::Generic
        ));
    }

    #[test]
    fn stage_roles_cover_default_tool_commands() {
        for (name, expected) in [
            ("cat", "source"),
            ("rg", "search"),
            ("grep", "search"),
            ("find", "search"),
            ("fd", "search"),
            ("tree", "search"),
            ("sed", "filter"),
            ("awk", "filter"),
            ("cut", "filter"),
            ("sort", "filter"),
            ("uniq", "filter"),
            ("head", "summarize"),
            ("tail", "summarize"),
            ("wc", "summarize"),
            ("cargo", "build"),
        ] {
            assert_eq!(classify_stage_role(name).as_str(), expected, "{name}");
        }
    }

    #[test]
    fn search_profile_detected_for_find_results() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = (0..32)
            .map(|idx| format!("src/module_{idx:03}.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        let normalized = normalize_text(
            "find",
            &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()],
            "stdout",
            CommandKind::Search,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value = value_from_json(&normalized);
        assert_eq!(value["p"], "pathlist");
        assert!(value["pl"].is_object());
    }

    #[test]
    fn file_profile_detected_for_numbered_code_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = (1..=40)
            .map(|idx| match idx % 8 {
                1 => format!("{idx:>6}\tpub fn function_{idx}() {{"),
                2 => format!("{idx:>6}\t    let value = {idx};"),
                3 => format!("{idx:>6}\t    println!(\"{{}}\", value);"),
                4 => format!("{idx:>6}\t}}"),
                5 => format!("{idx:>6}\t"),
                6 => format!("{idx:>6}\tpub struct Struct{idx} {{"),
                7 => format!("{idx:>6}\t    field: usize,"),
                _ => format!("{idx:>6}\t}}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let normalized = normalize_text(
            "nl",
            &["-ba".to_owned(), "src/lib.rs".to_owned()],
            "stdout",
            CommandKind::File,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value = value_from_json(&normalized);
        assert_eq!(value["p"], "file");
    }

    #[test]
    fn pathlist_profile_detected_for_find_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let text = (0..40)
            .map(|idx| format!("/root/project/src/module_{idx:03}.rs"))
            .collect::<Vec<_>>()
            .join("\n");
        let normalized = normalize_text(
            "find",
            &["src".to_owned(), "-name".to_owned(), "*.rs".to_owned()],
            "stdout",
            CommandKind::Search,
            &text,
            &cfg,
        )
        .expect("normalize");
        let value = value_from_json(&normalized);
        assert_eq!(value["p"], "pathlist");
        assert_eq!(value["pl"]["rc"], 40);
    }

    #[test]
    fn compare_rollout_reports_savings_for_path_list_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let output = (0..200)
            .map(|idx| {
                format!("/root/project/target/debug/incremental/tke/build-artifact-{idx:03}.o")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"find /root/project -type f | head -n 200\",\"yield_time_ms\":1000}",
                    "call_id": "call_path_1"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_path_1",
                    "output": format!(
                        "Chunk ID: demo\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 1000\nOutput:\n{output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let raw_stats = collect_rollout_output_stats(&jsonl, &cfg);
        let rewritten_stats = collect_rollout_output_stats(&rewritten, &cfg);
        assert!(rewritten_stats.approx_tokens < raw_stats.approx_tokens);
        assert!(rewritten_stats.bytes < raw_stats.bytes);
    }

    #[test]
    fn selected_stage_metadata_is_embedded_for_search_pipeline() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let parsed = parse_command_execution(
            "cat /tmp/tke-codex/huge.txt | rg -n '^SECTION 599' | head -n 1",
        );
        let selected = parsed.selected_stage();
        let json = normalize_text_with_stage(
            &selected.name,
            &selected.args,
            "stdout",
            classify_command(&selected.name, &selected.args),
            "2397:SECTION 599\n",
            &cfg,
            Some((&selected.name, selected.role.as_str())),
        )
        .expect("normalize");
        let value = value_from_json(&json);
        assert_eq!(value["sc"], "rg");
        assert_eq!(value["sr"], "search");
        assert_eq!(value["p"], "search");
    }

    #[test]
    fn codex_event_replay_preserves_selected_search_stage() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_9",
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/tke-codex/huge.txt | rg -n \"SECTION 599\" | head'",
                "aggregated_output": format!("{}\n", repeated_lines("2397:SECTION 599", 180)),
                "exit_code": 0,
                "status": "completed"
            }
        })
        .to_string();
        let rewritten = rewrite_codex_jsonl(&line, &cfg)
            .expect("rewrite")
            .expect("changed");
        let value = value_from_json(rewritten.lines().next().expect("line"));
        let nested = value_from_json(
            value["item"]["aggregated_output"]
                .as_str()
                .expect("aggregated_output")
                .trim_start_matches("__TKE__"),
        );
        assert_eq!(nested["sc"], "rg");
        assert_eq!(nested["sr"], "search");
    }

    #[test]
    fn rewrites_exec_command_function_call_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1\",\"yield_time_ms\":1000}",
                    "call_id": "call_1"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{}\n",
                        repeated_lines("2:pub fn beta() {}", 160)
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let second = rewritten.lines().nth(1).expect("second line");
        let value = value_from_json(second);
        let nested = value_from_json(
            value["payload"]["output"]
                .as_str()
                .expect("output")
                .trim_start_matches("__TKE__"),
        );
        assert_eq!(nested["sc"], "rg");
        assert_eq!(nested["sr"], "search");
        assert_eq!(nested["p"], "search");
    }

    #[test]
    fn rewrites_exec_command_error_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1 .\",\"yield_time_ms\":1000}",
                    "call_id": "call_2"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_2",
                    "output": format!(
                        "Chunk ID: 15b935\nWall time: 0.0000 seconds\nProcess exited with code 1\nOriginal token count: 500\nOutput:\n{}\n",
                        repeated_lines("head: error reading '.': Is a directory", 120)
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let second = rewritten.lines().nth(1).expect("second line");
        let value = value_from_json(second);
        let nested = value_from_json(
            value["payload"]["output"]
                .as_str()
                .expect("output")
                .trim_start_matches("__TKE__"),
        );
        assert_eq!(nested["s"], "stderr");
        assert_eq!(nested["sc"], "rg");
    }

    #[test]
    fn compare_rollout_reports_savings() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let large_output = (0..80)
            .map(|idx| format!("src/demo.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx))
            .collect::<Vec<_>>()
            .join("\n");
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"cat /tmp/demo.rs | rg -n beta | head -n 1\",\"yield_time_ms\":1000}",
                    "call_id": "call_3"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_3",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{large_output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let raw_stats = collect_rollout_output_stats(&jsonl, &cfg);
        let rewritten_stats = collect_rollout_output_stats(&rewritten, &cfg);
        let report = RolloutCompareReport::from_stats(
            Path::new("/tmp/demo.jsonl"),
            true,
            raw_stats,
            rewritten_stats,
        );
        assert!(report.changed);
        assert_eq!(report.raw_fields, 1);
        assert_eq!(report.rewritten_fields, 1);
        assert!(
            report.bytes_saved > 0,
            "report bytes_saved={}",
            report.bytes_saved
        );
        assert!(
            report.tokens_saved > 0,
            "report tokens_saved={}",
            report.tokens_saved
        );
    }

    #[test]
    fn compare_rollout_reports_grouped_search_savings() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let large_output = (0..120)
            .map(|idx| format!("src/lib.rs:{}:pub fn beta_{}() {{}}", idx + 1, idx))
            .chain(
                (0..80).map(|idx| format!("src/main.rs:{}:pub fn gamma_{}() {{}}", idx + 1, idx)),
            )
            .collect::<Vec<_>>()
            .join("\n");
        let jsonl = [
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"rg -n 'fn' src\",\"yield_time_ms\":1000}",
                    "call_id": "call_rg_group"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_rg_group",
                    "output": format!(
                        "Chunk ID: 13b6ef\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 500\nOutput:\n{large_output}\n"
                    )
                }
            })
            .to_string(),
        ]
        .join("\n");
        let rewritten = rewrite_codex_jsonl(&jsonl, &cfg)
            .expect("rewrite")
            .expect("changed");
        let raw_stats = collect_rollout_output_stats(&jsonl, &cfg);
        let rewritten_stats = collect_rollout_output_stats(&rewritten, &cfg);
        assert!(rewritten_stats.approx_tokens < raw_stats.approx_tokens);
        assert!(rewritten_stats.bytes < raw_stats.bytes);
    }

    #[test]
    fn parse_capture_interactive_dispatch() {
        let dispatch = parse_dispatch(
            "tke",
            vec![
                "tke".to_owned(),
                "capture-interactive".to_owned(),
                "--source".to_owned(),
                "/tmp/source.jsonl".to_owned(),
                "--output".to_owned(),
                "/tmp/out".to_owned(),
            ],
        )
        .expect("dispatch");
        match dispatch {
            Dispatch::CaptureInteractive { source, output } => {
                assert_eq!(source, Some(PathBuf::from("/tmp/source.jsonl")));
                assert_eq!(output, Some(PathBuf::from("/tmp/out")));
            }
            other => panic!("unexpected dispatch: {other:?}"),
        }
    }

    #[test]
    fn parse_activate_accepts_shell() {
        let dispatch = parse_dispatch(
            "tke",
            vec![
                "tke".to_owned(),
                "activate".to_owned(),
                "--shell".to_owned(),
                "powershell".to_owned(),
                "codex".to_owned(),
            ],
        )
        .expect("dispatch");
        match dispatch {
            Dispatch::Activate {
                agents,
                shim_dir,
                shell,
            } => {
                assert_eq!(agents, vec!["codex".to_owned()]);
                assert!(shim_dir.is_none());
                assert_eq!(shell, Some(ShellKind::PowerShell));
            }
            other => panic!("unexpected dispatch: {other:?}"),
        }
    }

    #[test]
    fn parse_shim_subcommand_dispatch() {
        let dispatch = parse_dispatch(
            "tke",
            vec![
                "tke".to_owned(),
                "shim".to_owned(),
                "rg".to_owned(),
                "-n".to_owned(),
                "fn".to_owned(),
                "src".to_owned(),
            ],
        )
        .expect("dispatch");
        match dispatch {
            Dispatch::ShimExec { name, args } => {
                assert_eq!(name, "rg");
                assert_eq!(args, vec!["-n", "fn", "src"]);
            }
            other => panic!("unexpected dispatch: {other:?}"),
        }
    }

    #[test]
    fn parse_benchmark_commands_dispatch() {
        let dispatch = parse_dispatch(
            "tke",
            vec!["tke".to_owned(), "benchmark-commands".to_owned()],
        )
        .expect("dispatch");
        assert!(matches!(
            dispatch,
            Dispatch::BenchmarkCommands { check: false }
        ));
    }

    #[test]
    fn benchmark_report_contains_expected_cases() {
        let cfg = Config::default();
        let report = build_benchmark_report(&cfg).expect("benchmark");
        for name in [
            "cat_code",
            "sed_code",
            "bat_code",
            "nl_code",
            "awk_code",
            "cut_code",
            "head_code",
            "tail_code",
            "rg_code",
            "grep_code",
            "find_paths",
            "fd_paths",
            "tree_paths",
            "sort_paths",
            "uniq_paths",
            "ls_long",
            "wc_summary",
            "git_diff",
            "cargo_build",
            "pytest_run",
            "npm_test",
            "pnpm_test",
            "yarn_test",
            "ps_table",
            "systemctl_table",
            "xargs_cat",
        ] {
            assert!(report.cases.iter().any(|case| case.name == name), "{name}");
        }
    }

    #[test]
    fn benchmark_report_shows_positive_savings_for_core_cases() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let report = build_benchmark_report(&cfg).expect("benchmark");
        for case in &report.cases {
            match case.expected.as_str() {
                "compress" => {
                    assert!(case.tokens_saved > 0, "{}", case.name);
                    assert!(case.bytes_saved > 0, "{}", case.name);
                }
                "pass_through" => {
                    assert_eq!(case.tokens_saved, 0, "{}", case.name);
                    assert_eq!(case.bytes_saved, 0, "{}", case.name);
                }
                other => panic!("unexpected expectation {other}"),
            }
        }
    }

    #[test]
    fn benchmark_report_covers_default_tool_families() {
        let report = build_benchmark_report(&Config::default()).expect("benchmark");
        let commands = report
            .cases
            .iter()
            .map(|case| case.command.as_str())
            .collect::<Vec<_>>();
        for needle in [
            "cat ",
            "sed ",
            "bat ",
            "nl ",
            "awk ",
            "cut ",
            "head ",
            "tail ",
            "rg ",
            "grep ",
            "git ",
            "cargo ",
            "pytest ",
            "npm ",
            "pnpm ",
            "yarn ",
            "find ",
            "fd ",
            "tree ",
            "sort",
            "uniq",
            "wc ",
            "ls ",
            "ps ",
            "systemctl ",
            "xargs ",
        ] {
            assert!(commands.iter().any(|cmd| cmd.contains(needle)), "{needle}");
        }
    }

    #[test]
    fn render_posix_activate_script() {
        let script = render_activate_script(
            ShellKind::Posix,
            Path::new("/tmp/tke"),
            Path::new("/tmp/shims"),
            "/usr/bin:/bin",
            &["codex".to_owned()],
            &["cat".to_owned(), "rg".to_owned()],
        );
        assert!(script.contains("export TKE_BIN='/tmp/tke'"));
        assert!(script.contains("export PATH='/tmp/shims':$PATH"));
    }

    #[test]
    fn render_powershell_activate_script() {
        let script = render_activate_script(
            ShellKind::PowerShell,
            Path::new("C:\\tke\\tke.exe"),
            Path::new("C:\\tke\\shims"),
            "C:\\Windows\\System32;C:\\Windows",
            &["codex".to_owned()],
            &["cat".to_owned(), "rg".to_owned()],
        );
        assert!(script.contains("$env:TKE_BIN = 'C:\\tke\\tke.exe'"));
        assert!(script.contains("$env:PATH = 'C:\\tke\\shims' + ';' + $env:PATH"));
    }

    #[test]
    fn render_cmd_activate_script() {
        let script = render_activate_script(
            ShellKind::Cmd,
            Path::new("C:\\tke\\tke.exe"),
            Path::new("C:\\tke\\shims"),
            "C:\\Windows\\System32;C:\\Windows",
            &["codex".to_owned()],
            &["cat".to_owned()],
        );
        assert!(script.contains("set \"TKE_BIN=C:\\tke\\tke.exe\""));
        assert!(script.contains("set \"PATH=C:\\tke\\shims;%PATH%\""));
    }

    #[test]
    fn render_powershell_deactivate_script_restores_real_path() {
        let script = render_deactivate_script(ShellKind::PowerShell);
        assert!(script.contains("$env:TKE_REAL_PATH"));
        assert!(script.contains("Remove-Item Env:TKE_BIN"));
    }

    #[test]
    fn candidate_command_names_expand_windows_pathext() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        set_env_var("PATHEXT", ".EXE;.CMD");
        let names = candidate_command_names("rg");
        remove_env_var("PATHEXT");
        let rendered = names
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if cfg!(windows) {
            assert!(rendered.contains(&"rg".to_owned()));
            assert!(
                rendered.contains(&"rg.EXE".to_owned()) || rendered.contains(&"rg.exe".to_owned())
            );
            assert!(
                rendered.contains(&"rg.CMD".to_owned()) || rendered.contains(&"rg.cmd".to_owned())
            );
        } else {
            assert_eq!(rendered, vec!["rg".to_owned()]);
        }
    }

    #[test]
    fn create_windows_cmd_shim_writes_wrapper() {
        let base = temp_test_dir("windows-shim");
        fs::create_dir_all(&base).expect("base");
        let exe = base.join("tke.exe");
        fs::write(&exe, b"").expect("exe");
        create_windows_cmd_shim(&base, &exe, "rg").expect("shim");
        let wrapper = fs::read_to_string(base.join("rg.cmd")).expect("wrapper");
        assert!(wrapper.contains("tke.exe"));
        assert!(wrapper.contains("shim \"%~n0\" %*"));
    }

    #[test]
    fn capture_interactive_rewrites_explicit_source_and_output() {
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let base = temp_test_dir("capture-explicit");
        let source = base.join("rollout.jsonl");
        let output_dir = base.join("out");
        fs::create_dir_all(&base).expect("base");
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_11",
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
                "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
                "exit_code": 0,
                "status": "completed"
            }
        })
        .to_string();
        fs::write(&source, format!("{line}\n")).expect("write source");

        capture_interactive(Some(source.clone()), Some(output_dir.clone()), &cfg).expect("capture");

        let mirrored = output_dir.join("rollout.jsonl");
        let raw = fs::read_to_string(mirrored).expect("mirrored");
        let value = value_from_json(raw.lines().next().expect("line"));
        let nested = value_from_json(
            value["item"]["aggregated_output"]
                .as_str()
                .expect("aggregated_output")
                .trim_start_matches("__TKE__"),
        );
        assert_eq!(nested["sc"], "rg");
        assert_eq!(nested["sr"], "search");
    }

    #[test]
    fn capture_interactive_uses_codex_home_latest_rollout() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let base = temp_test_dir("capture-defaults");
        let codex_home = base.join("codex-home");
        let sessions = codex_home.join("sessions/2026/05/23");
        let project = base.join("project");
        fs::create_dir_all(&sessions).expect("sessions");
        fs::create_dir_all(&project).expect("project");
        let rollout = sessions.join("rollout-latest.jsonl");
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_12",
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
                "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
                "exit_code": 0,
                "status": "completed"
            }
        })
        .to_string();
        fs::write(&rollout, format!("{line}\n")).expect("write rollout");

        let original_cwd = std::env::current_dir().expect("cwd");
        let original_codex_home = std::env::var_os("CODEX_HOME");
        std::env::set_current_dir(&project).expect("chdir");
        set_env_var("CODEX_HOME", &codex_home);

        let result = capture_interactive(None, None, &cfg);

        if let Some(value) = original_codex_home {
            set_env_var("CODEX_HOME", value);
        } else {
            remove_env_var("CODEX_HOME");
        }
        std::env::set_current_dir(original_cwd).expect("restore cwd");

        result.expect("capture");
        let mirrored = project.join(".tke/interactive/rollout-latest.jsonl");
        assert!(mirrored.exists());
    }

    #[test]
    fn capture_interactive_errors_without_rollout() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cfg = Config::default();
        let base = temp_test_dir("capture-missing");
        let codex_home = base.join("codex-home");
        let project = base.join("project");
        fs::create_dir_all(codex_home.join("sessions")).expect("sessions");
        fs::create_dir_all(&project).expect("project");

        let original_cwd = std::env::current_dir().expect("cwd");
        let original_codex_home = std::env::var_os("CODEX_HOME");
        std::env::set_current_dir(&project).expect("chdir");
        set_env_var("CODEX_HOME", &codex_home);

        let err = capture_interactive(None, None, &cfg).expect_err("missing rollout");

        if let Some(value) = original_codex_home {
            set_env_var("CODEX_HOME", value);
        } else {
            remove_env_var("CODEX_HOME");
        }
        std::env::set_current_dir(original_cwd).expect("restore cwd");

        assert!(matches!(err, AppError::Usage(_)));
        assert!(
            err.to_string()
                .contains("could not find a codex rollout jsonl")
        );
    }

    #[test]
    fn interactive_tracker_writes_rewritten_rollout_copy() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let mut cfg = Config::default();
        cfg.min_trim_bytes = 1;
        let base = temp_test_dir("tracker-write");
        let sessions = base.join("sessions/2026/05/23");
        let output_dir = base.join("project/.tke/interactive");
        fs::create_dir_all(&sessions).expect("sessions");
        fs::create_dir_all(base.join("project")).expect("project");

        let rollout = sessions.join("rollout-test.jsonl");
        let line = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": "/bin/bash -lc 'cat /tmp/demo.rs | rg -n fn | head'",
                "aggregated_output": format!("{}\n", repeated_lines("1:pub fn alpha() {}", 160)),
                "exit_code": 0,
                "status": "completed"
            }
        })
        .to_string();
        fs::write(&rollout, format!("{line}\n")).expect("write rollout");

        let original_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(base.join("project")).expect("chdir");
        let tracker = InteractiveTracker {
            sessions_dir: base.join("sessions"),
            started_at_ms: 0,
        };
        tracker.finish(&cfg).expect("finish");
        std::env::set_current_dir(original_cwd).expect("restore cwd");

        let mirrored = output_dir.join("rollout-test.jsonl");
        let raw = fs::read_to_string(mirrored).expect("mirrored");
        let value = value_from_json(raw.lines().next().expect("line"));
        let nested = value_from_json(
            value["item"]["aggregated_output"]
                .as_str()
                .expect("aggregated_output")
                .trim_start_matches("__TKE__"),
        );
        assert_eq!(nested["sc"], "rg");
        assert_eq!(nested["sr"], "search");
    }

    #[test]
    fn interactive_tracker_skips_unmodified_rollouts() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let cfg = Config::default();
        let base = temp_test_dir("tracker-skip");
        let sessions = base.join("sessions/2026/05/23");
        fs::create_dir_all(&sessions).expect("sessions");
        fs::create_dir_all(base.join("project")).expect("project");
        fs::write(
            sessions.join("rollout-test.jsonl"),
            "{\"type\":\"message\",\"payload\":{\"text\":\"hello\"}}\n",
        )
        .expect("write rollout");

        let original_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(base.join("project")).expect("chdir");
        let tracker = InteractiveTracker {
            sessions_dir: base.join("sessions"),
            started_at_ms: 0,
        };
        tracker.finish(&cfg).expect("finish");
        std::env::set_current_dir(original_cwd).expect("restore cwd");

        assert!(
            !base
                .join("project/.tke/interactive/rollout-test.jsonl")
                .exists()
        );
    }
}
