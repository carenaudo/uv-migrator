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

/// Run `uv python list` to discover available and installed python runtimes
pub fn uv_python_list(uv_path: &Path) -> Result<String, String> {
    let output = Command::new(uv_path)
        .args(["python", "list", "--all-versions"])
        .output()
        .map_err(|e| format!("Failed to execute 'uv python list': {}", e))?;

    let text = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        // Fallback without --all-versions if older uv
        let fallback = Command::new(uv_path)
            .args(["python", "list"])
            .output()
            .map_err(|e| format!("Failed to execute 'uv python list': {}", e))?;
        String::from_utf8_lossy(&fallback.stdout).to_string()
    };

    Ok(text)
}

/// Download/install a standalone CPython runtime cleanly via `uv python install <version>`
/// (This NEVER alters system package managers or system files)
pub fn uv_python_install(uv_path: &Path, version: &str) -> Result<String, String> {
    let output = Command::new(uv_path)
        .args(["python", "install", version])
        .output()
        .map_err(|e| format!("Failed to run 'uv python install {}': {}", version, e))?;

    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        Ok(format!("{}{}", out, err).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("'uv python install {}' failed: {}", version, err))
    }
}

/// Create a new uv-managed virtual environment at `.venv`
pub fn uv_create_venv(uv_path: &Path, project_dir: &Path, python_version: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new(uv_path);
    cmd.current_dir(project_dir);
    cmd.arg("venv");

    if let Some(ver) = python_version {
        cmd.arg("--python").arg(ver);
    }
    cmd.arg(".venv");

    let output = cmd.output().map_err(|e| format!("Failed to run 'uv venv': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        // Retry with --allow-existing or --clear if leftover files
        let mut retry_cmd = Command::new(uv_path);
        retry_cmd.current_dir(project_dir);
        retry_cmd.args(["venv", "--allow-existing"]);
        if let Some(ver) = python_version {
            retry_cmd.arg("--python").arg(ver);
        }
        retry_cmd.arg(".venv");

        let retry_out = retry_cmd.output().map_err(|e| format!("Failed retry 'uv venv': {}", e))?;
        if retry_out.status.success() {
            Ok(())
        } else {
            Err(format!("Failed to create uv venv in {:?}: {}", project_dir, err))
        }
    }
}

/// Install dependencies from requirements.txt via `uv pip install -r <file>`
pub fn uv_pip_install_requirements(uv_path: &Path, project_dir: &Path, req_file: &Path) -> Result<(), String> {
    let mut cmd = Command::new(uv_path);
    cmd.current_dir(project_dir);
    cmd.args(["pip", "install", "-r"]);
    cmd.arg(req_file);

    let output = cmd.output().map_err(|e| format!("Failed to run 'uv pip install -r': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("uv pip install -r failed: {}", err))
    }
}

/// Install project package editable via `uv pip install -e .`
pub fn uv_pip_install_editable(uv_path: &Path, project_dir: &Path) -> Result<(), String> {
    let mut cmd = Command::new(uv_path);
    cmd.current_dir(project_dir);
    cmd.args(["pip", "install", "-e", "."]);

    let output = cmd.output().map_err(|e| format!("Failed to run 'uv pip install -e .': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("uv pip install -e . failed: {}", err))
    }
}

/// Sync project environment via `uv sync`
pub fn uv_sync(uv_path: &Path, project_dir: &Path) -> Result<(), String> {
    let mut cmd = Command::new(uv_path);
    cmd.current_dir(project_dir);
    cmd.arg("sync");

    let output = cmd.output().map_err(|e| format!("Failed to run 'uv sync': {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("uv sync failed: {}", err))
    }
}
