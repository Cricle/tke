use crate::app::AppError;
#[cfg(unix)]
use crate::shim::write_all_resilient;
use crate::trim::real_path_string;
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
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) fn passthrough_with_native_pty(
    real_cmd: &Path,
    args: &[String],
    extra_envs: Option<Vec<(String, Option<OsString>)>>,
    keep_shim_path: bool,
) -> Result<i32, AppError> {
    #[cfg(unix)]
    {
        passthrough_with_native_pty_unix(real_cmd, args, extra_envs, keep_shim_path)
    }
    #[cfg(not(unix))]
    {
        crate::shim::passthrough(real_cmd, args, extra_envs, None, keep_shim_path)
    }
}

#[cfg(unix)]
fn passthrough_with_native_pty_unix(
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

        if control_fd.is_none()
            && let Some((fd, termios, winsize)) =
                capture_terminal_from_fd(1).or_else(|| capture_terminal_from_fd(2))
        {
            return Self {
                control_fd: Some(fd),
                termios,
                winsize: winsize.or_else(env_winsize),
                input,
            };
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
        thread::spawn(move || -> Result<(), AppError> {
            let mut last = read_winsize(control_fd);
            while !resize_shutdown.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                let current = read_winsize(control_fd);
                if current.is_some()
                    && current != last
                    && let Some(winsize) = current
                {
                    apply_winsize(resize_master.as_raw_fd(), &winsize)?;
                    let _ = kill(child, Signal::SIGWINCH);
                    last = Some(winsize);
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
