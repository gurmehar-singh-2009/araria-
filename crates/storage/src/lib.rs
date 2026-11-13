//! # Araria storage
//!
//! Everything Araria remembers — projects, installed games, settings, game
//! progress — lives in one [`Storage`]: a flat store of files addressed by
//! `/`-separated paths such as `projects/sky-run/scripts/main.lua`.
//!
//! * **Desktop:** files in a data directory (`$ARARIA_DATA`, or the
//!   platform's per-user data folder). They are ordinary files, so scripts
//!   can be edited with external tools.
//! * **Browser:** an in-memory copy mirrored to IndexedDB. Reads are
//!   synchronous; the copy is filled asynchronously at startup (see
//!   [`Storage::is_ready`]) and writes are persisted in the background.
//!
//! There are no accounts and nothing leaves the device.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

/// Shared in-memory file map used by the memory and browser backends.
#[derive(Default)]
pub(crate) struct Memory {
    pub(crate) files: BTreeMap<String, Vec<u8>>,
    pub(crate) ready: bool,
    /// Why persistence is unavailable, if it is.
    pub(crate) warning: Option<String>,
}

#[derive(Clone)]
enum Backend {
    Memory(Rc<RefCell<Memory>>),
    #[cfg(not(target_arch = "wasm32"))]
    Native(native::NativeStore),
    #[cfg(target_arch = "wasm32")]
    Web(web::WebStore),
}

/// A handle to the local file store. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Storage {
    backend: Backend,
}

/// Normalizes a path and rejects anything that could escape the store.
pub fn clean_path(path: &str) -> Option<String> {
    let path = path.trim_matches('/');
    if path.is_empty() || path.contains('\\') || path.contains(':') {
        return None;
    }
    if path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return None;
    }
    Some(path.to_string())
}

impl Storage {
    /// Opens the device's persistent store.
    pub fn open() -> Storage {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Storage { backend: Backend::Native(native::NativeStore::open()) }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Storage { backend: Backend::Web(web::WebStore::open()) }
        }
    }

    /// A store that lives only as long as the process (tests, fallbacks).
    pub fn memory() -> Storage {
        Storage { backend: Backend::Memory(Rc::new(RefCell::new(Memory { ready: true, ..Default::default() }))) }
    }

    /// A store rooted at an explicit directory (desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn at(root: impl Into<std::path::PathBuf>) -> Storage {
        Storage { backend: Backend::Native(native::NativeStore::at(root.into())) }
    }

    fn memory_ref(&self) -> Option<&Rc<RefCell<Memory>>> {
        match &self.backend {
            Backend::Memory(m) => Some(m),
            #[cfg(target_arch = "wasm32")]
            Backend::Web(w) => Some(w.memory()),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(_) => None,
        }
    }

    /// False while the browser store is still loading from IndexedDB.
    pub fn is_ready(&self) -> bool {
        self.memory_ref().is_none_or(|m| m.borrow().ready)
    }

    /// A human-readable note when data cannot be persisted (for example in
    /// a private browsing window).
    pub fn warning(&self) -> Option<String> {
        self.memory_ref().and_then(|m| m.borrow().warning.clone())
    }

    /// Where the data lives, for display in settings.
    pub fn location(&self) -> String {
        match &self.backend {
            Backend::Memory(_) => "memory (not saved)".to_string(),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.root().display().to_string(),
            #[cfg(target_arch = "wasm32")]
            Backend::Web(_) => "this browser (IndexedDB)".to_string(),
        }
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = clean_path(path)?;
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.read(&path),
            _ => self.memory_ref()?.borrow().files.get(&path).cloned(),
        }
    }

    pub fn read_string(&self, path: &str) -> Option<String> {
        self.read(path).and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn exists(&self, path: &str) -> bool {
        let Some(path) = clean_path(path) else { return false };
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.exists(&path),
            _ => self.memory_ref().is_some_and(|m| m.borrow().files.contains_key(&path)),
        }
    }

    pub fn write(&self, path: &str, data: &[u8]) -> Result<(), String> {
        let path = clean_path(path).ok_or_else(|| format!("invalid path '{path}'"))?;
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.write(&path, data),
            Backend::Memory(m) => {
                m.borrow_mut().files.insert(path, data.to_vec());
                Ok(())
            }
            #[cfg(target_arch = "wasm32")]
            Backend::Web(w) => {
                w.memory().borrow_mut().files.insert(path.clone(), data.to_vec());
                w.persist(&path, Some(data));
                Ok(())
            }
        }
    }

    pub fn remove(&self, path: &str) {
        let Some(path) = clean_path(path) else { return };
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.remove(&path),
            Backend::Memory(m) => {
                m.borrow_mut().files.remove(&path);
            }
            #[cfg(target_arch = "wasm32")]
            Backend::Web(w) => {
                w.memory().borrow_mut().files.remove(&path);
                w.persist(&path, None);
            }
        }
    }

    /// Every file whose path starts with `prefix/`, sorted.
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.trim_matches('/');
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.list(prefix),
            _ => {
                let Some(memory) = self.memory_ref() else { return Vec::new() };
                let start = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
                memory.borrow().files.keys().filter(|k| k.starts_with(&start)).cloned().collect()
            }
        }
    }

    /// Names of the immediate sub-folders of `prefix`.
    pub fn folders(&self, prefix: &str) -> Vec<String> {
        let prefix = prefix.trim_matches('/');
        let skip = if prefix.is_empty() { 0 } else { prefix.len() + 1 };
        let mut names: Vec<String> = self
            .list(prefix)
            .iter()
            .filter_map(|path| path[skip..].split_once('/').map(|(folder, _)| folder.to_string()))
            .collect();
        names.dedup();
        names
    }

    /// Deletes every file under `prefix/`.
    pub fn remove_all(&self, prefix: &str) {
        for path in self.list(prefix) {
            self.remove(&path);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Backend::Native(n) = &self.backend {
            n.prune(prefix.trim_matches('/'));
        }
    }

    /// Copies every file under `from/` to the same relative path under `to/`.
    pub fn copy_all(&self, from: &str, to: &str) -> Result<usize, String> {
        let (from, to) = (from.trim_matches('/'), to.trim_matches('/'));
        let mut count = 0;
        for path in self.list(from) {
            let Some(data) = self.read(&path) else { continue };
            self.write(&format!("{to}/{}", &path[from.len() + 1..]), &data)?;
            count += 1;
        }
        Ok(count)
    }

    /// A value that changes when the file's content may have changed
    /// (modification time natively, a content hash elsewhere). Used to
    /// detect scripts edited outside the built-in editor.
    pub fn fingerprint(&self, path: &str) -> Option<u64> {
        let path = clean_path(path)?;
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(n) => n.fingerprint(&path),
            _ => self.memory_ref()?.borrow().files.get(&path).map(|data| hash(data)),
        }
    }
}

