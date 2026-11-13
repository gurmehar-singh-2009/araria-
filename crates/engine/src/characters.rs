//! Character presentation: the rigged model, per-character colours and the
//! animation controller that turns character state into blended clips.

use std::collections::HashMap;
use std::time::Duration;

use araria_core::InstanceId;
use bevy::gltf::GltfMaterialName;
use bevy::prelude::*;

use crate::{EngineSet, Game, color};

pub const CHARACTER_MODEL: &str = "models/character.glb";
/// Clips that play once and hold their last frame.
const ONE_SHOT: &[&str] = &["Jump", "Land", "Wave", "Cheer", "Point", "Collapse"];
const CROSSFADE: Duration = Duration::from_millis(160);

/// The shared character model and its animation graph.
#[derive(Resource)]
pub struct CharacterAssets {
    gltf: Handle<Gltf>,
    pub scene: Handle<WorldAsset>,
    graph: Option<(Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>)>,
}

/// Placed on the model root spawned for a `Character` instance.
#[derive(Component)]
pub struct CharacterModel {
    pub instance: InstanceId,
    pub skin: araria_core::Color,
    pub shirt: araria_core::Color,
    pub pants: araria_core::Color,
    pub hair: araria_core::Color,
}

/// Marks a model whose materials and animation player have been set up.
#[derive(Component)]
struct CharacterReady;

/// Drives one character's `AnimationPlayer`.
#[derive(Component)]
struct CharacterAnimator {
    instance: InstanceId,
    current: String,
}

pub struct CharactersPlugin;

impl Plugin for CharactersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_assets)
            .add_systems(Update, (build_graph, prepare_models, animate).chain().in_set(EngineSet::Present));
    }
}

fn load_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(CharacterAssets {
        gltf: asset_server.load(CHARACTER_MODEL),
        scene: asset_server.load(GltfAssetLabel::Scene(0).from_asset(CHARACTER_MODEL)),
        graph: None,
    });
}

fn build_graph(mut assets: ResMut<CharacterAssets>, gltfs: Res<Assets<Gltf>>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    if assets.graph.is_some() {
        return;
    }
    let Some(gltf) = gltfs.get(&assets.gltf) else { return };
    let mut graph = AnimationGraph::new();
    let mut nodes = HashMap::new();
    for (name, clip) in &gltf.named_animations {
        nodes.insert(name.to_string(), graph.add_clip(clip.clone(), 1.0, graph.root));
    }
    assets.graph = Some((graphs.add(graph), nodes));
}

/// Recolours a freshly spawned model and attaches its animation graph.
fn prepare_models(
    mut commands: Commands,
    assets: Res<CharacterAssets>,
    models: Query<(Entity, &CharacterModel), Without<CharacterReady>>,
    children: Query<&Children>,
    players: Query<(), With<AnimationPlayer>>,
    mesh_materials: Query<(&MeshMaterial3d<StandardMaterial>, &GltfMaterialName)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some((graph, _)) = &assets.graph else { return };
    for (root, model) in &models {
        // The scene spawns asynchronously; wait for its animation player.
        let Some(player) = children.iter_descendants(root).find(|e| players.contains(*e)) else { continue };
        for descendant in children.iter_descendants(root) {
            let Ok((material, name)) = mesh_materials.get(descendant) else { continue };
            let tint = match name.0.as_str() {
                "Skin" => model.skin,
                "Shirt" => model.shirt,
                "Pants" => model.pants,
                "Hair" => model.hair,
                _ => continue,
            };
            let Some(mut custom) = materials.get(material.id()).cloned() else { continue };
            custom.base_color = color(tint);
            commands.entity(descendant).insert(MeshMaterial3d(materials.add(custom)));
        }
        commands.entity(player).insert((
            AnimationGraphHandle(graph.clone()),
            AnimationTransitions::new(),
            CharacterAnimator { instance: model.instance, current: String::new() },
        ));
        commands.entity(root).insert(CharacterReady);
    }
}

/// Picks the clip for a character from its state.
fn clip_for(state: &str, emote: &str) -> &'static str {
    match state {
        "Walking" => "Walk",
        "Running" => "Run",
        "Jumping" => "Jump",
        "Falling" => "Fall",
        "Landing" => "Land",
        "Seated" => "Sit",
        "Dead" => "Collapse",
        _ => match emote {
            "Wave" => "Wave",
            "Cheer" => "Cheer",
            "Dance" => "Dance",
            "Point" => "Point",
            "Sit" => "Sit",
            _ => "Idle",
        },
    }
}

fn animate(
    game: NonSend<Game>,
    assets: Res<CharacterAssets>,
    mut animators: Query<(&mut CharacterAnimator, &mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let (Some(runtime), Some((_, nodes))) = (game.runtime.as_ref(), &assets.graph) else { return };
    let dm = runtime.dm.borrow();
    for (mut animator, mut player, mut transitions) in &mut animators {
        let Some(character) = dm.get(animator.instance) else { continue };
        let clip = clip_for(character.str("State"), character.str("Emote"));
        let Some(&node) = nodes.get(clip) else { continue };
        if animator.current != clip {
            animator.current = clip.to_string();
            let active = transitions.play(&mut player, node, CROSSFADE);
            if !ONE_SHOT.contains(&clip) {
                active.repeat();
            }
        }
        // Match foot speed to ground speed so feet do not slide.
        if let Some(active) = player.animation_mut(node) {
            let velocity = character.vec3("Velocity");
            let speed = Vec2::new(velocity.x, velocity.z).length();
            let rate = match clip {
                "Walk" => (speed / 6.0).clamp(0.5, 1.6),
                "Run" => (speed / 10.0).clamp(0.7, 1.5),
                _ => 1.0,
            };
            active.set_speed(rate);
        }
    }
}
