use crate::adapter::rewrite_agent_transcript;
use crate::app::{AppError, Config};
use crate::benchmark_data::estimate_text_tokens;
use crate::rewrite::{LivePipelineDecision, detect_linux_parent_pipeline, live_pipeline_decision};
use crate::rollout_io::InteractiveTracker;
use crate::trim::{
    CommandKind, TrimEnvelope, TrimProfile, TrimStats, classify_command, collect_build_summary,
    collect_diff_summary, collect_git_status_summary, collect_kept_ranges,
    collect_path_list_kept_ranges, collect_path_list_summary, collect_profile_chunks,
    collect_table_kept_ranges, collect_table_summary, compact_args, compact_json_body_for_command,
    compute_omitted_ranges, create_single_shim, exit_code, match_terms, merge_ranges,
    profile_limits, read_stdin_if_piped, read_stdin_if_piped_non_blocking, real_path_string,
    select_profile, should_force_trim, take_head, take_tail,
};
#[cfg(unix)]
use nix::pty::{ForkptyResult, Winsize, forkpty};
#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::sys::termios::{SetArg, Termios, cfmakeraw, tcgetattr, tcsetattr};
#[cfg(unix)]
use nix::sys::wait::{WaitStatus, waitpid};
#[cfg(unix)]
use nix::unistd::{Pid, execve, read as nix_read, write as nix_write};
use std::collections::BTreeSet;
use std::env;
#[cfg(unix)]
use std::ffi::CString;
use std::ffi::OsString;
#[cfg(unix)]
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::Read;
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
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
        let tracker = InteractiveTracker::start_for_agent(name);
        let code = passthrough(real_cmd, args, Some(envs), stdin_payload, true)?;
        if let Some(tracker) = tracker {
            tracker.finish(config)?;
        }
        return Ok(code);
    }

    if should_bridge_agent_with_pty(name, &stdin_payload) {
        let tracker = InteractiveTracker::start_for_agent(name);
        let code = passthrough_with_native_pty(real_cmd, args, Some(envs), true)?;
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

fn should_bridge_agent_with_pty(name: &str, stdin_payload: &Option<Vec<u8>>) -> bool {
    cfg!(target_os = "linux") && name == "codex" && stdin_payload.is_none()
}

fn should_passthrough_agent_output(name: &str) -> bool {
    if name != "claude" {
        return false;
    }
    // Claude agent output is always passed through. Tool compression happens
    // via PATH shims (run_tool_command), not via agent output capture.
    // Claude's stdout is either the interactive TUI (TTY) or the final answer
    // text (-p mode); neither contains raw tool output worth compressing.
    true
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
    let stdin_payload = read_stdin_if_piped_non_blocking()?;
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
    let mut cmd = build_command(real_cmd, args);
    cmd.stdin(if stdin_payload.is_some() {
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

#[cfg(unix)]
fn passthrough_with_native_pty(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    keep_shim_path: bool,
) -> Result<i32, AppError> {
    let terminal = ParentTerminal::capture();
    let mut command_env = std::env::vars_os().collect::<Vec<_>>();
    if !keep_shim_path {
        command_env.retain(|(key, _)| key != "PATH");
        command_env.push(("PATH".into(), real_path_string().into()));
    }
    if let Some(envs) = extra_envs {
        for (key, value) in envs {
            command_env.retain(|(existing, _)| existing != &std::ffi::OsString::from(&key));
            if let Some(v) = value {
                command_env.push((key.into(), v));
            }
        }
    }

    let c_path = CString::new(real_cmd.as_os_str().to_string_lossy().as_bytes())
        .map_err(|_| AppError::Usage("command path contains interior NUL byte".to_owned()))?;
    let mut c_args = Vec::with_capacity(args.len() + 1);
    c_args.push(c_path.clone());
    for arg in args {
        c_args.push(
            CString::new(arg.as_bytes()).map_err(|_| {
                AppError::Usage("command arg contains interior NUL byte".to_owned())
            })?,
        );
    }
    let argv = c_args.iter().map(|arg| arg.as_c_str()).collect::<Vec<_>>();
    let c_env = command_env
        .into_iter()
        .map(|(key, value)| {
            let mut joined = key.into_encoded_bytes();
            joined.push(b'=');
            joined.extend(value.into_encoded_bytes());
            CString::new(joined)
                .map_err(|_| AppError::Usage("environment contains interior NUL byte".to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let envp = c_env
        .iter()
        .map(|entry| entry.as_c_str())
        .collect::<Vec<_>>();

    // SAFETY: forkpty follows the libc fork contract. We only call execve in the child.
    let fork = unsafe { forkpty(terminal.winsize.as_ref(), terminal.termios.as_ref())? };
    match fork {
        ForkptyResult::Child => {
            let _ = execve(c_path.as_c_str(), &argv, &envp);
            std::process::exit(127);
        }
        ForkptyResult::Parent { child, master } => relay_native_pty(child, master, terminal),
    }
}

#[cfg(not(unix))]
fn passthrough_with_native_pty(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    keep_shim_path: bool,
) -> Result<i32, AppError> {
    passthrough(real_cmd, args, extra_envs, None, keep_shim_path)
}

#[cfg(unix)]
fn wait_for_child(child: Pid) -> Result<i32, AppError> {
    loop {
        match waitpid(child, None)? {
            WaitStatus::Exited(_, code) => return Ok(code),
            WaitStatus::Signaled(_, signal, _) => return Ok(128 + signal as i32),
            _ => {}
        }
    }
}

#[cfg(unix)]
struct ParentTerminal {
    control_fd: Option<RawFd>,
    termios: Option<Termios>,
    winsize: Option<Winsize>,
    input: TerminalInput,
}

#[cfg(unix)]
enum TerminalInput {
    Stdin,
    Tty(File),
}

#[cfg(unix)]
impl ParentTerminal {
    fn capture() -> Self {
        let input = if io::stdin().is_terminal() {
            TerminalInput::Stdin
        } else {
            match OpenOptions::new().read(true).write(true).open("/dev/tty") {
                Ok(file) => TerminalInput::Tty(file),
                Err(_) => TerminalInput::Stdin,
            }
        };

        let (control_fd, termios, winsize) = match &input {
            TerminalInput::Tty(file) => {
                let fd = file.as_raw_fd();
                (
                    Some(fd),
                    tcgetattr(file).ok(),
                    read_winsize(fd).or_else(env_winsize),
                )
            }
            TerminalInput::Stdin => capture_standard_terminal(),
        };

        if control_fd.is_none() {
            if let Some((fd, termios, winsize)) =
                capture_terminal_from_fd(1).or_else(|| capture_terminal_from_fd(2))
            {
                return Self {
                    control_fd: Some(fd),
                    termios,
                    winsize: winsize.or_else(env_winsize),
                    input,
                };
            }
        }

        Self {
            control_fd,
            termios,
            winsize,
            input,
        }
    }

    fn raw_mode_guard(&self) -> Result<Option<RawTerminalGuard>, AppError> {
        let Some(fd) = self.control_fd else {
            return Ok(None);
        };
        let Some(original) = self.termios.clone() else {
            return Ok(None);
        };
        let mut raw = original.clone();
        cfmakeraw(&mut raw);
        tcsetattr(borrow_fd(fd), SetArg::TCSANOW, &raw)?;
        Ok(Some(RawTerminalGuard { fd, original }))
    }
}

#[cfg(unix)]
struct RawTerminalGuard {
    fd: RawFd,
    original: Termios,
}

#[cfg(unix)]
impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        let _ = tcsetattr(borrow_fd(self.fd), SetArg::TCSANOW, &self.original);
    }
}

#[cfg(unix)]
fn capture_standard_terminal() -> (Option<RawFd>, Option<Termios>, Option<Winsize>) {
    capture_terminal_from_fd(0)
        .or_else(|| capture_terminal_from_fd(1))
        .or_else(|| capture_terminal_from_fd(2))
        .map(|(fd, termios, winsize)| (Some(fd), termios, winsize.or_else(env_winsize)))
        .unwrap_or((None, None, env_winsize()))
}

#[cfg(unix)]
fn capture_terminal_from_fd(fd: RawFd) -> Option<(RawFd, Option<Termios>, Option<Winsize>)> {
    let termios = tcgetattr(borrow_fd(fd)).ok()?;
    let winsize = read_winsize(fd);
    Some((fd, Some(termios), winsize))
}

#[cfg(unix)]
fn read_winsize(fd: RawFd) -> Option<Winsize> {
    let mut winsize = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `winsize` is valid for writes and `fd` is only borrowed for the ioctl duration.
    let rc = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCGWINSZ, &mut winsize) };
    if rc == 0 && winsize.ws_row > 0 && winsize.ws_col > 0 {
        Some(winsize)
    } else {
        None
    }
}

#[cfg(unix)]
fn env_winsize() -> Option<Winsize> {
    let rows = env::var("LINES").ok()?.parse::<u16>().ok()?;
    let cols = env::var("COLUMNS").ok()?.parse::<u16>().ok()?;
    if rows == 0 || cols == 0 {
        return None;
    }
    Some(Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    })
}

#[cfg(unix)]
fn borrow_fd(fd: RawFd) -> BorrowedFd<'static> {
    // SAFETY: the caller only passes process-owned terminal fds that remain valid for the call.
    unsafe { BorrowedFd::borrow_raw(fd) }
}

#[cfg(unix)]
fn relay_native_pty(
    child: Pid,
    master: OwnedFd,
    terminal: ParentTerminal,
) -> Result<i32, AppError> {
    let _raw_guard = terminal.raw_mode_guard()?;
    let master = Arc::new(master);
    let stdin_master = Arc::clone(&master);
    let stdout_master = Arc::clone(&master);
    let resize_master = Arc::clone(&master);
    let shutdown = Arc::new(AtomicBool::new(false));
    let resize_shutdown = Arc::clone(&shutdown);

    let resize_thread = terminal.control_fd.map(|control_fd| {
        let child = child;
        thread::spawn(move || -> Result<(), AppError> {
            let mut last = read_winsize(control_fd);
            while !resize_shutdown.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                let current = read_winsize(control_fd);
                if current.is_some() && current != last {
                    if let Some(winsize) = current {
                        apply_winsize(resize_master.as_raw_fd(), &winsize)?;
                        let _ = kill(child, Signal::SIGWINCH);
                        last = Some(winsize);
                    }
                }
            }
            Ok(())
        })
    });

    let stdin_thread = thread::spawn(move || -> Result<(), AppError> {
        let mut buf = [0_u8; 4096];
        match terminal.input {
            TerminalInput::Stdin => {
                let mut stdin = io::stdin();
                loop {
                    let read = stdin.read(&mut buf)?;
                    if read == 0 {
                        break;
                    }
                    write_fd_all(&stdin_master, &buf[..read])?;
                }
            }
            TerminalInput::Tty(mut file) => loop {
                let read = file.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                write_fd_all(&stdin_master, &buf[..read])?;
            },
        }
        Ok(())
    });

    let stdout_thread = thread::spawn(move || -> Result<(), AppError> {
        let mut stdout = io::stdout();
        let mut buf = [0_u8; 4096];
        loop {
            let read = match nix_read(&*stdout_master, &mut buf) {
                Ok(read) => read,
                Err(nix::errno::Errno::EIO) => break,
                Err(err) => return Err(err.into()),
            };
            if read == 0 {
                break;
            }
            write_all_resilient(&mut stdout, &buf[..read])?;
            stdout.flush()?;
        }
        Ok(())
    });

    let status = wait_for_child(child);
    shutdown.store(true, Ordering::Relaxed);
    let _ = stdout_thread.join();
    if let Some(resize_thread) = resize_thread {
        let _ = resize_thread.join();
    }
    drop(stdin_thread);
    status
}

#[cfg(unix)]
fn write_fd_all(fd: &OwnedFd, mut bytes: &[u8]) -> Result<(), AppError> {
    while !bytes.is_empty() {
        let written = nix_write(fd, bytes)?;
        bytes = &bytes[written..];
    }
    Ok(())
}

#[cfg(unix)]
fn apply_winsize(fd: RawFd, winsize: &Winsize) -> Result<(), AppError> {
    // SAFETY: `winsize` points to a valid struct for the duration of the ioctl call.
    let rc = unsafe { nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, winsize) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}

pub fn run_tty_wrapped(
    name: &str,
    args: &[String],
    shim_dir: Option<std::path::PathBuf>,
    config: &Config,
) -> Result<i32, AppError> {
    let shim_home = match shim_dir {
        Some(path) => {
            std::fs::create_dir_all(&path)?;
            RuntimeShimDir::persistent(path)
        }
        None => RuntimeShimDir::temporary()?,
    };
    let shim_dir = shim_home.path().to_path_buf();

    let mut agents = config.agent_commands.clone();
    if !agents.iter().any(|agent| agent == name) {
        agents.push(name.to_owned());
    }
    create_shims(&shim_dir, &agents, &config.tool_commands)?;

    let shim_dir_abs =
        crate::trim::normalize_runtime_path(std::fs::canonicalize(&shim_dir).unwrap_or(shim_dir));
    let real_path =
        env::var("TKE_REAL_PATH").unwrap_or_else(|_| env::var("PATH").unwrap_or_default());
    let path =
        env::join_paths(std::iter::once(shim_dir_abs.clone()).chain(env::split_paths(&real_path)))
            .map_err(|err| AppError::Usage(format!("failed to construct PATH: {err}")))?;
    let exe = env::current_exe()?;
    let envs = vec![
        ("PATH".to_owned(), Some(path)),
        ("TKE_BIN".to_owned(), Some(exe.into_os_string())),
        (
            "TKE_SHIM_DIR".to_owned(),
            Some(shim_dir_abs.clone().into_os_string()),
        ),
        ("TKE_REAL_PATH".to_owned(), Some(OsString::from(real_path))),
        (
            "TKE_AGENT_CMDS".to_owned(),
            Some(OsString::from(config.agent_commands.join(","))),
        ),
        (
            "TKE_TOOL_CMDS".to_owned(),
            Some(OsString::from(config.tool_commands.join(","))),
        ),
    ];

    let target_cmd = crate::trim::resolve_real_command(name)?;
    let target_args = args.to_vec();
    let tracker = InteractiveTracker::start_for_agent(name);
    let code = passthrough_with_native_pty(&target_cmd, &target_args, Some(envs), true)?;
    if let Some(tracker) = tracker {
        tracker.finish(config)?;
    }
    Ok(code)
}

struct RuntimeShimDir {
    path: std::path::PathBuf,
    cleanup_on_drop: bool,
}

impl RuntimeShimDir {
    fn persistent(path: std::path::PathBuf) -> Self {
        Self {
            path,
            cleanup_on_drop: false,
        }
    }

    fn temporary() -> Result<Self, AppError> {
        let path = crate::app::default_runtime_shim_dir();
        std::fs::create_dir_all(&path)?;
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
            let _ = std::fs::remove_dir_all(
                self.path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.path.clone()),
            );
        }
    }
}

