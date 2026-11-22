//! # Accrete core
//!
//! Deterministic, agent-based simulation of an emergent civilization. This
//! crate is pure Rust with no platform-specific code and no rendering.
//!
//! **Determinism contract:** the same seed, configuration, and crate version
//! produce bit-identical state on every platform. The crate uses only exactly
//! specified float operations (`+ - * /`, `sqrt`, comparisons, rounding, `as`),
//! never `std` transcendentals or `mul_add` (enforced by `clippy.toml`), keeps
//! all reductions in a fixed order, draws randomness from named ChaCha8
//! streams, and hashes state with FNV-1a 64 over a canonical encoding.

pub mod agents;
pub mod config;
pub mod geom;
pub mod hash;
pub mod inventory;
pub mod rng;
pub mod scenario;
pub mod seed;
pub mod simulation;
pub mod world;

pub use config::Config;
pub use geom::{Dims, Pos};
pub use simulation::Simulation;
pub use world::{World, generate_world};
