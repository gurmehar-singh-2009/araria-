//! # Araria Studio
//!
//! The built-in development environment: a 3D scene editor with explorer,
//! properties inspector and asset browser, a Lua code editor, and a
//! terminal connected to the running game.
//!
//! The studio edits the real thing: the engine renders the same data model
//! the explorer and inspector change, scripts are files in the project
//! folder, and Play runs the game in place. Stopping returns the scene to
//! how it was before Play (or keeps the played state when asked), so a test
//! run never overwrites the saved project.

pub mod assets;
pub mod code;
pub mod explorer;
pub mod history;
pub mod inspector;
pub mod project;
pub mod terminal;
pub mod viewport;

use std::collections::HashMap;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::scene::{load_subtree, save_subtree};
use araria_core::{DataModel, InstanceId, Vec3};
use araria_platform::{icons, theme};
use araria_runtime::{Command, Mode, Runtime};
use egui::{Align2, CornerRadius, Key, Modifiers, Rect, RichText, Stroke, Ui, vec2};

use assets::{AssetAction, AssetBrowser, AssetTab, DraggedModel};
use code::editor::{EditorAction, ScriptEditor};
use code::reference::Reference;
use explorer::{Explorer, ExplorerAction};
use history::History;
use inspector::Inspector;
use project::{MODULE_TEMPLATE, ProjectFolder, SCRIPT_TEMPLATE};
use terminal::{Request, Terminal};
use viewport::{Tool, Viewport};

