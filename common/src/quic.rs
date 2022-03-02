use quinn::{
    ClientConfig, Endpoint, IdleTimeout, Incoming, NewConnection, ServerConfig, TransportConfig,
    VarInt,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// "I am Spartacus!"
const DEFAULT_SERVER_NAME: &str = "spartacus";

// We don't need all trust built into QUIC. Using "dangerous configuration", which is simpler.
// Taken from the tutorial: https://quinn-rs.github.io/quinn/quinn/certificate.html

// Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

fn transport_config() -> TransportConfig {
    const IDLE_TIMEOUT_MS: u32 = 2 * 60 * 1_000;

    let mut transport = TransportConfig::default();
    transport.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(IDLE_TIMEOUT_MS))));
    transport.keep_alive_interval(Some(Duration::from_millis(IDLE_TIMEOUT_MS as u64 / 4)));

    transport
}

fn client_config() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    let mut client_config = ClientConfig::new(Arc::new(crypto));
    client_config.transport = Arc::new(transport_config());

    client_config
}

fn server_config() -> ServerConfig {
    let cert = rcgen::generate_simple_self_signed(vec![DEFAULT_SERVER_NAME.into()]).unwrap();
    let key = rustls::PrivateKey(cert.serialize_private_key_der());
    let cert = rustls::Certificate(cert.serialize_der().unwrap());

    let mut server_config =
        quinn::ServerConfig::with_single_cert(vec![cert], key).expect("can build server config");
    server_config.transport = Arc::new(transport_config());

    server_config
}

pub fn new_default(bind_addr: SocketAddr) -> (Endpoint, Incoming) {
    let (mut endpoint, incoming) =
        Endpoint::server(server_config(), bind_addr).expect("can bind endpoint");
    endpoint.set_default_client_config(client_config());

    (endpoint, incoming)
}

pub async fn connect(
    endpoint: &Endpoint,
    remote_addr: SocketAddr,
) -> Result<NewConnection, crate::Error> {
    Ok(endpoint
        .connect(remote_addr, DEFAULT_SERVER_NAME)
        .expect("failed to start connecting")
        .await?)
}
