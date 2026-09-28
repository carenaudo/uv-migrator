use std::path::{Path, PathBuf};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::platform::{dir_size_bytes, venv_python_path};
use crate::python_detector::{find_latest_minor_version, find_latest_overall_version, PythonRuntime};
use crate::snapshot::{self, InstalledPackage};
use crate::uv::{uv_create_venv, uv_pip_install_requirements, uv_python_install, UvInfo};
use crate::venv_detector::VenvInfo;

/// Suffix of the directory the old environment is moved to while its replacement is built.
pub const BACKUP_SUFFIX: &str = ".uv-migrator-backup";

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
    pub dry_run: bool,
    pub old_size_bytes: u64,
    /// Apparent size of the new environment. Files uv hardlinks from its cache are counted
    /// in full, so this overstates what the environment adds; the change in free space on
    /// the volume is the real figure. Zero in a dry run, where nothing is measured.
    pub new_size_bytes: u64,
    pub old_python_version: String,
    pub new_python_version: String,
    /// Number of packages recorded in the snapshot and required in the new environment.
    pub package_count: usize,
    pub success: bool,
    pub error: Option<String>,
    pub snapshot_path: Option<PathBuf>,
    /// The migration failed and the original environment was put back.
    pub restored: bool,
}

/// Migrate one environment without ever leaving the project without one:
///
/// 1. snapshot every installed package, with its source, to a sidecar file;
/// 2. move the old environment aside (not delete it);
/// 3. build the new one in its place from the snapshot;
/// 4. verify that the same packages are installed at the same versions;
/// 5. only then delete the old environment. If any step fails, put it back.
pub fn migrate_environment(
    venv: &VenvInfo,
    uv: &UvInfo,
    detected_pythons: &[PythonRuntime],
    options: &MigrationOptions,
) -> MigrationResult {
    let installed = snapshot::read_installed(&venv.venv_dir);
    let mut result = MigrationResult {
        rel_path: venv.rel_path.clone(),
        project_dir: venv.project_dir.clone(),
        dry_run: options.dry_run,
        old_size_bytes: venv.size_bytes,
        new_size_bytes: 0,
        old_python_version: venv.python_version_raw.clone(),
        new_python_version: determine_target_python(venv, detected_pythons, options),
        package_count: installed.len(),
        success: false,
        error: None,
        snapshot_path: None,
        restored: false,
    };

    if options.dry_run {
        result.success = true;
        return result;
    }

    if options.auto_install_python {
        let _ = uv_python_install(&uv.path, &result.new_python_version);
    }

    // 1. Snapshot. Nothing has been touched yet, so a failure here is harmless.
    let snap_path = snapshot::snapshot_path(&venv.venv_dir);
    let snap = snapshot::render(&installed, &venv.venv_dir, &venv.python_version_raw);
    if let Err(e) = std::fs::write(&snap_path, snap) {
        result.error = Some(format!("Could not write snapshot {}: {} (environment untouched)", snap_path.display(), e));
        return result;
    }
    result.snapshot_path = Some(snap_path.clone());

    // 2. Move the old environment aside.
    let backup = backup_path(&venv.venv_dir);
    if backup.exists() {
        result.error = Some(format!(
            "A backup from an earlier run exists at {}; restore or remove it first (environment untouched)",
            backup.display()
        ));
        return result;
    }
    if let Err(e) = rename_with_retry(&venv.venv_dir, &backup) {
        result.error = Some(format!("Could not move the old environment aside: {} (environment untouched)", e));
        return result;
    }

    // 3-4. Build and verify. 5. Delete the old one only on success.
    let system_site = uses_system_site_packages(&backup);
    match build_and_verify(uv, &venv.venv_dir, &result.new_python_version, system_site, &snap_path, &installed) {
        Ok(new_size) => {
            result.new_size_bytes = new_size;
            result.success = true;
            if let Err(e) = safe_remove_dir_all(&backup) {
                result.error = Some(format!("Migrated, but the old environment could not be deleted and is still at {}: {}", backup.display(), e));
            }
        }
        Err(e) => {
            let cleared = !venv.venv_dir.exists() || safe_remove_dir_all(&venv.venv_dir).is_ok();
            if cleared && rename_with_retry(&backup, &venv.venv_dir).is_ok() {
                result.restored = true;
                result.error = Some(format!("{} (original environment restored)", e));
            } else {
                result.error = Some(format!(
                    "{} -- AND the original environment could not be moved back; it is intact at {}",
                    e,
                    backup.display()
                ));
            }
        }
    }

    result
}

fn build_and_verify(
    uv: &UvInfo,
    venv_dir: &Path,
    target_python: &str,
    system_site_packages: bool,
    snap_path: &Path,
    before: &[InstalledPackage],
) -> Result<u64, String> {
    uv_create_venv(&uv.path, venv_dir, target_python, system_site_packages)?;
    let py = venv_python_path(venv_dir);
    if !py.exists() {
        return Err("New environment has no Python binary".to_string());
    }
    if !before.is_empty() {
        uv_pip_install_requirements(&uv.path, &py, snap_path)?;
    }
    let after = snapshot::read_installed(venv_dir);
    let problems = snapshot::diff(before, &after);
    if !problems.is_empty() {
        return Err(format!("New environment does not match the old one: {}", problems.join("; ")));
    }
    Ok(dir_size_bytes(venv_dir))
}

pub fn backup_path(venv_dir: &Path) -> PathBuf {
    let mut name = venv_dir.file_name().unwrap_or_default().to_os_string();
    name.push(BACKUP_SUFFIX);
    venv_dir.with_file_name(name)
}

/// Whether the environment was created with `--system-site-packages`.
fn uses_system_site_packages(venv_dir: &Path) -> bool {
    std::fs::read_to_string(venv_dir.join("pyvenv.cfg"))
        .map(|cfg| {
            cfg.lines().any(|line| {
                let mut kv = line.splitn(2, '=');
                let key = kv.next().unwrap_or("").trim();
                let val = kv.next().unwrap_or("").trim();
                key == "include-system-site-packages" && val.eq_ignore_ascii_case("true")
            })
        })
        .unwrap_or(false)
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

/// Rename with a few retries: on Windows an antivirus scan or an indexer can briefly hold
/// files open inside a freshly used environment.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match std::fs::rename(from, to) {
            Ok(_) => return Ok(()),
            Err(_) if attempt < 4 => std::thread::sleep(Duration::from_millis(200 * (attempt + 1))),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_sits_next_to_the_venv() {
        assert_eq!(backup_path(Path::new("/proj/.venv")), Path::new("/proj/.venv.uv-migrator-backup"));
        assert_eq!(backup_path(Path::new("/proj/venv")), Path::new("/proj/venv.uv-migrator-backup"));
    }

    #[test]
    fn reads_system_site_packages_flag() {
        let dir = std::env::temp_dir().join(format!("uvm-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pyvenv.cfg"), "home = /usr/bin\ninclude-system-site-packages = true\n").unwrap();
        assert!(uses_system_site_packages(&dir));
        std::fs::write(dir.join("pyvenv.cfg"), "home = /usr/bin\ninclude-system-site-packages = false\n").unwrap();
        assert!(!uses_system_site_packages(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