pub(crate) fn capture_process(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    stdin_payload: Option<Vec<u8>>,
    keep_shim_path: bool,
) -> Result<std::process::Output, AppError> {
    let mut cmd = build_command(real_cmd, args);
    cmd.stdout(Stdio::piped())
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

fn build_command(real_cmd: &Path, args: &[String]) -> Command {
    #[cfg(windows)]
    {
        if should_spawn_via_cmd(real_cmd) {
            let mut cmd = Command::new("cmd.exe");
            cmd.arg("/C").arg(real_cmd);
            cmd.args(args);
            return cmd;
        }
    }

    let mut cmd = Command::new(real_cmd);
    cmd.args(args);
    cmd
}

#[cfg(windows)]
fn should_spawn_via_cmd(real_cmd: &Path) -> bool {
    matches!(
        real_cmd
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("cmd" | "bat")
    )
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
    let profile = select_profile(name, args, kind, &lines);
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
    let profile = select_profile(name, args, kind, &lines);
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
    let diff_summary = if forced && profile == TrimProfile::Diff {
        collect_diff_summary(&lines)
    } else {
        None
    };
    let log_summary = if forced && profile == TrimProfile::Log {
        Some(crate::log_profile::collect_log_summary(&lines))
    } else {
        None
    };
    let git_status = if forced && profile == TrimProfile::GitStatus {
        collect_git_status_summary(&lines)
    } else {
        None
    };
    let build_summary = if forced && matches!(profile, TrimProfile::Log) {
        collect_build_summary(name, &lines)
    } else {
        None
    };
    let json_body = if forced && profile == TrimProfile::Json {
        compact_json_body_for_command(name, text)
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
    } else if profile == TrimProfile::GitStatus || profile == TrimProfile::Json {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0)
    } else {
        let matches = collect_profile_chunks(&lines, &terms, profile, limits);
        let use_log_chunks_only = forced && profile == TrimProfile::Log;
        let head = if use_log_chunks_only {
            Vec::new()
        } else if total_lines == 0 {
            Vec::new()
        } else {
            take_head(&lines, limits.head_lines)
        };
        let tail = if use_log_chunks_only {
            Vec::new()
        } else if total_lines == 0 {
            Vec::new()
        } else {
            take_tail(&lines, limits.tail_lines)
        };
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
        df: diff_summary,
        gs: git_status,
        bd: build_summary,
        b: json_body.or(body),
    };
    Ok(serde_json::to_string(&envelope)?)
}
