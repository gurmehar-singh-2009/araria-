//! Shape conventions shared by rendering and physics so that what players
//! see is exactly what they collide with.

use glam::Vec3;

/// The dimensions actually used for a primitive of the given shape: spheres
/// are uniform, and cylinders and cones have a circular cross-section.
pub fn effective_size(shape: &str, size: Vec3) -> Vec3 {
    match shape {
        "Sphere" => Vec3::splat(size.min_element()),
        "Cylinder" | "Cone" => {
            let d = size.x.min(size.z);
            Vec3::new(d, size.y, d)
        }
        "Plane" => Vec3::new(size.x, PLANE_THICKNESS, size.z),
        _ => size,
    }
}

/// Collision thickness of the `Plane` shape.
pub const PLANE_THICKNESS: f32 = 0.05;

/// Corner points of a wedge filling a box of `size` centered on the origin.
/// The wedge is full height along its back edge (+Z) and slopes down to the
/// front edge (-Z).
pub fn wedge_points(size: Vec3) -> [Vec3; 6] {
    let h = size * 0.5;
    [
        Vec3::new(-h.x, -h.y, -h.z),
        Vec3::new(h.x, -h.y, -h.z),
        Vec3::new(-h.x, -h.y, h.z),
        Vec3::new(h.x, -h.y, h.z),
        Vec3::new(-h.x, h.y, h.z),
        Vec3::new(h.x, h.y, h.z),
    ]
}

/// Standing height of a character at scale 1.
pub const CHARACTER_HEIGHT: f32 = 1.8;
/// Collision radius of a character at scale 1.
pub const CHARACTER_RADIUS: f32 = 0.36;

/// Yaw (in degrees) that makes a character face along `direction`.
/// Characters face +Z at yaw 0.
pub fn yaw_toward(direction: Vec3) -> f32 {
    direction.x.atan2(direction.z).to_degrees()
}

/// Unit vector a character with the given yaw (degrees) is facing.
pub fn facing_direction(yaw_deg: f32) -> Vec3 {
    let r = yaw_deg.to_radians();
    Vec3::new(r.sin(), 0.0, r.cos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_sizes() {
        let size = Vec3::new(4.0, 2.0, 6.0);
        assert_eq!(effective_size("Block", size), size);
        assert_eq!(effective_size("Sphere", size), Vec3::splat(2.0));
        assert_eq!(effective_size("Cylinder", size), Vec3::new(4.0, 2.0, 4.0));
    }

    #[test]
    fn yaw_round_trip() {
        for yaw in [-170.0, -90.0, 0.0, 45.0, 135.0] {
            assert!((yaw_toward(facing_direction(yaw)) - yaw).abs() < 1e-3);
        }
        assert!((facing_direction(0.0) - Vec3::Z).length() < 1e-6);
        assert!((facing_direction(90.0) - Vec3::X).length() < 1e-6);
    }
}
