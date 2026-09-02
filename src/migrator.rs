use std::path::{Path, PathBuf};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::platform::{dir_size_bytes, venv_python_path};
use crate::python_detector::{find_latest_minor_version, find_latest_overall_version, PythonRuntime};
use crate::uv::{uv_create_venv, uv_pip_install_editable, uv_pip_install_requirements, uv_python_install, uv_sync, UvInfo};
use crate::venv_detector::VenvInfo;

#[derive(Debug, Clone)]
pub struct MigrationOptions {
    pub dry_run: bool,
    pub upgrade_minor: bool,
    pub upgrade_latest: bool,
    pub target_python_override: Option<String>,
    pub auto_install_python: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationResult {
    pub rel_path: String,
    pub project_dir: PathBuf,
    pub old_size_bytes: u64,
    pub new_size_bytes: u64,
    pub old_python_version: String,
    pub new_python_version: String,
    pub success: bool,
    pub error: Option<String>,
    pub snapshot_created: bool,
}

pub fn migrate_environment(
    venv: &VenvInfo,
    uv: &UvInfo,
    detected_pythons: &[PythonRuntime],
    options: &MigrationOptions,
) -> MigrationResult {
    let mut result = MigrationResult {
        rel_path: venv.rel_path.clone(),
        project_dir: venv.project_dir.clone(),
        old_size_bytes: venv.size_bytes,
        new_size_bytes: 0,
        old_python_version: venv.python_version_raw.clone(),
        new_python_version: String::new(),
        success: false,
        error: None,
        snapshot_created: false,
    };

    // 1. Determine target Python version
    let target_python = determine_target_python(venv, detected_pythons, options);
    result.new_python_version = target_python.clone();

    if options.dry_run {
        result.success = true;
        result.new_size_bytes = (venv.size_bytes as f64 * 0.3) as u64; // estimated
        return result;
    }

    // 2. Ensure target python is available (optional uv install)
    if options.auto_install_python {
        let _ = uv_python_install(&uv.path, &target_python);
    }

    // 3. Snapshot requirements if project lacks dependency manifests
    let req_file = venv.project_dir.join("requirements.txt");
    if !venv.has_pyproject && !venv.has_requirements && !venv.has_uv_lock && !venv.has_setup_py {
        if !venv.installed_packages.is_empty() {
            let content = venv.installed_packages.join("\n") + "\n";
            if std::fs::write(&req_file, content).is_ok() {
                result.snapshot_created = true;
            }
        }
    }

    // 4. Safely delete old virtual environment with retry on Windows
    if venv.venv_dir.exists() {
        if let Err(e) = safe_remove_dir_all(&venv.venv_dir) {
            result.error = Some(format!("Failed to delete old .venv: {}", e));
            return result;
        }
    }

    // 5. Create new uv virtual environment
    if let Err(e) = uv_create_venv(&uv.path, &venv.project_dir, Some(&target_python)) {
        result.error = Some(format!("Failed to create uv venv: {}", e));
        return result;
    }

    // 6. Install project dependencies
    let mut install_error = None;
    if venv.has_uv_lock {
        if let Err(e) = uv_sync(&uv.path, &venv.project_dir) {
            install_error = Some(e);
        }
    } else if venv.has_pyproject || venv.has_setup_py {
        if let Err(e) = uv_pip_install_editable(&uv.path, &venv.project_dir) {
            // Fallback to requirements.txt if editable install fails
            if req_file.exists() {
                if let Err(e2) = uv_pip_install_requirements(&uv.path, &venv.project_dir, &req_file) {
                    install_error = Some(format!("{} | fallback: {}", e, e2));
                }
            } else {
                install_error = Some(e);
            }
        }
    } else if req_file.exists() {
        if let Err(e) = uv_pip_install_requirements(&uv.path, &venv.project_dir, &req_file) {
            // Fallback: try installing unpinned package names if strict bounds fail on newer python
            let fallback_success = try_flexible_install(&uv.path, &venv.project_dir, &req_file);
            if !fallback_success {
                install_error = Some(e);
            }
        }
    }

    // 7. Measure new virtualenv size and check python binary
    let new_venv_dir = venv.project_dir.join(".venv");
    if new_venv_dir.exists() {
        result.new_size_bytes = dir_size_bytes(&new_venv_dir);
        let py_bin = venv_python_path(&new_venv_dir);
        if py_bin.exists() {
            if let Some(err) = install_error {
                result.error = Some(format!("Venv created, but dependency warning: {}", err));
                result.success = true; // Environment exists and functions
            } else {
                result.success = true;
            }
        } else {
            result.error = Some("Virtual environment created but python binary is missing".to_string());
        }
    } else {
        result.error = Some("New .venv directory was not created".to_string());
    }

    result
}

fn determine_target_python(
    venv: &VenvInfo,
    detected_pythons: &[PythonRuntime],
    options: &MigrationOptions,
) -> String {
    // 1. Explicit override
    if let Some(ref override_ver) = options.target_python_override {
        return override_ver.clone();
    }

    // 2. Upgrade to newest overall
    if options.upgrade_latest {
        if let Some(latest) = find_latest_overall_version(detected_pythons) {
            return format!("{}.{}", latest.major, latest.minor);
        }
    }

    // 3. Upgrade to latest minor of current major.minor
    if options.upgrade_minor {
        if let Some((maj, min, _)) = venv.parsed_version {
            if let Some(latest_minor) = find_latest_minor_version(detected_pythons, maj, min) {
                return latest_minor.version.clone();
            }
        }
    }

    // 4. Default: match major.minor of the venv
    if let Some((maj, min, _)) = venv.parsed_version {
        format!("{}.{}", maj, min)
    } else if venv.python_version_raw != "unknown" && !venv.python_version_raw.is_empty() {
        venv.python_version_raw.clone()
    } else {
        "3.14".to_string()
    }
}

fn safe_remove_dir_all(dir: &Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match std::fs::remove_dir_all(dir) {
            Ok(_) => return Ok(()),
            Err(_) if attempt < 4 => {
                std::thread::sleep(Duration::from_millis(200 * (attempt + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn try_flexible_install(uv_path: &Path, project_dir: &Path, req_file: &Path) -> bool {
    if let Ok(content) = std::fs::read_to_string(req_file) {
        let mut clean_pkgs = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Strip version bounds (e.g. numpy==2.0.0 -> numpy)
            let pkg_name = trimmed.split(&['=', '<', '>', '~', '!'][..]).next().unwrap_or(trimmed).trim();
            if !pkg_name.is_empty() {
                clean_pkgs.push(pkg_name.to_string());
            }
        }

        if !clean_pkgs.is_empty() {
            let mut cmd = std::process::Command::new(uv_path);
            cmd.current_dir(project_dir);
            cmd.args(["pip", "install"]);
            for pkg in clean_pkgs {
                cmd.arg(pkg);
            }
            if let Ok(out) = cmd.output() {
                return out.status.success();
            }
        }
    }
    false
}
