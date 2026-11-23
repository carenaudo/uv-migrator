use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use uv_migrator::ignore::IgnoreEngine;
use uv_migrator::migrator::{
    backup_path, migrate_environment, prepare_migration_cache, preview_environment, PythonPolicy,
};
use uv_migrator::python_detector::{PythonKind, PythonRuntime};
use uv_migrator::snapshot::{read_installed, snapshot_path};
use uv_migrator::uv::UvInfo;
use uv_migrator::venv_detector::{scan_with_inspection, Inspection};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("uvm-workflows-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::write(path.join(".uv-migrator-ignore"), "").unwrap();
        Self(path)
    }

    fn venv(&self, name: &str, config: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("pyvenv.cfg"), config).unwrap();
        path
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uv-migrator"))
            .args(args)
            .current_dir(&self.0)
            .env("PATH", "")
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn packages(venv: &Path) -> PathBuf {
    venv.join(if cfg!(windows) {
        "Lib/site-packages"
    } else {
        "lib/python3.12/site-packages"
    })
}

#[test]
fn cache_preparation_leaves_the_batch_intact_on_failure() {
    let fixture = Fixture::new();
    let first = fixture.venv("a/.venv", "uv = 0.12\nversion = 3.12.4\n");
    let second = fixture.venv("b/.venv", "version = 3.12.4\n");
    fs::create_dir_all(packages(&first)).unwrap();
    fixture.venv(
        "a/.uv-migrator-cache-prep-orphan/.venv",
        "version = 3.12.4\n",
    );
    let ignores = IgnoreEngine::new(Vec::new()).unwrap();
    let environments =
        scan_with_inspection(&fixture.0, &ignores, false, Inspection::Quick).unwrap();
    assert_eq!(environments.len(), 2);

    let uv = UvInfo {
        path: fixture.0.join("unavailable"),
        version: String::new(),
    };
    let error =
        prepare_migration_cache(&environments, &uv, &[], &PythonPolicy::Same, false, false).unwrap_err();
    assert_eq!(environments[1].venv_dir, second);
    assert!(
        error.contains(&environments[1].venv_dir.display().to_string()),
        "{error}"
    );
    assert!(error.contains("Rebuilds did not start"));
    for path in [first, second] {
        assert!(path.join("pyvenv.cfg").is_file());
        assert!(!snapshot_path(&path).exists());
        assert!(!backup_path(&path).exists());
    }
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn quick_scan_and_preview_only_read_config_and_manifests() {
    let fixture = Fixture::new();
    let legacy = fixture.venv(
        "demo/venv",
        "home = /path/with/uv\nvirtualenv = 20.29\nversion=3.12.4\n",
    );
    fixture.venv("demo/.venv", "uv=0.8\nversion_info = 3.13.2.final.0\n");
    fs::write(fixture.0.join("demo/pyproject.toml"), "[project]\n").unwrap();
    let dist = packages(&legacy).join("demo-1.0.dist-info");
    fs::create_dir_all(&dist).unwrap();
    fs::write(dist.join("METADATA"), "Version: 1.0\n").unwrap();

    let quick = fixture.cli(&["scan", "--format", "json"]);
    let detailed = fixture.cli(&["scan", "--details", "--format", "json"]);
    assert!(quick.status.success() && detailed.status.success());
    let quick = json(&quick);
    let detailed = json(&detailed);
    let quick_rows = quick["virtual_environments"].as_array().unwrap();
    let detailed_rows = detailed["virtual_environments"].as_array().unwrap();
    assert_eq!(quick_rows.len(), 2);
    assert_eq!(detailed_rows.len(), 2);
    for (quick, detailed) in quick_rows.iter().zip(detailed_rows) {
        assert_eq!(quick["venv_dir"], detailed["venv_dir"]);
        assert!(quick["has_pyproject"].as_bool().unwrap());
        assert!(quick["size_bytes"].is_null());
        assert!(quick["installed_packages"].is_null());
        assert!(detailed["size_bytes"].as_u64().unwrap() > 0);
        assert!(detailed["installed_packages"].is_array());
    }
    assert_eq!(quick_rows[0]["is_uv_managed"], true);
    assert_eq!(quick_rows[1]["is_uv_managed"], false);

    let preview = fixture.cli(&["migrate", "--format", "json"]);
    assert!(preview.status.success());
    let preview = json(&preview);
    assert_eq!(preview["dry_run"], true);
    assert!(preview["initial_free_gb"].is_null());
    assert!(preview["final_free_gb"].is_null());
    let rows = preview["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["new_python_version"], "3.12");
    assert!(rows[0]["old_size_bytes"].is_null());
    assert!(rows[0]["package_count"].is_null());
    assert!(!snapshot_path(&legacy).exists());
    assert!(!backup_path(&legacy).exists());
    assert_eq!(
        fs::read_to_string(dist.join("METADATA")).unwrap(),
        "Version: 1.0\n"
    );

    let detailed = fixture.cli(&["migrate", "--details", "--format", "json"]);
    assert_eq!(detailed.status.code(), Some(1));
    assert!(json(&detailed)["results"][0]["error"]
        .as_str()
        .unwrap()
        .contains("package name"));
    let include_uv = fixture.cli(&["migrate", "--include-uv", "--format", "json"]);
    assert!(include_uv.status.success());
    assert_eq!(json(&include_uv)["results"].as_array().unwrap().len(), 2);
}

#[test]
fn json_handles_empty_and_mixed_previews_and_parser_exit_codes() {
    let fixture = Fixture::new();
    let help = fixture.cli(&[]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));
    let invalid = fixture.cli(&["migrate", "--no-python-downloads"]);
    assert_eq!(invalid.status.code(), Some(2));

    let empty = fixture.cli(&["migrate", "--format", "json"]);
    assert!(empty.status.success());
    assert_eq!(json(&empty)["results"], serde_json::json!([]));

    fixture.venv("unknown/.venv", "");
    fixture.venv("known/.venv", "version = 3.12.4\n");
    let mixed = fixture.cli(&["migrate", "--format", "json"]);
    assert_eq!(mixed.status.code(), Some(1));
    let mixed = json(&mixed);
    assert_eq!(mixed["results"][0]["success"], true);
    assert_eq!(mixed["results"][1]["success"], false);
    assert!(mixed["results"][1]["new_python_version"].is_null());

    let explicit = fixture.cli(&["migrate", "--python", "3.13.1", "--format", "json"]);
    assert!(explicit.status.success());
    assert_eq!(
        json(&explicit)["results"][1]["new_python_version"],
        "3.13.1"
    );
}

#[test]
fn upgrade_policies_choose_installed_series_and_patches() {
    let fixture = Fixture::new();
    fixture.venv("demo/.venv", "version=3.12.4\n");
    let ignores = IgnoreEngine::new(Vec::new()).unwrap();
    let scanned = scan_with_inspection(&fixture.0, &ignores, false, Inspection::Quick).unwrap();
    let runtimes = [(12, 8), (13, 2), (12, 7)].map(|(minor, patch)| PythonRuntime {
        version: format!("3.{minor}.{patch}"),
        major: 3,
        minor,
        patch,
        path: PathBuf::new(),
        kind: PythonKind::System,
        architecture: String::new(),
        is_default: false,
    });
    for (policy, expected) in [
        (PythonPolicy::Same, "3.12"),
        (PythonPolicy::LatestPatch, "3.12.8"),
        (PythonPolicy::Latest, "3.13"),
    ] {
        let preview = preview_environment(&scanned[0], &runtimes, &policy, false);
        assert_eq!(preview.new_python_version.as_deref(), Some(expected));
    }
    let fallback = preview_environment(&scanned[0], &[], &PythonPolicy::Latest, false);
    assert_eq!(fallback.new_python_version.as_deref(), Some("3.12"));
}

#[test]
fn invalid_or_unreadable_ignore_rules_fail_closed() {
    let fixture = Fixture::new();
    fixture.venv("protected/.venv", "version = 3.12.4\n");
    let ignore_file = fixture.0.join(".uv-migrator-ignore");
    fs::write(&ignore_file, "protected/**\n[bad\n").unwrap();

    let invalid = fixture.cli(&["migrate", "--format", "json"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stderr.is_empty());
    assert!(json(&invalid)["error"]
        .as_str()
        .unwrap()
        .contains(".uv-migrator-ignore:2"));

    fs::write(ignore_file, "protected/**\n").unwrap();
    let ignored = fixture.cli(&["scan", "--include-ignored", "--format", "json"]);
    assert!(ignored.status.success());
    assert_eq!(
        json(&ignored)["virtual_environments"][0]["is_ignored"],
        true
    );
    let skipped = fixture.cli(&["migrate", "--format", "json"]);
    assert_eq!(json(&skipped)["results"], serde_json::json!([]));

    let invalid_exclude = fixture.cli(&["scan", "--exclude", "[bad"]);
    assert_eq!(invalid_exclude.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid_exclude.stderr).contains("--exclude"));
    for source in ["missing.rules", "protected"] {
        let output = fixture.cli(&["scan", "--ignore-file", source]);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains(source));
    }
}

