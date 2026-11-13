//! Property tweens (`TweenService:Create(instance, info, goals):Play()`).

use araria_core::{Color, DataModel, InstanceId, Quat, UDim2, Variant, Vec2, Vec3};

use crate::host::ScriptId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Linear,
    Sine,
    Quad,
    Cubic,
    Quart,
    Expo,
    Back,
    Bounce,
    Elastic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    InOut,
}

pub const STYLES: &[(&str, Style)] = &[
    ("Linear", Style::Linear),
    ("Sine", Style::Sine),
    ("Quad", Style::Quad),
    ("Cubic", Style::Cubic),
    ("Quart", Style::Quart),
    ("Expo", Style::Expo),
    ("Back", Style::Back),
    ("Bounce", Style::Bounce),
    ("Elastic", Style::Elastic),
];

fn ease_in(style: Style, t: f32) -> f32 {
    match style {
        Style::Linear => t,
        Style::Sine => 1.0 - (t * std::f32::consts::FRAC_PI_2).cos(),
        Style::Quad => t * t,
        Style::Cubic => t * t * t,
        Style::Quart => t * t * t * t,
        Style::Expo => {
            if t <= 0.0 {
                0.0
            } else {
                2f32.powf(10.0 * t - 10.0)
            }
        }
        Style::Back => 2.70158 * t * t * t - 1.70158 * t * t,
        Style::Bounce => 1.0 - bounce_out(1.0 - t),
        Style::Elastic => {
            if t <= 0.0 || t >= 1.0 {
                t
            } else {
                -(2f32.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * (std::f32::consts::TAU / 3.0)).sin()
            }
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

/// Eased progress for a linear progress `t` in `0..=1`.
pub fn ease(style: Style, direction: Direction, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match direction {
        Direction::In => ease_in(style, t),
        Direction::Out => 1.0 - ease_in(style, 1.0 - t),
        Direction::InOut => {
            if t < 0.5 {
                ease_in(style, t * 2.0) / 2.0
            } else {
                1.0 - ease_in(style, (1.0 - t) * 2.0) / 2.0
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TweenInfo {
    pub duration: f32,
    pub style: Style,
    pub direction: Direction,
    /// Extra repetitions; negative repeats forever.
    pub repeat: i32,
    pub reverses: bool,
    pub delay: f32,
}

impl Default for TweenInfo {
    fn default() -> Self {
        TweenInfo { duration: 1.0, style: Style::Quad, direction: Direction::Out, repeat: 0, reverses: false, delay: 0.0 }
    }
}

pub fn lerp_variant(a: &Variant, b: &Variant, t: f32) -> Option<Variant> {
    Some(match (a, b) {
        (Variant::Number(x), Variant::Number(y)) => Variant::Number(x + (y - x) * f64::from(t)),
        (Variant::Vec3(x), Variant::Vec3(y)) => Variant::Vec3(Vec3::lerp(*x, *y, t)),
        (Variant::Vec2(x), Variant::Vec2(y)) => Variant::Vec2(Vec2::lerp(*x, *y, t)),
        (Variant::Quat(x), Variant::Quat(y)) => Variant::Quat(Quat::slerp(*x, *y, t)),
        (Variant::Color(x), Variant::Color(y)) => Variant::Color(Color::lerp(*x, *y, t)),
        (Variant::UDim2(x), Variant::UDim2(y)) => Variant::UDim2(UDim2::lerp(*x, *y, t)),
        // Everything else switches at the end.
        (_, y) => {
            if t >= 1.0 {
                y.clone()
            } else {
                return None;
            }
        }
    })
}

pub struct Tween {
    pub id: u64,
    pub script: ScriptId,
    pub instance: InstanceId,
    pub info: TweenInfo,
    /// (property, start, goal)
    pub tracks: Vec<(String, Variant, Variant)>,
    pub elapsed: f32,
    pub playing: bool,
    pub completed_cycles: i32,
}

impl Tween {
    /// Advances the tween. Returns true when it has finished.
    pub fn step(&mut self, dm: &mut DataModel, dt: f32) -> bool {
        if !self.playing {
            return false;
        }
        if !dm.contains(self.instance) {
            return true;
        }
        self.elapsed += dt;
        let local = self.elapsed - self.info.delay;
        if local < 0.0 {
            return false;
        }
        let duration = self.info.duration.max(1e-4);
        let cycle_length = if self.info.reverses { duration * 2.0 } else { duration };
        let cycles_done = (local / cycle_length).floor() as i32;
        let finished = self.info.repeat >= 0 && cycles_done > self.info.repeat;
        let within = if finished { cycle_length } else { local - cycles_done as f32 * cycle_length };
        let mut t = (within / duration).min(2.0);
        if self.info.reverses && t > 1.0 {
            t = 2.0 - t;
        }
        let eased = ease(self.info.style, self.info.direction, t.clamp(0.0, 1.0));
        for (property, start, goal) in &self.tracks {
            if let Some(value) = lerp_variant(start, goal, if t >= 1.0 { 1.0 } else { eased }) {
                let _ = dm.set(self.instance, property, value);
            }
        }
        self.completed_cycles = cycles_done;
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_hits_endpoints_and_is_monotonic_where_expected() {
        for (_, style) in STYLES {
            for direction in [Direction::In, Direction::Out, Direction::InOut] {
                assert!(ease(*style, direction, 0.0).abs() < 1e-4, "{style:?} {direction:?}");
                assert!((ease(*style, direction, 1.0) - 1.0).abs() < 1e-4, "{style:?} {direction:?}");
            }
        }
        let mut last = 0.0;
        for i in 0..=20 {
            let v = ease(Style::Quad, Direction::InOut, i as f32 / 20.0);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn tweens_move_properties_and_finish() {
        let mut dm = DataModel::new();
        let part = dm.create("Part", Some(dm.workspace())).unwrap();
        let mut tween = Tween {
            id: 1,
            script: 0,
            instance: part,
            info: TweenInfo { duration: 1.0, style: Style::Linear, ..Default::default() },
            tracks: vec![("Position".into(), Variant::Vec3(Vec3::ZERO), Variant::Vec3(Vec3::new(10.0, 0.0, 0.0)))],
            elapsed: 0.0,
            playing: true,
            completed_cycles: 0,
        };
        assert!(!tween.step(&mut dm, 0.5));
        assert!((dm.get(part).unwrap().vec3("Position").x - 5.0).abs() < 1e-4);
        assert!(tween.step(&mut dm, 0.6));
        assert_eq!(dm.get(part).unwrap().vec3("Position").x, 10.0);

        // Reversing tweens come back to the start.
        tween.info.reverses = true;
        tween.elapsed = 0.0;
        tween.step(&mut dm, 1.5);
        assert!((dm.get(part).unwrap().vec3("Position").x - 5.0).abs() < 1e-3);
    }
}
