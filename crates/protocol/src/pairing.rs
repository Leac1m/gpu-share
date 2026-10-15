//! Pairing (spec §6.7): `POST /api/pair` bodies and the 6-digit code both screens show.
//!
//! The code is derived, not chosen: each side hashes the host's certificate fingerprint *as it
//! sees it*, the client's fingerprint as it sees it, and a nonce the client picks. If someone in
//! the middle terminates TLS on either leg, one of the two fingerprints differs between the
//! sides and so does the code, which is what the owner is checking when they compare screens.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `POST /api/pair` request. The client's identity is its TLS client certificate; the body
/// only carries what the owner's popup shows and the nonce for the code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairRequest {
    pub protocol: String,
    /// Shown in the owner's popup: "<name> wants to use your GPU".
    pub name: String,
    /// 32 hex characters (16 random bytes) chosen by the client.
    pub nonce: String,
}

/// `POST /api/pair` response once the owner clicks Allow. The token goes in
/// `Authorization: Bearer` on every paired request, alongside the pinned client certificate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairResponse {
    pub token: String,
}

/// SHA-256 fingerprint of a DER certificate, lowercase hex. A device's permanent id.
pub fn fingerprint(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

/// The 6-digit code shown on both screens, zero-padded.
pub fn pairing_code(host_fingerprint: &str, client_fingerprint: &str, nonce: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"gpu-share pairing code v1\0");
    h.update(host_fingerprint.as_bytes());
    h.update(b"\0");
    h.update(client_fingerprint.as_bytes());
    h.update(b"\0");
    h.update(nonce.as_bytes());
    let d = h.finalize();
    let n = u64::from_be_bytes(d[..8].try_into().unwrap()) % 1_000_000;
    format!("{n:06}")
}

/// A nonce is exactly 32 lowercase hex characters.
pub fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CLIENT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn both_sides_compute_the_same_six_digits() {
        let nonce = "0123456789abcdef0123456789abcdef";
        let a = pairing_code(HOST, CLIENT, nonce);
        assert_eq!(a.len(), 6);
        assert!(a.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(a, pairing_code(HOST, CLIENT, nonce));
    }

    #[test]
    fn a_different_fingerprint_on_either_side_changes_the_code() {
        let nonce = "0123456789abcdef0123456789abcdef";
        let real = pairing_code(HOST, CLIENT, nonce);
        let mitm = "cc".repeat(32);
        assert_ne!(real, pairing_code(&mitm, CLIENT, nonce));
        assert_ne!(real, pairing_code(HOST, &mitm, nonce));
        assert_ne!(real, pairing_code(CLIENT, HOST, nonce), "order matters");
    }

    #[test]
    fn the_code_is_pinned_so_both_installs_agree_across_versions() {
        // Golden value: if this changes, old and new installs show different codes.
        assert_eq!(pairing_code(HOST, CLIENT, "0123456789abcdef0123456789abcdef"), GOLDEN);
    }
    const GOLDEN: &str = "208790";

    #[test]
    fn nonce_validation() {
        assert!(valid_nonce("0123456789abcdef0123456789abcdef"));
        assert!(!valid_nonce("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!valid_nonce("short"));
    }

    #[test]
    fn fingerprint_is_lowercase_sha256_hex() {
        assert_eq!(
            fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