#[test]
fn strict_inventory_distinguishes_missing_empty_and_malformed() {
    let fixture = Fixture::new();
    let venv = fixture.venv(".venv", "version = 3.12.4\n");
    assert!(read_installed(&venv).is_err());
    let site = packages(&venv);
    fs::create_dir_all(&site).unwrap();
    assert!(read_installed(&venv).unwrap().is_empty());
    let dist = site.join("demo-1.0.dist-info");
    fs::create_dir(&dist).unwrap();
    assert_eq!(read_installed(&venv).unwrap()[0].requirement, "demo==1.0");
    fs::write(dist.join("direct_url.json"), "{bad").unwrap();
    assert!(read_installed(&venv)
        .unwrap_err()
        .contains("direct_url.json"));
    fs::write(dist.join("METADATA"), "Name: demo\n").unwrap();
    assert!(read_installed(&venv).unwrap_err().contains("METADATA"));
}

#[test]
fn preflight_keeps_existing_backup_and_snapshot_intact() {
    let fixture = Fixture::new();
    let path = fixture.venv(".venv", "version = 3.12.4\n");
    let ignores = IgnoreEngine::new(Vec::new()).unwrap();
    let scanned = scan_with_inspection(&fixture.0, &ignores, false, Inspection::Quick).unwrap();
    let snapshot = snapshot_path(&path);
    let backup = backup_path(&path);
    let uv = UvInfo {
        path: fixture.0.join("unavailable"),
        version: String::new(),
    };
    fs::write(&snapshot, "original snapshot").unwrap();
    fs::create_dir(&backup).unwrap();

    let result = migrate_environment(&scanned[0], &uv, &[], &PythonPolicy::Same, false, false);
    assert!(!result.success);
    assert!(result.error.unwrap().contains("backup from an earlier run"));
    assert_eq!(fs::read_to_string(&snapshot).unwrap(), "original snapshot");
    assert!(path.join("pyvenv.cfg").is_file());
    assert!(backup.is_dir());

    fs::remove_dir(&backup).unwrap();
    let result = migrate_environment(&scanned[0], &uv, &[], &PythonPolicy::Same, false, false);
    assert!(!result.success);
    assert!(!result.restored);
    assert_eq!(fs::read_to_string(&snapshot).unwrap(), "original snapshot");
    assert!(path.join("pyvenv.cfg").is_file());
    assert!(!backup.exists());

    fs::create_dir_all(packages(&path)).unwrap();
    let result = migrate_environment(&scanned[0], &uv, &[], &PythonPolicy::Same, false, false);
    assert!(!result.success);
    assert!(result.restored);
    assert!(result
        .error
        .unwrap()
        .contains("original environment restored"));
    assert!(path.join("pyvenv.cfg").is_file());
    assert!(!backup.exists());
}
