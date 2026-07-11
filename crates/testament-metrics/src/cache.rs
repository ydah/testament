use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use testament_core::TestFileIr;

const CACHE_VERSION: &str = concat!("ir-v5-", env!("CARGO_PKG_VERSION"));
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn read_ir(path: &Path, content: &str) -> Option<TestFileIr> {
    let cache_path = cache_path(path, content);
    let cached = fs::read_to_string(cache_path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&cached).ok()?;
    if value.get("version")?.as_str()? != CACHE_VERSION
        || value.get("path")?.as_str()? != normalized_path(path)
        || value.get("content")?.as_str()? != content
    {
        return None;
    }
    serde_json::from_value::<TestFileIr>(value.get("ir")?.clone()).ok()
}

pub fn write_ir(path: &Path, content: &str, ir: &TestFileIr) {
    let cache_path = cache_path(path, content);
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let value = serde_json::json!({
        "version": CACHE_VERSION,
        "path": normalized_path(path),
        "content": content,
        "ir": ir,
    });
    let Ok(json) = serde_json::to_vec(&value) else {
        return;
    };
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_path = cache_path.with_extension(format!("{}.{}.tmp", std::process::id(), sequence));
    if fs::write(&temp_path, json).is_ok() {
        let _ = fs::rename(&temp_path, &cache_path);
    }
}

fn cache_path(path: &Path, content: &str) -> PathBuf {
    cache_root(path)
        .join(".testament")
        .join("cache")
        .join(format!("{}.json", cache_key(path, content)))
}

fn cache_key(path: &Path, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_VERSION);
    hasher.update([0]);
    hasher.update(normalized_path(path));
    hasher.update([0]);
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn cache_root(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute
        .ancestors()
        .skip(1)
        .find(|ancestor| ancestor.join("testament.toml").is_file())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}
