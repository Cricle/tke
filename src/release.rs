use crate::AppError;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn install_self(bin_dir: Option<PathBuf>) -> Result<(), AppError> {
    let exe = env::current_exe()?;
    let dest_dir = bin_dir.unwrap_or_else(default_user_bin_dir);
    fs::create_dir_all(&dest_dir)?;

    let exe_name = if cfg!(windows) { "tke.exe" } else { "tke" };
    let dest = dest_dir.join(exe_name);
    fs::copy(&exe, &dest)?;

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

pub fn package_release(_: &crate::Config) -> Result<(), AppError> {
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
