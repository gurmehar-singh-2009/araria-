//! Property value types shared by the engine, the scripting layer and tools.

use std::fmt;

use glam::{EulerRot, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Identifies an instance within one [`crate::DataModel`]. Ids are never
/// reused, so a stale id simply fails to resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InstanceId(pub u64);

impl InstanceId {
    /// Ids at or above this value are local to one client and never replicated.
    pub const LOCAL_BASE: u64 = 1 << 48;

    pub fn is_local(self) -> bool {
        self.0 >= Self::LOCAL_BASE
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// An sRGB color with components in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Color { r, g, b }
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
    }

    /// Parses `#rgb`, `#rrggbb` or the same without the hash.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim().trim_start_matches('#');
        let parse = |s: &str| u8::from_str_radix(s, 16).ok();
        match hex.len() {
            3 => {
                let c: Vec<u8> = hex.chars().filter_map(|c| c.to_digit(16)).map(|d| (d * 17) as u8).collect();
                (c.len() == 3).then(|| Color::from_rgb8(c[0], c[1], c[2]))
            }
            6 => Some(Color::from_rgb8(parse(&hex[0..2])?, parse(&hex[2..4])?, parse(&hex[4..6])?)),
            _ => None,
        }
    }

    pub fn to_rgb8(self) -> [u8; 3] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b)]
    }

    pub fn to_hex(self) -> String {
        let [r, g, b] = self.to_rgb8();
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// Hue in degrees, saturation and value in `0..=1`.
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(360.0) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        Color::rgb(r + m, g + m, b + m)
    }

    pub fn to_hsv(self) -> (f32, f32, f32) {
        let max = self.r.max(self.g).max(self.b);
        let min = self.r.min(self.g).min(self.b);
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == self.r {
            60.0 * ((self.g - self.b) / d).rem_euclid(6.0)
        } else if max == self.g {
            60.0 * ((self.b - self.r) / d + 2.0)
        } else {
            60.0 * ((self.r - self.g) / d + 4.0)
        };
        (h, if max == 0.0 { 0.0 } else { d / max }, max)
    }

    pub fn lerp(self, other: Color, t: f32) -> Color {
        Color::rgb(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
        )
    }
}

/// A 2D size or position mixing a fraction of the parent (`scale`) with a
/// fixed pixel `offset` on each axis.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct UDim2 {
    pub x_scale: f32,
    pub x_offset: f32,
    pub y_scale: f32,
    pub y_offset: f32,
}

impl UDim2 {
    pub const fn new(x_scale: f32, x_offset: f32, y_scale: f32, y_offset: f32) -> Self {
        UDim2 { x_scale, x_offset, y_scale, y_offset }
    }

    pub const fn from_offset(x: f32, y: f32) -> Self {
        UDim2::new(0.0, x, 0.0, y)
    }

    pub const fn from_scale(x: f32, y: f32) -> Self {
        UDim2::new(x, 0.0, y, 0.0)
    }

    /// Resolves against a parent size in pixels.
    pub fn resolve(self, parent: Vec2) -> Vec2 {
        Vec2::new(self.x_scale * parent.x + self.x_offset, self.y_scale * parent.y + self.y_offset)
    }

    pub fn lerp(self, other: UDim2, t: f32) -> UDim2 {
        let l = |a: f32, b: f32| a + (b - a) * t;
        UDim2::new(
            l(self.x_scale, other.x_scale),
            l(self.x_offset, other.x_offset),
            l(self.y_scale, other.y_scale),
            l(self.y_offset, other.y_offset),
        )
    }
}

/// Converts a rotation to Euler angles in degrees (applied Y, then X, then Z —
/// yaw, pitch, roll), the representation shown to users.
pub fn quat_to_euler_deg(q: Quat) -> Vec3 {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

/// Inverse of [`quat_to_euler_deg`].
pub fn euler_deg_to_quat(deg: Vec3) -> Quat {
    Quat::from_euler(EulerRot::YXZ, deg.y.to_radians(), deg.x.to_radians(), deg.z.to_radians())
}

/// A dynamically typed property or attribute value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Variant {
    #[default]
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    Vec2(Vec2),
    Vec3(Vec3),
    Quat(Quat),
    Color(Color),
    UDim2(UDim2),
    Ref(Option<InstanceId>),
}

