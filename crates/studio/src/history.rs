//! Undo and redo for scene edits.
//!
//! The history keeps snapshots of the scene. Rather than every editor
//! operation recording its own inverse, the studio *checkpoints* after each
//! completed edit (a finished drag, a committed property, a terminal
//! command): whatever changed — through the inspector, gizmos, explorer or
//! Lua — becomes one undo step. Restoring applies only the differences, so
//! the renderer and physics update incrementally.

use std::collections::VecDeque;

use araria_core::DataModel;

const LIMIT: usize = 100;

pub struct History {
    /// States to go back to, most recent last, each with a label.
    undo: VecDeque<(String, DataModel)>,
    redo: Vec<(String, DataModel)>,
    /// The scene as of the last checkpoint.
    stable: DataModel,
    /// `DataModel::revision` at the last checkpoint.
    revision: u64,
    /// Label for the next checkpoint.
    pending: Option<String>,
}

impl History {
    pub fn new(scene: &DataModel) -> Self {
        History { undo: VecDeque::new(), redo: Vec::new(), stable: scene.clone(), revision: scene.revision(), pending: None }
    }

    /// Names the edit in progress (shown as "Undo <label>").
    pub fn label(&mut self, label: impl Into<String>) {
        self.pending.get_or_insert_with(|| label.into());
    }

    /// Whether the scene changed since the last checkpoint.
    pub fn has_uncommitted(&self, scene: &DataModel) -> bool {
        scene.revision() != self.revision
    }

    /// Records the scene's changes since the last checkpoint as one undo
    /// step. Returns true if there was anything to record.
    pub fn checkpoint(&mut self, scene: &DataModel) -> bool {
        if !self.has_uncommitted(scene) {
            return false;
        }
        let label = self.pending.take().unwrap_or_else(|| "Edit".to_string());
        let previous = std::mem::replace(&mut self.stable, scene.clone());
        self.undo.push_back((label, previous));
        if self.undo.len() > LIMIT {
            self.undo.pop_front();
        }
        self.redo.clear();
        self.revision = scene.revision();
        true
    }

    /// Forgets the scene's current changes as edits (after loading, or
    /// leaving play mode without keeping changes).
    pub fn rebase(&mut self, scene: &DataModel) {
        self.stable = scene.clone();
        self.revision = scene.revision();
        self.pending = None;
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|(label, _)| label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(label, _)| label.as_str())
    }

    /// Returns the scene to restore, or `None` if there is nothing to undo.
    /// The caller restores it and then calls [`History::rebase`].
    pub fn undo(&mut self, scene: &DataModel) -> Option<(String, DataModel)> {
        self.checkpoint(scene);
        let (label, target) = self.undo.pop_back()?;
        self.redo.push((label.clone(), self.stable.clone()));
        Some((label, target))
    }

    pub fn redo(&mut self, scene: &DataModel) -> Option<(String, DataModel)> {
        if self.has_uncommitted(scene) {
            // New edits since the undo replace the redo branch.
            self.checkpoint(scene);
            return None;
        }
        let (label, target) = self.redo.pop()?;
        self.undo.push_back((label.clone(), self.stable.clone()));
        Some((label, target))
    }
}

#[cfg(test)]
mod tests {
    use araria_core::Vec3;

    use super::*;

    #[test]
    fn checkpoints_undo_and_redo() {
        let mut dm = DataModel::new();
        let mut history = History::new(&dm);
        assert!(!history.checkpoint(&dm), "nothing changed");
        let part = dm.create("Part", Some(dm.workspace())).unwrap();
        history.label("Insert Part");
        assert!(history.checkpoint(&dm));
        dm.set(part, "Position", Vec3::new(5.0, 0.0, 0.0)).unwrap();
        dm.set(part, "Size", Vec3::splat(3.0)).unwrap();
        // Two property writes, one step.
        history.label("Move");
        dm.clear_journal();
        assert_eq!(history.undo_label(), Some("Insert Part"));

        let (label, target) = history.undo(&dm).unwrap();
        assert_eq!(label, "Move");
        dm.restore_from(&target);
        history.rebase(&dm);
        assert_eq!(dm.get(part).unwrap().vec3("Position"), Vec3::ZERO);
        let (_, target) = history.undo(&dm).unwrap();
        dm.restore_from(&target);
        history.rebase(&dm);
        assert!(!dm.contains(part));
        assert!(history.undo(&dm).is_none());

        let (label, target) = history.redo(&dm).unwrap();
        assert_eq!(label, "Insert Part");
        dm.restore_from(&target);
        history.rebase(&dm);
        assert!(dm.contains(part));
        assert_eq!(history.redo_label(), Some("Move"));
        // A new edit discards the redo branch.
        dm.set_name(part, "Renamed").unwrap();
        assert!(history.redo(&dm).is_none());
        assert!(history.redo_label().is_none());
        assert_eq!(history.undo_label(), Some("Edit"));
    }
}