/// What fills the middle of the studio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterView {
    Scene,
    /// The 3D view and the code editor side by side.
    Split,
    Code,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeftTab {
    Explorer,
    Assets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RightTab {
    Properties,
    Reference,
}

/// A question waiting for the user's answer.
enum Confirm {
    Leave,
    CloseTab(usize),
    DeleteFile(String),
}

/// What the app needs to do after the studio drew a frame.
#[derive(Default)]
pub struct StudioOutput {
    /// Where the 3D view should be rendered (logical pixels), or `None`
    /// when it is hidden behind the code editor.
    pub viewport: Option<Rect>,
    /// The game should ignore the pointer / keyboard this frame.
    pub capture_pointer: bool,
    pub capture_keyboard: bool,
    /// The user closed the studio.
    pub leave: bool,
    /// Point the editing camera at this (center, radius).
    pub focus: Option<(Vec3, f32)>,
    /// Play just started: put the camera behind the character.
    pub started_playing: bool,
}

pub struct Studio {
    pub name: String,
    folder: ProjectFolder,
    assets: Arc<AssetManifest>,
    pub selection: Vec<InstanceId>,
    explorer: Explorer,
    inspector: Inspector,
    browser: AssetBrowser,
    reference: Reference,
    pub editor: ScriptEditor,
    pub terminal: Terminal,
    pub viewport: Viewport,
    history: History,
    /// `DataModel::revision` when the scene was last saved.
    saved_revision: u64,
    center: CenterView,
    left: LeftTab,
    right: RightTab,
    /// Fraction of the center width given to the 3D view in split mode.
    split: f32,
    clipboard: Vec<serde_json::Value>,
    confirm: Option<Confirm>,
    files: Vec<String>,
    files_checked: f64,
    /// Fingerprints of open files as last read or written, to notice when
    /// another program changes them.
    prints: HashMap<String, u64>,
    /// A short message shown in the top bar: (text, until).
    status: Option<(String, f64)>,
    /// Whether the scene had unsaved edits when play started.
    dirty_before_play: bool,
}

fn is_typing(ctx: &egui::Context) -> bool {
    ctx.egui_wants_keyboard_input()
}

impl Studio {
    pub fn new(name: &str, folder: ProjectFolder, assets: Arc<AssetManifest>, runtime: &Runtime) -> Self {
        let dm = runtime.dm.borrow();
        let mut studio = Studio {
            name: name.to_string(),
            files: folder.files(),
            folder,
            viewport: Viewport::new(assets.clone()),
            assets,
            selection: Vec::new(),
            explorer: Explorer::default(),
            inspector: Inspector::default(),
            browser: AssetBrowser::default(),
            reference: Reference::default(),
            editor: ScriptEditor::default(),
            terminal: Terminal::default(),
            history: History::new(&dm),
            saved_revision: dm.revision(),
            center: CenterView::Scene,
            left: LeftTab::Explorer,
            right: RightTab::Properties,
            split: 0.5,
            clipboard: Vec::new(),
            confirm: None,
            files_checked: 0.0,
            prints: HashMap::new(),
            status: None,
            dirty_before_play: false,
        };
        let scripts = dm.instances().filter(|i| i.class_name() == "Script").count();
        studio.terminal.system(format!("Opened “{name}” — {} objects, {scripts} script{}. Press Play (F5) to test.", dm.len(), if scripts == 1 { "" } else { "s" }));
        studio
    }

    pub fn folder(&self) -> &ProjectFolder {
        &self.folder
    }

    /// Whether the scene has changes that are not saved.
    pub fn scene_dirty(&self, runtime: &Runtime) -> bool {
        if runtime.mode() == Mode::Edit { runtime.dm.borrow().revision() != self.saved_revision } else { self.dirty_before_play }
    }

    pub fn has_unsaved_changes(&self, runtime: &Runtime) -> bool {
        self.scene_dirty(runtime) || self.editor.has_unsaved()
    }

    fn flash(&mut self, ui: &Ui, text: impl Into<String>) {
        let text = text.into();
        self.terminal.system(text.clone());
        self.status = Some((text, ui.input(|i| i.time) + 3.0));
    }

    // ------------------------------------------------------------ files

    fn refresh_files(&mut self) {
        self.files = self.folder.files();
    }

    /// Opens a project file in the editor and shows the code.
    pub fn open_file(&mut self, path: &str, line: u32) {
        let folder = self.folder.clone();
        match self.editor.reveal(path, line as usize, || folder.read(path)) {
            Ok(()) => {
                if let Some(print) = self.folder.fingerprint(path) {
                    self.prints.insert(path.to_string(), print);
                }
                if self.center == CenterView::Scene {
                    self.center = CenterView::Split;
                }
            }
            Err(e) => self.terminal.error(e),
        }
    }

    fn save_document(&mut self, index: usize, runtime: &mut Runtime) -> bool {
        let Some(document) = self.editor.documents.get_mut(index) else { return false };
        let path = document.path.clone();
        if let Err(e) = self.folder.write(&path, &document.text) {
            self.terminal.error(format!("Could not save {path}: {e}"));
            return false;
        }
        document.mark_saved();
        document.check();
        let syntax = document.syntax_error.clone();
        if let Some(print) = self.folder.fingerprint(&path) {
            self.prints.insert(path.clone(), print);
        }
        self.folder.touch();
        // Hot reload right away rather than waiting for the file watcher.
        if runtime.mode() != Mode::Edit {
            match runtime.scripts.reload(&path) {
                Ok(0) => {}
                Ok(_) => {}
                Err(_) => {}
            }
            let log = runtime.scripts.take_log();
            self.forward_errors(&log);
            self.terminal.absorb(log);
        }
        if let Some(error) = syntax {
            self.terminal.error(format!("Saved {path} with a syntax error on line {}: {}", error.line + 1, error.message));
        } else {
            self.terminal.system(format!("Saved {path}"));
        }
        self.refresh_files();
        true
    }

    /// Saves the scene (when editing) and every open script.
    pub fn save_all(&mut self, ui: &Ui, runtime: &mut Runtime) {
        for index in 0..self.editor.documents.len() {
            if self.editor.documents[index].is_dirty() {
                self.save_document(index, runtime);
            }
        }
        if runtime.mode() == Mode::Edit {
            let dm = runtime.dm.borrow();
            match self.folder.save_scene(&dm) {
                Ok(()) => {
                    self.saved_revision = dm.revision();
                    drop(dm);
                    self.flash(ui, "Saved the scene and scripts");
                }
                Err(e) => {
                    drop(dm);
                    self.terminal.error(format!("Could not save the scene: {e}"));
                }
            }
        } else {
            self.flash(ui, "Scripts saved. The scene is saved when you stop playing (use Stop → keep changes to save what the game changed).");
        }
        self.refresh_files();
    }

    fn new_script(&mut self, name: &str, module: bool) -> Option<String> {
        let path = self.folder.unused_script_path(name);
        let template = if module { MODULE_TEMPLATE.replace("NAME", name) } else { SCRIPT_TEMPLATE.to_string() };
        match self.folder.write(&path, &template) {
            Ok(()) => {
                self.refresh_files();
                Some(path)
            }
            Err(e) => {
                self.terminal.error(format!("Could not create {path}: {e}"));
                None
            }
        }
    }

    /// Notices files changed by other programs and reloads unmodified tabs.
    fn watch_files(&mut self, now: f64) {
        if now - self.files_checked < 1.0 {
            return;
        }
        self.files_checked = now;
        for document in &mut self.editor.documents {
            let Some(print) = self.folder.fingerprint(&document.path) else { continue };
            let known = self.prints.get(&document.path).copied();
            if known.is_some_and(|k| k != print) {
                if document.is_dirty() {
                    self.terminal.system(format!("{} changed on disk; your unsaved edits are kept (save to overwrite).", document.path));
                } else if let Some(text) = self.folder.read(&document.path) {
                    document.reload_from_disk(text);
                    self.terminal.system(format!("Reloaded {} (changed on disk)", document.path));
                }
            }
            self.prints.insert(document.path.clone(), print);
        }
    }

    fn forward_errors(&mut self, log: &[araria_runtime::LogEntry]) {
        for entry in log {
            if entry.level == araria_runtime::LogLevel::Error {
                if let Some((path, line)) = &entry.location {
                    self.editor.report_runtime_error(path, *line, &entry.text);
                } else if let Some((path, line)) = terminal::locations(&entry.text).into_iter().next() {
                    self.editor.report_runtime_error(&path, line, &entry.text);
                }
            }
        }
    }

    // ------------------------------------------------------- play mode

    pub fn play(&mut self, ui: &Ui, runtime: &mut Runtime, out: &mut StudioOutput) {
        if runtime.mode() != Mode::Edit {
            return;
        }
        // Scripts run from their files: save edits first.
        let unsaved: Vec<usize> = (0..self.editor.documents.len()).filter(|i| self.editor.documents[*i].is_dirty()).collect();
        for index in unsaved {
            self.save_document(index, runtime);
        }
        self.history.checkpoint(&runtime.dm.borrow());
        self.dirty_before_play = self.scene_dirty(runtime);
        self.viewport.cancel();
        runtime.start();
        out.started_playing = true;
        self.flash(ui, "Playing — edits made now are undone when you stop.");
    }

    pub fn stop(&mut self, ui: &Ui, runtime: &mut Runtime, keep_changes: bool) {
        if runtime.mode() == Mode::Edit {
            return;
        }
        self.viewport.cancel();
        runtime.stop(keep_changes);
        let dm = runtime.dm.borrow();
        self.selection.retain(|id| dm.contains(*id));
        if keep_changes {
            self.history.label("Keep play changes");
            self.history.checkpoint(&dm);
            // The kept state is an unsaved edit.
            self.saved_revision = self.saved_revision.min(dm.revision().saturating_sub(1));
            if self.saved_revision == dm.revision() {
                self.saved_revision = u64::MAX;
            }
        } else {
            self.history.rebase(&dm);
            if !self.dirty_before_play {
                self.saved_revision = dm.revision();
            }
        }
        drop(dm);
        self.flash(ui, if keep_changes { "Stopped — kept the game's changes (save to make them permanent)." } else { "Stopped — the scene is back to how it was." });
    }

    // ---------------------------------------------------- scene editing

    fn select(&mut self, id: InstanceId, extend: bool, dm: &DataModel) {
        if extend {
            if let Some(at) = self.selection.iter().position(|s| *s == id) {
                self.selection.remove(at);
            } else {
                self.selection.push(id);
            }
        } else {
            self.selection = vec![id];
        }
        self.explorer.reveal(dm, id);
    }

    /// Deletable, movable selection (no services).
    fn editable_selection(&self, dm: &DataModel) -> Vec<InstanceId> {
        let mut ids: Vec<InstanceId> = self.selection.iter().copied().filter(|id| dm.get(*id).is_some_and(|i| i.parent.is_some() && !i.info().def.service)).collect();
        // Drop objects whose ancestor is also selected.
        let all = ids.clone();
        ids.retain(|id| !all.iter().any(|other| other != id && dm.is_descendant_of(*id, *other)));
        ids
    }

    fn insert(&mut self, class: &'static str, parent: InstanceId, runtime: &mut Runtime) -> Option<InstanceId> {
        let size = Vec3::new(4.0, 1.0, 2.0);
        let position = viewport::placement(runtime, size);
        let script_path = match class {
            "Script" => self.new_script("Script", false),
            "ModuleScript" => self.new_script("Module", true),
            _ => None,
        };
        let mut dm = runtime.dm.borrow_mut();
        // Scripts go to the Scripts service unless a specific object was chosen.
        let parent = if dm.contains(parent) { parent } else { dm.workspace() };
        let created = match dm.create(class, Some(parent)) {
            Ok(id) => id,
            Err(e) => {
                drop(dm);
                self.terminal.error(format!("Cannot insert {class}: {e}"));
                return None;
            }
        };
        let is_part = dm.get(created).is_some_and(|i| i.is_a("BasePart"));
        if is_part {
            let _ = dm.set(created, "Position", position);
            if class == "Part" {
                let _ = dm.set(created, "Size", size);
            }
        }
        if let Some(path) = &script_path {
            let _ = dm.set(created, "Source", path.as_str());
            let name = path.trim_start_matches("scripts/").trim_end_matches(".lua").to_string();
            let _ = dm.set_name(created, &name);
        }
        self.selection = vec![created];
        self.explorer.reveal(&dm, created);
        drop(dm);
        self.history.label(format!("Insert {class}"));
        if let Some(path) = script_path {
            self.open_file(&path, 4);
        }
        Some(created)
    }

    fn insert_model(&mut self, asset: &str, at: Option<Vec3>, runtime: &mut Runtime) {
        let Some(model) = self.assets.models.get(asset) else { return };
        let size = model.size();
        let name = model.name.clone();
        let center = model.center();
        let position = at.map_or_else(|| viewport::placement(runtime, size), |p| p + Vec3::Y * (size.y * 0.5 - center.y).max(0.0));
        let mut dm = runtime.dm.borrow_mut();
        let workspace = dm.workspace();
        let Ok(id) = dm.create("MeshPart", Some(workspace)) else { return };
        let _ = dm.set(id, "Asset", asset);
        let _ = dm.set_name(id, &name);
        let _ = dm.set(id, "Position", position);
        self.selection = vec![id];
        self.explorer.reveal(&dm, id);
        drop(dm);
        self.history.label(format!("Insert {name}"));
    }

    fn delete_selection(&mut self, runtime: &mut Runtime) {
        let mut dm = runtime.dm.borrow_mut();
        let targets = self.editable_selection(&dm);
        for id in &targets {
            if let Err(e) = dm.destroy(*id) {
                self.terminal.error(format!("Cannot delete: {e}"));
            }
        }
        self.selection.retain(|id| dm.contains(*id));
        drop(dm);
        if !targets.is_empty() {
            self.history.label(format!("Delete {} object{}", targets.len(), if targets.len() == 1 { "" } else { "s" }));
        }
    }

    fn duplicate_selection(&mut self, runtime: &mut Runtime) {
        let mut dm = runtime.dm.borrow_mut();
        let targets = self.editable_selection(&dm);
        let mut copies = Vec::new();
        for id in targets {
            let parent = dm.get(id).and_then(|i| i.parent);
            let width = dm.bounds(id).map_or(0.0, |(lo, hi)| hi.x - lo.x);
            match dm.clone_tree(id, parent) {
                Ok(copy) => {
                    // Offset the copy so it is visible next to the original.
                    let mut moved = vec![copy];
                    moved.extend(dm.descendants(copy));
                    for part in moved {
                        if dm.get(part).is_some_and(|i| i.is_a("BasePart")) {
                            let position = dm.get(part).map_or(Vec3::ZERO, |i| i.vec3("Position"));
                            let _ = dm.set(part, "Position", position + Vec3::X * (width + 1.0).round());
                        }
                    }
                    copies.push(copy);
                }
                Err(e) => self.terminal.error(format!("Cannot duplicate: {e}")),
            }
        }
        if !copies.is_empty() {
            self.selection = copies;
            self.history.label("Duplicate");
        }
    }

    fn group_selection(&mut self, runtime: &mut Runtime) {
        let mut dm = runtime.dm.borrow_mut();
        let targets = self.editable_selection(&dm);
        let Some(first) = targets.first() else { return };
        let parent = dm.get(*first).and_then(|i| i.parent).unwrap_or(dm.workspace());
        let Ok(model) = dm.create("Model", Some(parent)) else { return };
        for id in &targets {
            let _ = dm.set_parent(*id, Some(model));
        }
        self.selection = vec![model];
        self.explorer.reveal(&dm, model);
        drop(dm);
        self.history.label("Group");
    }

    fn ungroup_selection(&mut self, runtime: &mut Runtime) {
        let mut dm = runtime.dm.borrow_mut();
        let mut released = Vec::new();
        for id in self.editable_selection(&dm) {
            if !dm.get(id).is_some_and(|i| i.is_a("Model") || i.is_a("Folder")) {
                continue;
            }
            let parent = dm.get(id).and_then(|i| i.parent);
            for child in dm.children(id).to_vec() {
                let _ = dm.set_parent(child, parent);
                released.push(child);
            }
            let _ = dm.destroy(id);
        }
        if !released.is_empty() {
            self.selection = released;
            drop(dm);
            self.history.label("Ungroup");
        }
    }

    fn copy_selection(&mut self, dm: &DataModel) {
        self.clipboard = self.editable_selection(dm).into_iter().map(|id| save_subtree(dm, id, false)).collect();
        if !self.clipboard.is_empty() {
            self.terminal.system(format!("Copied {} object{}", self.clipboard.len(), if self.clipboard.len() == 1 { "" } else { "s" }));
        }
    }

    fn paste(&mut self, parent: Option<InstanceId>, runtime: &mut Runtime) {
        if self.clipboard.is_empty() {
            return;
        }
        let mut dm = runtime.dm.borrow_mut();
        let parent = parent.filter(|p| dm.contains(*p)).unwrap_or(dm.workspace());
        let mut pasted = Vec::new();
        for node in &self.clipboard {
            match load_subtree(&mut dm, node, parent) {
                Ok(id) => pasted.push(id),
                Err(e) => self.terminal.error(format!("Cannot paste: {e}")),
            }
        }
        if !pasted.is_empty() {
            self.selection = pasted;
            drop(dm);
            self.history.label("Paste");
        }
    }

    fn focus_on(&self, dm: &DataModel, ids: &[InstanceId]) -> Option<(Vec3, f32)> {
        let mut bounds: Option<(Vec3, Vec3)> = None;
        for id in ids {
            let found = dm.bounds(*id).or_else(|| dm.world_position(*id).map(|p| (p - Vec3::ONE, p + Vec3::ONE)));
            if let Some((lo, hi)) = found {
                bounds = Some(bounds.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
            }
        }
        bounds.map(|(lo, hi)| ((lo + hi) * 0.5, ((hi - lo).length() * 0.5).max(1.0)))
    }

    fn undo(&mut self, ui: &Ui, runtime: &mut Runtime) {
        if runtime.mode() != Mode::Edit {
            self.flash(ui, "Undo is available while editing (stop the game first).");
            return;
        }
        let target = self.history.undo(&runtime.dm.borrow());
        if let Some((label, scene)) = target {
            runtime.restore_scene(&scene);
            let dm = runtime.dm.borrow();
            self.history.rebase(&dm);
            self.selection.retain(|id| dm.contains(*id));
            drop(dm);
            self.flash(ui, format!("Undid: {label}"));
        }
    }

    fn redo(&mut self, ui: &Ui, runtime: &mut Runtime) {
        if runtime.mode() != Mode::Edit {
            return;
        }
        let target = self.history.redo(&runtime.dm.borrow());
        if let Some((label, scene)) = target {
            runtime.restore_scene(&scene);
            let dm = runtime.dm.borrow();
            self.history.rebase(&dm);
            self.selection.retain(|id| dm.contains(*id));
            drop(dm);
            self.flash(ui, format!("Redid: {label}"));
        }
    }

    /// Opens the script of a Script/ModuleScript instance, creating a file
    /// for it if it has none.
    fn open_script_of(&mut self, id: InstanceId, runtime: &mut Runtime) {
        let (name, source) = {
            let dm = runtime.dm.borrow();
            let Some(instance) = dm.get(id) else { return };
            (instance.name.clone(), instance.str("Source").to_string())
        };
        let path = if source.is_empty() {
            let module = runtime.dm.borrow().get(id).is_some_and(|i| i.class_name() == "ModuleScript");
            let Some(path) = self.new_script(&name, module) else { return };
            let _ = runtime.dm.borrow_mut().set(id, "Source", path.as_str());
            self.history.label("Create script file");
            path
        } else {
            source
        };
        if !self.folder.exists(&path) {
            let _ = self.folder.write(&path, SCRIPT_TEMPLATE);
            self.refresh_files();
            self.terminal.system(format!("Created {path}"));
        }
        self.open_file(&path, 1);
    }

    // ------------------------------------------------------------ frame

    /// Draws the studio into `ui` (the whole window area it may use).
    #[allow(clippy::too_many_lines)]
    pub fn show(&mut self, ui: &mut Ui, runtime: &mut Runtime) -> StudioOutput {
        let mut out = StudioOutput::default();
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let log = runtime.scripts.take_log();
        self.forward_errors(&log);
        self.terminal.absorb(log);
        self.watch_files(now);
        {
            let dm = runtime.dm.borrow();
            self.selection.retain(|id| dm.contains(*id));
        }

        // ---- shortcuts that work everywhere
        // Shift+F5 first: plain F5 would also match it.
        let (save, stop, play) = ui.input_mut(|i| (i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) || i.consume_key(Modifiers::COMMAND, Key::S), i.consume_key(Modifiers::SHIFT, Key::F5), i.consume_key(Modifiers::NONE, Key::F5)));
        if save {
            self.save_all(ui, runtime);
        }
        if stop {
            self.stop(ui, runtime, false);
        } else if play {
            if runtime.mode() == Mode::Edit {
                self.play(ui, runtime, &mut out);
            } else {
                let paused = runtime.mode() == Mode::Paused;
                runtime.set_paused(!paused);
            }
        }
        // ---- scene shortcuts (not while typing)
        if !is_typing(&ctx) {
            let command = Modifiers::COMMAND;
            let keys = ui.input_mut(|i| {
                [
                    i.consume_key(command | Modifiers::SHIFT, Key::Z) || i.consume_key(command, Key::Y),
                    i.consume_key(command, Key::Z),
                    i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace),
                    i.consume_key(command, Key::D),
                    i.consume_key(command | Modifiers::SHIFT, Key::G),
                    i.consume_key(command, Key::G),
                    i.consume_key(command, Key::C),
                    i.consume_key(command, Key::V),
                    i.consume_key(Modifiers::NONE, Key::F),
                    i.consume_key(command, Key::Num1),
                    i.consume_key(command, Key::Num2),
                    i.consume_key(command, Key::Num3),
                    i.consume_key(command, Key::Num4),
                    i.consume_key(command, Key::L),
                ]
            });
            let [redo, undo, delete, duplicate, ungroup, group, copy, paste, focus, select_tool, move_tool, rotate_tool, scale_tool, snap] = keys;
            if redo {
                self.redo(ui, runtime);
            } else if undo {
                self.undo(ui, runtime);
            }
            if delete {
                self.delete_selection(runtime);
            }
            if duplicate {
                self.duplicate_selection(runtime);
            }
            if ungroup {
                self.ungroup_selection(runtime);
            } else if group {
                self.group_selection(runtime);
            }
            if copy {
                self.copy_selection(&runtime.dm.borrow());
            }
            if paste {
                let parent = self.selection.first().and_then(|id| runtime.dm.borrow().get(*id).and_then(|i| i.parent));
                self.paste(parent, runtime);
            }
            if focus {
                out.focus = self.focus_on(&runtime.dm.borrow(), &self.selection);
            }
            for (pressed, tool) in [(select_tool, Tool::Select), (move_tool, Tool::Move), (rotate_tool, Tool::Rotate), (scale_tool, Tool::Scale)] {
                if pressed {
                    self.viewport.tool = tool;
                }
            }
            if snap {
                self.viewport.snap.enabled = !self.viewport.snap.enabled;
            }
        }

        // ---- layout
        egui::Panel::top("studio-top").exact_size(52.0).frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(10, 8)).stroke(Stroke::new(1.0, theme::BORDER))).show(ui, |ui| {
            self.top_bar(ui, runtime, &mut out);
        });
        let panel_frame = egui::Frame::new().fill(theme::SIDEBAR).inner_margin(egui::Margin::same(8)).stroke(Stroke::new(1.0, theme::BORDER));
        // Panels start smaller in small windows so the 3D view keeps room.
        let area = ui.available_rect_before_wrap();
        let side = |fraction: f32, preferred: f32| (area.width() * fraction).clamp(150.0, preferred);
        egui::Panel::bottom("studio-terminal").resizable(true).default_size((area.height() * 0.24).clamp(100.0, 210.0)).size_range(90.0..=(area.height() * 0.6).max(120.0)).frame(panel_frame).show(ui, |ui| {
            let selection = self.selection.last().copied();
            let requests = self.terminal.show(ui, runtime, selection);
            self.handle_requests(ui, runtime, requests, &mut out);
        });
        egui::Panel::left("studio-left").resizable(true).default_size(side(0.2, 260.0)).size_range(150.0..=(area.width() * 0.35).max(160.0)).frame(panel_frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.left, LeftTab::Explorer, RichText::new(format!("{} Explorer", icons::TREE_STRUCTURE)).font(theme::medium(12.5)));
                ui.selectable_value(&mut self.left, LeftTab::Assets, RichText::new(format!("{} Assets", icons::PACKAGE)).font(theme::medium(12.5)));
            });
            ui.separator();
            match self.left {
                LeftTab::Explorer => {
                    let actions = {
                        let dm = runtime.dm.borrow();
                        self.explorer.show(ui, &dm, &self.selection, true)
                    };
                    self.handle_explorer(actions, runtime, &mut out);
                }
                LeftTab::Assets => {
                    let files = self.files.clone();
                    let actions = self.browser.show(ui, &self.assets, &files);
                    self.handle_assets(ui, actions, runtime);
                }
            }
        });
        let mut inspector_editing = false;
        egui::Panel::right("studio-right").resizable(true).default_size(side(0.22, 300.0)).size_range(170.0..=(area.width() * 0.4).max(180.0)).frame(panel_frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.right, RightTab::Properties, RichText::new(format!("{} Properties", icons::SLIDERS)).font(theme::medium(12.5)));
                ui.selectable_value(&mut self.right, RightTab::Reference, RichText::new(format!("{} API", icons::BOOKS)).font(theme::medium(12.5)));
            });
            ui.separator();
            match self.right {
                RightTab::Properties => {
                    let scripts: Vec<String> = self.files.iter().filter(|f| f.ends_with(".lua")).cloned().collect();
                    let result = {
                        let mut dm = runtime.dm.borrow_mut();
                        self.inspector.show(ui, &mut dm, &self.selection, &self.assets, &scripts)
                    };
                    inspector_editing = result.editing;
                    if let Some(path) = result.open_script {
                        self.open_file(&path, 1);
                    }
                    if let Some(id) = result.select {
                        let dm = runtime.dm.borrow();
                        self.select(id, false, &dm);
                    }
                    if result.new_script {
                        if let Some(id) = self.selection.last().copied() {
                            self.open_script_of(id, runtime);
                        }
                    }
                }
                RightTab::Reference => self.reference.show(ui),
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            let (scene_rect, code_rect) = match self.center {
                CenterView::Scene => (Some(rect), None),
                CenterView::Code => (None, Some(rect)),
                CenterView::Split => {
                    let split = rect.left() + rect.width() * self.split;
                    (Some(Rect::from_min_max(rect.min, egui::pos2(split - 3.0, rect.bottom()))), Some(Rect::from_min_max(egui::pos2(split + 3.0, rect.top()), rect.max)))
                }
            };
            if self.center == CenterView::Split {
                let handle = Rect::from_center_size(egui::pos2(rect.left() + rect.width() * self.split, rect.center().y), vec2(6.0, rect.height()));
                let response = ui.interact(handle, egui::Id::new("studio-split"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if response.dragged() {
                    self.split = ((self.split * rect.width() + response.drag_delta().x) / rect.width()).clamp(0.2, 0.8);
                }
                ui.painter().rect_filled(handle, CornerRadius::ZERO, if response.hovered() || response.dragged() { theme::ACCENT } else { theme::BORDER });
            }
            if let Some(scene_rect) = scene_rect {
                out.viewport = Some(scene_rect);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(scene_rect));
                self.scene_view(&mut child, scene_rect, runtime, &mut out);
            }
            if let Some(code_rect) = code_rect {
                ui.painter().rect_filled(code_rect, CornerRadius::ZERO, theme::BG);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(code_rect.shrink(6.0)));
                let actions = self.editor.show(&mut child);
                self.handle_editor(&child, actions, runtime);
            }
        });

        // ---- undo checkpoints: after an edit has finished
        if runtime.mode() == Mode::Edit {
            let busy = inspector_editing || self.viewport.dragging() || ctx.input(|i| i.pointer.any_down());
            if !busy {
                self.history.checkpoint(&runtime.dm.borrow());
            }
        }

        self.dialogs(&ctx, runtime, &mut out);
        out.capture_keyboard = is_typing(&ctx);
        out
    }

    #[allow(clippy::too_many_lines)]
    fn top_bar(&mut self, ui: &mut Ui, runtime: &mut Runtime, out: &mut StudioOutput) {
        let mode = runtime.mode();
        // Narrow windows get icon-only controls.
        let compact = ui.available_width() < 1180.0;
        let tiny = ui.available_width() < 900.0;
        let square = vec2(32.0, 32.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if ui.add(egui::Button::new(RichText::new(icons::ARROW_LEFT).font(theme::regular(16.0))).min_size(square)).on_hover_text("Leave the studio").clicked() {
                if self.has_unsaved_changes(runtime) {
                    self.confirm = Some(Confirm::Leave);
                } else {
                    out.leave = true;
                }
            }
            let dirty = self.has_unsaved_changes(runtime);
            if tiny {
                ui.label(RichText::new(if dirty { "●" } else { "" }).color(theme::WARNING)).on_hover_text(format!("{}{}", self.name, if dirty { " — unsaved changes" } else { "" }));
            } else {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let name = araria_platform::widgets::elide(ui, &self.name, theme::bold(14.0), if compact { 110.0 } else { 200.0 });
                    ui.label(RichText::new(name).font(theme::bold(14.0)));
                    ui.label(RichText::new(if dirty { "● unsaved changes" } else { "all changes saved" }).font(theme::regular(10.5)).color(if dirty { theme::WARNING } else { theme::TEXT_FAINT }));
                });
            }
            ui.separator();
            // Tools.
            for (tool, icon, label, key) in [(Tool::Select, icons::CURSOR, "Select", "Ctrl+1"), (Tool::Move, icons::ARROWS_OUT_CARDINAL, "Move", "Ctrl+2"), (Tool::Rotate, icons::ARROW_CLOCKWISE, "Rotate", "Ctrl+3"), (Tool::Scale, icons::ARROWS_OUT, "Scale", "Ctrl+4")] {
                let selected = self.viewport.tool == tool;
                if ui.add(egui::Button::new(RichText::new(icon).font(theme::regular(16.0))).selected(selected).min_size(square)).on_hover_text(format!("{label} ({key})")).clicked() {
                    self.viewport.tool = tool;
                }
            }
            let snap_label = if compact { RichText::new(icons::SQUARES_FOUR).font(theme::regular(14.0)) } else { RichText::new("Snap").font(theme::regular(12.0)) };
            ui.checkbox(&mut self.viewport.snap.enabled, snap_label).on_hover_text("Snap to the grid (Ctrl+L)");
            if !tiny {
                ui.add_enabled_ui(self.viewport.snap.enabled, |ui| {
                    ui.add(egui::DragValue::new(&mut self.viewport.snap.step).range(0.05..=50.0).speed(0.05).suffix(" st").max_decimals(2)).on_hover_text("Move and resize step in studs");
                    ui.add(egui::DragValue::new(&mut self.viewport.snap.angle).range(1.0..=90.0).speed(0.5).suffix("°").max_decimals(0)).on_hover_text("Rotation step");
                });
            }
            ui.separator();
            let parent = self.selection.last().copied().unwrap_or(runtime.dm.borrow().workspace());
            let mut insert: Option<&'static str> = None;
            let insert_label = if compact { RichText::new(icons::PLUS).font(theme::regular(15.0)) } else { RichText::new(format!("{} Insert", icons::PLUS)).font(theme::medium(12.5)) };
            ui.menu_button(insert_label, |ui| {
                for class in explorer::COMMON_CLASSES {
                    if ui.button(format!("{}  {class}", explorer::class_icon(class))).clicked() {
                        insert = Some(class);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Insert an object");
            if let Some(class) = insert {
                // Parts go into the world; attachments (lights, sounds) into the selection.
                let dm = runtime.dm.borrow();
                let attachable = matches!(class, "Light" | "Sound" | "ParticleEmitter" | "Prompt" | "BillboardLabel");
                let target = if attachable || dm.get(parent).is_some_and(|i| i.is_a("Model") || i.is_a("Folder")) { parent } else if matches!(class, "Script" | "ModuleScript") { dm.service("Scripts") } else { dm.workspace() };
                drop(dm);
                self.insert(class, target, runtime);
            }
            let undo = self.history.undo_label().map(str::to_string);
            let redo = self.history.redo_label().map(str::to_string);
            if ui.add_enabled(mode == Mode::Edit && undo.is_some(), egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).font(theme::regular(15.0))).min_size(square)).on_hover_text(format!("Undo {} (Ctrl+Z)", undo.unwrap_or_default())).clicked() {
                self.undo(ui, runtime);
            }
            if ui.add_enabled(mode == Mode::Edit && redo.is_some(), egui::Button::new(RichText::new(icons::ARROW_CLOCKWISE).font(theme::regular(15.0))).min_size(square)).on_hover_text(format!("Redo {} (Ctrl+Y)", redo.unwrap_or_default())).clicked() {
                self.redo(ui, runtime);
            }
            ui.separator();
            // Play controls.
            let text = |icon: &str, label: &str| if compact { RichText::new(icon).font(theme::regular(15.0)) } else { RichText::new(format!("{icon}  {label}")).font(theme::semibold(13.0)) };
            match mode {
                Mode::Edit => {
                    let play = egui::Button::new(text(icons::PLAY, "Play").color(egui::Color32::WHITE)).fill(theme::SUCCESS.gamma_multiply(0.75)).min_size(square);
                    if ui.add(play).on_hover_text("Run the game here (F5)").clicked() {
                        self.play(ui, runtime, out);
                    }
                }
                Mode::Play | Mode::Paused => {
                    let paused = mode == Mode::Paused;
                    let (icon, label) = if paused { (icons::PLAY, "Resume") } else { (icons::PAUSE, "Pause") };
                    if ui.add(egui::Button::new(text(icon, label)).min_size(square)).on_hover_text(format!("{label} (F5)")).clicked() {
                        runtime.set_paused(!paused);
                    }
                    if ui.add(egui::Button::new(text(icons::STOP, "Stop").color(egui::Color32::WHITE)).fill(theme::DANGER.gamma_multiply(0.7)).min_size(square)).on_hover_text("Stop and restore the scene (Shift+F5)").clicked() {
                        self.stop(ui, runtime, false);
                    }
                    ui.menu_button(RichText::new(icons::CARET_DOWN).font(theme::regular(13.0)), |ui| {
                        if ui.button(format!("{}  Stop and keep changes", icons::FLOPPY_DISK)).on_hover_text("Leave play mode with the world as the game left it").clicked() {
                            self.stop(ui, runtime, true);
                            ui.close();
                        }
                    });
                    if ui.add(egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).font(theme::regular(15.0))).min_size(square)).on_hover_text("Reset: restart the play session from the edited scene").clicked() {
                        self.stop(ui, runtime, false);
                        self.play(ui, runtime, out);
                    }
                    if !compact {
                        ui.label(RichText::new(format!("{} {:.0}s", if paused { "paused" } else { "playing" }, runtime.time())).font(theme::mono(12.0)).color(if paused { theme::WARNING } else { theme::SUCCESS }));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(text(icons::FLOPPY_DISK, "Save")).min_size(square)).on_hover_text("Save the scene and all scripts (Ctrl+S)").clicked() {
                    self.save_all(ui, runtime);
                }
                ui.separator();
                for (view, icon, label) in [(CenterView::Code, icons::CODE, "Code"), (CenterView::Split, icons::SQUARES_FOUR, "Split"), (CenterView::Scene, icons::CUBE, "Scene")] {
                    if ui.add(egui::Button::new(RichText::new(icon).font(theme::regular(15.0))).selected(self.center == view).min_size(square)).on_hover_text(format!("{label} view")).clicked() {
                        self.center = view;
                    }
                }
                let now = ui.input(|i| i.time);
                if let Some((text, until)) = &self.status {
                    let room = ui.available_width() - 10.0;
                    if now < *until && room > 80.0 {
                        let shown = araria_platform::widgets::elide(ui, text, theme::regular(12.0), room);
                        ui.label(RichText::new(shown).font(theme::regular(12.0)).color(theme::ACCENT_2));
                    }
                }
            });
        });
    }

    fn scene_view(&mut self, ui: &mut Ui, rect: Rect, runtime: &mut Runtime, out: &mut StudioOutput) {
        let selection = self.selection.clone();
        let result = self.viewport.show(ui, rect, runtime, &selection);
        // Models dragged in from the asset browser.
        let response = ui.interact(rect, egui::Id::new("studio-viewport-drop"), egui::Sense::hover());
        if let Some(model) = response.dnd_release_payload::<DraggedModel>() {
            let ray = runtime.input.borrow().mouse_ray;
            let at = ray.and_then(|(origin, direction)| runtime.physics.borrow().pick(origin, direction, 2000.0, &[]).map(|hit| hit.position));
            self.insert_model(&model.0, at, runtime);
        }
        if let Some(clicked) = result.clicked {
            match clicked {
                Some(id) => {
                    let dm = runtime.dm.borrow();
                    self.select(id, result.shift, &dm);
                }
                None if !result.shift => self.selection.clear(),
                None => {}
            }
        }
        if let Some(label) = result.finished_edit {
            self.history.label(label);
        }
        let pointer_inside = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p)) && !viewport::pointer_over_popup(ui.ctx());
        out.capture_pointer = !pointer_inside || result.busy;
        // Hints.
        let painter = ui.painter_at(rect);
        let hint = match runtime.mode() {
            Mode::Edit => "Right-drag to look · WASD/QE to fly · Shift faster · click to select · Alt+click picks inside models",
            _ => "Playing in the studio · F5 pause · Shift+F5 stop",
        };
        let galley = painter.layout_no_wrap(hint.to_string(), theme::regular(11.5), theme::TEXT_DIM);
        let pill = Rect::from_min_size(rect.left_bottom() + vec2(10.0, -galley.size().y - 18.0), galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(pill, CornerRadius::same(8), theme::with_alpha(theme::BG, 0.7));
        painter.galley(pill.min + vec2(8.0, 4.0), galley, theme::TEXT_DIM);
        if runtime.mode() != Mode::Edit {
            painter.rect_stroke(rect.shrink(1.0), CornerRadius::ZERO, Stroke::new(2.0, if runtime.mode() == Mode::Paused { theme::WARNING } else { theme::SUCCESS }), egui::StrokeKind::Inside);
        }
        if let Some(tool) = [(Tool::Select, "Select"), (Tool::Move, "Move"), (Tool::Rotate, "Rotate"), (Tool::Scale, "Scale")].iter().find(|(t, _)| *t == self.viewport.tool) {
            painter.text(rect.right_top() + vec2(-12.0, 14.0), Align2::RIGHT_CENTER, format!("{} tool{}", tool.1, if self.viewport.snap.enabled { format!(" · snap {}", self.viewport.snap.step) } else { String::new() }), theme::regular(11.5), theme::TEXT_DIM);
        }
    }

    fn handle_requests(&mut self, ui: &Ui, runtime: &mut Runtime, requests: Vec<Request>, out: &mut StudioOutput) {
        for request in requests {
            match request {
                Request::Play => self.play(ui, runtime, out),
                Request::Pause => runtime.set_paused(true),
                Request::Resume => runtime.set_paused(false),
                Request::Stop => self.stop(ui, runtime, false),
                Request::Select(id) => {
                    let dm = runtime.dm.borrow();
                    self.select(id, false, &dm);
                }
                Request::Open(path, line) => self.open_file(&path, line),
                Request::Save => self.save_all(ui, runtime),
            }
        }
    }

    fn handle_explorer(&mut self, actions: Vec<ExplorerAction>, runtime: &mut Runtime, out: &mut StudioOutput) {
        for action in actions {
            match action {
                ExplorerAction::Select { id, extend } => {
                    let dm = runtime.dm.borrow();
                    self.select(id, extend, &dm);
                }
                ExplorerAction::Rename(id, name) => {
                    if let Err(e) = runtime.dm.borrow_mut().set_name(id, &name) {
                        self.terminal.error(format!("Cannot rename: {e}"));
                    }
                    self.history.label("Rename");
                }
                ExplorerAction::Reparent { id, parent } => {
                    if let Err(e) = runtime.dm.borrow_mut().set_parent(id, Some(parent)) {
                        self.terminal.error(format!("Cannot move it there: {e}"));
                    }
                    self.history.label("Reparent");
                }
                ExplorerAction::Insert { class, parent } => {
                    self.insert(class, parent, runtime);
                }
                ExplorerAction::Delete => self.delete_selection(runtime),
                ExplorerAction::Duplicate => self.duplicate_selection(runtime),
                ExplorerAction::Group => self.group_selection(runtime),
                ExplorerAction::Ungroup => self.ungroup_selection(runtime),
                ExplorerAction::OpenScript(id) => self.open_script_of(id, runtime),
                ExplorerAction::Focus(id) => out.focus = self.focus_on(&runtime.dm.borrow(), &[id]),
                ExplorerAction::Copy => self.copy_selection(&runtime.dm.borrow()),
                ExplorerAction::Paste(parent) => self.paste(Some(parent), runtime),
            }
        }
    }

    fn handle_assets(&mut self, ui: &Ui, actions: Vec<AssetAction>, runtime: &mut Runtime) {
        for action in actions {
            match action {
                AssetAction::InsertModel(id) => self.insert_model(&id, None, runtime),
                AssetAction::ApplyAnimation { model, clip } => {
                    let mut dm = runtime.dm.borrow_mut();
                    let target = self.selection.last().copied().filter(|id| dm.get(*id).is_some_and(|i| i.is_a("MeshPart")));
                    match target {
                        Some(id) => {
                            if dm.get(id).is_some_and(|i| i.str("Asset") != model) {
                                let _ = dm.set(id, "Asset", model.as_str());
                            }
                            let _ = dm.set(id, "Animation", clip.as_str());
                            drop(dm);
                            self.history.label("Set animation");
                        }
                        None => {
                            drop(dm);
                            self.insert_model(&model, None, runtime);
                            if let Some(id) = self.selection.last().copied() {
                                let _ = runtime.dm.borrow_mut().set(id, "Animation", clip.as_str());
                            }
                        }
                    }
                }
                AssetAction::PreviewSound(asset) => runtime.queue_command(Command::PlaySound { asset, volume: 0.8, pitch: 1.0, position: None }),
                AssetAction::InsertSound(asset) => {
                    let parent = self.selection.last().copied().unwrap_or(runtime.dm.borrow().workspace());
                    if let Some(id) = self.insert("Sound", parent, runtime) {
                        let mut dm = runtime.dm.borrow_mut();
                        let _ = dm.set(id, "Asset", asset.as_str());
                        let name = self.assets.audio.get(&asset).map_or("Sound".to_string(), |a| a.name.clone());
                        let _ = dm.set_name(id, &name);
                    }
                }
                AssetAction::UseTexture(asset) => {
                    let mut dm = runtime.dm.borrow_mut();
                    let target = self.selection.last().copied().filter(|id| dm.get(*id).is_some_and(|i| i.class_name() == "ImageLabel"));
                    let id = match target {
                        Some(id) => id,
                        None => {
                            let gui = dm.service("Gui");
                            match dm.create("ImageLabel", Some(gui)) {
                                Ok(id) => id,
                                Err(e) => {
                                    drop(dm);
                                    self.terminal.error(format!("Cannot add an image: {e}"));
                                    continue;
                                }
                            }
                        }
                    };
                    let _ = dm.set(id, "Image", asset.as_str());
                    self.selection = vec![id];
                    drop(dm);
                    self.history.label("Set image");
                }
                AssetAction::OpenFile(path) => self.open_file(&path, 1),
                AssetAction::NewScript => {
                    if let Some(path) = self.new_script("Script", false) {
                        // A Script object runs the new file.
                        let mut dm = runtime.dm.borrow_mut();
                        let holder = dm.service("Scripts");
                        if let Ok(id) = dm.create("Script", Some(holder)) {
                            let _ = dm.set(id, "Source", path.as_str());
                            let _ = dm.set_name(id, path.trim_start_matches("scripts/").trim_end_matches(".lua"));
                            self.selection = vec![id];
                        }
                        drop(dm);
                        self.history.label("New script");
                        self.open_file(&path, 4);
                    }
                }
                AssetAction::DeleteFile(path) => self.confirm = Some(Confirm::DeleteFile(path)),
                AssetAction::RenameFile(from, to) => {
                    let to = if to.contains('/') { to } else { format!("scripts/{to}") };
                    match self.folder.rename(&from, &to) {
                        Ok(()) => {
                            self.editor.renamed(&from, &to);
                            // Scripts that ran the old file follow it.
                            let mut dm = runtime.dm.borrow_mut();
                            let users: Vec<InstanceId> = dm.instances().filter(|i| matches!(i.class_name(), "Script" | "ModuleScript") && i.str("Source") == from).map(|i| i.id).collect();
                            for id in users {
                                let _ = dm.set(id, "Source", to.as_str());
                            }
                            drop(dm);
                            self.refresh_files();
                            self.flash(ui, format!("Renamed {from} to {to}"));
                        }
                        Err(e) => self.terminal.error(e),
                    }
                }
            }
        }
    }

    fn handle_editor(&mut self, ui: &Ui, actions: Vec<EditorAction>, runtime: &mut Runtime) {
        for action in actions {
            match action {
                EditorAction::Save(index) => {
                    self.save_document(index, runtime);
                }
                EditorAction::SaveAll => self.save_all(ui, runtime),
                EditorAction::Apply(index) => {
                    if self.save_document(index, runtime) && runtime.mode() == Mode::Edit {
                        self.flash(ui, "Saved. Press Play to run it — while playing, saving reloads scripts instantly.");
                    }
                }
                EditorAction::ConfirmClose(index) => self.confirm = Some(Confirm::CloseTab(index)),
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, runtime: &mut Runtime, out: &mut StudioOutput) {
        let Some(confirm) = &self.confirm else { return };
        let (title, body) = match confirm {
            Confirm::Leave => ("Leave the studio?".to_string(), "You have unsaved changes.".to_string()),
            Confirm::CloseTab(index) => (format!("Close {}?", self.editor.documents.get(*index).map_or("", |d| d.name())), "It has unsaved changes.".to_string()),
            Confirm::DeleteFile(path) => (format!("Delete {path}?"), "The file is removed from the project. Scripts using it will report an error.".to_string()),
        };
        let mut choice: Option<u8> = None;
        let frame = egui::Frame::new().fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(CornerRadius::same(14)).inner_margin(egui::Margin::same(22));
        let modal = egui::Modal::new(egui::Id::new("studio-confirm")).frame(frame).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(RichText::new(title).font(theme::bold(17.0)));
            ui.label(RichText::new(body).font(theme::regular(13.0)).color(theme::TEXT_DIM));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                match confirm {
                    Confirm::DeleteFile(_) => {
                        if ui.button(RichText::new("Delete").color(theme::DANGER)).clicked() {
                            choice = Some(1);
                        }
                    }
                    _ => {
                        if ui.button(RichText::new(format!("{} Save", icons::FLOPPY_DISK))).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(RichText::new("Discard").color(theme::DANGER)).clicked() {
                            choice = Some(2);
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(0);
        }
        let Some(choice) = choice else { return };
        let confirm = self.confirm.take().expect("open dialog");
        if choice == 0 {
            return;
        }
        // The dialog's own Ui is gone; use a throwaway one for status messages.
        let ui = Ui::new(ctx.clone(), egui::Id::new("studio-dialog-ui"), egui::UiBuilder::new());
        match confirm {
            Confirm::Leave => {
                if choice == 1 {
                    if runtime.mode() != Mode::Edit {
                        self.stop(&ui, runtime, false);
                    }
                    self.save_all(&ui, runtime);
                }
                out.leave = true;
            }
            Confirm::CloseTab(index) => {
                if choice == 1 {
                    self.save_document(index, runtime);
                }
                self.editor.close(index);
            }
            Confirm::DeleteFile(path) => {
                self.folder.remove(&path);
                if let Some(index) = self.editor.documents.iter().position(|d| d.path == path) {
                    self.editor.close(index);
                }
                self.refresh_files();
                self.terminal.system(format!("Deleted {path}"));
            }
        }
    }

    /// The editor tab for scripts; exposed for tests and tools.
    pub fn set_center(&mut self, view: CenterView) {
        self.center = view;
    }

    pub fn show_assets(&mut self, tab: AssetTab) {
        self.left = LeftTab::Assets;
        self.browser.tab = tab;
    }
}

#[cfg(test)]
mod tests {
    use araria_core::project::{MANIFEST_FILE, ProjectManifest};
    use araria_runtime::ProjectFiles;
    use araria_storage::Storage;

    use super::*;

    fn studio() -> (Studio, Runtime) {
        let storage = Storage::memory();
        let folder = ProjectFolder::new(storage, "projects/test");
        folder.write(MANIFEST_FILE, &ProjectManifest::new("test", "Test", 1).to_json()).unwrap();
        let mut dm = DataModel::new();
        let ws = dm.workspace();
        let floor = dm.create("Part", Some(ws)).unwrap();
        dm.set(floor, "Size", Vec3::new(50.0, 1.0, 50.0)).unwrap();
        let reader = folder.clone();
        let printer = folder.clone();
        let runtime = Runtime::with_files(dm, Arc::new(AssetManifest::default()), ProjectFiles::new(move |p| reader.read(p), move |p| printer.fingerprint(p)));
        (Studio::new("Test", folder, Arc::new(AssetManifest::default()), &runtime), runtime)
    }

    #[test]
    fn scene_operations_are_undoable() {
        let (mut studio, mut runtime) = studio();
        let floor = runtime.dm.borrow().instances().find(|i| i.class_name() == "Part").unwrap().id;
        studio.selection = vec![floor];
        studio.duplicate_selection(&mut runtime);
        let copy = studio.selection[0];
        assert_ne!(copy, floor);
        assert!(runtime.dm.borrow().get(copy).unwrap().vec3("Position").x > 40.0, "offset beside the original");
        studio.history.checkpoint(&runtime.dm.borrow());
        studio.selection = vec![floor, copy];
        studio.group_selection(&mut runtime);
        let model = studio.selection[0];
        assert_eq!(runtime.dm.borrow().children(model).len(), 2);
        studio.history.checkpoint(&runtime.dm.borrow());
        studio.copy_selection(&runtime.dm.borrow());
        studio.delete_selection(&mut runtime);
        assert!(!runtime.dm.borrow().contains(model));
        studio.history.checkpoint(&runtime.dm.borrow());
        studio.paste(None, &mut runtime);
        assert_eq!(runtime.dm.borrow().children(studio.selection[0]).len(), 2, "pasted with its children");
        studio.history.checkpoint(&runtime.dm.borrow());
        assert_eq!(studio.history.undo_label(), Some("Paste"));
        // Undo the paste and the delete: the original model is back.
        for _ in 0..2 {
            let (_, scene) = studio.history.undo(&runtime.dm.borrow()).unwrap();
            runtime.restore_scene(&scene);
            studio.history.rebase(&runtime.dm.borrow());
        }
        assert!(runtime.dm.borrow().contains(model));
        assert!(studio.scene_dirty(&runtime));
    }

    #[test]
    fn scripts_are_created_saved_and_hot_reloaded() {
        let (mut studio, mut runtime) = studio();
        let scripts = runtime.dm.borrow().service("Scripts");
        let id = studio.insert("Script", scripts, &mut runtime).unwrap();
        let path = runtime.dm.borrow().get(id).unwrap().str("Source").to_string();
        assert_eq!(path, "scripts/Script.lua");
        assert!(studio.folder.read(&path).unwrap().contains("Hello"));
        assert_eq!(studio.editor.documents[0].path, path, "opened in the editor");

        runtime.start();
        runtime.step(1.0 / 60.0);
        let _ = runtime.scripts.take_log();
        studio.editor.documents[0].text = "shared.version = 2".into();
        assert!(studio.save_document(0, &mut runtime));
        assert_eq!(runtime.scripts.eval("shared.version").unwrap(), "2", "saving reloads the running script");
        assert_eq!(studio.folder.read(&path).as_deref(), Some("shared.version = 2"));
        // A syntax error keeps the old version running and marks the line.
        studio.editor.documents[0].text = "shared.version = (3".into();
        studio.save_document(0, &mut runtime);
        assert_eq!(runtime.scripts.eval("shared.version").unwrap(), "2");
        assert!(studio.editor.documents[0].syntax_error.is_some());
        runtime.stop(false);
        assert_eq!(runtime.mode(), Mode::Edit);
    }

    #[test]
    fn the_whole_studio_runs_headless_and_plays_with_f5() {
        use egui::{Event, Key, Modifiers, RawInput, Rect, pos2, vec2};
        let (mut studio, mut runtime) = studio();
        let ctx = egui::Context::default();
        araria_platform::theme::install(&ctx);
        let mut frame = |studio: &mut Studio, runtime: &mut Runtime, events: Vec<Event>| {
            let input = RawInput { events, screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 860.0))), ..Default::default() };
            let mut out = None;
            let mut output = ctx.run_ui(input, |ui| out = Some(studio.show(ui, runtime)));
            output.textures_delta.clear();
            out.expect("studio drew a frame")
        };
        let key = |key: Key, modifiers: Modifiers| vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        let out = frame(&mut studio, &mut runtime, Vec::new());
        let view = out.viewport.expect("the 3D view is visible");
        assert!(view.width() > 400.0 && view.height() > 300.0, "{view:?}");
        frame(&mut studio, &mut runtime, Vec::new());
        let out = frame(&mut studio, &mut runtime, key(Key::F5, Modifiers::NONE));
        assert!(out.started_playing);
        assert_eq!(runtime.mode(), Mode::Play);
        frame(&mut studio, &mut runtime, key(Key::F5, Modifiers::NONE));
        assert_eq!(runtime.mode(), Mode::Paused);
        frame(&mut studio, &mut runtime, key(Key::F5, Modifiers::SHIFT));
        assert_eq!(runtime.mode(), Mode::Edit);
        // The code view hides the 3D view.
        studio.set_center(CenterView::Code);
        assert!(frame(&mut studio, &mut runtime, Vec::new()).viewport.is_none());
    }
}
