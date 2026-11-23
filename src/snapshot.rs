//! Snapshot of what is installed in a virtual environment, taken before it is touched.
//!
//! The snapshot is the only record of an environment whose manifest is missing or out of
//! date, so it is always written, and it keeps where each package came from (editable
//! checkouts, VCS commits, local paths), not just its version.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::platform::try_find_site_packages_dirs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// Distribution name as recorded in METADATA.
    pub name: String,
    pub version: Option<String>,
    /// How to reinstall it at the exact same version: a `requirements.txt` line.
    pub requirement: String,
    /// How to reinstall it when upgrading packages (unpinned for index packages; keeps direct/editable/VCS provenance).
    pub upgrade_requirement: String,
}

/// Read every `.dist-info` in the environment's site-packages.
pub fn read_installed(venv_dir: &Path) -> Result<Vec<InstalledPackage>, String> {
    let directories = try_find_site_packages_dirs(venv_dir)?;
    if directories.is_empty() {
        return Err(format!("No site-packages directory found in {}", venv_dir.display()));
    }
    let mut pkgs = Vec::new();
    for sp in directories {
        let entries = std::fs::read_dir(&sp)
            .map_err(|error| format!("Could not list {}: {error}", sp.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("Could not read an entry in {}: {error}", sp.display()))?;
            if entry.file_name().to_string_lossy().ends_with(".dist-info") {
                let path = entry.path();
                if !path.is_dir() {
                    return Err(format!("Could not read package metadata directory {}", path.display()));
                }
                pkgs.push(read_dist_info(&path)?);
            }
        }
    }
    pkgs.sort_by_cached_key(|package| normalize_name(&package.name));
    Ok(pkgs)
}

fn read_dist_info(dir: &Path) -> Result<InstalledPackage, String> {
    let metadata_path = dir.join("METADATA");
    let metadata = match std::fs::read_to_string(&metadata_path) {
        Ok(metadata) => parse_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let filename = dir.file_name().unwrap_or_default().to_string_lossy();
            parse_dist_info_dir_name(&filename)
        }
        Err(error) => return Err(format!("Could not read {}: {error}", metadata_path.display())),
    };
    let (name, version) = metadata
        .filter(|(name, version)| !name.is_empty() && version.as_ref().is_some_and(|v| !v.is_empty()))
        .ok_or_else(|| format!("Could not determine package name and version from {}", metadata_path.display()))?;

    let direct_url_path = dir.join("direct_url.json");
    let direct_url = match std::fs::read_to_string(&direct_url_path) {
        Ok(text) => {
            let source: serde_json::Value = serde_json::from_str(&text)
                .map_err(|error| format!("Invalid {}: {error}", direct_url_path.display()))?;
            if source.get("url").and_then(|url| url.as_str()).is_none_or(str::is_empty) {
                return Err(format!("Missing URL in {}", direct_url_path.display()));
            }
            Some(source)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Could not read {}: {error}", direct_url_path.display())),
    };
    let requirement = requirement_line(&name, version.as_deref(), direct_url.as_ref());
    let upgrade_requirement = if direct_url.is_some() {
        requirement.clone()
    } else {
        name.clone()
    };
    Ok(InstalledPackage { name, version, requirement, upgrade_requirement })
}

/// Name and version from the headers of a METADATA file.
fn parse_metadata(metadata: &str) -> Option<(String, Option<String>)> {
    let mut name = None;
    let mut version = None;
    for line in metadata.lines() {
        if line.is_empty() {
            break; // headers end at the first blank line
        }
        if let Some(v) = line.strip_prefix("Name:") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Version:") {
            version = Some(v.trim().to_string());
        }
    }
    Some((name?, version))
}

/// Fallback when METADATA is missing: `name-version.dist-info`.
fn parse_dist_info_dir_name(dir_name: &str) -> Option<(String, Option<String>)> {
    let stem = dir_name.strip_suffix(".dist-info")?;
    match stem.rsplit_once('-') {
        Some((name, ver)) if !name.is_empty() && ver.starts_with(|c: char| c.is_ascii_digit()) => {
            Some((name.to_string(), Some(ver.to_string())))
        }
        _ => Some((stem.to_string(), None)),
    }
}

