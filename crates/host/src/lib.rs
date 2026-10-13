//! Share mode (spec §6.2–6.5, §6.8): everything a host runs.

pub mod announce;
pub mod awake;
pub mod clamp;
pub mod core;
pub mod engine;
pub mod engine_setup;
pub mod gatekeeper;
pub mod gguf;
#[cfg(windows)]
mod jobobject;
pub mod library;
pub mod line;
pub mod pairing;
pub mod policy;
pub mod runtime;
pub mod settings;
pub mod store;
pub mod world;
