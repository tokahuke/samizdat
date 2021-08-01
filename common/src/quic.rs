use std::fs;
use quinn::ClientConfig;
use std::sync::Arc;
use rustls::ServerCertVerified;

// Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
struct SkipCertificationVerification;

impl rustls::ServerCertVerifier for SkipCertificationVerification {
    fn verify_server_cert(
        &self, _: &rustls::RootCertStore, _: &[rustls::Certificate], _: webpki::DNSNameRef, _: &[u8],
    ) -> Result<rustls::ServerCertVerified, rustls::TLSError> {
        Ok(ServerCertVerified::assertion())
    }
}

pub fn insecure() -> ClientConfig {
    let mut cfg = quinn::ClientConfigBuilder::default().build();

    // Get a mutable reference to the 'crypto' config in the 'client config'.
    let tls_cfg: &mut rustls::ClientConfig =
        std::sync::Arc::get_mut(&mut cfg.crypto).unwrap();

    // Change the certification verifier.
    // This is only available when compiled with the 'dangerous_configuration' feature.
    tls_cfg
        .dangerous()
        .set_certificate_verifier(Arc::new(SkipCertificationVerification));
    cfg
}

pub fn generate_self_signed_cert(cert_path: &str, key_path: &str) -> (quinn::Certificate, quinn::PrivateKey) {
    // Generate dummy certificate.
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let serialized_key = certificate.serialize_private_key_der();
    let serialized_certificate = certificate.serialize_der().unwrap();

    // Write to files.
    fs::write(&cert_path, &serialized_certificate).expect("failed to write certificate");
    fs::write(&key_path, &serialized_key).expect("failed to write private key");

    let cert = quinn::Certificate::from_der(&serialized_certificate).expect("failed to load cert");
    let key = quinn::PrivateKey::from_der(&serialized_key).expect("failed to load key");

    (cert, key)
}
