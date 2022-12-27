use futures::future::join;
use quinn::{Connecting, Connection, Endpoint};
use samizdat_common::{quic, BincodeOverQuic};
use std::net::SocketAddr;

use crate::utils;

use super::matcher::Matcher;

const MAX_TRANSFER_SIZE: usize = 2_048;

pub enum DropMode {
    DropIncoming,
    DropOutgoing,
}

pub struct ConnectionManager {
    endpoint: Endpoint,
    matcher: Matcher<SocketAddr, Connecting>,
}

impl ConnectionManager {
    pub fn new(endpoint: Endpoint) -> ConnectionManager {
        let matcher: Matcher<SocketAddr, Connecting> = Matcher::default();

        let matcher_task = matcher.clone();
        let endpoint_task = endpoint.clone();
        tokio::spawn(async move {
            while let Some(connecting) = endpoint_task.accept().await {
                let peer_addr = utils::socket_to_canonical(connecting.remote_address());
                log::info!("{peer_addr} arrived");
                matcher_task.arrive(peer_addr, connecting).await;
            }
        });

        ConnectionManager { endpoint, matcher }
    }

    pub async fn connect(&self, remote_addr: SocketAddr) -> Result<Connection, crate::Error> {
        let connection = quic::connect(&self.endpoint, remote_addr).await?;
        let remote = connection.remote_address();
        log::info!(
            "client connected to server at {}",
            SocketAddr::from((remote.ip().to_canonical(), remote.port())),
        );

        Ok(connection)
    }

    pub async fn transport<S, R>(
        &self,
        remote_addr: SocketAddr,
    ) -> Result<BincodeOverQuic<S, R>, crate::Error>
    where
        S: 'static + Send + serde::Serialize,
        R: 'static + Send + for<'a> serde::Deserialize<'a>,
    {
        let connection = self.connect(remote_addr).await?;

        Ok(BincodeOverQuic::new(connection, MAX_TRANSFER_SIZE))
    }

    /// TODO: very basic NAT/firewall traversal stuff that works well in IPv6,
    /// but not so much in IPv4. Is there a better solution? I am already using
    /// the hub as a STUN and not many people have the means to keep a TURN.
    pub(super) async fn punch_hole_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<Connection, crate::Error> {
        log::info!("punching hole to {peer_addr}");

        let incoming = self
            .endpoint
            .connect(peer_addr, "localhost")
            .expect("failed to start connecting");

        let outgoing = async move {
            if let Some(connecting) = self.matcher.expect(peer_addr).await {
                log::info!("found expected connection {peer_addr}");
                Ok(connecting.await?)
            } else {
                Err("peer not expected".into()) as Result<_, crate::Error>
            }
        };

        match join(incoming, outgoing).await {
            (Err(err), Ok(outgoing)) => {
                log::info!("only outgoing succeeded");
                log::info!("incoming got: {err}");
                Ok(outgoing)
            }
            (Ok(incoming), Err(err)) => {
                log::info!("only incoming succeeded");
                log::info!("outgoing got: {err}");
                Ok(incoming)
            }
            (Ok(incoming), Ok(outgoing)) => {
                log::info!("both connections succeeded");
                Ok(match drop_mode {
                    DropMode::DropIncoming => {
                        log::info!("choosing outgoing");
                        outgoing
                    }
                    DropMode::DropOutgoing => {
                        log::info!("choosing incoming");
                        incoming
                    }
                })
            }
            (Err(incoming_err), Err(outgoing_err)) => {
                log::info!("both connections failed");
                log::info!("incoming error: {}", incoming_err);
                log::info!("outgoing error: {}", outgoing_err);
                Err(format!(
                    "both connections failed: incoming got \"{incoming_err}\"; outgoing got \"{outgoing_err}\""
                ).into())
            }
        }
    }
}
