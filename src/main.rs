use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use indicatif::ProgressBar;
use serde_json::{json, Value};
use uv_migrator::cli::{Cli, Commands, InitArgs, MigrateArgs, OutputFormat, ScanArgs, SearchArgs};
use uv_migrator::ignore::{self, IgnoreEngine};
use uv_migrator::migrator::{migrate_environment, preview_environment};
use uv_migrator::platform::disk_free_space_gb;
use uv_migrator::python_detector::discover_all_pythons;
use uv_migrator::ui;
use uv_migrator::uv;
use uv_migrator::venv_detector::{scan_with_inspection, Inspection, VenvInfo};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (format, result) = match cli.command {
        Some(Commands::Scan(args)) => (args.output.format, scan(args)),
        Some(Commands::ListPythons(args)) => {
            let pythons = discover_all_pythons(uv::find_uv().as_ref());
            match args.output.format {
                OutputFormat::Json => {
                    print_json(json!({ "total_count": pythons.len(), "runtimes": pythons }))
                }
                OutputFormat::Text => ui::print_pythons_table(&pythons),
            }
            (args.output.format, Ok(true))
        }
        Some(Commands::Migrate(args)) => (args.output.format, migrate(args)),
        Some(Commands::Init(args)) => (OutputFormat::Text, init(args)),
        None => {
            let _ = Cli::command().print_help();
            println!();
            (OutputFormat::Text, Ok(true))
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            match format {
                OutputFormat::Json => print_json(json!({ "error": error })),
                OutputFormat::Text => eprintln!("Error: {error}"),
            }
            ExitCode::FAILURE
        }
    }
}

fn root_dir(path: Option<PathBuf>) -> Result<PathBuf, String> {
    let root = match path {
        Some(path) => path,
        None => std::env::current_dir()
            .map_err(|error| format!("Could not determine the current directory: {error}"))?,
    };
    if !root.is_dir() {
        return Err(format!(
            "Search root is not a directory: {}",
            root.display()
        ));
    }
    Ok(root)
}

fn search(
    args: SearchArgs,
    include_ignored: bool,
    inspection: Inspection,
) -> Result<(PathBuf, Vec<VenvInfo>), String> {
    let root = root_dir(args.path)?;
    let ignores = IgnoreEngine::load(&root, args.ignore_file.as_deref(), &args.exclude)?;
    let environments = scan_with_inspection(&root, &ignores, include_ignored, inspection)?;
    Ok((root, environments))
}

fn scan(args: ScanArgs) -> Result<bool, String> {
    let inspection = if args.details {
        Inspection::Detailed
    } else {
        Inspection::Quick
    };
    let (root, environments) = search(args.search, args.include_ignored, inspection)?;
    match args.output.format {
        OutputFormat::Json => print_json(json!({
            "root_dir": root,
            "total_count": environments.len(),
            "virtual_environments": environments,
        })),
        OutputFormat::Text => ui::print_venv_table(&environments),
    }
    Ok(true)
}

fn migrate(args: MigrateArgs) -> Result<bool, String> {
    let (root, mut environments) = search(args.search, false, Inspection::Quick)?;
    environments.retain(|venv| args.include_uv || !venv.is_uv_managed);

    let applying = args.apply && !environments.is_empty();
    let needs_discovery = !environments.is_empty() && args.python.needs_discovery();
    let uv = if applying || needs_discovery {
        uv::find_uv()
    } else {
        None
    };
    if applying && uv.is_none() {
        return Err(format!(
            "Migration requires uv.\n{}",
            uv::get_install_guide()
        ));
    }
    let pythons = if needs_discovery {
        discover_all_pythons(uv.as_ref())
    } else {
        Vec::new()
    };
    if args.details || (args.apply && args.python.needs_existing_version(&pythons)) {
        for venv in &mut environments {
            venv.probe_version_if_unknown();
        }
    }
    let initial_free = if applying {
        disk_free_space_gb(&root)
    } else {
        None
    };

    let progress = (applying && args.output.format == OutputFormat::Text)
        .then(|| ProgressBar::new(environments.len() as u64));
    let results: Vec<_> = environments
        .iter()
        .map(|venv| {
            let result = if let Some(uv) = uv.as_ref().filter(|_| args.apply) {
                migrate_environment(venv, uv, &pythons, &args.python, !args.no_python_downloads)
            } else {
                preview_environment(venv, &pythons, &args.python, args.details)
            };
            if let Some(progress) = &progress {
                progress.inc(1);
            }
            result
        })
        .collect();
    if let Some(progress) = progress {
        progress.finish_and_clear();
    }

    let final_free = if applying {
        disk_free_space_gb(&root)
    } else {
        None
    };
    match args.output.format {
        OutputFormat::Json => print_json(json!({
            "dry_run": !args.apply,
            "initial_free_gb": initial_free,
            "final_free_gb": final_free,
            "results": results,
        })),
        OutputFormat::Text => {
            ui::print_migration_summary(&results, initial_free, final_free, !args.apply)
        }
    }
    Ok(results
        .iter()
        .all(|result| result.success && result.error.is_none()))
}

fn init(args: InitArgs) -> Result<bool, String> {
    let root = root_dir(args.path)?;
    let ignore_file = root.join(ignore::DEFAULT_IGNORE_FILENAME);
    if ignore_file.exists() && !args.force {
        return Err(format!(
            "{} already exists. Pass --force to overwrite it.",
            ignore_file.display()
        ));
    }
    let pythons = discover_all_pythons(None);
    let path = ignore::generate_default_ignore_file(&root, &pythons)?;
    println!("Created {}", path.display());
    Ok(true)
}

fn print_json(output: Value) {
    println!("{output:#}");
}
