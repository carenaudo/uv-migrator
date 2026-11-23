use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::platform::{dir_size_bytes, venv_python_path};
use crate::python_detector::{find_latest_minor_version, find_latest_overall_version, PythonRuntime};
use crate::snapshot::{self, InstalledPackage};
use crate::uv::{uv_cache_dir, uv_create_venv, uv_pip_install_requirements, UvInfo};
use crate::venv_detector::VenvInfo;

/// Suffix of the directory the old environment is moved to while its replacement is built.
pub const BACKUP_SUFFIX: &str = ".uv-migrator-backup";
pub(crate) const CACHE_PREP_PREFIX: &str = ".uv-migrator-cache-prep-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonPolicy {
    Same,
    LatestPatch,
    Latest,
    Exact(String),
}

impl FromStr for PythonPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "same" => Ok(Self::Same),
            "latest-patch" => Ok(Self::LatestPatch),
            "latest" => Ok(Self::Latest),
            _ => {
                let parts = value.split('.');
                if matches!(parts.clone().count(), 2 | 3)
                    && parts.clone().all(|part| {
                        part.bytes().all(|byte| byte.is_ascii_digit()) && part.parse::<u32>().is_ok()
                    })
                {
                    Ok(Self::Exact(value.to_string()))
                } else {
                    Err("expected same, latest-patch, latest, or a version such as 3.12 or 3.12.4".to_string())
                }
            }
        }
    }
}

impl PythonPolicy {
    pub fn needs_discovery(&self) -> bool {
        matches!(self, Self::LatestPatch | Self::Latest)
    }

