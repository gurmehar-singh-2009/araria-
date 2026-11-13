//! # Araria core
//!
//! The engine-independent heart of Araria: the instance tree
//! ([`DataModel`]), the class [`schema`], property [`Variant`] values, and
//! the versioned [`scene`] and [`project`] file formats.
//!
//! This crate has no dependency on the renderer, the physics engine or the
//! scripting runtime, so it can be used by tools, tests and servers alike.

pub mod assets;
pub mod datamodel;
pub mod geometry;
pub mod input;
pub mod project;
pub mod scene;
pub mod schema;
pub mod types;

pub use datamodel::{Change, DataModel, Event, Instance, ModelError, ModelResult};
pub use glam::{EulerRot, Mat3, Quat, Vec2, Vec3};
pub use schema::{AssetKind, ClassId, ClassInfo, MemberDoc, PropDef, PropType, schema};
pub use types::{Color, InstanceId, UDim2, Variant, euler_deg_to_quat, quat_to_euler_deg};
