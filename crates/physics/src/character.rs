//! Character movement: a capsule moved by Rapier's kinematic character
//! controller with game-feel additions (acceleration, coyote time, jump
//! buffering, variable jump height, moving-platform carry).

use araria_core::geometry::{CHARACTER_HEIGHT, CHARACTER_RADIUS, yaw_toward};
use araria_core::{Instance, Vec3};
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};

/// Grace period after walking off a ledge during which a jump still works.
pub(crate) const COYOTE_TIME: f32 = 0.12;
/// How long a jump press is remembered before landing.
pub(crate) const JUMP_BUFFER: f32 = 0.14;
const GROUND_ACCEL: f32 = 70.0;
const AIR_ACCEL: f32 = 22.0;
const TURN_SPEED_DEG: f32 = 720.0;
/// Minimum fall speed that triggers the landing state.
const HARD_LANDING_SPEED: f32 = 9.0;
const LANDING_TIME: f32 = 0.22;

/// What a character is being asked to do this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterInput {
    /// Desired horizontal direction in world space (length 0..=1).
    pub move_dir: Vec3,
    pub run: bool,
    /// True while the jump control is held.
    pub jump: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterState {
    Idle,
    Walking,
    Running,
    Jumping,
    Falling,
    Landing,
    Seated,
    Dead,
}

impl CharacterState {
    pub fn name(self) -> &'static str {
        match self {
            CharacterState::Idle => "Idle",
            CharacterState::Walking => "Walking",
            CharacterState::Running => "Running",
            CharacterState::Jumping => "Jumping",
            CharacterState::Falling => "Falling",
            CharacterState::Landing => "Landing",
            CharacterState::Seated => "Seated",
            CharacterState::Dead => "Dead",
        }
    }
}

/// Tunables read from the character instance each step.
pub(crate) struct CharacterParams {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub jump_power: f32,
    pub auto_rotate: bool,
    pub anchored: bool,
    pub dead: bool,
}

impl CharacterParams {
    pub(crate) fn read(instance: &Instance) -> Self {
        CharacterParams {
            walk_speed: instance.f32("WalkSpeed"),
            run_speed: instance.f32("RunSpeed"),
            jump_power: instance.f32("JumpPower"),
            auto_rotate: instance.bool("AutoRotate"),
            anchored: instance.bool("Anchored"),
            dead: instance.number("Health") <= 0.0,
        }
    }
}

pub(crate) struct CharacterBody {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub scale: f32,
    /// Feet position.
    pub position: Vec3,
    pub facing: f32,
    pub velocity: Vec3,
    pub grounded: bool,
    /// Seconds since the character last stood on something.
    pub air_time: f32,
    pub jump_buffer: f32,
    pub jump_was_held: bool,
    /// True while rising from a jump (as opposed to an external impulse).
    pub jumping: bool,
    pub landing_timer: f32,
    /// Velocity of the surface underfoot (moving platforms, conveyors).
    pub carry: Vec3,
    pub state: CharacterState,
    pub input: CharacterInput,
    /// Set by the step when the corresponding event should fire.
    pub just_jumped: bool,
    pub just_landed: Option<f32>,
}

impl CharacterBody {
    pub(crate) fn height(&self) -> f32 {
        CHARACTER_HEIGHT * self.scale
    }

    pub(crate) fn radius(&self) -> f32 {
        CHARACTER_RADIUS * self.scale
    }

    /// Center of the collision capsule.
    pub(crate) fn center(&self) -> Vec3 {
        self.position + Vec3::Y * (self.height() * 0.5)
    }

