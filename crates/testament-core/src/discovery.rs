use crate::config::AppConfig;
use globset::GlobBuilder;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn discover_test_files(root: &Path, config: &AppConfig) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    visit(root, root, config, &mut files)?;
    files.sort();
    Ok(files)
}

pub fn matches_any_ignore(path: &Path, patterns: &[String]) -> bool {
    let normalized = normalize(path);
    patterns
        .iter()
        .any(|pattern| matches_pattern(&normalized, &normalize_pattern(pattern)))
}

pub fn matches_test_pattern(path: &Path, pattern: &str) -> bool {
    matches_pattern(&normalize(path), &normalize_pattern(pattern))
}

fn visit(
    root: &Path,
    current: &Path,
    config: &AppConfig,
    files: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if !current.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);

        if matches_any_ignore(relative, &config.ignore_paths) {
            continue;
        }

        if file_type.is_dir() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if matches!(name, ".git" | "target" | ".testament" | ".serena") {
                continue;
            }
            visit(root, &path, config, files)?;
            continue;
        }

        if path.extension().and_then(|extension| extension.to_str()) != Some("rb") {
            continue;
        }

        let normalized = normalize(relative);
        if config
            .test_globs
            .iter()
            .any(|pattern| matches_pattern(&normalized, &normalize_pattern(pattern)))
        {
            files.push(path);
        }
    }

    Ok(())
}

fn matches_pattern(path: &str, pattern: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .is_ok_and(|glob| glob.compile_matcher().is_match(path))
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_pattern(pattern: &str) -> String {
    pattern.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_common_ruby_test_patterns() {
        assert!(matches_pattern(
            "spec/models/user_spec.rb",
            "spec/**/*_spec.rb"
        ));
        assert!(matches_pattern(
            "test/unit/user_test.rb",
            "test/**/*_test.rb"
        ));
        assert!(matches_pattern(
            "test/unit/test_user.rb",
            "test/**/test_*.rb"
        ));
        assert!(!matches_pattern("lib/user.rb", "spec/**/*_spec.rb"));
        assert!(matches_pattern("spec/user_spec.rb", "spec/**/*_spec.rb"));
        assert!(matches_pattern("spec/user_spec.rb", "spec/*_spec.rb"));
        assert!(!matches_pattern(
            "spec/models/user_spec.rb",
            "spec/*_spec.rb"
        ));
    }
}