    pub fn needs_existing_version(&self, runtimes: &[PythonRuntime]) -> bool {
        match self {
            Self::Exact(_) => false,
            Self::Latest => runtimes.is_empty(),
            Self::Same | Self::LatestPatch => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationResult {
    pub rel_path: String,
    pub project_dir: PathBuf,
    pub venv_dir: PathBuf,
    pub dry_run: bool,
    pub old_size_bytes: Option<u64>,
    /// Apparent size of the new environment. Files uv hardlinks from its cache are counted
    /// in full, so this overstates what the environment adds; the change in free space on
    /// the volume is the real figure. Absent when the replacement has not been measured.
    pub new_size_bytes: Option<u64>,
    pub old_python_version: String,
    pub new_python_version: Option<String>,
    /// Number of packages recorded in the snapshot and required in the new environment.
    pub package_count: Option<usize>,
    pub success: bool,
    pub error: Option<String>,
    pub snapshot_path: Option<PathBuf>,
    /// The migration failed and the original environment was put back.
    pub restored: bool,
}

fn plan_environment(
    venv: &VenvInfo,
    detected_pythons: &[PythonRuntime],
    policy: &PythonPolicy,
    dry_run: bool,
) -> MigrationResult {
    let target = determine_target_python(venv, detected_pythons, policy);
    let error = target.is_none().then(|| "Could not determine the target Python version. Specify --python VERSION or enter a custom version.".to_string());
    MigrationResult {
        rel_path: venv.rel_path.clone(),
        project_dir: venv.project_dir.clone(),
        venv_dir: venv.venv_dir.clone(),
        dry_run,
        old_size_bytes: None,
        new_size_bytes: None,
        old_python_version: venv.python_version_raw.clone(),
        new_python_version: target,
        package_count: None,
        success: false,
        error,
        snapshot_path: None,
        restored: false,
    }
}

pub fn preview_environment(
    venv: &VenvInfo,
    detected_pythons: &[PythonRuntime],
    policy: &PythonPolicy,
    details: bool,
) -> MigrationResult {
    let mut result = plan_environment(venv, detected_pythons, policy, true);
    if details {
        if let Err(error) = collect_inventory(venv, &mut result) {
            result.error.get_or_insert(error);
        }
    }
    result.success = result.error.is_none();
    result
}

fn collect_inventory(venv: &VenvInfo, result: &mut MigrationResult) -> Result<Vec<InstalledPackage>, String> {
    let installed = snapshot::read_installed(&venv.venv_dir)?;
    result.package_count = Some(installed.len());
    result.old_size_bytes = Some(venv.size_bytes.unwrap_or_else(|| dir_size_bytes(&venv.venv_dir)));
    Ok(installed)
}

/// Populate uv's shared cache before any selected environment is moved.
pub fn prepare_migration_cache(
    venvs: &[VenvInfo],
    uv: &UvInfo,
    detected_pythons: &[PythonRuntime],
    policy: &PythonPolicy,
    allow_python_downloads: bool,
    upgrade_packages: bool,
) -> Result<(), String> {
    let mut checked_cache = false;
    for venv in venvs {
        let mut prepare = || {
            check_backup(&backup_path(&venv.venv_dir))?;
            let target = determine_target_python(venv, detected_pythons, policy)
                .ok_or("Could not determine the target Python version. Specify an explicit version.")?;
            let installed = snapshot::read_installed(&venv.venv_dir)?;
            if installed.is_empty() {
                return Ok(());
            }
            if !checked_cache {
                let cache = uv_cache_dir(&uv.path).ok_or("Could not resolve the uv cache directory")?;
                // With no-cache enabled, uv reports a new temporary cache per invocation.
                if uv_cache_dir(&uv.path).as_deref() != Some(cache.as_path()) {
                    return Err("Force reinstall requires a persistent uv cache. Check uv's no-cache setting.".to_string());
                }
                checked_cache = true;
            }

            // A staging environment on the project's volume can use uv's normal hardlinks.
            let staging = tempfile::Builder::new()
                .prefix(CACHE_PREP_PREFIX)
                .tempdir_in(&venv.project_dir)
                .map_err(|error| format!("Could not create a temporary directory: {error}"))?;
            let requirements = staging.path().join("requirements.txt");
            let rendered = if upgrade_packages {
                snapshot::render_unpinned(&installed, &venv.venv_dir, &venv.python_version_raw)
            } else {
                snapshot::render(&installed, &venv.venv_dir, &venv.python_version_raw)
            };
            std::fs::write(&requirements, rendered)
                .map_err(|error| format!("Could not write temporary requirements: {error}"))?;
            build_and_verify(
                uv,
                &staging.path().join(".venv"),
                &target,
                uses_system_site_packages(&venv.venv_dir),
                allow_python_downloads,
                &requirements,
                &installed,
                upgrade_packages,
            )?;
            staging.close()
                .map_err(|error| format!("Could not remove the temporary environment: {error}"))
        };
        prepare().map_err(|error| format!(
            "Cache preparation failed for {}: {error}. Rebuilds did not start.",
            venv.venv_dir.display()
        ))?;
    }
    Ok(())
}

/// Replace one environment and restore its backup on a rebuild failure:
///
/// 1. snapshot every installed package, with its source, to a sidecar file;
/// 2. move the old environment aside (not delete it);
/// 3. build the new one in its place from the snapshot (or unpinned requirements if `upgrade_packages` is true);
/// 4. verify that the expected packages are installed (at exact versions unless `upgrade_packages` is true);
/// 5. only then delete the old environment. If any step fails, put it back.
pub fn migrate_environment(
    venv: &VenvInfo,
    uv: &UvInfo,
    detected_pythons: &[PythonRuntime],
    policy: &PythonPolicy,
    allow_python_downloads: bool,
    upgrade_packages: bool,
) -> MigrationResult {
    let mut result = plan_environment(venv, detected_pythons, policy, false);
    let Some(target_python) = result.new_python_version.clone() else { return result };

    let backup = backup_path(&venv.venv_dir);
    let _lock = match MigrationLock::acquire(&backup) {
        Ok(lock) => lock,
        Err(error) => {
            result.error = Some(error);
            return result;
        }
    };
    if let Err(error) = check_backup(&backup) {
        result.error = Some(error);
        return result;
    }

    let installed = match collect_inventory(venv, &mut result) {
        Ok(installed) => installed,
        Err(error) => {
            result.error = Some(error);
            return result;
        }
    };

    // 1. Snapshot. Nothing has been touched yet, so a failure here is harmless.
    let snap_path = snapshot::snapshot_path(&venv.venv_dir);
    let snap = snapshot::render(&installed, &venv.venv_dir, &venv.python_version_raw);
    if let Err(e) = std::fs::write(&snap_path, snap) {
        result.error = Some(format!("Could not write snapshot {}: {} (environment untouched)", snap_path.display(), e));
        return result;
    }
    result.snapshot_path = Some(snap_path.clone());

    let upgrade_req_file = if upgrade_packages {
        let mut temp = match tempfile::Builder::new()
            .prefix(".uv-migrator-upgrade-")
            .suffix(".txt")
            .tempfile_in(&venv.project_dir)
        {
            Ok(file) => file,
            Err(e) => {
                result.error = Some(format!("Could not create temporary upgrade requirements: {} (environment untouched)", e));
                return result;
            }
        };
        let unpinned = snapshot::render_unpinned(&installed, &venv.venv_dir, &venv.python_version_raw);
        if let Err(e) = std::io::Write::write_all(&mut temp, unpinned.as_bytes()) {
            result.error = Some(format!("Could not write temporary upgrade requirements: {} (environment untouched)", e));
            return result;
        }
        Some(temp)
    } else {
        None
    };
    let install_requirements_path = upgrade_req_file
        .as_ref()
        .map(|file| file.path().to_path_buf())
        .unwrap_or_else(|| snap_path.clone());

    // 2. Move the old environment aside.
    if let Err(e) = rename_with_retry(&venv.venv_dir, &backup) {
        result.error = Some(format!("Could not move the old environment aside: {} (environment untouched)", e));
        return result;
    }

    // 3-4. Build and verify. 5. Delete the old one only on success.
    let system_site = uses_system_site_packages(&backup);
    match build_and_verify(
        uv,
        &venv.venv_dir,
        &target_python,
        system_site,
        allow_python_downloads,
        &install_requirements_path,
        &installed,
        upgrade_packages,
    ) {
        Ok(()) => {
            result.new_size_bytes = Some(dir_size_bytes(&venv.venv_dir));
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
    allow_python_downloads: bool,
    snap_path: &Path,
    before: &[InstalledPackage],
    upgrade_packages: bool,
) -> Result<(), String> {
    uv_create_venv(&uv.path, venv_dir, target_python, system_site_packages, allow_python_downloads)?;
    let py = venv_python_path(venv_dir);
    if !py.exists() {
        return Err("New environment has no Python binary".to_string());
    }
    if !before.is_empty() {
        uv_pip_install_requirements(&uv.path, &py, snap_path)?;
    }
    let after = snapshot::read_installed(venv_dir)?;
    let problems = snapshot::diff_with_policy(before, &after, upgrade_packages);
    if !problems.is_empty() {
        return Err(format!("New environment does not match the old one: {}", problems.join("; ")));
    }
    Ok(())
}

fn check_backup(backup: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(backup) {
        Ok(_) => Err(format!(
            "A backup from an earlier run exists at {}. Restore or remove it first.",
            backup.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not check {}: {error}", backup.display())),
    }
}

pub fn backup_path(venv_dir: &Path) -> PathBuf {
    let mut name = venv_dir.file_name().unwrap_or_default().to_os_string();
    name.push(BACKUP_SUFFIX);
    venv_dir.with_file_name(name)
}

struct MigrationLock(PathBuf);

impl MigrationLock {
    fn acquire(backup: &Path) -> Result<Self, String> {
        let mut name = backup.file_name().unwrap_or_default().to_os_string();
        name.push(".lock");
        let path = backup.with_file_name(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => Ok(Self(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
                "Migration lock exists at {}. Remove a stale lock only after confirming that no migration is active.",
                path.display()
            )),
            Err(error) => Err(format!("Could not acquire migration lock {}: {error}", path.display())),
        }
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
    policy: &PythonPolicy,
) -> Option<String> {
    let current_series = || venv.parsed_version.map(|(major, minor, _)| format!("{major}.{minor}"));
    match policy {
        PythonPolicy::Exact(version) => Some(version.clone()),
        PythonPolicy::Latest => find_latest_overall_version(detected_pythons)
            .map(|runtime| format!("{}.{}", runtime.major, runtime.minor))
            .or_else(current_series),
        PythonPolicy::LatestPatch => venv.parsed_version.and_then(|(major, minor, _)| {
            find_latest_minor_version(detected_pythons, major, minor)
                .map(|runtime| runtime.version.clone())
                .or_else(current_series)
        }),
        PythonPolicy::Same => current_series(),
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
    fn migrations_for_the_same_environment_are_exclusive() {
        let path = std::env::temp_dir().join(format!("uvm-test-backup-{}", std::process::id()));
        let lock = MigrationLock::acquire(&path).unwrap();
        assert!(MigrationLock::acquire(&path).err().unwrap().contains("Migration lock exists"));
        drop(lock);
        assert!(MigrationLock::acquire(&path).is_ok());
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
