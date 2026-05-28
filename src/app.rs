use crate::benchmark::build_benchmark_report;
use crate::e2e_report::compare_e2e;
use crate::rollout_io::{
    InteractiveTracker, UsageStatsFilter, UsageStatsGroupBy, UsageStatsSortBy,
};
use crate::shim::{create_shims, passthrough, run_agent_command, run_tool_command};
use crate::trim::{
    ShellKind, base_name, candidate_config_path, csv_list, default_head_lines, default_json_prefix,
    default_match_context, default_max_body_lines, default_max_matches, default_min_trim_bytes,
    default_output_trim, default_show_stats, default_tail_lines, detect_shell_kind,
    normalize_runtime_path, parse_usize, read_stdin_if_piped, render_activate_script,
    render_deactivate_script, resolve_real_command, shim_command_path,
};
#[cfg(unix)]
use nix::errno::Errno;
use serde::Deserialize;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process;

const DEFAULT_AGENT_COMMANDS: &[&str] = &["codex", "claude", "gemini", "aider"];
const DEFAULT_TOOL_COMMANDS: &[&str] = &[
    "cat",
    "Get-Content",
    "Get-Clipboard",
    "gc",
    "type",
    "sed",
    "gsed",
    "rg",
    "grep",
    "ggrep",
    "Select-String",
    "sls",
    "findstr",
    "git",
    "cargo",
    "pytest",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "pip",
    "uv",
    "poetry",
    "mvn",
    "gradle",
    "gradlew",
    "javac",
    "java",
    "bundle",
    "composer",
    "tail",
    "head",
    "more",
    "more.com",
    "dotnet",
    "go",
    "cmake",
    "ctest",
    "make",
    "ninja",
    "node",
    "ls",
    "gls",
    "Get-ChildItem",
    "gci",
    "dir",
    "find",
    "gfind",
    "mdfind",
    "mdls",
    "fd",
    "bat",
    "nl",
    "awk",
    "Where-Object",
    "cut",
    "sort",
    "Sort-Object",
    "uniq",
    "guniq",
    "wc",
    "gwc",
    "Measure-Object",
    "tree",
    "xargs",
    "jq",
    "plutil",
    "curl",
    "open",
    "qlmanage",
    "python",
    "python3",
    "docker",
    "ps",
    "ss",
    "netstat",
    "systemctl",
    "tr",
    "ghead",
    "gtail",
    "gdu",
    "gdf",
    "perl",
    "xattr",
    "du",
    "df",
    "pbpaste",
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

#[cfg(unix)]
impl From<Errno> for AppError {
    fn from(value: Errno) -> Self {
        Self::Io(io::Error::from_raw_os_error(value as i32))
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
                .any(|pattern| path_pattern_matches(arg, pattern))
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

fn path_pattern_matches(arg: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let arg_path = Path::new(arg);
    let pattern_path = Path::new(pattern);
    if arg_path == pattern_path {
        return true;
    }

    let arg_components = normalized_path_components(arg_path);
    let pattern_components = normalized_path_components(pattern_path);
    if pattern_components.is_empty() || arg_components.len() < pattern_components.len() {
        return false;
    }
    arg_components.starts_with(&pattern_components)
}

fn normalized_path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::RootDir => Some("/".to_owned()),
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

#[derive(Debug)]
pub enum Dispatch {
    Help,
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
    Tty {
        name: String,
        args: Vec<String>,
        shim_dir: Option<PathBuf>,
    },
    Deactivate,
    CaptureInteractive {
        source: Option<PathBuf>,
        output: Option<PathBuf>,
    },
    Stats {
        sources: Vec<PathBuf>,
        limit: Option<usize>,
        filter: UsageStatsFilter,
        group_by: UsageStatsGroupBy,
        changed_only: bool,
        refresh: bool,
        top: usize,
        sort_by: UsageStatsSortBy,
        json: bool,
    },
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

fn normalize_invoked_name(invoked: &str) -> String {
    #[cfg(windows)]
    if invoked.len() > 4 && invoked.to_ascii_lowercase().ends_with(".exe") {
        return invoked[..invoked.len() - 4].to_owned();
    }

    invoked.to_owned()
}

pub fn parse_dispatch(argv0: &str, args: Vec<String>) -> Result<Dispatch, AppError> {
    let invoked = normalize_invoked_name(&base_name(argv0));
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
        Some("activate") => parse_activate(args),
        Some("run") => parse_run(args),
        Some("tty") => parse_tty(args),
        Some("deactivate") => Ok(Dispatch::Deactivate),
        Some("capture-interactive") => parse_capture_interactive(args),
        Some("stats") => parse_stats(args),
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
        "  tke activate [--shim-dir PATH] [--shell SHELL] [agent ...]",

        "  tke run [--shim-dir PATH] <agent> [args ...]",
        "  tke tty [--shim-dir PATH] <command> [args ...]",
        "  tke deactivate",
        "  tke capture-interactive [--source PATH] [--output PATH]",
        "  tke stats [--source PATH]... [--limit N] [--profile NAME] [--command NAME] [--agent codex|claude] [--by day|profile|command|agent] [--changed-only] [--refresh] [--top N] [--sort saved|ratio|low-ratio|samples] [--json]",
        "",
        "Examples:",
        "  tke codex",
        "  tk codex",
        "  tke codex exec --json 'Reply with exactly OK.'",
        "  eval \"$(tke activate codex claude)\"",
        "  tke run codex",
        "  tke run codex exec --json 'Reply with exactly OK.'",
        "  tke tty codex",
        "  eval \"$(tke activate --shim-dir ./.tke/shims codex)\"",
        "  tke capture-interactive",
        "  tke stats",
        "  tke stats --json --limit 10",
        "  tke stats --profile pathlist --by command",
        "  tke stats --changed-only --top 8 --sort ratio",
        "  tke stats --by command --sort low-ratio",
        "  tke stats --refresh",
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

    let shim_dir = shim_dir.unwrap_or_else(default_activate_shim_dir);
    fs::create_dir_all(&shim_dir)?;
    create_shims(&shim_dir, &selected_agents, &config.tool_commands)?;

    let exe = env::current_exe()?;
    let shim_dir_abs = normalize_runtime_path(fs::canonicalize(&shim_dir).unwrap_or(shim_dir));
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
    let shim_home = prepare_runtime_shim_dir(shim_dir)?;
    let shim_dir = shim_home.path().to_path_buf();

    let mut agents = config.agent_commands.clone();
    if !agents.iter().any(|agent| agent == name) {
        agents.push(name.to_owned());
    }
    create_shims(&shim_dir, &[name.to_owned()], &config.tool_commands)?;

    let shim_dir_abs = normalize_runtime_path(fs::canonicalize(&shim_dir).unwrap_or(shim_dir));
    let real_path =
        env::var("TKE_REAL_PATH").unwrap_or_else(|_| env::var("PATH").unwrap_or_default());
    let path =
        env::join_paths(std::iter::once(shim_dir_abs.clone()).chain(env::split_paths(&real_path)))
            .map_err(|err| AppError::Usage(format!("failed to construct PATH: {err}")))?;
    let stdin_payload = read_stdin_if_piped()?;
    let exe = env::current_exe()?;
    let shim_cmd = shim_command_path(&shim_dir_abs, name);
    let tracker = InteractiveTracker::start_for_agent(name);
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
    if let Some(tracker) = tracker {
        tracker.finish(config)?;
    }
    Ok(code)
}

fn prepare_runtime_shim_dir(shim_dir: Option<PathBuf>) -> Result<RuntimeShimDir, AppError> {
    match shim_dir {
        Some(path) => {
            fs::create_dir_all(&path)?;
            Ok(RuntimeShimDir::persistent(path))
        }
        None => RuntimeShimDir::temporary(),
    }
}

pub(crate) fn default_runtime_shim_dir() -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "tke-run-{}-{}",
        process::id(),
        crate::trim::now_millis()
    ));
    path.push("shims");
    path
}

pub(crate) fn default_activate_shim_dir() -> PathBuf {
    env::temp_dir().join("tke").join("shims")
}

struct RuntimeShimDir {
    path: PathBuf,
    cleanup_on_drop: bool,
}

impl RuntimeShimDir {
    fn persistent(path: PathBuf) -> Self {
        Self {
            path,
            cleanup_on_drop: false,
        }
    }

    fn temporary() -> Result<Self, AppError> {
        let path = default_runtime_shim_dir();
        fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            cleanup_on_drop: true,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeShimDir {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = fs::remove_dir_all(
                self.path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.path.clone()),
            );
        }
    }
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

fn parse_tty(args: Vec<String>) -> Result<Dispatch, AppError> {
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
        return Ok(Dispatch::Tty {
            name,
            args,
            shim_dir,
        });
    }
    Err(AppError::Usage(format!(
        "missing command name for tty\n\n{}",
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



fn parse_stats(args: Vec<String>) -> Result<Dispatch, AppError> {
    let mut sources = Vec::new();
    let mut limit = None;
    let mut filter = UsageStatsFilter::None;
    let mut group_by = UsageStatsGroupBy::Day;
    let mut changed_only = false;
    let mut refresh = false;
    let mut top = 10usize;
    let mut sort_by = UsageStatsSortBy::Saved;
    let mut json = false;
    let mut iter = args.into_iter().skip(2);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --source\n\n{}", usage()))
                })?;
                sources.push(PathBuf::from(value));
            }
            "--limit" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --limit\n\n{}", usage()))
                })?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    AppError::Usage(format!("invalid --limit `{value}`\n\n{}", usage()))
                })?;
                limit = Some(parsed);
            }
            "--profile" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --profile\n\n{}", usage()))
                })?;
                filter = UsageStatsFilter::Profile(value);
            }
            "--command" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --command\n\n{}", usage()))
                })?;
                filter = UsageStatsFilter::Command(value);
            }
            "--agent" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --agent\n\n{}", usage()))
                })?;
                filter = UsageStatsFilter::Agent(value);
            }
            "--by" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --by\n\n{}", usage()))
                })?;
                group_by = match value.as_str() {
                    "day" => UsageStatsGroupBy::Day,
                    "profile" => UsageStatsGroupBy::Profile,
                    "command" => UsageStatsGroupBy::Command,
                    "agent" => UsageStatsGroupBy::Agent,
                    _ => {
                        return Err(AppError::Usage(format!(
                            "invalid --by `{value}`\n\n{}",
                            usage()
                        )));
                    }
                };
            }
            "--changed-only" => changed_only = true,
            "--refresh" => refresh = true,
            "--top" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --top\n\n{}", usage()))
                })?;
                top = value.parse::<usize>().map_err(|_| {
                    AppError::Usage(format!("invalid --top `{value}`\n\n{}", usage()))
                })?;
            }
            "--sort" => {
                let value = iter.next().ok_or_else(|| {
                    AppError::Usage(format!("missing value for --sort\n\n{}", usage()))
                })?;
                sort_by = match value.as_str() {
                    "saved" => UsageStatsSortBy::Saved,
                    "ratio" => UsageStatsSortBy::Ratio,
                    "low-ratio" => UsageStatsSortBy::LowRatio,
                    "samples" => UsageStatsSortBy::Samples,
                    _ => {
                        return Err(AppError::Usage(format!(
                            "invalid --sort `{value}`\n\n{}",
                            usage()
                        )));
                    }
                };
            }
            "--json" => json = true,
            other => {
                return Err(AppError::Usage(format!(
                    "unknown stats arg `{other}`\n\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Dispatch::Stats {
        sources,
        limit,
        filter,
        group_by,
        changed_only,
        refresh,
        top,
        sort_by,
        json,
    })
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
