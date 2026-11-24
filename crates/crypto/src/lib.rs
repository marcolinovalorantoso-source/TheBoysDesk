//! Cryptographic primitives: device identity, ephemeral TLS 1.3 certificates,
//! and constant-time session PIN verification.

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::Rng;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("certificate generation failed: {0}")]
    CertGen(#[from] rcgen::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
}

/// Device identity backed by an Ed25519 keypair and a TLS 1.3 certificate.
pub struct DeviceIdentity {
    pub device_id: String,
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivatePkcs8KeyDer<'static>,
}

impl DeviceIdentity {
    /// Generates a fresh Ed25519-backed identity and self-signed TLS certificate.
    pub fn generate() -> Result<Self, CryptoError> {
        let signing = SigningKey::generate(&mut OsRng);
        let pub_bytes = signing.verifying_key().to_bytes();
        let digest = Sha256::digest(pub_bytes);

        // Derive a 9-digit human-friendly ID from the first 4 bytes of SHA-256(pubkey).
        let num = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000_000;
        let device_id = format!("{:03} {:03} {:03}", num / 1_000_000, (num / 1_000) % 1_000, num % 1_000);

        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let mut params = rcgen::CertificateParams::new(vec!["anydesk-alt.p2p".to_string()])?;
        params.distinguished_name.push(rcgen::DnType::CommonName, &device_id);
        let cert = params.self_signed(&key_pair)?;

        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(key_pair.serialize_der());

        Ok(Self {
            device_id,
            cert_der,
            key_der,
        })
    }

    /// Builds a `rustls::ServerConfig` for accepting incoming P2P QUIC connections.
    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, CryptoError> {
        let mut cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![self.cert_der.clone()],
            PrivateKeyDer::Pkcs8(self.key_der.clone_key()),
        )?;

        cfg.alpn_protocols = vec![b"adp2p/1".to_vec()];
        Ok(Arc::new(cfg))
    }

    /// Builds a `rustls::ClientConfig` for P2P QUIC connections.
    ///
    /// Self-signed peer certificates are accepted at the TLS layer (while still
    /// verifying TLS 1.3 handshake signatures), and peer authorization is enforced
    /// via the challenge-response PIN + explicit host consent.
    pub fn client_config() -> Result<Arc<rustls::ClientConfig>, CryptoError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Arc::new(P2pSelfSignedVerifier {
            provider: provider.clone(),
        });
        let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        cfg.alpn_protocols = vec![b"adp2p/1".to_vec()];
        Ok(Arc::new(cfg))
    }
}

/// Generates a random 6-digit numeric session PIN.
pub fn generate_session_pin() -> String {
    let n: u32 = OsRng.gen_range(0..1_000_000);
    format!("{n:06}")
}

/// Generates a random 32-byte authentication challenge nonce.
pub fn generate_nonce() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    OsRng.fill(&mut nonce);
    nonce
}

/// Computes the SHA-256 fingerprint of a DER-encoded X.509 certificate for TLS channel binding.
pub fn cert_fingerprint_sha256(cert_der: &[u8]) -> [u8; 32] {
    Sha256::digest(cert_der).into()
}

/// Computes `SHA-256(nonce || host_cert_sha256 || pin)` so the plaintext PIN is never
/// transmitted and any MitM substituting the host TLS certificate is detected.
pub fn compute_pin_proof(nonce: &[u8; 32], host_cert_sha256: &[u8; 32], pin: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(nonce);
    hasher.update(host_cert_sha256);
    hasher.update(pin.trim().as_bytes());
    hasher.finalize().into()
}

/// Constant-time verification of a peer's `pin_proof` bound to the host's TLS certificate.
pub fn verify_pin_proof(
    nonce: &[u8; 32],
    host_cert_sha256: &[u8; 32],
    expected_pin: &str,
    proof: &[u8; 32],
) -> bool {
    let expected = compute_pin_proof(nonce, host_cert_sha256, expected_pin);
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(proof.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[derive(Debug)]
struct P2pSelfSignedVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for P2pSelfSignedVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
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
        rustls::crypto::verify_tls13_signature(
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
    fn pin_proof_verifies_and_rejects_wrong_pin_or_mitm_cert() {
        let nonce = generate_nonce();
        let pin = generate_session_pin();
        let real_cert = cert_fingerprint_sha256(b"real-host-cert");
        let mitm_cert = cert_fingerprint_sha256(b"mitm-attacker-cert");
        assert_eq!(pin.len(), 6);

        let proof = compute_pin_proof(&nonce, &real_cert, &pin);
        assert!(verify_pin_proof(&nonce, &real_cert, &pin, &proof));
        assert!(!verify_pin_proof(&nonce, &real_cert, "000000_wrong", &proof));
        // MitM certificate substitution must fail even with the right PIN
        assert!(!verify_pin_proof(&nonce, &mitm_cert, &pin, &proof));
    }

    #[test]
    fn identity_builds_tls_configs() {
        let id = DeviceIdentity::generate().unwrap();
        assert_eq!(id.device_id.len(), 11); // "XXX XXX XXX"
        let _srv = id.server_config().unwrap();
        let _cli = DeviceIdentity::client_config().unwrap();
    }
}
