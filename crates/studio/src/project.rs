//! The files of the project being edited, in local storage: its manifest,
//! scene and scripts.

use araria_core::DataModel;
use araria_core::project::{MANIFEST_FILE, ProjectManifest, is_safe_path};
use araria_core::scene;
use araria_storage::{Storage, now_secs};

/// Starting content for a new script.
pub const SCRIPT_TEMPLATE: &str = "-- This script runs when the game starts.\n-- Edit it while the game is playing: saving reloads it instantly.\n\nprint(\"Hello from \" .. script.Name)\n";
pub const MODULE_TEMPLATE: &str = "-- A module: other scripts load it with require(\"NAME\").\nlocal module = {}\n\nfunction module.hello()\n    return \"hello\"\nend\n\nreturn module\n";

/// A project folder such as `projects/my-game`.
#[derive(Clone)]
pub struct ProjectFolder {
    storage: Storage,
    root: String,
}

impl ProjectFolder {
    pub fn new(storage: Storage, root: impl Into<String>) -> Self {
        ProjectFolder { storage, root: root.into() }
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    fn full(&self, path: &str) -> Result<String, String> {
        if !is_safe_path(path) {
            return Err(format!("'{path}' is not a valid project path"));
        }
        Ok(format!("{}/{path}", self.root))
    }

    pub fn read(&self, path: &str) -> Option<String> {
        self.storage.read_string(&self.full(path).ok()?)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.full(path).is_ok_and(|p| self.storage.exists(&p))
    }

    pub fn write(&self, path: &str, text: &str) -> Result<(), String> {
        self.storage.write(&self.full(path)?, text.as_bytes())
    }

    pub fn remove(&self, path: &str) {
        if let Ok(full) = self.full(path) {
            self.storage.remove(&full);
        }
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        if self.exists(to) {
            return Err(format!("'{to}' already exists"));
        }
        let data = self.storage.read(&self.full(from)?).ok_or_else(|| format!("'{from}' was not found"))?;
        self.storage.write(&self.full(to)?, &data)?;
        self.remove(from);
        Ok(())
    }

    /// Changes whenever the file's content changes.
    pub fn fingerprint(&self, path: &str) -> Option<u64> {
        self.storage.fingerprint(&self.full(path).ok()?)
    }

    /// Every file, as project-relative paths, sorted.
    pub fn files(&self) -> Vec<String> {
        let prefix = format!("{}/", self.root);
        let mut files: Vec<String> = self.storage.list(&self.root).into_iter().filter_map(|p| p.strip_prefix(&prefix).map(str::to_string)).collect();
        files.sort();
        files
    }

    /// Lua files.
    pub fn scripts(&self) -> Vec<String> {
        self.files().into_iter().filter(|f| f.ends_with(".lua")).collect()
    }

    /// A path under `scripts/` for a new script named like `name` that does
    /// not exist yet.
    pub fn unused_script_path(&self, name: &str) -> String {
        let base: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        let base = if base.is_empty() { "script".to_string() } else { base };
        let mut path = format!("scripts/{base}.lua");
        let mut n = 2;
        while self.exists(&path) {
            path = format!("scripts/{base}{n}.lua");
            n += 1;
        }
        path
    }

    pub fn manifest(&self) -> Option<ProjectManifest> {
        ProjectManifest::parse(&self.read(MANIFEST_FILE)?).ok()
    }

    /// Writes the scene and marks the project as modified now.
    pub fn save_scene(&self, dm: &DataModel) -> Result<(), String> {
        let mut manifest = self.manifest().ok_or("the project's project.json is missing")?;
        self.write(&manifest.scene, &scene::save_scene_string(dm))?;
        manifest.modified = now_secs();
        self.write(MANIFEST_FILE, &manifest.to_json())
    }

    /// Updates the modified time after scripts were saved.
    pub fn touch(&self) {
        if let Some(mut manifest) = self.manifest() {
            manifest.modified = now_secs();
            let _ = self.write(MANIFEST_FILE, &manifest.to_json());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_writes_and_lists_project_files() {
        let storage = Storage::memory();
        let folder = ProjectFolder::new(storage.clone(), "projects/demo");
        folder.write(MANIFEST_FILE, &ProjectManifest::new("demo", "Demo", 1).to_json()).unwrap();
        folder.write("scripts/main.lua", "print(1)").unwrap();
        assert_eq!(folder.read("scripts/main.lua").as_deref(), Some("print(1)"));
        assert!(folder.write("../escape.lua", "x").is_err());
        assert_eq!(folder.scripts(), vec!["scripts/main.lua".to_string()]);
        assert_eq!(folder.unused_script_path("main"), "scripts/main2.lua");
        assert_eq!(folder.unused_script_path("Door Opener"), "scripts/Door_Opener.lua");
        let before = folder.fingerprint("scripts/main.lua");
        folder.write("scripts/main.lua", "print(2)").unwrap();
        assert_ne!(folder.fingerprint("scripts/main.lua"), before);
        folder.rename("scripts/main.lua", "scripts/game.lua").unwrap();
        assert!(!folder.exists("scripts/main.lua") && folder.exists("scripts/game.lua"));

        let mut dm = DataModel::new();
        dm.create("Part", Some(dm.workspace())).unwrap();
        folder.save_scene(&dm).unwrap();
        let manifest = folder.manifest().unwrap();
        assert!(manifest.modified > 1);
        let saved = scene::load_scene_str(&folder.read(&manifest.scene).unwrap()).unwrap();
        assert_eq!(saved.model.children(saved.model.workspace()).len(), dm.children(dm.workspace()).len());
    }
}
