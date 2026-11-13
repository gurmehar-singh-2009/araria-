//! The local library: the built-in catalog, installed games, the user's
//! projects and play history.
//!
//! Storage layout:
//!
//! ```text
//! installed/<id>/...   installed copies of catalog games
//! projects/<id>/...    the user's own projects
//! library.json         play counts and recent activity
//! ```
//!
//! Catalog games are embedded in the application (the `games/` folder), so
//! the platform is fully usable offline. Adding a folder there adds a game
//! to the store; no engine code changes are needed.

use std::collections::{BTreeMap, HashMap};

use araria_core::project::{self, DEFAULT_SCENE, MANIFEST_FILE, ProjectManifest, THUMBNAIL_FILE};
use araria_core::scene::{self, LoadedScene};
use araria_core::{DataModel, Vec3};
use araria_storage::{Storage, now_secs};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};

static BUILTIN: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../games");

const INSTALLED: &str = "installed";
const PROJECTS: &str = "projects";
const STATS_FILE: &str = "library.json";
const MAX_RECENT: usize = 12;

/// Where a library entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    /// A game from the built-in catalog.
    Catalog,
    /// One of the user's own projects.
    Project,
}

/// Identifies a game or project in the library.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GameKey {
    pub origin: Origin,
    pub id: String,
}

impl GameKey {
    pub fn catalog(id: &str) -> Self {
        GameKey { origin: Origin::Catalog, id: id.to_string() }
    }

    pub fn project(id: &str) -> Self {
        GameKey { origin: Origin::Project, id: id.to_string() }
    }

    /// Storage folder holding the playable/editable files.
    pub fn root(&self) -> String {
        match self.origin {
            Origin::Catalog => format!("{INSTALLED}/{}", self.id),
            Origin::Project => format!("{PROJECTS}/{}", self.id),
        }
    }

    /// A stable string form, e.g. for texture cache keys.
    pub fn label(&self) -> String {
        format!("{:?}:{}", self.origin, self.id)
    }
}

#[derive(Clone, Debug)]
pub struct GameEntry {
    pub key: GameKey,
    pub manifest: ProjectManifest,
    /// Catalog games: whether a local copy exists. Projects: always true.
    pub installed: bool,
    /// Total size of the game's files in bytes.
    pub size: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Stats {
    #[serde(default)]
    plays: HashMap<String, u32>,
    /// Most recent first: (key label, key, unix time).
    #[serde(default)]
    recent: Vec<(GameKey, u64)>,
    #[serde(default)]
    recent_projects: Vec<(String, u64)>,
}

/// A game ready to hand to the runtime.
pub struct LoadedGame {
    pub key: GameKey,
    pub manifest: ProjectManifest,
    pub model: DataModel,
    pub warnings: Vec<String>,
}

pub struct Library {
    storage: Storage,
    pub catalog: Vec<GameEntry>,
    pub projects: Vec<GameEntry>,
    stats: Stats,
}

fn builtin_manifest(dir: &Dir<'_>) -> Option<ProjectManifest> {
    let file = dir.get_file(dir.path().join(MANIFEST_FILE))?;
    match ProjectManifest::parse(file.contents_utf8()?) {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            log::error!("built-in game {} has an invalid manifest: {e}", dir.path().display());
            None
        }
    }
}

/// Every file of an embedded game as (relative path, bytes).
fn builtin_files(dir: &'static Dir<'static>) -> Vec<(String, &'static [u8])> {
    let mut files = Vec::new();
    let mut stack = vec![dir];
    while let Some(current) = stack.pop() {
        for file in current.files() {
            if let Ok(relative) = file.path().strip_prefix(dir.path()) {
                files.push((relative.to_string_lossy().replace('\\', "/"), file.contents()));
            }
        }
        stack.extend(current.dirs());
    }
    files
}

