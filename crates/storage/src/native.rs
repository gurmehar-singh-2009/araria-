//! Filesystem backend.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone)]
pub(crate) struct NativeStore {
    root: PathBuf,
}

impl NativeStore {
    pub(crate) fn open() -> Self {
        let root = std::env::var_os("ARARIA_DATA").map(PathBuf::from).unwrap_or_else(default_root);
        Self::at(root)
    }

    pub(crate) fn at(root: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&root) {
            log::error!("cannot create data directory {}: {e}", root.display());
        }
        NativeStore { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn full(&self, path: &str) -> PathBuf {
        path.split('/').fold(self.root.clone(), |p, part| p.join(part))
    }

    pub(crate) fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.full(path)).ok()
    }

    pub(crate) fn exists(&self, path: &str) -> bool {
        self.full(path).is_file()
    }

    pub(crate) fn write(&self, path: &str, data: &[u8]) -> Result<(), String> {
        let full = self.full(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        // Write-then-rename so a crash never leaves a half-written file.
        let temp = full.with_extension("araria-tmp");
        std::fs::write(&temp, data).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        std::fs::rename(&temp, &full).map_err(|e| format!("cannot write {}: {e}", full.display()))
    }

    pub(crate) fn remove(&self, path: &str) {
        let _ = std::fs::remove_file(self.full(path));
    }

    pub(crate) fn list(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        let start = if prefix.is_empty() { self.root.clone() } else { self.full(prefix) };
        let mut stack = vec![(start, prefix.to_string())];
        while let Some((dir, relative)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let child = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
                match entry.file_type() {
                    Ok(t) if t.is_dir() => stack.push((entry.path(), child)),
                    Ok(t) if t.is_file() && !name.ends_with(".araria-tmp") => out.push(child),
                    _ => {}
                }
            }
        }
        out.sort();
        out
    }

    /// Removes directories left empty under `prefix`.
    pub(crate) fn prune(&self, prefix: &str) {
        if !prefix.is_empty() && self.list(prefix).is_empty() {
            let _ = std::fs::remove_dir_all(self.full(prefix));
        }
    }

    pub(crate) fn fingerprint(&self, path: &str) -> Option<u64> {
        let meta = std::fs::metadata(self.full(path)).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(modified.as_nanos() as u64 ^ meta.len().rotate_left(40))
    }
}

/// The per-user data folder for the current platform.
fn default_root() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map_or_else(|| home.join("AppData").join("Roaming"), PathBuf::from).join("Araria")
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Araria")
    } else {
        std::env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local").join("share"), PathBuf::from).join("araria")
    }
}
