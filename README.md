# ⚡ Welcome to `uv-migrator` & `uv-migrator-gui`

> *A reliable, high-performance, and safe cross-platform tool to modernize your Python virtual environments with [`uv`](https://github.com/astral-sh/uv) and reclaim gigabytes of disk space.*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![Cross-Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20macOS%20%7C%20Linux%20%7C%20BSD-brightgreen.svg)]()

---

## 📖 Introduction & Philosophy

Welcome to **`uv-migrator`**!

Traditional Python virtual environments (`venv`, `virtualenv`) copy entire standard libraries and packages into every project folder, quickly consuming tens of gigabytes of disk space. **`uv`** (by Astral) introduces global package caching and hardlinking, reducing environment creation times to milliseconds and drastically minimizing disk footprints.

**`uv-migrator`** is built with safety, transparency, and reliability as core principles:
1. **Never Touch System Packages**: Operations are strictly restricted to isolated project virtual environments. System CPython or base Conda installations are never altered.
2. **Zero Data Loss**: If a project does not contain a `pyproject.toml` or `requirements.txt`, `uv-migrator` automatically snapshots all installed packages and their exact versions directly from `.dist-info` metadata before migration.
3. **Flexible Ignore System**: Easily protect sensitive folders, production environments, and Conda installations using `.uv-migrator-ignore` patterns.
4. **Lightweight & Self-Contained**:
   - **CLI (`uv-migrator`)**: ~3.1 MB native binary.
   - **GUI (`uv-migrator-gui`)**: ~6.6 MB native executable (no Electron, no WebView, zero web-runtime overhead).

---

## ✨ Features

- 🔍 **Automatic `uv` Check & Guided Setup**:
  - Automatically verifies if `uv` is installed on system startup.
  - If missing, provides detected OS-specific installation commands, one-click copy buttons, and official Astral documentation links.
- 🐍 **Universal Python Runtime Discovery**:
  - Detects System CPython, Conda, Mamba, Micromamba, Pyenv, ASDF, Mise, Homebrew, Windows Python Launcher, and `uv`-managed standalone runtimes.
- 📦 **Smart Upgrades**:
  - **Upgrade to Latest Minor** (e.g. `3.12.3` ➔ `3.12.14` or `3.14.0` ➔ `3.14.7`).
  - **Upgrade All to Newest System Python** (e.g. `3.14.7`).
  - **Keep Current Series** (matches existing `major.minor`).
  - **Custom Version** (target a specific Python version).
- 🛡️ **Do-Not-Touch Engine**:
  - Full `.gitignore`-style wildcard pattern support.
  - Built-in presets for Conda base directories, VCS folders, and production workloads.
- 🧪 **Dry-Run Mode & Live Disk Space Gauge**:
  - Simulate migrations safely without touching any files.
  - Real-time progress bar, detailed activity logs, and a reclaimed disk space counter.
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
1. **Startup Guard**: Checks if `uv` is detected and displays an active status badge or setup helper.
2. **Step 1: Select & Scan**: Pick any root project directory via native OS folder picker (`rfd`), discovering environments and available Python runtimes.
3. **Step 2: Review & Configure Ignores**: Toggle individual environments in an interactive table and customize `.uv-migrator-ignore` rules with preset templates.
4. **Step 3: Upgrade Strategy**: Choose your desired Python upgrade policy and toggle **Dry-Run Mode** if you wish to simulate first.
5. **Step 4: Live Migration & Monitor**: Watch the live progress bar, activity logs, and see your total disk space reclaimed in real time.

---

## ⌨️ Command-Line Interface (`uv-migrator`)

### 1. Discover all Python Runtimes
```bash
uv-migrator list-pythons
```

### 2. Scan Projects & Virtual Environments
```bash
uv-migrator scan D:\programacion
```

### 3. Generate Default Ignore File
```bash
uv-migrator init D:\programacion
```

### 4. Perform a Dry-Run Migration
```bash
uv-migrator migrate D:\programacion --dry-run
```

### 5. Migrate with Latest Minor Upgrades
```bash
uv-migrator migrate D:\programacion --latest-minor
```

### 6. Full CLI Reference
```
Usage: uv-migrator [OPTIONS] <COMMAND>

Commands:
  scan          Scan a directory for existing virtual environments
  list-pythons  Discover and display all Python runtimes on the host system
  init          Create a starter .uv-migrator-ignore file
  migrate       Execute virtual environment migration to uv
  help          Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose  Enable detailed debug logging
  -h, --help     Print help
  -V, --version  Print version
```

---

## 📦 Dependencies & Third-Party Licenses

We are deeply grateful to the open-source Rust community for the high-quality libraries that make `uv-migrator` possible:
- **`egui` / `eframe`**: Immediate-mode GUI framework (MIT / Apache-2.0)
- **`clap`**: Command line argument parser (MIT / Apache-2.0)
- **`rfd`**: Native file dialogs (MIT)
- **`indicatif`**: Terminal progress bars (MIT)
- **`inquire`**: Interactive CLI prompts (MIT)
- **`sysinfo`**: System and disk storage calculations (MIT)
- **`serde` / `serde_json` / `toml`**: Serialization & deserialization (MIT / Apache-2.0)
- **`walkdir` / `globset` / `rayon`**: Parallel filesystem traversal (MIT / Apache-2.0 / Unlicense)
- **`which` / `colored` / `directories` / `regex`**: System utilities (MIT / MPL-2.0 / Apache-2.0)

For a complete breakdown of versions and license texts, please see [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).

---

## 📄 License

This project is licensed under the [MIT License](LICENSE). You are welcome to use, modify, and distribute it freely.

---

## 💬 Feedback & Contributions

Contributions, bug reports, and suggestions are warmly welcomed! Please feel free to open an issue or pull request.

*Happy coding and may your disk stay clean and fast!* 🚀
