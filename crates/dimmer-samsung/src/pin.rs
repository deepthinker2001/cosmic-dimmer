//! Certificate pinning for the TV's TLS port.
//!
//! The TV's certificate on 8002 chains to Samsung's private
//! "`SmartViewSDK` Root CA G2" and names no IP address or hostname, so ordinary verification can't
//! succeed. Instead of switching verification off, the tool pins the SHA-256 of
//! the TV's own (leaf) certificate when the user pairs (trust on first use,
//! at the moment the user is at the TV accepting the prompt), and afterwards
//! refuses any other certificate. Handshake signatures are still checked
//! against the presented certificate, so a server must hold its private key.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// A certificate's SHA-256.
pub type Fingerprint = [u8; 32];

#[must_use]
pub fn fingerprint(cert_der: &[u8]) -> Fingerprint {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert_der);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

#[must_use]
pub fn to_hex(fp: &Fingerprint) -> String {
    fp.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[must_use]
pub fn from_hex(text: &str) -> Option<Fingerprint> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// What the verifier accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Only this certificate.
    Pinned(Fingerprint),
    /// Any certificate, recording it to be pinned. Only for `pair`.
    TrustOnFirstUse,
}

#[derive(Debug)]
pub struct PinVerifier {
    policy: Policy,
    seen: Arc<Mutex<Option<Fingerprint>>>,
    provider: Arc<CryptoProvider>,
}

impl PinVerifier {
    /// A verifier, and a handle to read the certificate it saw.
    #[must_use]
    pub fn new(policy: Policy) -> (Arc<Self>, Arc<Mutex<Option<Fingerprint>>>) {
        let seen = Arc::new(Mutex::new(None));
        let verifier = Arc::new(Self {
            policy,
            seen: Arc::clone(&seen),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        });
        (verifier, seen)
    }

    /// A rustls client config using this verifier and the ring provider.
    ///
    /// # Errors
    /// When rustls can't build the config (no usable protocol versions).
    pub fn client_config(self: &Arc<Self>) -> Result<rustls::ClientConfig, rustls::Error> {
        Ok(
            rustls::ClientConfig::builder_with_provider(Arc::clone(&self.provider))
                .with_safe_default_protocol_versions()?
                .dangerous()
                .with_custom_certificate_verifier(Arc::clone(self) as Arc<dyn ServerCertVerifier>)
                .with_no_client_auth(),
        )
    }

    /// The decision for a leaf certificate, apart from rustls (for tests).
    ///
    /// # Errors
    /// When it isn't the pinned certificate.
    pub fn check(&self, end_entity: &[u8]) -> Result<(), rustls::Error> {
        let fp = fingerprint(end_entity);
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(fp);
        }
        match self.policy {
            Policy::TrustOnFirstUse => Ok(()),
            Policy::Pinned(expected) if expected == fp => Ok(()),
            Policy::Pinned(_) => Err(rustls::Error::General(
                "certificate does not match the pinned fingerprint".into(),
            )),
        }
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.check(end_entity.as_ref())
            .map(|()| ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let fp = fingerprint(b"certificate");
        let hex = to_hex(&fp);
        assert_eq!(hex.len(), 64);
        assert_eq!(from_hex(&hex), Some(fp));
        assert_eq!(from_hex(&hex.to_uppercase()), Some(fp));
        assert_eq!(from_hex("ab"), None);
        assert_eq!(from_hex(&"zz".repeat(32)), None);
        assert_eq!(from_hex(&"é".repeat(32)), None);
    }

    #[test]
    fn known_digest() {
        // SHA-256("abc")
        assert_eq!(
            to_hex(&fingerprint(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pinned_accepts_only_its_certificate() {
        let (v, seen) = PinVerifier::new(Policy::Pinned(fingerprint(b"tv")));
        assert!(v.check(b"tv").is_ok());
        assert!(v.check(b"impostor").is_err());
        assert_eq!(*seen.lock().unwrap(), Some(fingerprint(b"impostor")));
    }

    #[test]
    fn first_use_records() {
        let (v, seen) = PinVerifier::new(Policy::TrustOnFirstUse);
        assert!(v.check(b"tv").is_ok());
        assert_eq!(*seen.lock().unwrap(), Some(fingerprint(b"tv")));
        assert!(v.client_config().is_ok());
    }
}
