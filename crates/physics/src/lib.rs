//! # Araria physics
//!
//! Mirrors the parts and characters under `Workspace` into a Rapier world,
//! steps it, and writes the results back into the [`DataModel`]:
//!
//! * anchored parts are fixed bodies (promoted to kinematic the first time a
//!   script moves them, so moving platforms carry what stands on them),
//! * unanchored parts are dynamic bodies,
//! * characters are capsules driven by a kinematic character controller,
//! * `Touched` / `TouchEnded` events are raised for overlapping pairs,
//! * ray casts and overlap tests are available to scripts and the editor.
//!
//! The crate has no rendering or scripting dependencies and runs headless.

mod character;
mod world;

pub use character::{CharacterInput, CharacterState};
pub use world::{Physics, RayHit};