    /// Advances velocity and timers; returns the translation to attempt
    /// (excluding any displacement of the surface underfoot).
    pub(crate) fn integrate(&mut self, params: &CharacterParams, gravity: f32, dt: f32) -> Vec3 {
        self.just_jumped = false;
        self.just_landed = None;
        if params.anchored {
            self.velocity = Vec3::ZERO;
            return Vec3::ZERO;
        }
        let input = if params.dead { CharacterInput::default() } else { self.input };
        let mut wish = Vec3::new(input.move_dir.x, 0.0, input.move_dir.z);
        if wish.length_squared() > 1.0 {
            wish = wish.normalize();
        }
        let speed = if input.run { params.run_speed } else { params.walk_speed };
        let target = wish * speed;
        let accel = if self.grounded { GROUND_ACCEL } else { AIR_ACCEL };
        let mut horizontal = Vec3::new(self.velocity.x, 0.0, self.velocity.z);
        let delta = target - horizontal;
        let max_step = accel * dt;
        horizontal += if delta.length() <= max_step { delta } else { delta.normalize() * max_step };

        if input.jump && !self.jump_was_held {
            self.jump_buffer = JUMP_BUFFER;
        }
        self.jump_was_held = input.jump;

        let mut vertical = self.velocity.y;
        if self.grounded && vertical <= 0.0 {
            // A small downward speed keeps the controller glued to slopes.
            vertical = -2.0;
        } else {
            vertical -= gravity * dt;
            // Releasing jump early cuts the ascent short.
            if vertical > 0.0 && self.jumping && !input.jump {
                vertical -= gravity * dt * 1.4;
            }
        }
        if self.jump_buffer > 0.0 && self.air_time <= COYOTE_TIME && params.jump_power > 0.0 {
            vertical = params.jump_power;
            self.jump_buffer = 0.0;
            self.air_time = COYOTE_TIME + 1.0;
            self.grounded = false;
            self.jumping = true;
            self.just_jumped = true;
            // Jumping off a moving surface keeps its momentum.
            horizontal += Vec3::new(self.carry.x, 0.0, self.carry.z);
        }
        if vertical <= 0.0 {
            self.jumping = false;
        }
        self.jump_buffer = (self.jump_buffer - dt).max(0.0);
        self.landing_timer = (self.landing_timer - dt).max(0.0);
        vertical = vertical.max(-90.0);

        self.velocity = Vec3::new(horizontal.x, vertical, horizontal.z);

        if params.auto_rotate && wish.length_squared() > 0.01 {
            let target_yaw = yaw_toward(wish);
            let mut diff = (target_yaw - self.facing).rem_euclid(360.0);
            if diff > 180.0 {
                diff -= 360.0;
            }
            let step = TURN_SPEED_DEG * dt;
            self.facing += diff.clamp(-step, step);
            self.facing = (self.facing + 180.0).rem_euclid(360.0) - 180.0;
        }
        self.velocity * dt
    }

    /// Applies the outcome of the collision-resolved move.
    pub(crate) fn resolve(&mut self, attempted: Vec3, applied: Vec3, grounded: bool, params: &CharacterParams, dt: f32) {
        let was_grounded = self.grounded;
        let fall_speed = -self.velocity.y;
        self.position += applied;
        // Never report "grounded" on the way up, or jumps would be eaten.
        self.grounded = grounded && self.velocity.y <= 0.0;
        if self.grounded {
            if !was_grounded && self.air_time > 0.05 {
                self.just_landed = Some(fall_speed.max(0.0));
                if fall_speed > HARD_LANDING_SPEED {
                    self.landing_timer = LANDING_TIME;
                }
            }
            self.air_time = 0.0;
            self.velocity.y = 0.0;
        } else {
            self.air_time += dt;
            // Bumped a ceiling: stop rising.
            if self.velocity.y > 0.0 && applied.y < attempted.y * 0.5 {
                self.velocity.y = 0.0;
            }
        }
        // Lose speed into walls instead of sticking to them.
        if dt > 0.0 {
            let attempted_h = Vec3::new(attempted.x, 0.0, attempted.z);
            let applied_h = Vec3::new(applied.x, 0.0, applied.z);
            if attempted_h.length() > 1e-5 && applied_h.length() < attempted_h.length() * 0.35 {
                let actual = applied_h / dt;
                self.velocity.x = actual.x;
                self.velocity.z = actual.z;
            }
        }
        self.state = self.classify(params);
    }

