//! Default configuration for QUIC used by Samizdat. Samizdat has its own way of dealing
//! with security. Therefore, much of the complexity involving security in QUIC can be igonred.

use quinn::crypto::rustls::QuicClientConfig;
use quinn::{
    ClientConfig, Connection, Endpoint, IdleTimeout, ServerConfig, TransportConfig, VarInt,
};
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// "I am Spartacus!"
const DEFAULT_SERVER_NAME: &str = "spartacus";

/// We don't need all trust built into QUIC. Using "dangerous configuration", which is simpler.
/// Taken from the tutorial: https://quinn-rs.github.io/quinn/quinn/certificate.html
///
/// Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

static CRYPTO_PROVIDER_INSTALLED: OnceLock<()> = OnceLock::new();

fn install_crypto_provider() {
    if !CRYPTO_PROVIDER_INSTALLED.get().is_some() {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .ok();
        CRYPTO_PROVIDER_INSTALLED.set(()).ok();
    }
}

/// Creates a default transport configuration for QUIC.
fn transport_config(keep_alive: bool) -> TransportConfig {
    const IDLE_TIMEOUT_MS: u32 = 10_000;

    let mut transport = TransportConfig::default();
    transport.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(IDLE_TIMEOUT_MS))));

    if keep_alive {
        transport.keep_alive_interval(Some(Duration::from_millis(IDLE_TIMEOUT_MS as u64 / 4)));
    } else {
        transport.keep_alive_interval(None);
    }

    transport
}

/// Creates a default client configuration for QUIC.
fn client_config(keep_alive: bool) -> ClientConfig {
    install_crypto_provider();

    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    let mut client_config =
        ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto).unwrap()));
    client_config.transport_config(Arc::new(transport_config(keep_alive)));

    client_config
}

/// Creates a default server configuration for QUIC.
fn server_config() -> ServerConfig {
    install_crypto_provider();

    let cert = rcgen::generate_simple_self_signed(vec![DEFAULT_SERVER_NAME.into()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert);
    let priv_key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut server_config = quinn::ServerConfig::with_single_cert(vec![cert_der], priv_key.into())
        .expect("can build server config");
    server_config.transport = Arc::new(transport_config(true));

    server_config
}

/// Opens a new QUIC listener on `bind_addr`.
pub fn new_default(bind_addr: SocketAddr) -> Endpoint {
    let mut endpoint = Endpoint::server(server_config(), bind_addr).expect("can bind endpoint");
    endpoint.set_default_client_config(client_config(true));

    endpoint
}

/// Connects to a remote host using an [`Endpoint`].
pub async fn connect(
    endpoint: &Endpoint,
    remote_addr: SocketAddr,
    keep_alive: bool,
) -> Result<Connection, crate::Error> {
    Ok(endpoint
        .connect_with(client_config(keep_alive), remote_addr, DEFAULT_SERVER_NAME)
        .expect("failed to start connecting")
        .await?)
}
