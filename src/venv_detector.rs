use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use crate::ignore::IgnoreEngine;
use crate::platform::{dir_size_bytes, find_site_packages_dirs, venv_python_path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenvInfo {
    pub project_dir: PathBuf,
    pub venv_dir: PathBuf,
    pub rel_path: String,
    pub python_version_raw: String,
    pub parsed_version: Option<(u32, u32, u32)>,
    pub is_uv_managed: bool,
    pub size_bytes: u64,
    pub size_mb: f64,
    pub has_pyproject: bool,
    pub has_requirements: bool,
    pub has_setup_py: bool,
    pub has_uv_lock: bool,
    pub is_ignored: bool,
    pub installed_packages: Vec<String>,
}

#[allow(dead_code)]
impl VenvInfo {
    pub fn is_upgradable_to(&self, target_major: u32, target_minor: u32, target_patch: u32) -> bool {
        if let Some((cur_maj, cur_min, cur_pat)) = self.parsed_version {
            (target_major, target_minor, target_patch) > (cur_maj, cur_min, cur_pat)
        } else {
            true
        }
    }
}

/// Recursively scan a root directory for Python virtual environments
pub fn scan_virtual_environments(
    root_dir: &Path,
    ignore_engine: &IgnoreEngine,
    include_all: bool,
) -> Vec<VenvInfo> {
    let mut venvs = Vec::new();

    let mut it = WalkDir::new(root_dir)
        .follow_links(false)
        .into_iter();

    while let Some(entry_res) = it.next() {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_dir() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy();
        // Skip common build / VCS / non-project directories without descending
        if file_name == ".git"
            || file_name == "node_modules"
            || file_name == "target"
            || file_name == ".cargo"
            || file_name == "__pycache__"
            || file_name == ".idea"
            || file_name == ".vscode"
            || file_name == ".pytest_cache"
            || file_name == ".mypy_cache"
        {
            it.skip_current_dir();
            continue;
        }

        let dir_path = entry.path();
        if is_virtual_env_dir(dir_path) {
            let ignored = ignore_engine.is_ignored(dir_path, root_dir);
            if !ignored || include_all {
                if let Some(info) = inspect_venv(dir_path, root_dir, ignored) {
                    venvs.push(info);
                }
            }
            // Do NOT descend into the virtual environment itself
            it.skip_current_dir();
        }
    }

    venvs.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    venvs
}

/// Check if a directory is a virtual environment
pub fn is_virtual_env_dir(dir: &Path) -> bool {
    let dir_name = dir.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let is_common_name = dir_name == ".venv" || dir_name == "venv" || dir_name == "env" || dir_name == ".env";

    let pyvenv_cfg = dir.join("pyvenv.cfg");
    let py_bin = venv_python_path(dir);

    pyvenv_cfg.exists() || (is_common_name && py_bin.exists())
}

/// Inspect virtual environment metadata, packages, manifests, and disk size
pub fn inspect_venv(venv_dir: &Path, base_root: &Path, is_ignored: bool) -> Option<VenvInfo> {
    let project_dir = venv_dir.parent().unwrap_or(venv_dir).to_path_buf();
    let rel_path = project_dir
        .strip_prefix(base_root)
        .unwrap_or(&project_dir)
        .to_string_lossy()
        .to_string();

    let mut py_ver_raw = "unknown".to_string();
    let mut is_uv = false;
    let mut parsed_ver: Option<(u32, u32, u32)> = None;

    // 1. Check pyvenv.cfg
    let pyvenv_cfg = venv_dir.join("pyvenv.cfg");
    if pyvenv_cfg.exists() {
        if let Ok(content) = std::fs::read_to_string(&pyvenv_cfg) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("version =") || trimmed.starts_with("version_info =") {
                    if let Some(val) = trimmed.split('=').nth(1) {
                        py_ver_raw = val.trim().to_string();
                    }
                }
                if trimmed.to_lowercase().contains("uv =")
                    || trimmed.to_lowercase().contains("virtualenv")
                    || trimmed.contains("uv")
                {
                    is_uv = true;
                }
            }
        }
    }

    // 2. If version not parsed from pyvenv.cfg, probe binary
    let py_bin = venv_python_path(venv_dir);
    if py_ver_raw == "unknown" && py_bin.exists() {
        if let Some((ver, maj, min, pat, _)) = crate::python_detector::probe_python(&py_bin) {
            py_ver_raw = ver;
            parsed_ver = Some((maj, min, pat));
        }
    } else if parsed_ver.is_none() {
        parsed_ver = parse_version_tuple(&py_ver_raw);
    }

    // 3. Scan site-packages for installed packages via .dist-info
    let installed_packages = scan_dist_info_packages(venv_dir);

    // 4. Check manifests in project root
    let has_pyproject = project_dir.join("pyproject.toml").exists();
    let has_requirements = project_dir.join("requirements.txt").exists();
    let has_setup_py = project_dir.join("setup.py").exists() || project_dir.join("setup.cfg").exists();
    let has_uv_lock = project_dir.join("uv.lock").exists();

    // 5. Measure size on disk
    let size_bytes = dir_size_bytes(venv_dir);
    let size_mb = (size_bytes as f64) / (1024.0 * 1024.0);

    Some(VenvInfo {
        project_dir,
        venv_dir: venv_dir.to_path_buf(),
        rel_path: if rel_path.is_empty() { ".".to_string() } else { rel_path },
        python_version_raw: py_ver_raw,
        parsed_version: parsed_ver,
        is_uv_managed: is_uv,
        size_bytes,
        size_mb,
        has_pyproject,
        has_requirements,
        has_setup_py,
        has_uv_lock,
        is_ignored,
        installed_packages,
    })
}

fn parse_version_tuple(ver_str: &str) -> Option<(u32, u32, u32)> {
    let clean: String = ver_str.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.len() >= 2 {
        let maj = parts[0].parse::<u32>().ok()?;
        let min = parts[1].parse::<u32>().ok()?;
        let pat = if parts.len() >= 3 { parts[2].parse::<u32>().unwrap_or(0) } else { 0 };
        Some((maj, min, pat))
    } else {
        None
    }
}

/// Extracts installed packages from .dist-info directories in site-packages
pub fn scan_dist_info_packages(venv_dir: &Path) -> Vec<String> {
    let mut pkgs = Vec::new();
    let site_packages_dirs = find_site_packages_dirs(venv_dir);

    for sp in site_packages_dirs {
        if let Ok(entries) = std::fs::read_dir(&sp) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".dist-info") {
                    let pkg_name_ver = name_str.trim_end_matches(".dist-info");
                    // Transform name-version format to package==version for requirements.txt
                    if let Some(idx) = pkg_name_ver.rfind('-') {
                        let name_part = &pkg_name_ver[..idx];
                        let ver_part = &pkg_name_ver[idx + 1..];
                        if !name_part.is_empty() && !ver_part.is_empty() && ver_part.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                            pkgs.push(format!("{}=={}", name_part, ver_part));
                            continue;
                        }
                    }
                    pkgs.push(pkg_name_ver.to_string());
                }
            }
        }
    }

    pkgs.sort();
    pkgs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        assert_eq!(parse_version_tuple("3.14.7"), Some((3, 14, 7)));
        assert_eq!(parse_version_tuple("3.12.14"), Some((3, 12, 14)));
        assert_eq!(parse_version_tuple("3.11"), Some((3, 11, 0)));
        assert_eq!(parse_version_tuple("invalid"), None);
    }
}

