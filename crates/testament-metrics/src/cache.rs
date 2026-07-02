use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use testament_core::TestFileIr;

const CACHE_VERSION: &str = "ir-v2";

pub fn read_ir(path: &Path, content: &str) -> Option<TestFileIr> {
    let cache_path = cache_path(path, content);
    let cached = fs::read_to_string(cache_path).ok()?;
    serde_json::from_str::<TestFileIr>(&cached).ok()
}

pub fn write_ir(path: &Path, content: &str, ir: &TestFileIr) {
    let cache_path = cache_path(path, content);
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string(ir) {
        let _ = fs::write(cache_path, json);
    }
}

fn cache_path(path: &Path, content: &str) -> PathBuf {
    PathBuf::from(".testament")
        .join("cache")
        .join(format!("{}.json", cache_key(path, content)))
}

fn cache_key(path: &Path, content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    CACHE_VERSION.hash(&mut hasher);
    path.to_string_lossy().hash(&mut hasher);
    content.hash(&mut hasher);
    hasher.finish()
}
