use crate::config::AppConfig;
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
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);

        if matches_any_ignore(relative, &config.ignore_paths) {
            continue;
        }

        if path.is_dir() {
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
    if pattern == path {
        return true;
    }

    if let Some(suffix) = pattern.strip_prefix("**/*") {
        return path.ends_with(suffix);
    }

    if let Some((prefix, suffix)) = pattern.split_once("/**/*") {
        return in_dir(path, prefix) && path.ends_with(suffix);
    }

    if let Some((prefix, suffix)) = pattern.split_once("/**/") {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        if let Some((file_prefix, file_suffix)) = suffix.split_once('*') {
            return in_dir(path, prefix)
                && file_name.starts_with(file_prefix)
                && file_name.ends_with(file_suffix);
        }
        return in_dir(path, prefix) && file_name == suffix;
    }

    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return path.starts_with(prefix) && path.ends_with(suffix);
    }

    false
}

fn in_dir(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
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
    }
}
