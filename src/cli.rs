use std::path::PathBuf;
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "uv-migrator",
    author = "Antigravity",
    version = "0.1.0",
    about = "Cross-platform CLI tool to discover, scan, and safely migrate Python virtual environments to uv",
    long_about = "⚡ uv-migrator scans and identifies Python virtual environments across your system and safely migrates them to uv-managed virtual environments, reducing disk footprint via hardlinks and providing clean Python version upgrade options without ever touching system Python packages."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Target search path if no subcommand specified (runs interactive wizard or scan)
    #[arg(global = true, value_name = "PATH")]
    pub path: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Recursively scan and inspect virtual environments in a directory
    Scan(ScanArgs),

    /// Discover and list all installed Python runtimes (System, Conda, Pyenv, UV, etc.)
    ListPythons(ListPythonsArgs),

    /// Migrate legacy virtual environments in a directory to uv
    Migrate(MigrateArgs),

    /// Generate a starter 'Do Not Touch' ignore file (.uv-migrator-ignore)
    Init(InitArgs),
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    /// Root directory to scan (defaults to current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Include virtual environments matched by 'Do Not Touch' ignore rules
    #[arg(short, long)]
    pub all: bool,

    /// Custom path to ignore file
    #[arg(long, value_name = "FILE")]
    pub ignore_file: Option<PathBuf>,

    /// Additional exclusion glob patterns
    #[arg(short, long = "exclude", value_name = "PATTERN")]
    pub excludes: Vec<String>,

    /// Output results in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ListPythonsArgs {
    /// Output detected Python runtimes in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Target directory to search and migrate virtual environments (defaults to current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Simulate migration and display planned actions without modifying any files or environments
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Upgrade each virtual environment to the latest minor version of its current Python major (e.g. 3.12.3 -> 3.12.14)
    #[arg(long, alias = "latest-minor")]
    pub upgrade_minor: bool,

    /// Upgrade all migrated virtual environments to the latest overall Python available (e.g. 3.14)
    #[arg(long, alias = "latest")]
    pub upgrade_latest: bool,

    /// Explicit Python version target for all environments (e.g. '3.14', '3.12', '3.14.7')
    #[arg(short = 'p', long = "python", value_name = "VERSION")]
    pub python: Option<String>,

    /// Do not automatically download/install missing Python versions via uv
    #[arg(long)]
    pub no_auto_install: bool,

    /// Prompt interactively before making changes
    #[arg(short, long)]
    pub interactive: bool,

    /// Custom path to ignore file
    #[arg(long, value_name = "FILE")]
    pub ignore_file: Option<PathBuf>,

    /// Additional exclusion glob patterns
    #[arg(short, long = "exclude", value_name = "PATTERN")]
    pub excludes: Vec<String>,

    /// Output migration results in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Directory where .uv-migrator-ignore will be generated (defaults to current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Overwrite existing .uv-migrator-ignore file
    #[arg(short, long)]
    pub force: bool,
}
