//! Reads keyboard and mouse state into the runtime's
//! [`araria_core::input::InputState`].

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::GameCamera;
use crate::{EngineSet, Game};

/// Set by the interface layer when it is using the pointer or keyboard, so
/// the game ignores them.
#[derive(Resource, Default)]
pub struct UiCapture {
    pub pointer: bool,
    pub keyboard: bool,
}

/// The on-screen rectangle (in logical pixels) the game is drawn into.
/// `None` means the whole window.
#[derive(Resource, Default)]
pub struct GameViewport(pub Option<Rect>);

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiCapture>()
            .init_resource::<GameViewport>()
            .add_systems(Update, gather_input.in_set(EngineSet::Input));
    }
}

/// Stable, script-facing name for a key.
pub fn key_name(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::KeyA => "A",
        KeyCode::KeyB => "B",
        KeyCode::KeyC => "C",
        KeyCode::KeyD => "D",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyI => "I",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyN => "N",
        KeyCode::KeyO => "O",
        KeyCode::KeyP => "P",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyS => "S",
        KeyCode::KeyT => "T",
        KeyCode::KeyU => "U",
        KeyCode::KeyV => "V",
        KeyCode::KeyW => "W",
        KeyCode::KeyX => "X",
        KeyCode::KeyY => "Y",
        KeyCode::KeyZ => "Z",
        KeyCode::Digit0 => "Zero",
        KeyCode::Digit1 => "One",
        KeyCode::Digit2 => "Two",
        KeyCode::Digit3 => "Three",
        KeyCode::Digit4 => "Four",
        KeyCode::Digit5 => "Five",
        KeyCode::Digit6 => "Six",
        KeyCode::Digit7 => "Seven",
        KeyCode::Digit8 => "Eight",
        KeyCode::Digit9 => "Nine",
        KeyCode::Space => "Space",
        KeyCode::ShiftLeft => "LeftShift",
        KeyCode::ShiftRight => "RightShift",
        KeyCode::ControlLeft => "LeftControl",
        KeyCode::ControlRight => "RightControl",
        KeyCode::AltLeft => "LeftAlt",
        KeyCode::AltRight => "RightAlt",
        KeyCode::Enter => "Enter",
        KeyCode::Escape => "Escape",
        KeyCode::Tab => "Tab",
        KeyCode::Backspace => "Backspace",
        KeyCode::ArrowUp => "Up",
        KeyCode::ArrowDown => "Down",
        KeyCode::ArrowLeft => "Left",
        KeyCode::ArrowRight => "Right",
        _ => return None,
    })
}

fn gather_input(
    game: NonSend<Game>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    capture: Res<UiCapture>,
    viewport: Res<GameViewport>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<GameCamera>>,
) {
    let Some(runtime) = game.runtime.as_ref() else { return };
    let mut input = runtime.input.borrow_mut();
    input.ui_captured = capture.keyboard;

    input.keys.clear();
    if !capture.keyboard {
        input.keys.extend(keys.get_pressed().filter_map(|k| key_name(*k)).map(str::to_string));
    }
    let held = |names: &[&str]| names.iter().any(|n| input.keys.contains(*n));
    let axis = |negative: &[&str], positive: &[&str]| f32::from(i8::from(held(positive)) - i8::from(held(negative)));
    let move_axis = Vec2::new(axis(&["A", "Left"], &["D", "Right"]), axis(&["S", "Down"], &["W", "Up"]));
    let (jump, run) = (held(&["Space"]), held(&["LeftShift", "RightShift"]));
    input.move_axis = move_axis;
    input.jump = jump;
    input.run = run;

    let pointer_free = !capture.pointer;
    input.mouse_buttons = [
        pointer_free && buttons.pressed(MouseButton::Left),
        pointer_free && buttons.pressed(MouseButton::Right),
        pointer_free && buttons.pressed(MouseButton::Middle),
    ];
    input.mouse_delta = motion.delta;
    input.wheel = if pointer_free { scroll.delta.y } else { 0.0 };

    let area = viewport.0.unwrap_or(Rect::new(0.0, 0.0, window.width(), window.height()));
    input.viewport = area.size();
    input.mouse_ray = None;
    if let Some(cursor) = window.cursor_position() {
        input.mouse_position = cursor - area.min;
        if let Ok((camera, transform)) = camera.single() {
            if let Ok(ray) = camera.viewport_to_world(transform, cursor) {
                input.mouse_ray = Some((ray.origin, *ray.direction));
            }
        }
    }
}
