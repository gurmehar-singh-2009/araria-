//! Device-independent input state. The engine fills it in each frame; the
//! runtime and scripts read it.

use std::collections::HashSet;

use glam::{Vec2, Vec3};

#[derive(Clone, Debug, Default)]
pub struct InputState {
    /// Names of the keys currently held, e.g. `"W"`, `"Space"`, `"LeftShift"`.
    pub keys: HashSet<String>,
    /// Left, right and middle mouse buttons.
    pub mouse_buttons: [bool; 3],
    /// Pointer position in viewport pixels.
    pub mouse_position: Vec2,
    pub mouse_delta: Vec2,
    pub wheel: f32,
    /// World-space ray through the pointer: origin and unit direction.
    pub mouse_ray: Option<(Vec3, Vec3)>,
    pub viewport: Vec2,
    /// Movement request: x is right, y is forward, each in `-1..=1`.
    pub move_axis: Vec2,
    pub jump: bool,
    pub run: bool,
    /// Yaw of the camera's viewing direction in degrees (0 looks along +Z).
    pub camera_yaw: f32,
    /// True while the pointer or keyboard is being used by the interface.
    pub ui_captured: bool,
}

impl InputState {
    pub fn key(&self, name: &str) -> bool {
        self.keys.contains(name)
    }

    /// The movement request as a world-space direction relative to the camera.
    pub fn move_direction(&self) -> Vec3 {
        let forward = crate::geometry::facing_direction(self.camera_yaw);
        let right = Vec3::new(-forward.z, 0.0, forward.x);
        let dir = forward * self.move_axis.y + right * self.move_axis.x;
        if dir.length_squared() > 1.0 { dir.normalize() } else { dir }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_is_relative_to_the_camera() {
        let mut input = InputState { move_axis: Vec2::new(0.0, 1.0), ..Default::default() };
        assert!((input.move_direction() - Vec3::Z).length() < 1e-5);
        input.camera_yaw = 180.0;
        assert!((input.move_direction() + Vec3::Z).length() < 1e-5);
        input.move_axis = Vec2::new(1.0, 0.0);
        // Looking along -Z (Bevy's default view), "right" is +X.
        assert!((input.move_direction() - Vec3::X).length() < 1e-5);
        input.move_axis = Vec2::new(1.0, 1.0);
        assert!((input.move_direction().length() - 1.0).abs() < 1e-5);
    }
}
