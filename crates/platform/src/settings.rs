//! User settings, persisted as `settings.json` in local storage.

use araria_storage::Storage;
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    pub quality: Quality,
    pub shadows: bool,
    pub bloom: bool,
    pub antialiasing: bool,
    /// Multiplier on each game's fog distances (0.5–2).
    pub view_distance: f32,
    pub particles: bool,
    pub field_of_view: f32,
}

impl Default for Graphics {
    fn default() -> Self {
        Graphics { quality: Quality::High, shadows: true, bloom: true, antialiasing: true, view_distance: 1.0, particles: true, field_of_view: 70.0 }
    }
}

impl Graphics {
    /// Applies a preset to the individual options.
    pub fn apply_preset(&mut self, quality: Quality) {
        self.quality = quality;
        (self.shadows, self.bloom, self.antialiasing, self.particles, self.view_distance) = match quality {
            Quality::Low => (false, false, false, false, 0.6),
            Quality::Medium => (true, false, true, true, 0.85),
            Quality::High => (true, true, true, true, 1.0),
        };
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub muted: bool,
}

impl Default for Audio {
    fn default() -> Self {
        Audio { master: 0.8, music: 0.6, effects: 0.9, muted: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    pub forward: String,
    pub back: String,
    pub left: String,
    pub right: String,
    pub jump: String,
    pub run: String,
    pub interact: String,
}

impl Default for Input {
    fn default() -> Self {
        Input {
            mouse_sensitivity: 1.0,
            invert_y: false,
            forward: "W".into(),
            back: "S".into(),
            left: "A".into(),
            right: "D".into(),
            jump: "Space".into(),
            run: "LeftShift".into(),
            interact: "E".into(),
        }
    }
}

impl Input {
    /// The rebindable actions as (label, binding) pairs.
    pub fn bindings_mut(&mut self) -> [(&'static str, &mut String); 7] {
        [
            ("Move forward", &mut self.forward),
            ("Move back", &mut self.back),
            ("Move left", &mut self.left),
            ("Move right", &mut self.right),
            ("Jump", &mut self.jump),
            ("Run", &mut self.run),
            ("Interact", &mut self.interact),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    pub ui_scale: f32,
    pub reduce_motion: bool,
    pub camera_shake: bool,
    pub high_contrast: bool,
}

impl Default for Accessibility {
    fn default() -> Self {
        Accessibility { ui_scale: 1.0, reduce_motion: false, camera_shake: true, high_contrast: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    pub show_fps: bool,
    /// 0 means unlimited (vsync).
    pub frame_limit: u32,
    pub pause_when_unfocused: bool,
}

impl Default for Performance {
    fn default() -> Self {
        Performance { show_fps: false, frame_limit: 0, pause_when_unfocused: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Editor {
    pub font_size: f32,
    pub tab_width: u32,
    pub autosave: bool,
    pub hot_reload: bool,
    pub line_numbers: bool,
    pub autocomplete: bool,
    pub snap_move: f32,
    pub snap_rotate: f32,
}

impl Default for Editor {
    fn default() -> Self {
        Editor { font_size: 14.0, tab_width: 4, autosave: true, hot_reload: true, line_numbers: true, autocomplete: true, snap_move: 0.5, snap_rotate: 15.0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub graphics: Graphics,
    pub audio: Audio,
    pub input: Input,
    pub accessibility: Accessibility,
    pub performance: Performance,
    pub editor: Editor,
    /// Shown on name tags in multiplayer sessions. Not an account.
    pub display_name: String,
}

impl Settings {
    /// Loads settings, falling back to defaults for anything missing or
    /// unreadable.
    pub fn load(storage: &Storage) -> Settings {
        let Some(text) = storage.read_string(SETTINGS_FILE) else { return Settings::default() };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            log::warn!("settings.json is invalid ({e}); using defaults");
            Settings::default()
        })
    }

    pub fn save(&self, storage: &Storage) {
        let text = serde_json::to_string_pretty(self).expect("settings are always serializable");
        if let Err(e) = storage.write(SETTINGS_FILE, text.as_bytes()) {
            log::error!("could not save settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_persist_and_tolerate_partial_files() {
        let storage = Storage::memory();
        assert_eq!(Settings::load(&storage), Settings::default());
        let mut settings = Settings::default();
        settings.graphics.apply_preset(Quality::Low);
        settings.input.jump = "F".into();
        settings.save(&storage);
        assert_eq!(Settings::load(&storage), settings);

        storage.write(SETTINGS_FILE, br#"{"audio":{"muted":true}}"#).unwrap();
        let partial = Settings::load(&storage);
        assert!(partial.audio.muted);
        assert_eq!(partial.audio.master, 0.8);
        assert_eq!(partial.graphics, Graphics::default());

        storage.write(SETTINGS_FILE, b"not json").unwrap();
        assert_eq!(Settings::load(&storage), Settings::default());
    }
}
