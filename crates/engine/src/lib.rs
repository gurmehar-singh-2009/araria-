//! # Araria engine
//!
//! The Bevy-based presentation layer. It owns no game state of its own: each
//! frame it feeds input into the active [`araria_runtime::Runtime`], steps
//! it, and mirrors the runtime's data model into Bevy entities.
//!
//! Frame order (all in `Update`, see [`EngineSet`]):
//!
//! 1. `Input` – devices → [`araria_core::input::InputState`]
//! 2. `Simulate` – the runtime advances (physics, scripts, players)
//! 3. `Present` – journal → entities, environment, camera
//! 4. `Finish` – the journal is cleared

pub mod camera;
pub mod environment;
pub mod input;
pub mod materials;
pub mod scene_sync;

use std::sync::Arc;

use araria_core::DataModel;
use araria_core::assets::AssetManifest;
use araria_runtime::Runtime;
use bevy::prelude::*;

/// The game currently loaded, if any. Non-send because the runtime shares
/// its state through `Rc` with the (single-threaded) scripting layer.
#[derive(Default)]
pub struct Game {
    pub runtime: Option<Runtime>,
    /// Bumped whenever a different runtime is installed, so presentation
    /// systems know to rebuild everything.
    pub generation: u64,
}

impl Game {
    /// Replaces the current game with a new scene in edit mode.
    pub fn load(&mut self, dm: DataModel, assets: Arc<AssetManifest>) -> &mut Runtime {
        self.generation += 1;
        self.runtime.insert(Runtime::new(dm, assets))
    }

    pub fn unload(&mut self) {
        self.runtime = None;
        self.generation += 1;
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineSet {
    Input,
    Simulate,
    Present,
    Finish,
}

pub struct EnginePlugin;

impl Plugin for EnginePlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(Game::default())
            .configure_sets(
                Update,
                (EngineSet::Input, EngineSet::Simulate, EngineSet::Present, EngineSet::Finish).chain(),
            )
            .add_plugins((
                materials::MaterialsPlugin,
                input::InputPlugin,
                scene_sync::SceneSyncPlugin,
                environment::EnvironmentPlugin,
                camera::CameraPlugin,
            ))
            .add_systems(Update, simulate.in_set(EngineSet::Simulate))
            .add_systems(Update, finish_frame.in_set(EngineSet::Finish));
    }
}

fn simulate(mut game: NonSendMut<Game>, time: Res<Time>) {
    if let Some(runtime) = game.runtime.as_mut() {
        // Long hitches (tab switches, asset loads) are not simulated in full.
        runtime.step(time.delta_secs().min(0.1));
    }
}

fn finish_frame(mut game: NonSendMut<Game>) {
    if let Some(runtime) = game.runtime.as_mut() {
        runtime.end_frame();
    }
}

/// Converts a core color to a Bevy color.
pub fn color(c: araria_core::Color) -> Color {
    Color::srgb(c.r, c.g, c.b)
}
