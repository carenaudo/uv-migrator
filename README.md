# ⚡ Welcome to `uv-migrator` & `uv-migrator-gui`

> *A reliable, high-performance, and safe cross-platform tool to modernize your Python virtual environments with [`uv`](https://github.com/astral-sh/uv) and reclaim gigabytes of disk space.*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)](https://www.rust-lang.org/)
[![Cross-Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20BSD-brightgreen.svg)]()

---

## 📖 Introduction & Philosophy

Welcome to **`uv-migrator`**!

Traditional Python virtual environments (`venv`, `virtualenv`) copy entire standard libraries and packages into every project folder, quickly consuming tens of gigabytes of disk space. **`uv`** (by Astral) introduces global package caching and hardlinking, reducing environment creation times to milliseconds and drastically minimizing disk footprints.

**`uv-migrator`** is built with safety, transparency, and reliability as core principles:
1. **Never Touch System Packages**: Operations are strictly restricted to isolated project virtual environments. System CPython or base Conda installations are never altered.
2. **Never Leave a Project Without an Environment**: Each migration runs in an order that can be undone:
   1. Every installed package is written to a sidecar snapshot (`<venv>.uv-migrator-snapshot.txt`, next to the environment) **with its source**: editable checkouts, VCS commits and local paths are kept as such, not flattened to `name==version`. The snapshot is always taken, whatever manifests the project has, and `requirements.txt` is never modified.
   2. The old environment is **moved aside** (`<venv>.uv-migrator-backup`), not deleted.
   3. The new environment is built in its place from the snapshot, with pins intact. There is no fallback that drops version bounds.
   4. The result is **verified**: every package from the snapshot must be installed at the same version.
   5. Only then is the old environment deleted. If any step fails, the new one is removed and the original is moved back.
3. **Flexible Ignore System**: Easily protect sensitive folders, production environments, and Conda installations using `.uv-migrator-ignore` patterns.
4. **Lightweight & Self-Contained**:
   - **CLI (`uv-migrator`)**: ~3.1 MB native binary.
   - **GUI (`uv-migrator-gui`)**: ~6.6 MB native executable (no Electron, no WebView, zero web-runtime overhead).

---

## ✨ Features

- 🔍 **Automatic `uv` Check & Guided Setup**:
  - The desktop app checks for uv on startup. The CLI checks when applying a migration or discovering runtimes.
  - If missing, provides detected OS-specific installation commands, one-click copy buttons, and official Astral documentation links.
- 🐍 **Universal Python Runtime Discovery**:
  - Detects System CPython, Conda, Mamba, Micromamba, Pyenv, ASDF, Mise, Homebrew, Windows Python Launcher, and `uv`-managed standalone runtimes.
- 📦 **Smart Upgrades**:
  - **Upgrade to Latest Installed Patch** (e.g. `3.12.3` ➔ `3.12.14` or `3.14.0` ➔ `3.14.7`).
  - **Upgrade All to Newest System Python** (e.g. `3.14.7`).
  - **Keep Current Series** (matches existing `major.minor`).
  - **Custom Version** (target a specific Python version).
- 🛡️ **Do-Not-Touch Engine**:
  - Glob patterns match relative paths, absolute paths, and path components.
  - Built-in presets for Conda base directories, VCS folders, and production workloads.
- 🧪 **Dry-Run Mode & Disk Space Report**:
  - Preview migration and the target Python. A detailed preview also counts the installed packages that would be rebuilt and verified.
  - Real-time progress bar and activity logs. Reclaimed space is reported as the change in free space on the volume, which is the honest figure: uv hardlinks packages from a shared cache, so summing file sizes inside each new environment overstates it.
- 🌐 **Cross-Platform**:
  - First-class support for **Windows, macOS, Linux, FreeBSD, OpenBSD, and NetBSD**.

---

## 🚀 Getting Started

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (edition 2021+)
- [`uv`](https://github.com/astral-sh/uv)

### 1. Build from Source
```bash
git clone https://github.com/yourusername/uv-migrator.git
cd uv-migrator
cargo build --release
```
Binaries will be available at:
- `target/release/uv-migrator` (CLI)
- `target/release/uv-migrator-gui` (GUI)

### 2. Install Globally into PATH
```bash
cargo install --path .
```
Both `uv-migrator` and `uv-migrator-gui` will be immediately accessible from any shell.

---

## 🖥️ Graphical Interface (`uv-migrator-gui`)

To launch the native graphical application, simply run:
```bash
uv-migrator-gui
```

### The 4-Step Guided Workflow:
1. **Startup Guard**: Checks for `uv` and shows its version and cache directory or the setup helper.
2. **Step 1: Select & Scan**: Browse for a folder or enter its path, then select **Scan**. Each scan refreshes the virtual environments and Python installations shown on this screen.
3. **Step 2: Review & Configure Ignores**: Toggle individual environments and select **Save** after editing `.uv-migrator-ignore` rules with preset templates.
4. **Step 3: Upgrade Strategy**: Choose your desired Python upgrade policy and toggle **Dry-Run Mode** if you wish to simulate first.
5. **Step 4: Live Migration & Monitor**: Watch the live progress bar, activity logs, and see your total disk space reclaimed in real time.

---

## ⌨️ Command-Line Interface (`uv-migrator`)

Use `uv-migrator --help` or `uv-migrator <command> --help` for the full reference. Commands that accept a path default to the current directory.

```text
uv-migrator scan ./projects
uv-migrator scan ./projects --details --include-ignored --format json
uv-migrator list-pythons --format json
uv-migrator init ./projects
uv-migrator migrate ./projects
uv-migrator migrate ./projects --details --python latest-patch
uv-migrator migrate ./projects --apply --python 3.12 --no-python-downloads
uv-migrator migrate ./projects --apply --force-reinstall
```

- **Scan.** The default reads virtual-environment paths, configuration, and manifest presence. Add `--details` to measure sizes, list local packages, and probe unresolved interpreter versions.
- **Preview and apply.** `migrate` previews legacy environments by default. Add `--apply` to rebuild or `--include-uv` to include environments created by uv. `--details` is available during preview. It adds installed-package counts, measured size, and probes unresolved versions. An apply requires uv when there are eligible environments.
- **Force reinstall.** `--apply --force-reinstall` includes existing uv environments. First it prepares and verifies the selected packages in disposable environments using uv's configured persistent cache. Rebuilds start only after preparation succeeds for the batch. The GUI offers the same option under **Upgrade Strategy**.
- **Python selection.** `--python same` keeps the current major.minor series. `latest-patch` chooses the highest installed patch in that series. `latest` chooses the newest detected major.minor. You can also pass a numeric version such as `3.12.4`. Use `--no-python-downloads` with `--apply` to require an installed interpreter.
- **Ignore rules.** `scan` and `migrate` accept `--ignore-file FILE` and repeatable `--exclude GLOB`. Invalid rules stop the command before migration. `scan --include-ignored` includes environments matched by ignore rules. Only `init --force` overwrites an existing ignore file from the CLI.
- **Script output.** `--format json` produces one result with `null` for unmeasured fields. An operational error before results are available produces an object with an `error` field. Command exit codes are `0` for success or no matches, `1` for operational or per-environment failures, and `2` for invalid arguments.

### Updating existing commands

| Previous usage | Current usage |
| --- | --- |
| `uv-migrator [PATH]` | `uv-migrator scan [PATH]` or `uv-migrator migrate [PATH]` |
| `migrate --dry-run`, `migrate -n` | `migrate` |
| `migrate` to rebuild | `migrate --apply` |
| `migrate --interactive`, `migrate -i` | Run `migrate` to review the preview, then rerun with `--apply` |
| `--upgrade-minor`, `--latest-minor` | `--python latest-patch` |
| `--upgrade-latest`, `--latest` | `--python latest` |
| `-p VERSION` | `--python VERSION` |
| `--no-auto-install` | `--apply --no-python-downloads` |
| `--json` | `--format json` |
| `scan --all`, `scan -a` | `scan --include-ignored` |
| `-e GLOB` | `--exclude GLOB` |

A rebuild keeps the original in a sibling backup until verification succeeds. The sibling snapshot remains available after the run. The per-environment lock is removed on normal completion. After an interrupted run, confirm that no migration is active before removing a stale `<venv>.uv-migrator-backup.lock`. If a backup remains, restore or move it aside before retrying.

---

## 📦 Dependencies & Third-Party Licenses

We are deeply grateful to the open-source Rust community for the high-quality libraries that make `uv-migrator` possible:
- **`egui` / `eframe`**: Immediate-mode GUI framework (MIT / Apache-2.0)
- **`clap`**: Command line argument parser (MIT / Apache-2.0)
- **`rfd`**: Native file dialogs (MIT)
- **`indicatif`**: Terminal progress bars (MIT)
- **`sysinfo`**: System and disk storage calculations (MIT)
- **`serde` / `serde_json` / `toml`**: Serialization & deserialization (MIT / Apache-2.0)
- **`walkdir` / `globset`**: Filesystem traversal and glob matching (MIT / Apache-2.0 / Unlicense)
- **`which` / `directories` / `regex`**: System utilities (MIT / Apache-2.0)

For a complete breakdown of versions and license texts, please see [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

---

## 📄 License

This project is licensed under the [MIT License](LICENSE). You are welcome to use, modify, and distribute it freely.

---

## 💬 Feedback & Contributions

See the [code map](CODEMAP.md) for the CLI and GUI entry points, migration lifecycle, and regression checks.

Contributions, bug reports, and suggestions are warmly welcomed! Please feel free to open an issue or pull request.

*Happy coding and may your disk stay clean and fast!* 🚀
