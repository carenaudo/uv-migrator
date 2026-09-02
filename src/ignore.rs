use std::path::{Path, PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use crate::python_detector::{PythonKind, PythonRuntime};

pub const DEFAULT_IGNORE_FILENAME: &str = ".uv-migrator-ignore";

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IgnoreEngine {
    glob_set: GlobSet,
    patterns: Vec<String>,
}

#[allow(dead_code)]
impl IgnoreEngine {
    pub fn new(patterns: Vec<String>) -> Result<Self, String> {
        let mut builder = GlobSetBuilder::new();
        for pat in &patterns {
            let pat_str = pat.trim();
            if pat_str.is_empty() || pat_str.starts_with('#') {
                continue;
            }
            // Normalize slashes for cross-platform matching
            let normalized = pat_str.replace('\\', "/");
            let glob = Glob::new(&normalized)
                .map_err(|e| format!("Invalid ignore glob pattern '{}': {}", pat_str, e))?;
            builder.add(glob);

            // If pattern ends with /**, also match the directory itself without /**
            if let Some(stripped) = normalized.strip_suffix("/**") {
                if let Ok(g) = Glob::new(stripped) {
                    builder.add(g);
                }
            }
            // If pattern doesn't end with /**, also match any subpaths
            if !normalized.ends_with("/**") && !normalized.contains('*') {
                if let Ok(g) = Glob::new(&format!("{}/**", normalized.trim_end_matches('/'))) {
                    builder.add(g);
                }
            }
        }

        let glob_set = builder.build().map_err(|e| format!("Failed to build GlobSet: {}", e))?;
        Ok(Self { glob_set, patterns })
    }

    /// Load ignore engine from optional ignore file path or search default in root_dir / working dir
    pub fn load(root_dir: &Path, custom_file: Option<&Path>, cli_excludes: &[String]) -> Self {
        let mut patterns = Vec::new();

        // Standard sensible default ignores (node_modules, .git, target, etc.)
        patterns.push("**/.git/**".to_string());
        patterns.push("**/node_modules/**".to_string());
        patterns.push("**/target/**".to_string());
        patterns.push("**/.cargo/**".to_string());
        patterns.push("**/.rustup/**".to_string());
        patterns.push("**/.cache/**".to_string());
        patterns.push("**/__pycache__/**".to_string());

        // 1. Check custom ignore file if specified
        if let Some(path) = custom_file {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    for line in content.lines() {
                        patterns.push(line.trim().to_string());
                    }
                }
            }
        } else {
            // 2. Check .uv-migrator-ignore in root_dir or current working directory
            let root_ignore = root_dir.join(DEFAULT_IGNORE_FILENAME);
            if root_ignore.exists() {
                if let Ok(content) = std::fs::read_to_string(&root_ignore) {
                    for line in content.lines() {
                        patterns.push(line.trim().to_string());
                    }
                }
            } else if let Ok(cwd) = std::env::current_dir() {
                let cwd_ignore = cwd.join(DEFAULT_IGNORE_FILENAME);
                if cwd_ignore.exists() {
                    if let Ok(content) = std::fs::read_to_string(&cwd_ignore) {
                        for line in content.lines() {
                            patterns.push(line.trim().to_string());
                        }
                    }
                }
            }
        }

        // 3. Add CLI excludes
        for exc in cli_excludes {
            patterns.push(exc.clone());
        }

        Self::new(patterns).unwrap_or_else(|_| Self {
            glob_set: GlobSetBuilder::new().build().unwrap(),
            patterns: Vec::new(),
        })
    }

    /// Check if a path should be ignored / skipped ("Do Not Touch")
    pub fn is_ignored(&self, path: &Path, base_root: &Path) -> bool {
        // Match relative path
        if let Ok(rel) = path.strip_prefix(base_root) {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if self.glob_set.is_match(&rel_str) {
                return true;
            }
        }

        // Match full path normalized
        let full_str = path.to_string_lossy().replace('\\', "/");
        if self.glob_set.is_match(&full_str) {
            return true;
        }

        // Also check if any parent directory name matches direct pattern
        for comp in path.components() {
            let name = comp.as_os_str().to_string_lossy();
            if self.glob_set.is_match(&*name) {
                return true;
            }
        }

        false
    }

    pub fn get_patterns(&self) -> &[String] {
        &self.patterns
    }
}

/// Generates a well-documented `.uv-migrator-ignore` file in `target_dir`
pub fn generate_default_ignore_file(
    target_dir: &Path,
    detected_pythons: &[PythonRuntime],
) -> Result<PathBuf, String> {
    let out_path = target_dir.join(DEFAULT_IGNORE_FILENAME);
    let mut content = String::new();

    content.push_str("# ========================================================\n");
    content.push_str("# uv-migrator 'Do Not Touch' / Ignore Configuration\n");
    content.push_str("# ========================================================\n");
    content.push_str("# Virtual environments matching any of these patterns will be\n");
    content.push_str("# skipped and left untouched during scan and migration.\n");
    content.push_str("# Uses standard glob syntax (case-sensitive or insensitive per OS).\n\n");

    content.push_str("# --- Version Control & Build Artifacts ---\n");
    content.push_str("**/.git/**\n");
    content.push_str("**/node_modules/**\n");
    content.push_str("**/target/**\n");
    content.push_str("**/build/**\n");
    content.push_str("**/dist/**\n");
    content.push_str("**/__pycache__/**\n\n");

    content.push_str("# --- Conda / Mamba Environments (Protected by default) ---\n");
    for py in detected_pythons {
        if matches!(py.kind, PythonKind::Conda) {
            if let Some(parent) = py.path.parent() {
                let env_name = parent.file_name().unwrap_or_default().to_string_lossy();
                content.push_str(&format!("# Conda env: {}\n", py.path.display()));
                content.push_str(&format!("**/{env_name}/**\n"));
            }
        }
    }
    content.push_str("**/miniconda3/**\n");
    content.push_str("**/anaconda3/**\n");
    content.push_str("**/miniforge3/**\n\n");

    content.push_str("# --- Custom Do-Not-Touch Folders ---\n");
    content.push_str("# Add any legacy project folders or environments you want untouched:\n");
    content.push_str("# **/legacy_system_app/**\n");
    content.push_str("# **/do_not_touch/**\n");
    content.push_str("# **/third_party/**\n");

    std::fs::write(&out_path, content)
        .map_err(|e| format!("Failed to create ignore file at {:?}: {}", out_path, e))?;

    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_ignore_matching() {
        let patterns = vec![
            "**/node_modules/**".to_string(),
            "**/.git/**".to_string(),
            "**/legacy_app/**".to_string(),
            "*.tmp".to_string(),
        ];
        let engine = IgnoreEngine::new(patterns).unwrap();
        let base = PathBuf::from("D:/projects");

        assert!(engine.is_ignored(&PathBuf::from("D:/projects/legacy_app/.venv"), &base));
        assert!(engine.is_ignored(&PathBuf::from("D:/projects/web/node_modules/pkg"), &base));
        assert!(engine.is_ignored(&PathBuf::from("D:/projects/.git"), &base));
        assert!(!engine.is_ignored(&PathBuf::from("D:/projects/active_app/.venv"), &base));
    }
}

