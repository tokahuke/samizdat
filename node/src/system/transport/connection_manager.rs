use futures::future::join;
use quinn::{Connection, Endpoint, Incoming};
use samizdat_common::quic;
use std::{net::SocketAddr, sync::OnceLock};

use crate::utils;

use super::matcher::Matcher;

static CONNECTION_MANAGER: OnceLock<ConnectionManager> = OnceLock::new();

pub fn connection_manager<'a>() -> &'a ConnectionManager {
    CONNECTION_MANAGER.get_or_init(|| {
        let endpoint = quic::new_default("[::]:0".parse().expect("valid address"));

        if let Ok(local_addr) = endpoint.local_addr() {
            tracing::info!("QUIC connection bound to {local_addr}");
        }

        ConnectionManager::new(endpoint)
    })
}

#[derive(Debug, Clone, Copy)]
pub enum DropMode {
    DropIncoming,
    DropOutgoing,
}

pub struct ConnectionManager {
    endpoint: Endpoint,
    matcher: Matcher<SocketAddr, Incoming>,
}

impl ConnectionManager {
    pub fn new(endpoint: Endpoint) -> ConnectionManager {
        let matcher: Matcher<SocketAddr, Incoming> = Matcher::default();

        let matcher_task = matcher.clone();
        let endpoint_task = endpoint.clone();
        tokio::spawn(async move {
            while let Some(incoming) = endpoint_task.accept().await {
                let peer_addr = utils::socket_to_canonical(incoming.remote_address());
                tracing::info!("{peer_addr} arrived");
                matcher_task.arrive(peer_addr, incoming).await;
            }
        });

        ConnectionManager { endpoint, matcher }
    }

    pub async fn connect(&self, remote_addr: SocketAddr) -> Result<Connection, crate::Error> {
        let connection = quic::connect(&self.endpoint, remote_addr, true).await?;
        let remote = connection.remote_address();
        tracing::info!(
            "client connected to server at {}",
            SocketAddr::from((remote.ip().to_canonical(), remote.port())),
        );

        Ok(connection)
    }

    /// Very basic NAT/firewall traversal stuff that works well in IPv6,
    /// but not so much in IPv4. Is there a better solution? I am already using
    /// the hub as a STUN and not many people have the means to keep a TURN.
    pub(super) async fn punch_hole_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<Connection, crate::Error> {
        tracing::info!("punching hole to {peer_addr}");

        let incoming = self
            .endpoint
            .connect(peer_addr, "localhost")
            .expect("failed to start connecting");

        let outgoing = async move {
            if let Some(connecting) = self.matcher.expect(peer_addr).await {
                tracing::info!("found expected connection {peer_addr}");
                Ok(connecting.await?)
            } else {
                Err("peer not expected".into()) as Result<_, crate::Error>
            }
        };

        match join(incoming, outgoing).await {
            (Err(err), Ok(outgoing)) => {
                tracing::info!("only outgoing succeeded");
                tracing::info!("incoming got: {err}");
                Ok(outgoing)
            }
            (Ok(incoming), Err(err)) => {
                tracing::info!("only incoming succeeded");
                tracing::info!("outgoing got: {err}");
                Ok(incoming)
            }
            (Ok(incoming), Ok(outgoing)) => {
                tracing::info!("both connections succeeded");
                Ok(match drop_mode {
                    DropMode::DropIncoming => {
                        tracing::info!("choosing outgoing");
                        outgoing
                    }
                    DropMode::DropOutgoing => {
                        tracing::info!("choosing incoming");
                        incoming
                    }
                })
            }
            (Err(incoming_err), Err(outgoing_err)) => {
                tracing::info!("both connections failed");
                tracing::info!("incoming error: {}", incoming_err);
                tracing::info!("outgoing error: {}", outgoing_err);
                Err(format!(
                    "both connections failed: incoming got \"{incoming_err}\"; outgoing got \"{outgoing_err}\""
                ).into())
            }
        }
    }
}
