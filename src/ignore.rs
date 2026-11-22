use std::path::{Path, PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use crate::python_detector::{PythonKind, PythonRuntime};

pub const DEFAULT_IGNORE_FILENAME: &str = ".uv-migrator-ignore";

pub(crate) fn conda_ignore_rules(detected_pythons: &[PythonRuntime]) -> Vec<String> {
    let mut rules: Vec<_> = ["miniconda3", "anaconda3", "miniforge3"].into_iter().map(str::to_string).collect();
    for runtime in detected_pythons.iter().filter(|runtime| runtime.kind == PythonKind::Conda) {
        let prefix = runtime.path.parent().map(|parent| {
            if parent.file_name().is_some_and(|name| name == "bin") {
                parent.parent().unwrap_or(parent)
            } else {
                parent
            }
        });
        if let Some(name) = prefix.and_then(Path::file_name).and_then(|name| name.to_str()) {
            rules.push(globset::escape(name));
        }
    }
    rules.sort();
    rules.dedup();
    rules.into_iter().map(|name| format!("**/{name}/**")).collect()
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IgnoreEngine {
    glob_set: GlobSet,
    patterns: Vec<String>,
}

#[allow(dead_code)]
impl IgnoreEngine {
    pub fn new(patterns: Vec<String>) -> Result<Self, String> {
        Self::compile(patterns.into_iter().map(|pattern| {
            let source = format!("pattern {pattern:?}");
            (pattern, source)
        }))
    }

    fn compile(patterns: impl IntoIterator<Item = (String, String)>) -> Result<Self, String> {
        let mut builder = GlobSetBuilder::new();
        let mut loaded = Vec::new();
        for (pat, source) in patterns {
            let pat_str = pat.trim();
            if pat_str.is_empty() || pat_str.starts_with('#') {
                continue;
            }
            // Normalize slashes for cross-platform matching
            let normalized = pat_str.replace('\\', "/");
            let glob = Glob::new(&normalized)
                .map_err(|e| format!("Invalid ignore glob {pat_str:?} in {source}: {e}"))?;
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
            loaded.push(pat);
        }

        let glob_set = builder.build().map_err(|e| format!("Failed to build GlobSet: {}", e))?;
        Ok(Self { glob_set, patterns: loaded })
    }

    /// Load defaults and the chosen ignore file, then apply additional CLI exclusions.
    pub fn load(root_dir: &Path, custom_file: Option<&Path>, cli_excludes: &[String]) -> Result<Self, String> {
        let file = if let Some(path) = custom_file {
            Some((path.to_path_buf(), Self::read_file(path)?))
        } else {
            Self::read_default(root_dir)?
        };
        Self::from_sources(file.as_ref().map(|(path, text)| (path.as_path(), text.as_str())), cli_excludes)
    }

    pub fn read_default(root_dir: &Path) -> Result<Option<(PathBuf, String)>, String> {
        let root_file = root_dir.join(DEFAULT_IGNORE_FILENAME);
        let cwd_file = std::env::current_dir().ok().map(|cwd| cwd.join(DEFAULT_IGNORE_FILENAME));
        for path in std::iter::once(root_file).chain(cwd_file) {
            match std::fs::read_to_string(&path) {
                Ok(text) => return Ok(Some((path, text))),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Could not read ignore file {}: {error}", path.display())),
            }
        }
        Ok(None)
    }

    fn read_file(path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|error| format!("Could not read ignore file {}: {error}", path.display()))
    }

    pub fn from_text(path: &Path, text: &str) -> Result<Self, String> {
        Self::from_sources(Some((path, text)), &[])
    }

    fn from_sources(file: Option<(&Path, &str)>, cli_excludes: &[String]) -> Result<Self, String> {
        let defaults = [".git", "node_modules", "target", ".cargo", ".rustup", ".cache", "__pycache__"]
            .into_iter()
            .map(|name| (format!("**/{name}/**"), "built-in rules".to_string()));
        let lines = file.into_iter().flat_map(|(path, text)| {
            text.lines().enumerate().map(move |(index, line)| {
                (line.to_string(), format!("{}:{}", path.display(), index + 1))
            })
        });
        let excludes = cli_excludes.iter().map(|pattern| (pattern.clone(), "--exclude".to_string()));
        Self::compile(defaults.chain(lines).chain(excludes))
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
    for rule in conda_ignore_rules(detected_pythons) {
        content.push_str(&rule);
        content.push('\n');
    }
    content.push('\n');

    content.push_str("# --- Custom Do-Not-Touch Folders ---\n");
    content.push_str("# Add any legacy project folders or environments you want untouched:\n");
    content.push_str("# **/legacy_system_app/**\n");
    content.push_str("# **/do_not_touch/**\n");
    content.push_str("# **/third_party/**\n");

    IgnoreEngine::from_text(&out_path, &content)?;
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

