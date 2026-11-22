use crate::migrator::MigrationResult;
use crate::platform::format_bytes;
use crate::python_detector::PythonRuntime;
use crate::venv_detector::VenvInfo;

pub fn print_pythons_table(pythons: &[PythonRuntime]) {
    if pythons.is_empty() {
        println!("No Python runtimes found.");
        return;
    }

    println!("{:<14} {:<16} {:<10} Path", "Version", "Kind", "Arch");
    for runtime in pythons {
        let default = if runtime.is_default { " *" } else { "" };
        println!(
            "{:<14} {:<16} {:<10} {}",
            format!("{}{default}", runtime.version),
            runtime.kind.to_string(),
            runtime.architecture,
            runtime.path.display()
        );
    }
    println!("* Default system Python");
}

pub fn print_venv_table(venvs: &[VenvInfo]) {
    if venvs.is_empty() {
        println!("No virtual environments found.");
        return;
    }

    println!(
        "{:<12} {:<8} {:<12} {:<12} {:<12} Path",
        "Python", "Engine", "Status", "Size", "Packages"
    );
    for venv in venvs {
        let engine = if venv.is_uv_managed { "uv" } else { "legacy" };
        let status = if venv.is_ignored { "ignored" } else { "active" };
        println!(
            "{:<12} {:<8} {:<12} {:<12} {:<12} {}",
            venv.python_version_raw,
            engine,
            status,
            measured_size(venv.size_bytes),
            measured_count(venv.installed_packages.as_ref().map(Vec::len)),
            venv.venv_dir.display()
        );
        if let Some(packages) = venv
            .installed_packages
            .as_ref()
            .filter(|packages| !packages.is_empty())
        {
            println!("  {}", packages.join(", "));
        }
        let manifests: Vec<_> = [
            (venv.has_pyproject, "pyproject.toml"),
            (venv.has_requirements, "requirements.txt"),
            (venv.has_setup_py, "setup.py/setup.cfg"),
            (venv.has_uv_lock, "uv.lock"),
        ]
        .into_iter()
        .filter_map(|(exists, name)| exists.then_some(name))
        .collect();
        if !manifests.is_empty() {
            println!("  Manifests: {}", manifests.join(", "));
        }
    }
}

pub fn print_migration_summary(
    results: &[MigrationResult],
    initial_free_gb: Option<f64>,
    final_free_gb: Option<f64>,
    dry_run: bool,
) {
    if results.is_empty() {
        println!("No eligible virtual environments found.");
        return;
    }

    if dry_run {
        println!("Migration preview. Pass --apply to rebuild.");
    }
    for result in results {
        let status = match (result.success, result.error.is_some(), dry_run) {
            (false, _, _) => "Failed",
            (true, true, _) => "Warning",
            (true, false, true) => "Planned",
            (true, false, false) => "Migrated",
        };
        println!(
            "{status} {} | Python {} -> {} | Size {} -> {} | Packages {}",
            result.venv_dir.display(),
            result.old_python_version,
            result.new_python_version.as_deref().unwrap_or("unresolved"),
            measured_size(result.old_size_bytes),
            measured_size(result.new_size_bytes),
            measured_count(result.package_count),
        );
        if let Some(error) = &result.error {
            println!("  {error}");
        }
        if let Some(path) = &result.snapshot_path {
            println!("  Snapshot: {}", path.display());
        }
    }

    if !dry_run {
        println!("Replacement sizes count hardlinked files in full.");
    }
    if let (Some(initial), Some(final_free)) = (initial_free_gb, final_free_gb) {
        println!(
            "Change in volume free space: {:+.2} GiB",
            final_free - initial
        );
    }
}

fn measured_size(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "unmeasured".to_string(), format_bytes)
}

fn measured_count(count: Option<usize>) -> String {
    count.map_or_else(|| "unmeasured".to_string(), |count| count.to_string())
}
