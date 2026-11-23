use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::migrator::PythonPolicy;

#[derive(Parser, Debug)]
#[command(
    name = "uv-migrator",
    author = clap::crate_authors!(", "),
    version,
    about = "Inspect Python virtual environments and migrate them to uv"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Find virtual environments under a directory
    Scan(ScanArgs),
    /// List installed Python runtimes
    ListPythons(ListPythonsArgs),
    /// Preview migration, or rebuild environments with --apply
    Migrate(MigrateArgs),
    /// Create a starter .uv-migrator-ignore file
    Init(InitArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Args, Debug)]
pub struct OutputArgs {
    /// Result format
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Directory to search [default: current directory]
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// Read ignore rules from this file
    #[arg(long, value_name = "FILE")]
    pub ignore_file: Option<PathBuf>,
    /// Exclude paths matching this glob (repeatable)
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    #[command(flatten)]
    pub search: SearchArgs,
    /// Include virtual environments matched by ignore rules
    #[arg(long)]
    pub include_ignored: bool,
    /// Measure sizes, list packages, and probe unresolved Python versions
    #[arg(long)]
    pub details: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Args, Debug)]
pub struct ListPythonsArgs {
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    #[command(flatten)]
    pub search: SearchArgs,
    /// Rebuild environments after planning
    #[arg(long)]
    pub apply: bool,
    /// Prepare packages in the shared uv cache, then rebuild legacy and uv environments
    #[arg(long, requires = "apply")]
    pub force_reinstall: bool,
    /// Include size and package counts and probe unresolved versions in the preview
    #[arg(long, conflicts_with = "apply")]
    pub details: bool,
    /// Also select environments created by uv
    #[arg(long)]
    pub include_uv: bool,
    /// Python version policy or a numeric version
    #[arg(
        long,
        default_value = "same",
        value_name = "same|latest-patch|latest|VERSION"
    )]
    pub python: PythonPolicy,
    /// Require an installed Python interpreter
    #[arg(long, requires = "apply")]
    pub no_python_downloads: bool,
    /// Upgrade all index packages to the latest versions compatible with the target Python
    #[arg(long)]
    pub upgrade_packages: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Directory for the ignore file [default: current directory]
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    /// Overwrite an existing .uv-migrator-ignore file
    #[arg(short, long)]
    pub force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{error::ErrorKind, CommandFactory};

    #[test]
    fn author_comes_from_cargo_toml() {
        assert_eq!(Cli::command().get_author(), Some("carenaudo, Antigravity"));
    }

    #[test]
    fn migration_previews_by_default() {
        let cli = Cli::try_parse_from(["uv-migrator", "migrate", "projects"]).unwrap();
        let Some(Commands::Migrate(args)) = cli.command else {
            panic!("expected migrate")
        };
        assert!(!args.apply);
        assert_eq!(args.python, PythonPolicy::Same);
        assert_eq!(args.search.path, Some(PathBuf::from("projects")));
    }

    #[test]
    fn removed_and_conflicting_options_are_rejected() {
        for args in [
            &["--dry-run"][..],
            &["--upgrade-minor"],
            &["--json"],
            &["--interactive"],
            &["--apply", "--details"],
            &["--no-python-downloads"],
            &["--force-reinstall"],
            &["--python", "3"],
        ] {
            assert!(
                Cli::try_parse_from(
                    ["uv-migrator", "migrate"]
                        .into_iter()
                        .chain(args.iter().copied())
                )
                .is_err(),
                "{args:?}"
            );
        }
        let result = Cli::try_parse_from(["uv-migrator", "--help"]).unwrap_err();
        assert_eq!(result.kind(), ErrorKind::DisplayHelp);
        assert!(Cli::try_parse_from([
            "uv-migrator",
            "migrate",
            "--apply",
            "--force-reinstall",
            "--no-python-downloads",
            "--python",
            "3.12.4"
        ])
        .is_ok());
    }
}
