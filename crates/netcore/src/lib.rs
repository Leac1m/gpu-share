//! Device identity and pinned TLS (spec §6.7), shared by the host and the client.
//!
//! Every install has one self-signed certificate. Its SHA-256 fingerprint is the device's
//! permanent id, announced on the LAN and stored when two devices pair. There is no CA: each
//! side checks the other's certificate by fingerprint ([`tls`]).

pub mod identity;
pub mod tls;

pub use identity::Identity;
