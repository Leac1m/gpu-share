//! rustls configurations that trust certificates by fingerprint instead of by CA (`P4-T2`).
//!
//! * The host's server config asks every client for its certificate but doesn't require one
//!   (a subnet scan or a first contact has none to offer). Which endpoints need a *paired*
//!   certificate is decided above TLS, by the gatekeeper.
//! * A client config pins the host's fingerprint, the one it learned from the announcement or
//!   from pairing. [`ServerPin::Capture`] is for "Add by IP", where the fingerprint isn't known
//!   yet: it accepts any certificate and records what it saw, and the pairing code then catches
//!   a man in the middle.
//!
//! Signatures are still verified in both directions, so a peer must hold the private key of
//! the certificate it presents.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme};

use crate::Identity;
use protocol::pairing::fingerprint;

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The host's TLS server config.
pub fn server_config(identity: &Identity) -> Result<ServerConfig, rustls::Error> {
    let p = provider();
    let verifier = Arc::new(AnyClientCert { algs: p.signature_verification_algorithms });
    let mut cfg = ServerConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.cert()], identity.key())?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

/// How a client decides whether to trust the host's certificate.
#[derive(Clone, Debug)]
pub enum ServerPin {
    /// Only this fingerprint is accepted.
    Fingerprint(String),
    /// Anything is accepted and the fingerprint is recorded. First contact by IP only.
    Capture(Arc<Mutex<Option<String>>>),
}

/// A client config that presents `identity` and checks the host per `pin`.
pub fn client_config(identity: &Identity, pin: ServerPin) -> Result<ClientConfig, rustls::Error> {
    let p = provider();
    let verifier = Arc::new(PinnedServer { pin, algs: p.signature_verification_algorithms });
    let mut cfg = ClientConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![identity.cert()], identity.key())?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

/// A client config with no certificate of its own: what a device that isn't GPU Share (or a
/// curious script) looks like to the gatekeeper. Used by tests.
pub fn anonymous_client_config(pin: ServerPin) -> Result<ClientConfig, rustls::Error> {
    let p = provider();
    let verifier = Arc::new(PinnedServer { pin, algs: p.signature_verification_algorithms });
    let mut cfg = ClientConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

#[derive(Debug)]
struct PinnedServer {
    pin: ServerPin,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let seen = fingerprint(end_entity);
        match &self.pin {
            ServerPin::Fingerprint(want) if *want == seen => Ok(ServerCertVerified::assertion()),
            ServerPin::Fingerprint(_) => Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )),
            ServerPin::Capture(slot) => {
                *slot.lock().unwrap() = Some(seen);
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

#[derive(Debug)]
struct AnyClientCert {
    algs: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyClientCert {
    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // Identity, not trust: the gatekeeper looks the fingerprint up in the paired list.
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// The fingerprint of the certificate a peer presented, if any.
pub fn peer_fingerprint(certs: Option<&[CertificateDer<'_>]>) -> Option<String> {
    certs.and_then(|c| c.first()).map(|c| fingerprint(c))
}

/// The name a client puts in SNI. Hosts are reached by IP and checked by fingerprint, so this
/// is a fixed placeholder that never resolves.
pub fn server_name() -> ServerName<'static> {
    ServerName::try_from("gpu-share.invalid").unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    /// Runs one handshake over an in-memory pipe. Returns the client-cert fingerprint the
    /// server saw, or the client's handshake error.
    async fn handshake(server_id: &Identity, client_id: &Identity, pin: ServerPin) -> Result<Option<String>, String> {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let acceptor = TlsAcceptor::from(Arc::new(server_config(server_id).unwrap()));
        let connector = TlsConnector::from(Arc::new(client_config(client_id, pin).unwrap()));
        let server = tokio::spawn(async move {
            let mut s = acceptor.accept(a).await.map_err(|e| e.to_string())?;
            let seen = peer_fingerprint(s.get_ref().1.peer_certificates());
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
            Ok::<_, String>(seen)
        });
        let mut c = connector.connect(server_name(), b).await.map_err(|e| e.to_string())?;
        c.write_all(b"ping").await.map_err(|e| e.to_string())?;
        c.flush().await.unwrap();
        server.await.unwrap()
    }

    #[tokio::test]
    async fn the_right_pin_connects_and_the_host_learns_the_client_id() {
        let host = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let seen = handshake(&host, &client, ServerPin::Fingerprint(host.fingerprint().into())).await.unwrap();
        assert_eq!(seen.as_deref(), Some(client.fingerprint()));
    }

    #[tokio::test]
    async fn a_different_host_certificate_is_refused() {
        let host = Identity::generate().unwrap();
        let impostor = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let err = handshake(&impostor, &client, ServerPin::Fingerprint(host.fingerprint().into())).await;
        assert!(err.is_err(), "a pinned client must not talk to another certificate");
    }

    #[tokio::test]
    async fn capture_records_what_it_saw() {
        let host = Identity::generate().unwrap();
        let client = Identity::generate().unwrap();
        let slot = Arc::new(Mutex::new(None));
        handshake(&host, &client, ServerPin::Capture(slot.clone())).await.unwrap();
        assert_eq!(slot.lock().unwrap().as_deref(), Some(host.fingerprint()));
    }
}
