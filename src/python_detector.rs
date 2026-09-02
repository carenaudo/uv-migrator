use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};
use crate::platform::{current_os, is_windows, OsFamily};
use crate::uv::UvInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PythonKind {
    System,
    UvManaged,
    Conda,
    Pyenv,
    Asdf,
    Mise,
    Homebrew,
    WindowsStore,
    Custom(String),
}

impl std::fmt::Display for PythonKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PythonKind::System => write!(f, "System"),
            PythonKind::UvManaged => write!(f, "uv-managed"),
            PythonKind::Conda => write!(f, "Conda/Mamba"),
            PythonKind::Pyenv => write!(f, "pyenv"),
            PythonKind::Asdf => write!(f, "asdf"),
            PythonKind::Mise => write!(f, "mise"),
            PythonKind::Homebrew => write!(f, "Homebrew"),
            PythonKind::WindowsStore => write!(f, "MS Store"),
            PythonKind::Custom(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonRuntime {
    pub version: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub path: PathBuf,
    pub kind: PythonKind,
    pub architecture: String,
    pub is_default: bool,
}

impl PythonRuntime {
    pub fn new(path: PathBuf, kind: PythonKind, is_default: bool) -> Option<Self> {
        let (version, major, minor, patch, arch) = probe_python(&path)?;
        Some(PythonRuntime {
            version,
            major,
            minor,
            patch,
            path,
            kind,
            architecture: arch,
            is_default,
        })
    }
}

pub fn probe_python(python_exe: &Path) -> Option<(String, u32, u32, u32, String)> {
    if !python_exe.exists() || !python_exe.is_file() {
        return None;
    }

    // Skip Windows Store 0-byte execution alias shims
    if let Ok(meta) = python_exe.metadata() {
        if meta.len() == 0 {
            return None;
        }
    }
    let path_str = python_exe.to_string_lossy();
    if path_str.contains("WindowsApps") {
        return None;
    }

    let output = Command::new(python_exe)
        .arg("-c")
        .arg("import sys, platform; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}|{sys.version_info.major}|{sys.version_info.minor}|{sys.version_info.micro}|{platform.machine()}')")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = text.trim().split('|').collect();
    if parts.len() >= 5 {
        let ver = parts[0].to_string();
        let major = parts[1].parse::<u32>().ok()?;
        let minor = parts[2].parse::<u32>().ok()?;
        let patch = parts[3].parse::<u32>().ok()?;
        let arch = parts[4].to_string();
        Some((ver, major, minor, patch, arch))
    } else {
        None
    }
}

/// Discovers all available Python runtimes on the system
pub fn discover_all_pythons(uv_info: Option<&UvInfo>) -> Vec<PythonRuntime> {
    let mut runtimes = Vec::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();

    // 1. Check uv-managed pythons if uv is available
    if let Some(uv) = uv_info {
        if let Ok(output) = crate::uv::uv_python_list(&uv.path) {
            parse_uv_python_list_output(&output, &mut runtimes, &mut seen_paths);
        }
    }

    // 2. Check Windows py launcher if on Windows
    if is_windows() {
        if let Ok(path) = which::which("py") {
            if let Ok(output) = Command::new(&path).arg("-0p").output() {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if let Some(idx) = trimmed.find('\\') {
                            let candidate_str = &trimmed[idx..];
                            let candidate = PathBuf::from(candidate_str.trim());
                            let canon = candidate.canonicalize().unwrap_or_else(|_| candidate.clone());
                            if !seen_paths.contains(&canon) {
                                if let Some(rt) = PythonRuntime::new(candidate, PythonKind::System, line.contains('*')) {
                                    seen_paths.insert(canon);
                                    runtimes.push(rt);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Check standard PATH executables
    let path_names = vec![
        "python", "python3", "python3.15", "python3.14", "python3.13",
        "python3.12", "python3.11", "python3.10", "python3.9", "python3.8",
        "pypy3", "pypy",
    ];

    for name in path_names {
        if let Ok(p) = which::which(name) {
            let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
            if !seen_paths.contains(&canon) {
                let kind = if p.to_string_lossy().contains("conda") || p.to_string_lossy().contains("mamba") {
                    PythonKind::Conda
                } else if p.to_string_lossy().contains("homebrew") {
                    PythonKind::Homebrew
                } else if p.to_string_lossy().contains("WindowsApps") {
                    PythonKind::WindowsStore
                } else {
                    PythonKind::System
                };

                if let Some(rt) = PythonRuntime::new(p, kind, name == "python" || name == "python3") {
                    seen_paths.insert(canon);
                    runtimes.push(rt);
                }
            }
        }
    }

    // 4. Check Conda / Mamba environment directory paths
    scan_conda_environments(&mut runtimes, &mut seen_paths);

    // 5. Check Pyenv versions
    scan_pyenv_environments(&mut runtimes, &mut seen_paths);

    // 6. Check ASDF / Mise
    scan_asdf_mise(&mut runtimes, &mut seen_paths);

    // 7. Check OS-specific standard binary directories
    scan_system_locations(&mut runtimes, &mut seen_paths);

    // Sort runtimes by version descending (newest first)
    runtimes.sort_by(|a, b| {
        (b.major, b.minor, b.patch).cmp(&(a.major, a.minor, a.patch))
    });

    runtimes
}

fn parse_uv_python_list_output(output: &str, runtimes: &mut Vec<PythonRuntime>, seen_paths: &mut HashSet<PathBuf>) {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Output lines like:
        // cpython-3.14.7-windows-x86_64-none   <path>
        // cpython-3.12.14-windows-x86_64-none  <path>
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let candidate = PathBuf::from(parts[parts.len() - 1]);
            let canon = candidate.canonicalize().unwrap_or_else(|_| candidate.clone());
            if !seen_paths.contains(&canon) && candidate.exists() {
                let kind = if parts[0].contains("cpython-") && (line.contains("uv") || line.contains("standalone")) {
                    PythonKind::UvManaged
                } else if line.contains("conda") {
                    PythonKind::Conda
                } else {
                    PythonKind::System
                };

                if let Some(rt) = PythonRuntime::new(candidate, kind, false) {
                    seen_paths.insert(canon);
                    runtimes.push(rt);
                }
            }
        }
    }
}

fn scan_conda_environments(runtimes: &mut Vec<PythonRuntime>, seen_paths: &mut HashSet<PathBuf>) {
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();

        // 1. ~/.conda/environments.txt
        let envs_txt = home.join(".conda").join("environments.txt");
        if envs_txt.exists() {
            if let Ok(content) = std::fs::read_to_string(&envs_txt) {
                for line in content.lines() {
                    let env_dir = PathBuf::from(line.trim());
                    let py = if is_windows() {
                        env_dir.join("python.exe")
                    } else {
                        env_dir.join("bin").join("python")
                    };
                    check_and_add_runtime(py, PythonKind::Conda, runtimes, seen_paths);
                }
            }
        }

        // 2. Common conda root paths
        let conda_roots = vec![
            home.join("miniconda3"),
            home.join("anaconda3"),
            home.join("miniforge3"),
            home.join("micromamba"),
            home.join(".miniforge3"),
            PathBuf::from("/opt/conda"),
            PathBuf::from("/opt/anaconda3"),
            PathBuf::from("/usr/local/anaconda3"),
            #[cfg(windows)]
            PathBuf::from(r"C:\ProgramData\anaconda3"),
            #[cfg(windows)]
            PathBuf::from(r"C:\ProgramData\miniconda3"),
            #[cfg(windows)]
            PathBuf::from(r"C:\ProgramData\miniforge3"),
        ];

        for root in conda_roots {
            if root.exists() {
                // Root python
                let py = if is_windows() { root.join("python.exe") } else { root.join("bin").join("python") };
                check_and_add_runtime(py, PythonKind::Conda, runtimes, seen_paths);

                // Envs subfolder
                let envs_dir = root.join("envs");
                if envs_dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(&envs_dir) {
                        for entry in entries.flatten() {
                            let env_path = entry.path();
                            if env_path.is_dir() {
                                let py = if is_windows() {
                                    env_path.join("python.exe")
                                } else {
                                    env_path.join("bin").join("python")
                                };
                                check_and_add_runtime(py, PythonKind::Conda, runtimes, seen_paths);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn scan_pyenv_environments(runtimes: &mut Vec<PythonRuntime>, seen_paths: &mut HashSet<PathBuf>) {
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();
        let pyenv_roots = vec![
            home.join(".pyenv").join("versions"),
            home.join(".pyenv").join("pyenv-win").join("versions"),
        ];

        for root in pyenv_roots {
            if root.exists() {
                if let Ok(entries) = std::fs::read_dir(&root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let py = if is_windows() {
                                path.join("python.exe")
                            } else {
                                path.join("bin").join("python")
                            };
                            check_and_add_runtime(py, PythonKind::Pyenv, runtimes, seen_paths);
                        }
                    }
                }
            }
        }
    }
}

fn scan_asdf_mise(runtimes: &mut Vec<PythonRuntime>, seen_paths: &mut HashSet<PathBuf>) {
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();
        // ASDF
        let asdf_dir = home.join(".asdf").join("installs").join("python");
        if asdf_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&asdf_dir) {
                for entry in entries.flatten() {
                    let py = entry.path().join("bin").join("python");
                    check_and_add_runtime(py, PythonKind::Asdf, runtimes, seen_paths);
                }
            }
        }

        // Mise
        let mise_dir = home.join(".local").join("share").join("mise").join("installs").join("python");
        if mise_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&mise_dir) {
                for entry in entries.flatten() {
                    let py = entry.path().join("bin").join("python");
                    check_and_add_runtime(py, PythonKind::Mise, runtimes, seen_paths);
                }
            }
        }
    }
}

fn scan_system_locations(runtimes: &mut Vec<PythonRuntime>, seen_paths: &mut HashSet<PathBuf>) {
    match current_os() {
        OsFamily::Windows => {
            if let Some(dirs) = directories::BaseDirs::new() {
                let local_programs = dirs.data_local_dir().join("Programs").join("Python");
                if local_programs.exists() {
                    if let Ok(entries) = std::fs::read_dir(&local_programs) {
                        for entry in entries.flatten() {
                            let py = entry.path().join("python.exe");
                            check_and_add_runtime(py, PythonKind::System, runtimes, seen_paths);
                        }
                    }
                }
            }
            // Check C:\Python*
            if let Ok(entries) = std::fs::read_dir(r"C:\") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("Python") {
                        let py = entry.path().join("python.exe");
                        check_and_add_runtime(py, PythonKind::System, runtimes, seen_paths);
                    }
                }
            }
        }
        OsFamily::MacOS => {
            let brew_dirs = vec![
                PathBuf::from("/opt/homebrew/opt"),
                PathBuf::from("/usr/local/opt"),
            ];
            for brew_dir in brew_dirs {
                if brew_dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(&brew_dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name();
                            if name.to_string_lossy().starts_with("python@") {
                                let py = entry.path().join("bin").join("python3");
                                check_and_add_runtime(py, PythonKind::Homebrew, runtimes, seen_paths);
                            }
                        }
                    }
                }
            }
        }
        OsFamily::Linux | OsFamily::FreeBsd | OsFamily::OpenBsd | OsFamily::NetBsd | OsFamily::Other => {
            let bin_dirs = vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/pkg/bin"),        // NetBSD pkgsrc
                PathBuf::from("/usr/local/pkg/bin"),  // FreeBSD
                PathBuf::from("/opt/local/bin"),      // MacPorts / Unix
            ];

            for dir in bin_dirs {
                if dir.exists() {
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let file_name = entry.file_name();
                            let s = file_name.to_string_lossy();
                            if (s.starts_with("python3.") || s == "python3" || s == "pypy3") && !s.ends_with("-config") {
                                let py = entry.path();
                                check_and_add_runtime(py, PythonKind::System, runtimes, seen_paths);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn check_and_add_runtime(
    py: PathBuf,
    kind: PythonKind,
    runtimes: &mut Vec<PythonRuntime>,
    seen_paths: &mut HashSet<PathBuf>,
) {
    if py.exists() && py.is_file() {
        let canon = py.canonicalize().unwrap_or_else(|_| py.clone());
        if !seen_paths.contains(&canon) {
            if let Some(rt) = PythonRuntime::new(py, kind, false) {
                seen_paths.insert(canon);
                runtimes.push(rt);
            }
        }
    }
}

/// Find the highest installed minor version for a given major.minor (e.g. for (3, 12) -> 3.12.14)
pub fn find_latest_minor_version<'a>(runtimes: &'a [PythonRuntime], major: u32, minor: u32) -> Option<&'a PythonRuntime> {
    runtimes
        .iter()
        .filter(|r| r.major == major && r.minor == minor)
        .max_by_key(|r| r.patch)
}

/// Find the highest available Python version overall on the system
pub fn find_latest_overall_version<'a>(runtimes: &'a [PythonRuntime]) -> Option<&'a PythonRuntime> {
    runtimes.iter().max_by_key(|r| (r.major, r.minor, r.patch))
}