/// The `requirements.txt` line that reinstalls a package from where it came from (PEP 610).
fn requirement_line(name: &str, version: Option<&str>, direct_url: Option<&serde_json::Value>) -> String {
    let pinned = || match version {
        Some(v) => format!("{}=={}", name, v),
        None => name.to_string(),
    };
    let Some(du) = direct_url else { return pinned() };
    let Some(url) = du.get("url").and_then(|u| u.as_str()) else { return pinned() };

    if let Some(vcs) = du.get("vcs_info") {
        let kind = vcs.get("vcs").and_then(|v| v.as_str()).unwrap_or("git");
        return match vcs.get("commit_id").and_then(|c| c.as_str()) {
            Some(commit) => format!("{} @ {}+{}@{}", name, kind, url, commit),
            None => format!("{} @ {}+{}", name, kind, url),
        };
    }
    if let Some(dir_info) = du.get("dir_info") {
        if dir_info.get("editable").and_then(|e| e.as_bool()).unwrap_or(false) {
            return format!("-e {}", url);
        }
        return format!("{} @ {}", name, url);
    }
    format!("{} @ {}", name, url)
}

/// PEP 503 normalisation, so `Foo_Bar` and `foo-bar` compare equal.
pub fn normalize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_sep = false;
    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !last_sep {
                out.push('-');
            }
            last_sep = true;
        } else {
            out.push(c.to_ascii_lowercase());
            last_sep = false;
        }
    }
    out
}

/// Path of the sidecar snapshot for a venv: `<project>/<venv-name>.uv-migrator-snapshot.txt`.
pub fn snapshot_path(venv_dir: &Path) -> PathBuf {
    let name = venv_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".venv".to_string());
    venv_dir
        .parent()
        .unwrap_or(venv_dir)
        .join(format!("{}.uv-migrator-snapshot.txt", name))
}

pub fn render(pkgs: &[InstalledPackage], venv_dir: &Path, python_version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# uv-migrator snapshot of {}", venv_dir.display());
    let _ = writeln!(out, "# Python {}", python_version);
    let _ = writeln!(out, "# Every package that was installed, with the source it was installed from.");
    let _ = writeln!(out, "# Reinstall with: uv pip install -r <this file>");
    for p in pkgs {
        let _ = writeln!(out, "{}", p.requirement);
    }
    out
}

/// Render an unpinned requirements list for upgrading packages to the latest compatible versions.
/// Editable, VCS, and direct-URL installs preserve their original source specification.
pub fn render_unpinned(pkgs: &[InstalledPackage], venv_dir: &Path, python_version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# uv-migrator upgrade requirements for {}", venv_dir.display());
    let _ = writeln!(out, "# Source Python {}", python_version);
    for p in pkgs {
        let _ = writeln!(out, "{}", p.upgrade_requirement);
    }
    out
}

/// Differences between what was installed before and what is installed now.
/// Empty means the new environment holds the same packages at the same versions.
pub fn diff(before: &[InstalledPackage], after: &[InstalledPackage]) -> Vec<String> {
    diff_with_policy(before, after, false)
}

