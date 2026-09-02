use std::path::Path;
use colored::Colorize;
use crate::migrator::{MigrationOptions, MigrationResult};
use crate::platform::{format_bytes, format_mb};
use crate::python_detector::PythonRuntime;
use crate::uv::UvInfo;
use crate::venv_detector::VenvInfo;

pub fn print_banner() {
    println!("{}", "================================================================================".cyan());
    println!("{}", "               ⚡ uv-migrator - Python Virtual Environment Migrator ⚡           ".bold().cyan());
    println!("{}", "================================================================================".cyan());
}

pub fn print_uv_status(uv: Option<&UvInfo>) {
    match uv {
        Some(info) => {
            println!("  {} Found uv: {} ({})", "✔".green().bold(), info.version.bold().green(), info.path.display().to_string().dimmed());
        }
        None => {
            println!("  {} uv was NOT found on PATH or standard directories.", "✖".red().bold());
            println!("\n{}", crate::uv::get_install_guide().yellow());
        }
    }
}

pub fn print_pythons_table(pythons: &[PythonRuntime]) {
    println!("\n{}", "--- Detected Python Runtimes ---".bold());
    if pythons.is_empty() {
        println!("  {}", "No Python runtimes detected.".yellow());
        return;
    }

    println!("{:<14} {:<10} {:<12} {:<10} {}", "Version", "Major.Minor", "Kind", "Arch", "Path");
    println!("{:-<14} {:-<10} {:-<12} {:-<10} {:-<30}", "", "", "", "", "");

    for p in pythons {
        let ver_str = if p.is_default {
            format!("{} *", p.version).green().bold().to_string()
        } else {
            p.version.clone()
        };
        let mm = format!("{}.{}", p.major, p.minor);
        let kind_str = format!("{}", p.kind);
        println!("{:<23} {:<10} {:<12} {:<10} {}", ver_str, mm, kind_str, p.architecture, p.path.display().to_string().dimmed());
    }
    println!("  (* = default system python)");
}

pub fn print_venv_table(venvs: &[VenvInfo]) {
    println!("\n{}", "--- Discovered Virtual Environments ---".bold());
    if venvs.is_empty() {
        println!("  {}", "No virtual environments found in search path.".yellow());
        return;
    }

    println!("{:<38} {:<12} {:<10} {:<12} {:<16} {}", "Project Path", "Python", "Engine", "Size", "Manifests", "Status");
    println!("{:-<38} {:-<12} {:-<10} {:-<12} {:-<16} {:-<10}", "", "", "", "", "", "");

    let mut total_size_mb = 0.0;
    let mut total_legacy = 0;

    for v in venvs {
        total_size_mb += v.size_mb;
        let engine = if v.is_uv_managed { "uv".green().to_string() } else {
            total_legacy += 1;
            "legacy".yellow().to_string()
        };

        let mut manifests = Vec::new();
        if v.has_pyproject { manifests.push("pyproject"); }
        if v.has_requirements { manifests.push("req.txt"); }
        if v.has_uv_lock { manifests.push("uv.lock"); }
        if v.has_setup_py { manifests.push("setup.py"); }
        let manifest_str = if manifests.is_empty() { "none (auto)".dimmed().to_string() } else { manifests.join("+") };

        let status = if v.is_ignored {
            "IGNORED (Do Not Touch)".yellow().to_string()
        } else if v.is_uv_managed {
            "Ready (uv)".green().to_string()
        } else {
            "Migratable".cyan().bold().to_string()
        };

        let trunc_path = if v.rel_path.len() > 36 {
            format!("...{}", &v.rel_path[v.rel_path.len() - 33..])
        } else {
            v.rel_path.clone()
        };

        println!(
            "{:<38} {:<12} {:<10} {:<12} {:<16} {}",
            trunc_path,
            v.python_version_raw,
            engine,
            format_mb(v.size_mb),
            manifest_str,
            status
        );
    }

    println!("{:-<105}", "");
    println!(
        "Total: {} environments ({} legacy venv, {} uv) | Total disk footprint: {}",
        venvs.len(),
        total_legacy,
        venvs.len() - total_legacy,
        format_mb(total_size_mb).bold()
    );
}