/// FNV-1a, enough to notice edits.
pub fn hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(storage: &Storage) {
        assert!(storage.is_ready());
        assert!(storage.read("settings.json").is_none());
        storage.write("settings.json", b"{}").unwrap();
        storage.write("projects/a/project.json", b"A").unwrap();
        storage.write("projects/a/scripts/main.lua", b"print('hi')").unwrap();
        storage.write("projects/b/project.json", b"B").unwrap();
        assert_eq!(storage.read_string("projects/a/project.json").as_deref(), Some("A"));
        assert!(storage.exists("projects/a/scripts/main.lua") && !storage.exists("projects/c/x"));
        assert_eq!(storage.folders("projects"), vec!["a", "b"]);
        assert_eq!(storage.list("projects/a"), vec!["projects/a/project.json", "projects/a/scripts/main.lua"]);
        assert_eq!(storage.list("").len(), 4);

        let before = storage.fingerprint("projects/a/scripts/main.lua").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        storage.write("projects/a/scripts/main.lua", b"print('changed!')").unwrap();
        assert_ne!(storage.fingerprint("projects/a/scripts/main.lua").unwrap(), before);

        assert_eq!(storage.copy_all("projects/a", "projects/copy").unwrap(), 2);
        assert_eq!(storage.read_string("projects/copy/scripts/main.lua").as_deref(), Some("print('changed!')"));
        storage.remove_all("projects/a");
        assert_eq!(storage.folders("projects"), vec!["b", "copy"]);
        storage.remove("settings.json");
        assert!(!storage.exists("settings.json"));

        for bad in ["../escape", "/abs/../x", "a//b", "c:\\x", ""] {
            assert!(storage.write(bad, b"x").is_err(), "{bad}");
            assert!(storage.read(bad).is_none());
        }
    }

    #[test]
    fn memory_backend() {
        exercise(&Storage::memory());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_backend() {
        let dir = std::env::temp_dir().join(format!("araria-storage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        exercise(&Storage::at(&dir));
        // Data survives reopening.
        let storage = Storage::at(&dir);
        assert_eq!(storage.read_string("projects/b/project.json").as_deref(), Some("B"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Seconds since the Unix epoch, on every platform.
pub fn now_secs() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
}
