use std::path::{Path, PathBuf};
use std::process::Command;
use crate::platform::{current_os, OsFamily};

#[derive(Debug, Clone)]
pub struct UvInfo {
    pub path: PathBuf,
    pub version: String,
}

/// Detect if `uv` is installed on the host system
pub fn find_uv() -> Option<UvInfo> {
    // 1. Check standard PATH
    if let Ok(path) = which::which("uv") {
        if let Some(version) = get_uv_version(&path) {
            return Some(UvInfo { path, version });
        }
    }

    // 2. Check user-specific default installation paths
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();
        let candidates = vec![
            home.join(".cargo").join("bin").join(if cfg!(windows) { "uv.exe" } else { "uv" }),
            home.join(".local").join("bin").join("uv"),
            home.join("bin").join("uv"),
            #[cfg(windows)]
            home.join("AppData").join("Local").join("Programs").join("uv").join("uv.exe"),
            #[cfg(target_os = "macos")]
            PathBuf::from("/opt/homebrew/bin/uv"),
            #[cfg(target_os = "macos")]
            PathBuf::from("/usr/local/bin/uv"),
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
            PathBuf::from("/usr/local/bin/uv"),
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
            PathBuf::from("/usr/bin/uv"),
        ];

        for candidate in candidates {
            if candidate.exists() && candidate.is_file() {
                if let Some(version) = get_uv_version(&candidate) {
                    return Some(UvInfo { path: candidate, version });
                }
            }
        }
    }

    None
}

fn get_uv_version(uv_path: &Path) -> Option<String> {
    let output = Command::new(uv_path).arg("--version").output().ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        Some(text.trim().to_string())
    } else {
        None
    }
}

/// Get OS-specific instructions on how to install `uv`
pub fn get_install_guide() -> String {
    match current_os() {
        OsFamily::Windows => {
            "To install uv on Windows:\n\
               powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\"\n\
             Or with winget / scoop / cargo:\n\
               winget install --id=astral-sh.uv -e\n\
               scoop install uv\n\
               cargo install --locked uv".to_string()
        }
        OsFamily::MacOS => {
            "To install uv on macOS:\n\
               curl -LsSf https://astral.sh/uv/install.sh | sh\n\
             Or with Homebrew / MacPorts / cargo:\n\
               brew install uv\n\
               port install uv\n\
               cargo install --locked uv".to_string()
        }
        OsFamily::Linux => {
            "To install uv on Linux:\n\
               curl -LsSf https://astral.sh/uv/install.sh | sh\n\
             Or with your distribution package manager / cargo:\n\
               pacman -S uv                # Arch Linux\n\
               cargo install --locked uv   # Any distro via Rust".to_string()
        }
        OsFamily::FreeBsd | OsFamily::OpenBsd | OsFamily::NetBsd => {
            "To install uv on BSD:\n\
               pkg install py311-uv         # FreeBSD pkg\n\
             Or with standalone cargo:\n\
               cargo install --locked uv\n\
             Or installer:\n\
               curl -LsSf https://astral.sh/uv/install.sh | sh".to_string()
        }
        OsFamily::Other => {
            "To install uv:\n\
               curl -LsSf https://astral.sh/uv/install.sh | sh\n\
             Or via cargo:\n\
               cargo install --locked uv".to_string()
        }
    }
}

/// Locate the managed runtime directory without initializing uv's package cache.
pub fn uv_python_dir(uv_path: &Path) -> Option<PathBuf> {
    read_uv_directory(uv_path, "python")
}

/// Locate uv's effective cache directory, including overrides recognized by uv.
pub fn uv_cache_dir(uv_path: &Path) -> Option<PathBuf> {
    read_uv_directory(uv_path, "cache")
}

fn read_uv_directory(uv_path: &Path, command: &str) -> Option<PathBuf> {
    let output = Command::new(uv_path)
        .args([command, "dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_absolute().then_some(path)
}

/// Create a new uv-managed virtual environment at `venv_dir`, which must not exist yet.
pub fn uv_create_venv(
    uv_path: &Path,
    venv_dir: &Path,
    python_version: &str,
    system_site_packages: bool,
    allow_python_downloads: bool,
) -> Result<(), String> {
    let output = venv_command(uv_path, venv_dir, python_version, system_site_packages, allow_python_downloads)
        .output()
        .map_err(|e| format!("Failed to run 'uv venv': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("uv venv failed for Python {}: {}", python_version, err.trim()))
    }
}

fn venv_command(uv_path: &Path, venv_dir: &Path, python_version: &str, system_site_packages: bool, allow_python_downloads: bool) -> Command {
    let mut cmd = Command::new(uv_path);
    cmd.arg("venv").arg("--python").arg(python_version);
    if !allow_python_downloads {
        cmd.arg("--no-python-downloads");
    }
    if system_site_packages {
        cmd.arg("--system-site-packages");
    }
    cmd.arg(venv_dir);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_can_be_disabled_during_venv_creation() {
        let command = venv_command(Path::new("uv"), Path::new("venv"), "3.12", true, false);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["venv", "--python", "3.12", "--no-python-downloads", "--system-site-packages", "venv"]
        );
        let command = venv_command(Path::new("uv"), Path::new("venv"), "3.12", false, true);
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["venv", "--python", "3.12", "venv"]);
    }
}

/// Install a requirements file into the environment that owns `python`.
pub fn uv_pip_install_requirements(uv_path: &Path, python: &Path, req_file: &Path) -> Result<(), String> {
    let output = Command::new(uv_path)
        .args(["pip", "install", "--python"])
        .arg(python)
        .arg("-r")
        .arg(req_file)
        .output()
        .map_err(|e| format!("Failed to run 'uv pip install -r': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("uv pip install -r failed: {}", err.trim()))
    }
}
