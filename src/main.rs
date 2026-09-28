mod cli;
mod ignore;
mod migrator;
mod platform;
mod python_detector;
mod snapshot;
mod ui;
mod uv;
mod venv_detector;

use std::path::{Path, PathBuf};
use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

use cli::{Cli, Commands, InitArgs, ListPythonsArgs, MigrateArgs, ScanArgs};
use ignore::IgnoreEngine;
use migrator::{migrate_environment, MigrationOptions};
use platform::disk_free_space_gb;
use python_detector::discover_all_pythons;
use venv_detector::scan_virtual_environments;

fn main() {
    let cli = Cli::parse();
    let uv_info = uv::find_uv();

    match cli.command {
        Some(Commands::Scan(args)) => handle_scan(args, uv_info.as_ref()),
        Some(Commands::ListPythons(args)) => handle_list_pythons(args, uv_info.as_ref()),
        Some(Commands::Migrate(args)) => handle_migrate(args, uv_info.as_ref()),
        Some(Commands::Init(args)) => handle_init(args, uv_info.as_ref()),
        None => handle_default(cli.path, uv_info.as_ref()),
    }
}

fn resolve_root_dir(path_opt: Option<PathBuf>) -> PathBuf {
    path_opt
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn handle_scan(args: ScanArgs, uv_info: Option<&uv::UvInfo>) {
    let root_dir = resolve_root_dir(args.path);
    let ignore_engine = IgnoreEngine::load(&root_dir, args.ignore_file.as_deref(), &args.excludes);
    let venvs = scan_virtual_environments(&root_dir, &ignore_engine, args.all);

    if args.json {
        let output = serde_json::json!({
            "root_dir": root_dir,
            "total_count": venvs.len(),
            "virtual_environments": venvs,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        ui::print_banner();
        ui::print_uv_status(uv_info);
        println!("Scanning root directory: {}", root_dir.display().to_string().cyan());
        ui::print_venv_table(&venvs);
    }
}

fn handle_list_pythons(args: ListPythonsArgs, uv_info: Option<&uv::UvInfo>) {
    let pythons = discover_all_pythons(uv_info);

    if args.json {
        let output = serde_json::json!({
            "total_count": pythons.len(),
            "runtimes": pythons,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        ui::print_banner();
        ui::print_uv_status(uv_info);
        ui::print_pythons_table(&pythons);
    }
}

fn handle_init(args: InitArgs, uv_info: Option<&uv::UvInfo>) {
    let root_dir = resolve_root_dir(args.path);
    let ignore_file = root_dir.join(ignore::DEFAULT_IGNORE_FILENAME);

    if ignore_file.exists() && !args.force {
        eprintln!(
            "{} File already exists at {}. Use --force to overwrite.",
            "Warning:".yellow().bold(),
            ignore_file.display()
        );
        return;
    }

    let pythons = discover_all_pythons(uv_info);
    match ignore::generate_default_ignore_file(&root_dir, &pythons) {
        Ok(path) => {
            println!("{} Successfully generated 'Do Not Touch' ignore list at:", "✔".green().bold());
            println!("  {}", path.display().to_string().cyan().bold());
        }
        Err(e) => {
            eprintln!("{} Failed to create ignore file: {}", "✖".red().bold(), e);
        }
    }
}

fn handle_migrate(args: MigrateArgs, uv_info: Option<&uv::UvInfo>) {
    let uv = match uv_info {
        Some(u) => u,
        None => {
            eprintln!("{} Cannot perform migration because 'uv' is not installed.", "Error:".red().bold());
            eprintln!("\n{}", uv::get_install_guide().yellow());
            std::process::exit(1);
        }
    };

    let root_dir = resolve_root_dir(args.path);
    let ignore_engine = IgnoreEngine::load(&root_dir, args.ignore_file.as_deref(), &args.excludes);
    let all_venvs = scan_virtual_environments(&root_dir, &ignore_engine, false);
    let pythons = discover_all_pythons(Some(uv));

    let migratable_venvs: Vec<_> = all_venvs.into_iter().filter(|v| !v.is_ignored).collect();

    if migratable_venvs.is_empty() {
        if !args.json {
            ui::print_banner();
            println!("No active virtual environments found in {} (or all are ignored).", root_dir.display());
        }
        return;
    }

    if args.interactive {
        ui::print_banner();
        ui::print_uv_status(Some(uv));
        ui::print_pythons_table(&pythons);
        ui::print_venv_table(&migratable_venvs);

        if let Some(opts) = ui::run_interactive_wizard(&root_dir, &pythons, &migratable_venvs) {
            execute_migration(&migratable_venvs, uv, &pythons, &opts, &root_dir, args.json);
        }
        return;
    }

    let options = MigrationOptions {
        dry_run: args.dry_run,
        upgrade_minor: args.upgrade_minor,
        upgrade_latest: args.upgrade_latest,
        target_python_override: args.python,
        auto_install_python: !args.no_auto_install,
    };

    if !args.json {
        ui::print_banner();
        ui::print_uv_status(Some(uv));
    }

    execute_migration(&migratable_venvs, uv, &pythons, &options, &root_dir, args.json);
}

fn execute_migration(
    venvs: &[venv_detector::VenvInfo],
    uv: &uv::UvInfo,
    pythons: &[python_detector::PythonRuntime],
    options: &MigrationOptions,
    root_dir: &Path,
    json_output: bool,
) {
    let initial_free_gb = disk_free_space_gb(root_dir);

    if !json_output {
        if options.dry_run {
            println!("\n{}", "--- Simulating Virtual Environment Migration (Dry Run) ---".bold().yellow());
        } else {
            println!("\n{}", "--- Starting Virtual Environment Migration to uv ---".bold().cyan());
        }
    }

    let pb = if !json_output {
        let p = ProgressBar::new(venvs.len() as u64);
        p.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(p)
    } else {
        None
    };

    let mut results = Vec::new();
    for venv in venvs {
        if let Some(ref p) = pb {
            p.set_message(format!("Migrating {}", venv.rel_path));
        }

        let res = migrate_environment(venv, uv, pythons, options);
        results.push(res);

        if let Some(ref p) = pb {
            p.inc(1);
        }
    }

    if let Some(p) = pb {
        p.finish_and_clear();
    }

    let final_free_gb = disk_free_space_gb(root_dir);

    if json_output {
        let output = serde_json::json!({
            "dry_run": options.dry_run,
            "initial_free_gb": initial_free_gb,
            "final_free_gb": final_free_gb,
            "results": results,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        ui::print_migration_summary(&results, initial_free_gb, final_free_gb, options.dry_run);
    }
}

fn handle_default(path_opt: Option<PathBuf>, uv_info: Option<&uv::UvInfo>) {
    let root_dir = resolve_root_dir(path_opt);
    let ignore_engine = IgnoreEngine::load(&root_dir, None, &[]);
    let pythons = discover_all_pythons(uv_info);
    let venvs = scan_virtual_environments(&root_dir, &ignore_engine, false);

    ui::print_banner();
    ui::print_uv_status(uv_info);
    println!("Scanning target: {}", root_dir.display().to_string().cyan().bold());

    ui::print_pythons_table(&pythons);
    ui::print_venv_table(&venvs);

    // If interactive terminal and uv is available and there are migratable venvs
    if uv_info.is_some() && venvs.iter().any(|v| !v.is_ignored && !v.is_uv_managed) {
        if let Some(opts) = ui::run_interactive_wizard(&root_dir, &pythons, &venvs) {
            let migratables: Vec<_> = venvs.into_iter().filter(|v| !v.is_ignored).collect();
            execute_migration(&migratables, uv_info.unwrap(), &pythons, &opts, &root_dir, false);
        }
    } else {
        println!("\n{}", "Usage tips:".bold());
        println!("  uv-migrator scan [PATH]            - Scan virtual environments");
        println!("  uv-migrator list-pythons           - List all system Python runtimes");
        println!("  uv-migrator migrate --dry-run      - Preview migration to uv");
        println!("  uv-migrator migrate --latest-minor - Migrate and upgrade to latest minor Python");
        println!("  uv-migrator init                   - Create starter 'Do Not Touch' ignore list");
    }
}

