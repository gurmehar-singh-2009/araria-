//! # Araria platform
//!
//! The consumer-facing shell around the engine: the design system
//! ([`theme`], [`widgets`]), the local [`library`] of games and projects,
//! [`settings`], and the screens. It depends only on `egui`, the data
//! model and storage, not on the renderer.

pub mod icons;
pub mod library;
pub mod settings;
pub mod theme;
pub mod widgets;

pub use library::{GameEntry, GameKey, Library, LoadedGame, Origin};
pub use settings::Settings;
