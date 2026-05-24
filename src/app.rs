use crate::benchmark::build_benchmark_report;
use crate::e2e_report::compare_e2e;
use crate::shim::{create_shims, passthrough, run_agent_command, run_tool_command};
use crate::trim::{
    ShellKind, base_name, candidate_config_path, csv_list, default_head_lines,
    default_json_prefix, default_match_context, default_max_body_lines, default_max_matches,
    default_min_trim_bytes, default_output_trim, default_show_stats, default_tail_lines,
    detect_shell_kind, parse_usize, read_stdin_if_piped, render_activate_script,
    render_deactivate_script, resolve_real_command,
};
use serde::Deserialize;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;

const DEFAULT_AGENT_COMMANDS: &[&str] = &["codex", "claude", "gemini", "aider"];
const DEFAULT_TOOL_COMMANDS: &[&str] = &[
    "cat", "sed", "rg", "grep", "git", "cargo", "pytest", "npm", "pnpm", "yarn", "tail", "head",
    "dotnet", "go", "cmake", "ctest", "make", "ninja", "node", "ls", "find", "fd", "bat", "nl",
    "awk", "cut", "sort", "uniq", "wc", "tree", "xargs",
];

#[cfg(test)]
pub(crate) fn default_tool_commands() -> &'static [&'static str] {
    DEFAULT_TOOL_COMMANDS
}

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
    Install {
        bin_dir: Option<PathBuf>,
    },
    Activate {
        agents: Vec<String>,
        shim_dir: Option<PathBuf>,
        shell: Option<ShellKind>,
    },
    Run {
        name: String,
        args: Vec<String>,
        shim_dir: Option<PathBuf>,
    },
    Deactivate,
    CaptureInteractive {
        source: Option<PathBuf>,
        output: Option<PathBuf>,
    },
    CompareRollout {
        source: Option<PathBuf>,
    },
    CompareE2e {
        sources: Vec<PathBuf>,
        agent: Option<String>,
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

fn is_driver_name(invoked: &str) -> bool {
    matches!(
        invoked.to_ascii_lowercase().as_str(),
        "tke" | "tk" | "tke.exe" | "tk.exe"
    )
}

pub fn parse_dispatch(argv0: &str, args: Vec<String>) -> Result<Dispatch, AppError> {
    let invoked = base_name(argv0);
    if !is_driver_name(&invoked) {
        return Ok(Dispatch::Shim {
            name: invoked,
            args: args.into_iter().skip(1).collect(),
        });
    }

    let sub = args.get(1).map(String::as_str);
    if let Some(name) = sub
        && DEFAULT_AGENT_COMMANDS.iter().any(|agent| agent == &name)
    {
        return Ok(Dispatch::Run {
            name: name.to_owned(),
            args: args.into_iter().skip(2).collect(),
            shim_dir: None,
        });
    }
    match sub {
        None | Some("-h") | Some("--help") | Some("help") => Ok(Dispatch::Help),
        Some("install") => parse_install(args),
        Some("activate") | Some("env") => parse_activate(args),
        Some("run") => parse_run(args),
        Some("deactivate") => Ok(Dispatch::Deactivate),
        Some("capture-interactive") => parse_capture_interactive(args),
        Some("compare-rollout") => parse_compare_rollout(args),
        Some("compare-e2e") => parse_compare_e2e(args),
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
        "  tke <agent> [args ...]",
        "  tke install [--bin-dir PATH]",
        "  tke activate [--shim-dir PATH] [--shell SHELL] [agent ...]",
        "  tke env [--shim-dir PATH] [--shell SHELL] [agent ...]",
        "  tke run [--shim-dir PATH] <agent> [args ...]",
        "  tke deactivate",
        "  tke capture-interactive [--source PATH] [--output PATH]",
        "  tke compare-rollout [--source PATH]",
        "  tke compare-e2e [--source DIR]... [--agent codex|claude]",
        "  tke benchmark-commands [--check]",
        "  tke package-release",
        "",
        "Examples:",
        "  tke install",
        "  tke codex",
        "  tk codex",
        "  tke codex exec --json 'Reply with exactly OK.'",
        "  eval \"$(tke activate codex claude)\"",
        "  tke run codex",
        "  tke run codex exec --json 'Reply with exactly OK.'",
        "  eval \"$(tke activate --shim-dir ./.tke/shims codex)\"",
        "  tke capture-interactive",
        "  tke compare-rollout",
        "  tke compare-e2e",
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

pub fn run_wrapped(
    name: &str,
    args: &[String],
    shim_dir: Option<PathBuf>,
    config: &Config,
) -> Result<i32, AppError> {
    let cwd = env::current_dir()?;
    let shim_dir = shim_dir.unwrap_or_else(|| cwd.join(".tke").join("shims"));
    fs::create_dir_all(&shim_dir)?;

    let mut agents = config.agent_commands.clone();
    if !agents.iter().any(|agent| agent == name) {
        agents.push(name.to_owned());
    }
    create_shims(&shim_dir, &[name.to_owned()], &config.tool_commands)?;

    let shim_dir_abs = fs::canonicalize(&shim_dir).unwrap_or(shim_dir);
    let real_path =
        env::var("TKE_REAL_PATH").unwrap_or_else(|_| env::var("PATH").unwrap_or_default());
    let path =
        env::join_paths(std::iter::once(shim_dir_abs.clone()).chain(env::split_paths(&real_path)))
            .map_err(|err| AppError::Usage(format!("failed to construct PATH: {err}")))?;
    let stdin_payload = read_stdin_if_piped()?;
    let exe = env::current_exe()?;
    let shim_cmd = shim_dir_abs.join(name);
    let code = passthrough(
        &shim_cmd,
        args,
        Some(vec![
            ("PATH".to_owned(), Some(path)),
            ("TKE_BIN".to_owned(), Some(exe.into_os_string())),
            (
                "TKE_SHIM_DIR".to_owned(),
                Some(shim_dir_abs.clone().into_os_string()),
            ),
            ("TKE_REAL_PATH".to_owned(), Some(OsString::from(real_path))),
            (
                "TKE_AGENT_CMDS".to_owned(),
                Some(OsString::from(agents.join(","))),
            ),
            (
                "TKE_TOOL_CMDS".to_owned(),
                Some(OsString::from(config.tool_commands.join(","))),
            ),
        ]),
        stdin_payload,
        true,
    )?;
    Ok(code)
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

fn parse_install(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut bin_dir = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--bin-dir" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --bin-dir\n\n{}", usage()))
                })?;
                bin_dir = Some(PathBuf::from(value));
            }
            other => {
                return Err(AppError::Usage(format!(
                    "unknown install arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::Install { bin_dir })
}

fn parse_run(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut shim_dir = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        if arg == "--shim-dir" {
            let value = iter.next().ok_or_else(|| {
                AppError::Usage(format!("missing value for --shim-dir\n\n{}", usage()))
            })?;
            shim_dir = Some(PathBuf::from(value));
            continue;
        }
        let name = arg;
        let args = iter.collect();
        return Ok(Dispatch::Run {
            name,
            args,
            shim_dir,
        });
    }
    Err(AppError::Usage(format!(
        "missing agent name for run\n\n{}",
        usage()
    )))
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

fn parse_compare_e2e(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut sources = Vec::new();
    let mut agent = None;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --source\n\n{}", usage()))
                })?;
                sources.push(PathBuf::from(value));
            }
            "--agent" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --agent\n\n{}", usage()))
                })?;
                agent = Some(value);
            }
            other => {
                return Err(AppError::Usage(format!(
                    "unknown compare-e2e arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::CompareE2e { sources, agent })
}

pub fn benchmark_commands(config: &Config, check: bool) -> Result<(), AppError> {
    let report = build_benchmark_report(config)?;
    if check {
        crate::benchmark::benchmark_report_check(&report)?;
    }
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

pub fn compare_e2e_command(
    sources: Vec<PathBuf>,
    agent: Option<String>,
    config: &Config,
) -> Result<(), AppError> {
    compare_e2e(sources, agent, config)
}