fn builtin_dir(id: &str) -> Option<&'static Dir<'static>> {
    BUILTIN.dirs().find(|d| d.path().file_name().is_some_and(|n| n == id))
}

impl Library {
    pub fn new(storage: Storage) -> Self {
        let mut library = Library { storage, catalog: Vec::new(), projects: Vec::new(), stats: Stats::default() };
        library.refresh();
        library
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Re-reads the catalog, the projects and the statistics.
    pub fn refresh(&mut self) {
        self.stats = self.storage.read_string(STATS_FILE).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        self.catalog = BUILTIN
            .dirs()
            .filter_map(|dir| {
                let manifest = builtin_manifest(dir)?;
                let key = GameKey::catalog(&manifest.id);
                let installed = self.storage.exists(&format!("{}/{MANIFEST_FILE}", key.root()));
                let size = builtin_files(dir).iter().map(|(_, bytes)| bytes.len() as u64).sum();
                Some(GameEntry { key, manifest, installed, size })
            })
            .collect();
        self.catalog.sort_by(|a, b| b.manifest.featured.cmp(&a.manifest.featured).then(a.manifest.name.cmp(&b.manifest.name)));
        self.projects = self
            .storage
            .folders(PROJECTS)
            .into_iter()
            .filter_map(|id| {
                let key = GameKey::project(&id);
                let text = self.storage.read_string(&format!("{}/{MANIFEST_FILE}", key.root()))?;
                let manifest = ProjectManifest::parse(&text).ok()?;
                let size = self.storage.list(&key.root()).iter().filter_map(|p| self.storage.read(p)).map(|d| d.len() as u64).sum();
                Some(GameEntry { key, manifest, installed: true, size })
            })
            .collect();
        self.projects.sort_by(|a, b| b.manifest.modified.cmp(&a.manifest.modified));
    }

    fn save_stats(&self) {
        let text = serde_json::to_string(&self.stats).expect("stats are always serializable");
        if let Err(e) = self.storage.write(STATS_FILE, text.as_bytes()) {
            log::error!("could not save library.json: {e}");
        }
    }

    pub fn find(&self, key: &GameKey) -> Option<&GameEntry> {
        let list = if key.origin == Origin::Catalog { &self.catalog } else { &self.projects };
        list.iter().find(|e| e.key == *key)
    }

    pub fn installed(&self) -> impl Iterator<Item = &GameEntry> {
        self.catalog.iter().filter(|e| e.installed)
    }

    pub fn play_count(&self, key: &GameKey) -> u32 {
        self.stats.plays.get(&key.label()).copied().unwrap_or(0)
    }

    /// Recently played games that still exist, most recent first.
    pub fn recent(&self) -> Vec<(&GameEntry, u64)> {
        self.stats.recent.iter().filter_map(|(key, time)| self.find(key).map(|e| (e, *time))).collect()
    }

    /// Catalog games ordered by how often they were played here, then by
    /// editorial weight.
    pub fn popular(&self) -> Vec<&GameEntry> {
        let mut games: Vec<&GameEntry> = self.catalog.iter().collect();
        games.sort_by(|a, b| self.play_count(&b.key).cmp(&self.play_count(&a.key)).then(b.manifest.featured.cmp(&a.manifest.featured)));
        games
    }

    /// Games in categories the player has not tried yet, falling back to
    /// the least played.
    pub fn recommended(&self) -> Vec<&GameEntry> {
        let played: Vec<&str> = self.catalog.iter().filter(|e| self.play_count(&e.key) > 0).map(|e| e.manifest.category.as_str()).collect();
        let mut games: Vec<&GameEntry> = self.catalog.iter().collect();
        games.sort_by_key(|e| (played.contains(&e.manifest.category.as_str()), self.play_count(&e.key), std::cmp::Reverse(e.manifest.featured)));
        games
    }

    pub fn record_play(&mut self, key: &GameKey) {
        *self.stats.plays.entry(key.label()).or_default() += 1;
        self.stats.recent.retain(|(k, _)| k != key);
        self.stats.recent.insert(0, (key.clone(), now_secs()));
        self.stats.recent.truncate(MAX_RECENT);
        self.save_stats();
    }

    pub fn record_project_opened(&mut self, id: &str) {
        self.stats.recent_projects.retain(|(p, _)| p != id);
        self.stats.recent_projects.insert(0, (id.to_string(), now_secs()));
        self.stats.recent_projects.truncate(MAX_RECENT);
        self.save_stats();
    }

    /// PNG bytes of a game's cover image, if it has one.
    pub fn thumbnail(&self, key: &GameKey) -> Option<Vec<u8>> {
        if let Some(bytes) = self.storage.read(&format!("{}/{THUMBNAIL_FILE}", key.root())) {
            return Some(bytes);
        }
        if key.origin == Origin::Catalog {
            let dir = builtin_dir(&key.id)?;
            return dir.get_file(dir.path().join(THUMBNAIL_FILE)).map(|f| f.contents().to_vec());
        }
        None
    }

    // ---------------------------------------------------- catalog games

    /// Copies a catalog game into local storage.
    pub fn install(&mut self, id: &str) -> Result<(), String> {
        let dir = builtin_dir(id).ok_or_else(|| format!("'{id}' is not in the catalog"))?;
        let root = GameKey::catalog(id).root();
        for (path, bytes) in builtin_files(dir) {
            self.storage.write(&format!("{root}/{path}"), bytes)?;
        }
        self.refresh();
        Ok(())
    }

    pub fn uninstall(&mut self, id: &str) {
        self.storage.remove_all(&GameKey::catalog(id).root());
        self.refresh();
    }

    // -------------------------------------------------------- projects

    fn unique_project_id(&self, name: &str) -> String {
        let base = project::slugify(name);
        let mut id = base.clone();
        let mut n = 2;
        while self.storage.exists(&format!("{PROJECTS}/{id}/{MANIFEST_FILE}")) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// Creates a project. `template` is a catalog game id to start from, or
    /// `None` for an empty baseplate.
    pub fn create_project(&mut self, name: &str, template: Option<&str>) -> Result<GameKey, String> {
        let name = if name.trim().is_empty() { "Untitled Project" } else { name.trim() };
        let id = self.unique_project_id(name);
        let key = GameKey::project(&id);
        let root = key.root();
        let now = now_secs();
        let mut manifest = ProjectManifest::new(&id, name, now);
        match template {
            Some(template) => {
                let dir = builtin_dir(template).ok_or_else(|| format!("unknown template '{template}'"))?;
                for (path, bytes) in builtin_files(dir) {
                    if path != MANIFEST_FILE {
                        self.storage.write(&format!("{root}/{path}"), bytes)?;
                    }
                }
                if let Some(source) = builtin_manifest(dir) {
                    manifest.category = source.category;
                    manifest.scene = source.scene;
                    manifest.controls = source.controls;
                    manifest.description = format!("A remix of {}.", source.name);
                }
            }
            None => {
                let scene = scene::save_scene_string(&baseplate());
                self.storage.write(&format!("{root}/{DEFAULT_SCENE}"), scene.as_bytes())?;
            }
        }
        self.storage.write(&format!("{root}/{MANIFEST_FILE}"), manifest.to_json().as_bytes())?;
        self.refresh();
        Ok(key)
    }

    pub fn duplicate_project(&mut self, id: &str) -> Result<GameKey, String> {
        let source = self.find(&GameKey::project(id)).ok_or("project not found")?.manifest.clone();
        let name = format!("{} Copy", source.name);
        let new_id = self.unique_project_id(&name);
        let key = GameKey::project(&new_id);
        self.storage.copy_all(&GameKey::project(id).root(), &key.root())?;
        let now = now_secs();
        let manifest = ProjectManifest { id: new_id, name, created: now, modified: now, ..source };
        self.storage.write(&format!("{}/{MANIFEST_FILE}", key.root()), manifest.to_json().as_bytes())?;
        self.refresh();
        Ok(key)
    }

    pub fn delete_project(&mut self, id: &str) {
        self.storage.remove_all(&GameKey::project(id).root());
        self.stats.recent.retain(|(k, _)| !(k.origin == Origin::Project && k.id == id));
        self.stats.recent_projects.retain(|(p, _)| p != id);
        self.save_stats();
        self.refresh();
    }

    /// Updates a project's manifest (name, description, ...).
    pub fn save_manifest(&mut self, key: &GameKey, manifest: &ProjectManifest) -> Result<(), String> {
        let mut manifest = manifest.clone();
        manifest.modified = now_secs();
        self.storage.write(&format!("{}/{MANIFEST_FILE}", key.root()), manifest.to_json().as_bytes())?;
        self.refresh();
        Ok(())
    }

    /// Packs a project into a portable `.araria` bundle.
    pub fn export_project(&self, key: &GameKey) -> Result<String, String> {
        let root = key.root();
        let mut files = BTreeMap::new();
        for path in self.storage.list(&root) {
            if let Some(data) = self.storage.read(&path) {
                files.insert(path[root.len() + 1..].to_string(), data);
            }
        }
        if files.is_empty() {
            return Err("the project has no files".to_string());
        }
        Ok(project::pack_bundle(&files))
    }

    /// Adds a project from a bundle. The imported copy gets a fresh id, so
    /// importing never overwrites existing work.
    pub fn import_bundle(&mut self, text: &str) -> Result<GameKey, String> {
        let files = project::unpack_bundle(text).map_err(|e| e.to_string())?;
        let manifest_bytes = files.get(MANIFEST_FILE).ok_or("the bundle has no project.json")?;
        let manifest = ProjectManifest::parse(&String::from_utf8_lossy(manifest_bytes)).map_err(|e| e.to_string())?;
        let id = self.unique_project_id(&manifest.name);
        let key = GameKey::project(&id);
        for (path, data) in &files {
            if path != MANIFEST_FILE {
                self.storage.write(&format!("{}/{path}", key.root()), data)?;
            }
        }
        let manifest = ProjectManifest { id, modified: now_secs(), ..manifest };
        self.storage.write(&format!("{}/{MANIFEST_FILE}", key.root()), manifest.to_json().as_bytes())?;
        self.refresh();
        Ok(key)
    }

    // --------------------------------------------------------- loading

    /// Loads a game's scene for playing or editing. Catalog games are
    /// installed first if needed.
    pub fn load(&mut self, key: &GameKey) -> Result<LoadedGame, String> {
        if key.origin == Origin::Catalog && !self.find(key).is_some_and(|e| e.installed) {
            self.install(&key.id)?;
        }
        let root = key.root();
        let manifest_text = self.storage.read_string(&format!("{root}/{MANIFEST_FILE}")).ok_or("the game's project.json is missing")?;
        let manifest = ProjectManifest::parse(&manifest_text).map_err(|e| e.to_string())?;
        let scene_text = self.storage.read_string(&format!("{root}/{}", manifest.scene)).ok_or_else(|| format!("scene '{}' is missing", manifest.scene))?;
        let LoadedScene { model, warnings } = scene::load_scene_str(&scene_text).map_err(|e| e.to_string())?;
        Ok(LoadedGame { key: key.clone(), manifest, model, warnings })
    }
}

/// The scene new blank projects start with: a baseplate, a spawn point and
/// daylight.
pub fn baseplate() -> DataModel {
    let mut dm = DataModel::new();
    let workspace = dm.workspace();
    let set = |dm: &mut DataModel, id, property: &str, value: araria_core::Variant| {
        if let Err(e) = dm.set(id, property, value) {
            log::error!("baseplate: {e}");
        }
    };
    if let Ok(ground) = dm.create("Part", Some(workspace)) {
        let _ = dm.set_name(ground, "Baseplate");
        set(&mut dm, ground, "Size", Vec3::new(200.0, 2.0, 200.0).into());
        set(&mut dm, ground, "Position", Vec3::new(0.0, -1.0, 0.0).into());
        set(&mut dm, ground, "Color", araria_core::Color::from_rgb8(95, 174, 87).into());
        set(&mut dm, ground, "Material", "Grass".into());
        set(&mut dm, ground, "Locked", true.into());
    }
    if let Ok(spawn) = dm.create("SpawnPoint", Some(workspace)) {
        set(&mut dm, spawn, "Size", Vec3::new(4.0, 0.3, 4.0).into());
        set(&mut dm, spawn, "Position", Vec3::new(0.0, 0.15, 0.0).into());
        set(&mut dm, spawn, "Color", araria_core::Color::from_rgb8(76, 201, 240).into());
    }
    dm.clear_journal();
    dm.take_events();
    dm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_embedded_games_and_installs_them() {
        let mut library = Library::new(Storage::memory());
        let first = library.catalog.first().expect("at least one built-in game").key.clone();
        assert!(!library.find(&first).unwrap().installed);
        assert!(library.find(&first).unwrap().size > 0);
        library.install(&first.id).unwrap();
        assert!(library.find(&first).unwrap().installed);
        assert_eq!(library.installed().count(), 1);

        // Every catalog game loads into a valid scene.
        for key in library.catalog.iter().map(|e| e.key.clone()).collect::<Vec<_>>() {
            let game = library.load(&key).unwrap_or_else(|e| panic!("{}: {e}", key.id));
            assert!(game.warnings.is_empty(), "{}: {:?}", key.id, game.warnings);
            assert!(!game.model.children(game.model.workspace()).is_empty());
        }
        library.uninstall(&first.id);
        assert!(!library.find(&first).unwrap().installed);
    }

    #[test]
    fn play_history_drives_recent_and_popular() {
        let mut library = Library::new(Storage::memory());
        let key = library.catalog[0].key.clone();
        assert!(library.recent().is_empty());
        library.record_play(&key);
        library.record_play(&key);
        assert_eq!(library.play_count(&key), 2);
        assert_eq!(library.recent()[0].0.key, key);
        assert_eq!(library.popular()[0].key, key);

        // History survives a restart.
        let reopened = Library::new(library.storage().clone());
        assert_eq!(reopened.play_count(&key), 2);
    }

    #[test]
    fn project_lifecycle() {
        let mut library = Library::new(Storage::memory());
        let blank = library.create_project("My First Game", None).unwrap();
        assert_eq!(blank.id, "my-first-game");
        let again = library.create_project("My First Game", None).unwrap();
        assert_eq!(again.id, "my-first-game-2", "ids never collide");
        let game = library.load(&blank).unwrap();
        assert!(game.model.find_by_path("Workspace.Baseplate").is_some());

        let template = library.catalog[0].key.id.clone();
        let remix = library.create_project("Remix", Some(&template)).unwrap();
        assert!(library.load(&remix).is_ok());
        assert_eq!(library.projects.len(), 3);

        let copy = library.duplicate_project(&blank.id).unwrap();
        assert_eq!(library.find(&copy).unwrap().manifest.name, "My First Game Copy");

        let bundle = library.export_project(&remix).unwrap();
        let mut other = Library::new(Storage::memory());
        let imported = other.import_bundle(&bundle).unwrap();
        assert_eq!(other.find(&imported).unwrap().manifest.name, "Remix");
        assert!(other.load(&imported).is_ok());
        assert!(other.import_bundle("garbage").is_err());

        library.delete_project(&blank.id);
        assert!(library.find(&blank).is_none());
        assert_eq!(library.projects.len(), 3);
    }
}
