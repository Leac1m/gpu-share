//! Shared wire types for GPU Share.
//!
//! Both the host and the client link this crate, so a message can never drift
//! out of sync between the two sides of one machine. It does **not** protect
//! against drift between two different *installs* talking over the network —
//! that is what [`PROTOCOL_VERSION`] and the golden fixtures under `tests/`
//! are for. See `docs/roadmap.md`, Phase 0 and Phase 3, for the tasks that
//! build on this crate.

pub mod api;
pub mod discovery;
pub mod gatekeeper;
pub mod models;
pub mod pairing;

/// The wire protocol version this build speaks. Bump it whenever a message
/// shape changes in a way that isn't backward compatible, and see spec
/// §6.10 ("Protocol versioning") before doing so.
pub const PROTOCOL_VERSION: &str = "1.0";

/// TCP port the gatekeeper's HTTPS server listens on (roadmap D7).
pub const GATEKEEPER_PORT: u16 = 50123;

/// Whether an install speaking `other` can talk to this one: same major version.
pub fn compatible(other: &str) -> bool {
    let major = |v: &str| v.split('.').next().map(str::to_string);
    major(other).is_some() && major(other) == major(PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_major_version_is_compatible() {
        assert!(super::compatible("1.0"));
        assert!(super::compatible("1.7"));
        assert!(!super::compatible("2.0"));
        assert!(!super::compatible(""));
    }
}