impl Variant {
    pub fn type_name(&self) -> &'static str {
        match self {
            Variant::Nil => "nil",
            Variant::Bool(_) => "boolean",
            Variant::Number(_) => "number",
            Variant::String(_) => "string",
            Variant::Vec2(_) => "Vector2",
            Variant::Vec3(_) => "Vector3",
            Variant::Quat(_) => "Rotation",
            Variant::Color(_) => "Color3",
            Variant::UDim2(_) => "UDim2",
            Variant::Ref(_) => "Instance",
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Variant::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Variant::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|n| n as f32)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Variant::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<Vec3> {
        match self {
            Variant::Vec3(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            Variant::Vec2(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_quat(&self) -> Option<Quat> {
        match self {
            Variant::Quat(q) => Some(*q),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<Color> {
        match self {
            Variant::Color(c) => Some(*c),
            _ => None,
        }
    }

    pub fn as_udim2(&self) -> Option<UDim2> {
        match self {
            Variant::UDim2(u) => Some(*u),
            _ => None,
        }
    }

    pub fn as_ref_id(&self) -> Option<InstanceId> {
        match self {
            Variant::Ref(id) => *id,
            _ => None,
        }
    }
}

impl From<bool> for Variant {
    fn from(v: bool) -> Self {
        Variant::Bool(v)
    }
}
impl From<f64> for Variant {
    fn from(v: f64) -> Self {
        Variant::Number(v)
    }
}
impl From<f32> for Variant {
    fn from(v: f32) -> Self {
        Variant::Number(f64::from(v))
    }
}
impl From<i32> for Variant {
    fn from(v: i32) -> Self {
        Variant::Number(f64::from(v))
    }
}
impl From<&str> for Variant {
    fn from(v: &str) -> Self {
        Variant::String(v.to_string())
    }
}
impl From<String> for Variant {
    fn from(v: String) -> Self {
        Variant::String(v)
    }
}
impl From<Vec3> for Variant {
    fn from(v: Vec3) -> Self {
        Variant::Vec3(v)
    }
}
impl From<Vec2> for Variant {
    fn from(v: Vec2) -> Self {
        Variant::Vec2(v)
    }
}
impl From<Quat> for Variant {
    fn from(v: Quat) -> Self {
        Variant::Quat(v)
    }
}
impl From<Color> for Variant {
    fn from(v: Color) -> Self {
        Variant::Color(v)
    }
}
impl From<UDim2> for Variant {
    fn from(v: UDim2) -> Self {
        Variant::UDim2(v)
    }
}
impl From<InstanceId> for Variant {
    fn from(v: InstanceId) -> Self {
        Variant::Ref(Some(v))
    }
}
impl From<Option<InstanceId>> for Variant {
    fn from(v: Option<InstanceId>) -> Self {
        Variant::Ref(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_hex_round_trip() {
        let c = Color::from_hex("#3fa7d6").unwrap();
        assert_eq!(c.to_hex(), "#3fa7d6");
        assert_eq!(Color::from_hex("fff").unwrap(), Color::WHITE);
        assert!(Color::from_hex("#12").is_none());
        let (h, s, v) = Color::rgb(1.0, 0.0, 0.0).to_hsv();
        assert_eq!((h, s, v), (0.0, 1.0, 1.0));
        assert_eq!(Color::from_hsv(120.0, 1.0, 1.0).to_rgb8(), [0, 255, 0]);
    }

    #[test]
    fn euler_round_trip() {
        let deg = Vec3::new(20.0, -135.0, 40.0);
        let back = quat_to_euler_deg(euler_deg_to_quat(deg));
        assert!((back - deg).length() < 1e-3, "{back:?}");
    }

    #[test]
    fn udim2_resolves_against_parent() {
        let u = UDim2::new(0.5, 10.0, 1.0, -20.0);
        assert_eq!(u.resolve(Vec2::new(200.0, 100.0)), Vec2::new(110.0, 80.0));
    }
}
