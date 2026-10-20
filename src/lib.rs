//! KiCad-to-SolidCraft orchestration. Geometry and native documents belong to the upstream tools.
pub mod assembly;
pub mod cad;
pub mod config;
mod container;
mod copper;
pub mod error;
pub mod inventory;
pub mod progress;
pub mod staging;
