//! The per-install key pair and self-signed certificate (`P4-T1`).

use std::fs;
use std::path::Path;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("could not read or write the identity file: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not create a certificate: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("the identity file is damaged")]
    Corrupt,
}

/// This device's certificate and private key.
#[derive(Clone)]
pub struct Identity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    fingerprint: String,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("fingerprint", &self.fingerprint).finish_non_exhaustive()
    }
}

const FILE: &str = "identity.pem";

impl Identity {
    /// A fresh ECDSA P-256 key and a self-signed certificate. The certificate's fields don't
    /// matter to anyone (peers pin the fingerprint), so they are fixed and anonymous.
    pub fn generate() -> Result<Self, IdentityError> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let mut params = rcgen::CertificateParams::new(vec!["gpu-share.invalid".to_string()])?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params.distinguished_name.push(rcgen::DnType::CommonName, "GPU Share device");
        params.not_before = rcgen::date_time_ymd(2026, 1, 1);
        params.not_after = rcgen::date_time_ymd(2126, 1, 1);
        let cert = params.self_signed(&key)?;
        Ok(Self::from_der(cert.der().to_vec(), key.serialize_der()))
    }

    fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let fingerprint = protocol::pairing::fingerprint(&cert_der);
        Self { cert_der, key_der, fingerprint }
    }

    /// Loads `identity.pem` from `dir`, creating it on first run. The id must never change
    /// once paired devices know it, so a damaged file is an error, not a silent regeneration.
    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        let path = dir.join(FILE);
        if path.exists() {
            return Self::from_pem(&fs::read_to_string(&path)?);
        }
        fs::create_dir_all(dir)?;
        let id = Self::generate()?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        fs::write(&tmp, id.to_pem())?;
        restrict_permissions(&tmp)?;
        fs::rename(&tmp, &path)?;
        Ok(id)
    }

    fn to_pem(&self) -> String {
        format!("{}{}", pem_block("CERTIFICATE", &self.cert_der), pem_block("PRIVATE KEY", &self.key_der))
    }

    fn from_pem(text: &str) -> Result<Self, IdentityError> {
        let cert = pem_decode(text, "CERTIFICATE").ok_or(IdentityError::Corrupt)?;
        let key = pem_decode(text, "PRIVATE KEY").ok_or(IdentityError::Corrupt)?;
        // make sure the key is usable before handing it out
        rcgen::KeyPair::try_from(key.as_slice()).map_err(|_| IdentityError::Corrupt)?;
        Ok(Self::from_der(cert, key))
    }

    /// Lowercase hex SHA-256 of the certificate: this device's permanent id.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn cert(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone()))
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    // Per-user app data on Windows is already private to the user.
    Ok(())
}

// --- minimal PEM, to avoid another dependency for two fixed block types ---

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn pem_block(label: &str, der: &[u8]) -> String {
    let mut b64 = String::new();
    for chunk in der.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                b64.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    let lines: Vec<&str> = b64.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", lines.join("\n"))
}

fn pem_decode(text: &str, label: &str) -> Option<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = text.find(&begin)? + begin.len();
    let stop = start + text[start..].find(&end)?;
    let body: Vec<u8> = text[start..stop].bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !body.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    for quad in body.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &c) in quad.iter().enumerate() {
            let v = match c {
                b'=' => {
                    pad += 1;
                    0
                }
                _ => B64.iter().position(|&x| x == c)? as u32,
            };
            n |= v << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint().len(), 64);
    }

    #[test]
    fn two_installs_get_different_ids() {
        assert_ne!(Identity::generate().unwrap().fingerprint(), Identity::generate().unwrap().fingerprint());
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), "garbage").unwrap();
        assert!(matches!(Identity::load_or_create(dir.path()), Err(IdentityError::Corrupt)));
    }

    #[test]
    fn pem_round_trips_every_length() {
        for len in 0..10 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37) as u8).collect();
            assert_eq!(pem_decode(&pem_block("X", &data), "X").unwrap(), data);
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_create(dir.path()).unwrap();
        let mode = fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