/// Differences between what was installed before and what is installed now.
/// When `allow_version_changes` is true, upgraded versions are accepted while missing packages still fail.
pub fn diff_with_policy(
    before: &[InstalledPackage],
    after: &[InstalledPackage],
    allow_version_changes: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    for old in before {
        let key = normalize_name(&old.name);
        match after.iter().find(|p| normalize_name(&p.name) == key) {
            None => problems.push(format!("{} is missing", old.name)),
            Some(new) => {
                if !allow_version_changes {
                    if let (Some(ov), Some(nv)) = (&old.version, &new.version) {
                        if ov != nv {
                            problems.push(format!("{} changed {} -> {}", old.name, ov, nv));
                        }
                    }
                }
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, version: &str) -> InstalledPackage {
        InstalledPackage {
            name: name.to_string(),
            version: Some(version.to_string()),
            requirement: format!("{}=={}", name, version),
            upgrade_requirement: name.to_string(),
        }
    }

    #[test]
    fn metadata_headers() {
        let md = "Metadata-Version: 2.1\nName: Foo-Bar\nVersion: 1.2.3\n\nName: not-a-header\n";
        assert_eq!(parse_metadata(md), Some(("Foo-Bar".into(), Some("1.2.3".into()))));
    }

    #[test]
    fn dist_info_dir_name_fallback() {
        assert_eq!(
            parse_dist_info_dir_name("foo_bar-2.0.1.dist-info"),
            Some(("foo_bar".into(), Some("2.0.1".into())))
        );
        assert_eq!(parse_dist_info_dir_name("weird.dist-info"), Some(("weird".into(), None)));
    }

    #[test]
    fn requirement_lines_keep_provenance() {
        assert_eq!(requirement_line("numpy", Some("2.1.0"), None), "numpy==2.1.0");
        let editable = r#"{"url": "file:///home/me/proj", "dir_info": {"editable": true}}"#;
        assert_eq!(requirement_line("proj", Some("0.1.0"), Some(&serde_json::from_str(editable).unwrap())), "-e file:///home/me/proj");
        let local = r#"{"url": "file:///home/me/lib", "dir_info": {}}"#;
        assert_eq!(requirement_line("lib", Some("1.0"), Some(&serde_json::from_str(local).unwrap())), "lib @ file:///home/me/lib");
        let git = r#"{"url": "https://github.com/a/b.git", "vcs_info": {"vcs": "git", "commit_id": "abc123"}}"#;
        assert_eq!(
            requirement_line("b", Some("0.3"), Some(&serde_json::from_str(git).unwrap())),
            "b @ git+https://github.com/a/b.git@abc123"
        );
        let wheel = r#"{"url": "file:///tmp/x-1.0-py3-none-any.whl", "archive_info": {}}"#;
        assert_eq!(requirement_line("x", Some("1.0"), Some(&serde_json::from_str(wheel).unwrap())), "x @ file:///tmp/x-1.0-py3-none-any.whl");
        assert_eq!(requirement_line("y", Some("1.0"), None), "y==1.0");
    }

    #[test]
    fn names_normalise() {
        assert_eq!(normalize_name("Foo_Bar"), "foo-bar");
        assert_eq!(normalize_name("zope.interface"), "zope-interface");
        assert_eq!(normalize_name("a__-b"), "a-b");
    }

    #[test]
    fn diff_reports_missing_and_changed() {
        let before = vec![pkg("numpy", "2.1.0"), pkg("Foo_Bar", "1.0"), pkg("gone", "3.0")];
        let after = vec![pkg("numpy", "2.2.0"), pkg("foo-bar", "1.0"), pkg("extra", "1.0")];
        assert_eq!(diff(&before, &after), vec!["numpy changed 2.1.0 -> 2.2.0", "gone is missing"]);
        assert_eq!(diff_with_policy(&before, &after, true), vec!["gone is missing"]);
        assert!(diff(&before, &before).is_empty());
    }

    #[test]
    fn render_unpinned_strips_index_versions_but_keeps_direct_sources() {
        let pkgs = vec![
            pkg("pillow", "10.4.0"),
            InstalledPackage {
                name: "localproj".to_string(),
                version: Some("0.1.0".to_string()),
                requirement: "-e file:///work/localproj".to_string(),
                upgrade_requirement: "-e file:///work/localproj".to_string(),
            },
        ];
        let rendered = render_unpinned(&pkgs, Path::new("/proj/.venv"), "3.12.4");
        let lines: Vec<&str> = rendered.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(lines, vec!["pillow", "-e file:///work/localproj"]);
    }

    #[test]
    fn snapshot_sits_next_to_the_venv() {
        let p = snapshot_path(Path::new("/proj/.venv"));
        assert_eq!(p, Path::new("/proj/.venv.uv-migrator-snapshot.txt"));
    }
}
