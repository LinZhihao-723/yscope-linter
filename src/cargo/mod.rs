//! The `cargo yscope-linter` mode: discovering a cargo workspace's packages, their files and their
//! `Cargo.toml` settings.

mod metadata;
mod plan;
mod settings;

pub use plan::Plan;
pub use settings::Settings;
