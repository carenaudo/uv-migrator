use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum OsFamily {
    Windows,
    MacOS,
    Linux,
    FreeBsd,
    OpenBsd,
    NetBsd,
    Other,
}

impl std::fmt::Display for OsFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OsFamily::Windows => write!(f, "Windows"),
            OsFamily::MacOS => write!(f, "macOS"),
            OsFamily::Linux => write!(f, "Linux"),
            OsFamily::FreeBsd => write!(f, "FreeBSD"),
            OsFamily::OpenBsd => write!(f, "OpenBSD"),
            OsFamily::NetBsd => write!(f, "NetBSD"),
            OsFamily::Other => write!(f, "Unix/Other"),
        }
    }
}

pub fn current_os() -> OsFamily {
    #[cfg(target_os = "windows")]
    return OsFamily::Windows;

    #[cfg(target_os = "macos")]
    return OsFamily::MacOS;

    #[cfg(target_os = "linux")]
    return OsFamily::Linux;

    #[cfg(target_os = "freebsd")]
    return OsFamily::FreeBsd;

    #[cfg(target_os = "openbsd")]
    return OsFamily::OpenBsd;

    #[cfg(target_os = "netbsd")]
    return OsFamily::NetBsd;

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )))]
    return OsFamily::Other;
}

pub fn is_windows() -> bool {
    matches!(current_os(), OsFamily::Windows)
}

/// Directory inside a venv containing python binary ("Scripts" on Windows, "bin" on Unix/Mac/BSD)
pub fn venv_bin_dir_name() -> &'static str {
    if is_windows() {
        "Scripts"
    } else {
        "bin"
    }
}

/// Python binary name inside a venv ("python.exe" on Windows, "python" on Unix/Mac/BSD)
pub fn venv_python_exe_name() -> &'static str {
    if is_windows() {
        "python.exe"
    } else {
        "python"
    }
}

/// Get the expected python binary path inside a virtual environment directory
pub fn venv_python_path(venv_root: &Path) -> PathBuf {
    venv_root.join(venv_bin_dir_name()).join(venv_python_exe_name())
}

/// Get the expected site-packages path pattern inside a virtual environment directory
pub fn find_site_packages_dirs(venv_root: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if is_windows() {
        let sp = venv_root.join("Lib").join("site-packages");
        if sp.exists() {
            results.push(sp);
        }
    } else {
        let lib_dir = venv_root.join("lib");
        if lib_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let sp = path.join("site-packages");
                        if sp.exists() {
                            results.push(sp);
                        }
                    }
                }
            }
        }
    }
    results
}

/// Calculate directory size in bytes recursively
pub fn dir_size_bytes(path: &Path) -> u64 {
    let mut total: u64 = 0;
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Get disk free space in GB for the drive/volume hosting the given path
pub fn disk_free_space_gb(path: &Path) -> Option<f64> {
    let mut sys = sysinfo::Disks::new_with_refreshed_list();
    sys.refresh(true);
    
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    
    // Find matching mount point with the longest prefix match
    let mut best_match: Option<(usize, &sysinfo::Disk)> = None;
    for disk in sys.list() {
        let mount = disk.mount_point();
        if canonical.starts_with(mount) {
            let len = mount.as_os_str().len();
            if best_match.as_ref().map_or(true, |(prev_len, _)| len > *prev_len) {
                best_match = Some((len, disk));
            }
        }
    }

    best_match.map(|(_, disk)| {
        let available_bytes = disk.available_space();
        available_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    })
}

pub fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{:.2} MB", mb)
    }
}

pub fn format_mb(mb: f64) -> String {
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{:.2} MB", mb)
    }
}
