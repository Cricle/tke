use crate::AppError;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn install_self(bin_dir: Option<PathBuf>) -> Result<(), AppError> {
    let exe = env::current_exe()?;
    let dest_dir = bin_dir.unwrap_or_else(default_user_bin_dir);
    fs::create_dir_all(&dest_dir)?;

    let exe_name = if cfg!(windows) { "tke.exe" } else { "tke" };
    let dest = dest_dir.join(exe_name);
    install_binary_atomically(&exe, &dest)?;

    install_short_alias(&dest_dir, &dest)?;

    let report = serde_json::json!({
        "v": 1,
        "installed": dest.display().to_string(),
        "alias": install_alias_path(&dest_dir).display().to_string(),
        "bin_dir": dest_dir.display().to_string(),
        "path_hint": path_hint(&dest_dir),
    });
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn install_binary_atomically(src: &Path, dest: &Path) -> Result<(), AppError> {
    let pid = std::process::id();
    let tmp_name = format!(
        ".{}.install-{pid}.tmp",
        dest.file_name().unwrap().to_string_lossy()
    );
    let tmp = dest.with_file_name(tmp_name);
    fs::copy(src, &tmp)?;

    #[cfg(windows)]
    {
        if dest.exists() {
            fs::remove_file(dest)?;
        }
    }

    match fs::rename(&tmp, dest) {
        Ok(_) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err.into())
        }
    }
}

fn default_user_bin_dir() -> PathBuf {
    if cfg!(windows) {
        if let Ok(local) = env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("Microsoft").join("WindowsApps");
        }
        if let Ok(home) = env::var("USERPROFILE") {
            return PathBuf::from(home)
                .join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("WindowsApps");
        }
        return PathBuf::from(".").join("bin");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local").join("bin");
    }
    PathBuf::from(".").join("bin")
}

fn install_alias_path(bin_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        bin_dir.join("tk.cmd")
    } else {
        bin_dir.join("tk")
    }
}

fn install_short_alias(bin_dir: &Path, dest: &Path) -> Result<(), AppError> {
    let alias = install_alias_path(bin_dir);
    if alias.exists() {
        fs::remove_file(&alias)?;
    }
    if cfg!(windows) {
        let exe = dest.to_string_lossy().replace('"', "\"\"");
        fs::write(
            &alias,
            format!("@echo off\r\n\"{exe}\" %*\r\nexit /b %ERRORLEVEL%\r\n"),
        )?;
        return Ok(());
    }
    match fs::hard_link(dest, &alias) {
        Ok(_) => Ok(()),
        Err(_) => {
            fs::copy(dest, &alias)?;
            Ok(())
        }
    }
}

fn path_hint(bin_dir: &Path) -> String {
    if cfg!(windows) {
        format!(
            "Add `{}` to PATH in PowerShell, CMD, or Windows Environment Variables if it is not already present.",
            bin_dir.display()
        )
    } else {
        format!(
            "Add `export PATH=\"{}:$PATH\"` to your shell profile if needed.",
            bin_dir.display()
        )
    }
}
