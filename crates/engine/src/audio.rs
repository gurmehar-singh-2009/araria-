//! Sound: one-shot effects and music requested by scripts, and `Sound`
//! instances in the scene (optionally positional).
//!
//! Positional sounds are attenuated by their distance from the camera
//! rather than panned, which keeps behaviour identical in browsers and on
//! desktop.

use std::collections::{HashMap, HashSet};

use araria_core::{Change, DataModel, InstanceId, Variant};
use araria_runtime::Command;
use bevy::audio::Volume;
use bevy::prelude::*;

use crate::camera::GameCamera;
use crate::scene_sync::AssetLibrary;
use crate::{EngineSet, Game, PresentationCommands};

/// Volume preferences from the settings screen, each `0..=1`.
#[derive(Resource, Clone, Copy)]
pub struct AudioSettings {
    pub master: f32,
    pub music: f32,
    pub effects: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings { master: 1.0, music: 0.7, effects: 1.0 }
    }
}

/// The background music track.
#[derive(Component)]
struct Music {
    volume: f32,
}

/// The playing voice of a `Sound` instance.
#[derive(Component)]
struct SoundVoice {
    instance: InstanceId,
    looped: bool,
}

/// Marks every audio entity that belongs to the current game.
#[derive(Component)]
struct GameAudio;

#[derive(Resource, Default)]
struct SoundVoices {
    map: HashMap<InstanceId, Entity>,
    scene: (u64, u64),
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioSettings>()
            .init_resource::<SoundVoices>()
            .add_systems(Update, (play_commands, sync_sounds, update_voices).chain().in_set(EngineSet::Present));
    }
}

/// How loud a sound at `position` is for a listener, `0..=1`.
fn attenuation(listener: Vec3, position: Vec3, range: f32) -> f32 {
    let nearness = 1.0 - (listener.distance(position) / range.max(0.01)).clamp(0.0, 1.0);
    nearness * nearness
}

fn play_commands(
    mut commands: Commands,
    queue: Res<PresentationCommands>,
    library: Res<AssetLibrary>,
    asset_server: Res<AssetServer>,
    settings: Res<AudioSettings>,
    camera: Query<&GlobalTransform, With<GameCamera>>,
    music: Query<Entity, With<Music>>,
    mut unknown: Local<HashSet<String>>,
) {
    let listener = camera.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    let mut source = |asset: &str| {
        let found = library.0.audio.get(asset).map(|a| asset_server.load::<AudioSource>(a.file.clone()));
        if found.is_none() && unknown.insert(asset.to_string()) {
            warn!("unknown audio asset '{asset}'");
        }
        found
    };
    for command in &queue.0 {
        match command {
            Command::PlaySound { asset, volume, pitch, position } => {
                let Some(handle) = source(asset) else { continue };
                let distance_gain = position.map_or(1.0, |p| attenuation(listener, p, 45.0));
                let gain = volume.clamp(0.0, 4.0) * distance_gain * settings.effects * settings.master;
                if gain > 0.001 {
                    let playback = PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)).with_speed(pitch.clamp(0.1, 4.0));
                    commands.spawn((AudioPlayer::new(handle), playback, GameAudio));
                }
            }
            Command::PlayMusic { asset, volume } => {
                for entity in &music {
                    commands.entity(entity).despawn();
                }
                let Some(handle) = source(asset) else { continue };
                let playback = PlaybackSettings::LOOP.with_volume(Volume::Linear(volume * settings.music * settings.master));
                commands.spawn((AudioPlayer::new(handle), playback, Music { volume: *volume }, GameAudio));
            }
            Command::StopMusic => {
                for entity in &music {
                    commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }
}

fn wants_voice(dm: &DataModel, id: InstanceId) -> bool {
    dm.get(id).is_some_and(|s| s.class_name() == "Sound" && s.bool("Playing") && !s.str("Asset").is_empty()) && !dm.is_descendant_of(id, dm.service("Storage"))
}

/// Starts, restarts and stops voices as `Sound.Playing` changes.
fn sync_sounds(
    mut commands: Commands,
    game: NonSend<Game>,
    mut voices: ResMut<SoundVoices>,
    library: Res<AssetLibrary>,
    asset_server: Res<AssetServer>,
    owned: Query<Entity, With<GameAudio>>,
) {
    let scene = game.scene_id();
    let rebuild = voices.scene != scene;
    if rebuild {
        // A different game (or a restored scene): silence everything.
        for entity in &owned {
            commands.entity(entity).despawn();
        }
        voices.map.clear();
        voices.scene = scene;
    }
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dm = runtime.dm.borrow();
    let mut touched: HashSet<InstanceId> = HashSet::new();
    if rebuild {
        touched.extend(dm.instances().filter(|i| i.class_name() == "Sound").map(|i| i.id));
    }
    for change in dm.journal() {
        match change {
            Change::Added(id) | Change::Removed(id) | Change::Reparented(id) => {
                touched.insert(*id);
            }
            Change::Prop(id, _) if dm.get(*id).is_some_and(|i| i.class_name() == "Sound") => {
                touched.insert(*id);
            }
            _ => {}
        }
    }
    for id in touched {
        // Any change restarts the voice with the new settings.
        if let Some(entity) = voices.map.remove(&id) {
            commands.entity(entity).despawn();
        }
        if !wants_voice(&dm, id) {
            continue;
        }
        let Some(sound) = dm.get(id) else { continue };
        let Some(asset) = library.0.audio.get(sound.str("Asset")) else {
            warn!("{}: unknown audio asset '{}'", dm.path(id), sound.str("Asset"));
            continue;
        };
        let looped = sound.bool("Looped");
        let base = if looped { PlaybackSettings::LOOP } else { PlaybackSettings::ONCE };
        // The real volume is applied by `update_voices` once the sink exists.
        let playback = base.with_volume(Volume::Linear(0.0)).with_speed(sound.f32("Pitch"));
        let entity = commands.spawn((AudioPlayer::new(asset_server.load::<AudioSource>(asset.file.clone())), playback, SoundVoice { instance: id, looped }, GameAudio)).id();
        voices.map.insert(id, entity);
    }
}

/// Keeps volumes current (settings, distance) and reports finished sounds.
fn update_voices(
    mut commands: Commands,
    game: NonSend<Game>,
    settings: Res<AudioSettings>,
    mut voices: ResMut<SoundVoices>,
    camera: Query<&GlobalTransform, With<GameCamera>>,
    mut playing: Query<(Entity, &SoundVoice, &mut AudioSink)>,
    mut music: Query<(&Music, &mut AudioSink), Without<SoundVoice>>,
) {
    for (track, mut sink) in &mut music {
        sink.set_volume(Volume::Linear(track.volume * settings.music * settings.master));
    }
    let Some(runtime) = game.runtime.as_ref() else { return };
    let listener = camera.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    let mut dm = runtime.dm.borrow_mut();
    for (entity, voice, mut sink) in &mut playing {
        let Some(sound) = dm.get(voice.instance) else { continue };
        if !voice.looped && sink.empty() {
            commands.entity(entity).despawn();
            voices.map.remove(&voice.instance);
            dm.set_quiet(voice.instance, "Playing", Variant::Bool(false));
            dm.fire(voice.instance, "Ended", Vec::new());
            continue;
        }
        let distance_gain = if sound.bool("Spatial") {
            dm.world_position(voice.instance).map_or(1.0, |p| attenuation(listener, p, sound.f32("Range")))
        } else {
            1.0
        };
        sink.set_volume(Volume::Linear(sound.f32("Volume") * distance_gain * settings.effects * settings.master));
    }
}