    fn classify(&self, params: &CharacterParams) -> CharacterState {
        if params.dead {
            return CharacterState::Dead;
        }
        if !self.grounded {
            return if self.velocity.y > 0.5 { CharacterState::Jumping } else if self.air_time > 0.08 { CharacterState::Falling } else { self.state };
        }
        if self.landing_timer > 0.0 {
            return CharacterState::Landing;
        }
        let speed = Vec3::new(self.velocity.x, 0.0, self.velocity.z).length();
        if speed < 0.3 {
            CharacterState::Idle
        } else if self.input.run && speed > params.walk_speed + 0.5 {
            CharacterState::Running
        } else {
            CharacterState::Walking
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> CharacterBody {
        CharacterBody {
            body: RigidBodyHandle::invalid(),
            collider: ColliderHandle::invalid(),
            scale: 1.0,
            position: Vec3::ZERO,
            facing: 0.0,
            velocity: Vec3::ZERO,
            grounded: true,
            air_time: 0.0,
            jump_buffer: 0.0,
            jump_was_held: false,
            jumping: false,
            landing_timer: 0.0,
            carry: Vec3::ZERO,
            state: CharacterState::Idle,
            input: CharacterInput::default(),
            just_jumped: false,
            just_landed: None,
        }
    }

    fn params() -> CharacterParams {
        CharacterParams { walk_speed: 6.0, run_speed: 10.0, jump_power: 10.0, auto_rotate: true, anchored: false, dead: false }
    }

    #[test]
    fn accelerates_to_walk_speed_and_turns() {
        let mut c = body();
        c.input.move_dir = Vec3::X;
        for _ in 0..60 {
            let attempt = c.integrate(&params(), 30.0, 1.0 / 60.0);
            c.resolve(attempt, attempt, true, &params(), 1.0 / 60.0);
        }
        assert!((c.velocity.x - 6.0).abs() < 1e-3, "{:?}", c.velocity);
        assert!((c.facing - 90.0).abs() < 1.0, "{}", c.facing);
        assert_eq!(c.state, CharacterState::Walking);
        c.input.run = true;
        for _ in 0..60 {
            let attempt = c.integrate(&params(), 30.0, 1.0 / 60.0);
            c.resolve(attempt, attempt, true, &params(), 1.0 / 60.0);
        }
        assert_eq!(c.state, CharacterState::Running);
    }

    #[test]
    fn jump_requires_ground_or_coyote_time() {
        let mut c = body();
        c.input.jump = true;
        let attempt = c.integrate(&params(), 30.0, 1.0 / 60.0);
        assert!(c.just_jumped && attempt.y > 0.0);
        c.resolve(attempt, attempt, false, &params(), 1.0 / 60.0);
        assert_eq!(c.state, CharacterState::Jumping);

        // Holding jump in mid-air does not jump again.
        let attempt = c.integrate(&params(), 30.0, 1.0 / 60.0);
        assert!(!c.just_jumped);
        c.resolve(attempt, attempt, false, &params(), 1.0 / 60.0);

        // Coyote time: just walked off a ledge.
        let mut c = body();
        c.grounded = false;
        c.air_time = COYOTE_TIME * 0.5;
        c.input.jump = true;
        c.integrate(&params(), 30.0, 1.0 / 60.0);
        assert!(c.just_jumped);

        // Too late.
        let mut c = body();
        c.grounded = false;
        c.air_time = COYOTE_TIME * 3.0;
        c.input.jump = true;
        c.integrate(&params(), 30.0, 1.0 / 60.0);
        assert!(!c.just_jumped);
    }

    #[test]
    fn hard_landings_are_reported() {
        let mut c = body();
        c.grounded = false;
        c.air_time = 1.0;
        c.velocity.y = -15.0;
        let attempt = c.integrate(&params(), 30.0, 1.0 / 60.0);
        c.resolve(attempt, Vec3::ZERO, true, &params(), 1.0 / 60.0);
        assert!(c.just_landed.is_some_and(|speed| speed > 15.0));
        assert_eq!(c.state, CharacterState::Landing);
    }
}
