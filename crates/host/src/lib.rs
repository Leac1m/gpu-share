//! Share mode (spec §6.2–6.5, §6.8): everything a host runs.

pub mod announce;
pub mod clamp;
pub mod core;
pub mod engine;
pub mod gatekeeper;
#[cfg(windows)]
mod jobobject;
pub mod line;
pub mod pairing;
pub mod runtime;
pub mod settings;
pub mod store;