pub fn print_migration_summary(
    results: &[MigrationResult],
    initial_free_gb: Option<f64>,
    final_free_gb: Option<f64>,
    dry_run: bool,
) {
    println!("\n{}", "================================================================================".cyan());
    if dry_run {
        println!("{}", "                     DRY-RUN MIGRATION SIMULATION REPORT                        ".bold().yellow());
    } else {
        println!("{}", "                         MIGRATION SUMMARY REPORT                               ".bold().green());
    }
    println!("{}", "================================================================================".cyan());

    let mut old_total = 0u64;
    let mut new_total = 0u64;
    let mut success_count = 0;
    let mut fail_count = 0;

    for (i, r) in results.iter().enumerate() {
        old_total += r.old_size_bytes;
        new_total += r.new_size_bytes;

        let status = if r.success {
            success_count += 1;
            "✔ OK".green().bold().to_string()
        } else {
            fail_count += 1;
            "✖ FAIL".red().bold().to_string()
        };

        let note = if let Some(ref e) = r.error {
            format!(" - {}", e.red())
        } else if r.snapshot_created {
            " (snapshot requirements.txt created)".dimmed().to_string()
        } else {
            String::new()
        };

        println!(
            "[{:02}/{:02}] {:<32} | {} -> {} | {} -> {} | {}{}",
            i + 1,
            results.len(),
            r.rel_path,
            r.old_python_version,
            r.new_python_version.bold().cyan(),
            format_bytes(r.old_size_bytes),
            format_bytes(r.new_size_bytes).green(),
            status,
            note
        );
    }

    println!("{:-<80}", "");
    println!("Environments Processed: {} | Succeeded: {} | Failed: {}", results.len(), success_count, fail_count);
    println!("Total Old Footprint: {}", format_bytes(old_total).yellow());
    println!("Total New Footprint: {}", format_bytes(new_total).green().bold());

    if let (Some(init_f), Some(fin_f)) = (initial_free_gb, final_free_gb) {
        let diff = fin_f - init_f;
        println!("Initial Free Disk Space: {:.2} GB", init_f);
        println!("Current Free Disk Space: {:.2} GB", fin_f);
        if diff >= 0.0 {
            println!("Disk Space Reclaimed:    {} (+ shared hardlinks)", format!("{:.2} GB", diff).green().bold());
        }
    }
}

pub fn run_interactive_wizard(
    root_dir: &Path,
    pythons: &[PythonRuntime],
    venvs: &[VenvInfo],
) -> Option<MigrationOptions> {
    println!("\n{}", "=== Interactive Setup Wizard ===".bold().cyan());

    // 1. Ask if user wants to generate .uv-migrator-ignore
    let ignore_file = root_dir.join(crate::ignore::DEFAULT_IGNORE_FILENAME);
    if !ignore_file.exists() {
        let create_ignore = inquire::Confirm::new("Create default 'Do Not Touch' ignore list (.uv-migrator-ignore)?")
            .with_default(true)
            .with_help_message("Automatically protects sensitive folders like Conda environments, .git, and node_modules")
            .prompt()
            .unwrap_or(true);

        if create_ignore {
            if let Ok(p) = crate::ignore::generate_default_ignore_file(root_dir, pythons) {
                println!("  {} Created ignore configuration at {}", "✔".green(), p.display());
            }
        }
    }

    // 2. Python version upgrade preference
    let upgrade_choices = vec![
        "Match major.minor version of each virtualenv (e.g. 3.12 -> 3.12, 3.14 -> 3.14)",
        "Upgrade to the latest MINOR version of current major (e.g. 3.12.3 -> 3.12.14)",
        "Upgrade ALL environments to newest available Python overall (e.g. 3.14)",
        "Dry run simulation only (preview changes without modifying disk)",
    ];

    let selection = inquire::Select::new("Choose migration & Python upgrade strategy:", upgrade_choices)
        .prompt()
        .ok()?;

    let mut options = MigrationOptions {
        dry_run: false,
        upgrade_minor: false,
        upgrade_latest: false,
        target_python_override: None,
        auto_install_python: true,
    };

    if selection.contains("latest MINOR") {
        options.upgrade_minor = true;
    } else if selection.contains("newest available") {
        options.upgrade_latest = true;
    } else if selection.contains("Dry run") {
        options.dry_run = true;
    }

    let migratable_count = venvs.iter().filter(|v| !v.is_ignored).count();
    let proceed = inquire::Confirm::new(&format!("Proceed with migration for {} environments?", migratable_count))
        .with_default(true)
        .prompt()
        .unwrap_or(false);

    if proceed {
        Some(options)
    } else {
        println!("{}", "Migration cancelled by user.".yellow());
        None
    }
}
