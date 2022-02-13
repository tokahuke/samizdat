use quinn::{Endpoint, Incoming, NewConnection, ServerConfig};
// use rustls::client::{ServerCertVerified, ServerCertVerifier, ServerName};
use std::net::SocketAddr;
// use std::time::SystemTime;

/// Everybody is spartacus.
const DEFAULT_SERVER_NAME: &str = "spartacus";

// // Implementation of `ServerCertVerifier` that verifies everything as trustworthy.
// struct SkipCertificationVerification;

// impl ServerCertVerifier for SkipCertificationVerification {
//     fn verify_server_cert(
//         &self,
//         _: &rustls::Certificate,
//         _: &[rustls::Certificate],
//         _: &ServerName,
//         _: &mut dyn Iterator<Item = &[u8]>,
//         _: &[u8],
//         _: SystemTime,
//     ) -> Result<ServerCertVerified, rustls::Error> {
//         Ok(ServerCertVerified::assertion())
//     }
// }

pub fn server_config() -> ServerConfig {
    let cert = rcgen::generate_simple_self_signed(vec![DEFAULT_SERVER_NAME.into()]).unwrap();
    let key = rustls::PrivateKey(cert.serialize_private_key_der());
    let cert = rustls::Certificate(cert.serialize_der().unwrap());

    quinn::ServerConfig::with_single_cert(vec![cert], key).expect("can build server config")
}

// fn generate_self_signed_cert(
//     cert_path: &str,
//     key_path: &str,
// ) -> (rustls::Certificate, rustls::PrivateKey) {
//     // Generate dummy certificate.
//     let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
//     let serialized_key = certificate.serialize_private_key_der();
//     let serialized_certificate = certificate.serialize_der().unwrap();

//     // Write to files.
//     fs::write(&cert_path, &serialized_certificate).expect("failed to write certificate");
//     fs::write(&key_path, &serialized_key).expect("failed to write private key");

//     let cert = rustls::Certificate(serialized_certificate);
//     let key = rustls::PrivateKey(serialized_key);

//     (cert, key)
// }

pub fn new_default(bind_addr: SocketAddr) -> (Endpoint, Incoming) {
    Endpoint::server(server_config(), bind_addr).expect("can bind endpoint")
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
