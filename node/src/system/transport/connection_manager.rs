use futures::future::join;
use futures::prelude::*;
use quinn::{Connecting, Endpoint, Incoming, NewConnection};
use samizdat_common::{quic, BincodeOverQuic};
use std::net::SocketAddr;

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
    pub fn new(endpoint: Endpoint, mut incoming: Incoming) -> ConnectionManager {
        let matcher: Matcher<SocketAddr, Connecting> = Matcher::default();

        let matcher_task = matcher.clone();
        tokio::spawn(async move {
            while let Some(connecting) = incoming.next().await {
                matcher_task
                    .arrive(connecting.remote_address(), connecting)
                    .await;
            }
        });

        ConnectionManager { endpoint, matcher }
    }

    pub async fn connect(
        &self,
        remote_addr: &SocketAddr,
        server_name: &str,
    ) -> Result<NewConnection, crate::Error> {
        let new_connection = quic::connect(&self.endpoint, remote_addr, server_name).await?;
        log::info!(
            "client connected to server at {}",
            new_connection.connection.remote_address()
        );

        Ok(new_connection)
    }

    pub async fn transport<S, R>(
        &self,
        remote_addr: &SocketAddr,
        server_name: &str,
    ) -> Result<BincodeOverQuic<S, R>, crate::Error>
    where
        S: 'static + Send + serde::Serialize,
        R: 'static + Send + for<'a> serde::Deserialize<'a>,
    {
        let new_connection = self.connect(remote_addr, server_name).await?;

        Ok(BincodeOverQuic::new(
            new_connection.connection.clone(),
            new_connection.uni_streams,
            MAX_TRANSFER_SIZE,
        ))
    }

    /// TODO: very basic NAT/firewall traversal stuff that works well in IPv6,
    /// but not so much in IPv4. Is there a better solution? I am already using
    /// the hub as a STUN and not many people have the means to keep a TURN.
    pub(super) async fn punch_hole_to(
        &self,
        peer_addr: SocketAddr,
        drop_mode: DropMode,
    ) -> Result<NewConnection, crate::Error> {
        log::info!("punching hole to {}", peer_addr);

        let incoming = self
            .endpoint
            .connect(&peer_addr, "localhost")
            .expect("failed to start connecting");

        let outgoing = async move {
            if let Some(connecting) = self.matcher.expect(peer_addr).await {
                log::info!("found expected conection {}", peer_addr);
                Ok(connecting.await?)
            } else {
                Err("peer not expected".into()) as Result<_, crate::Error>
            }
        };

        match join(incoming, outgoing).await {
            (Err(_), Ok(outgoing)) => {
                log::info!("only outgoing succeeded");
                Ok(outgoing)
            }
            (Ok(incoming), Err(_)) => {
                log::info!("only incoming succeeded");
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
                // TODO: better error message here.
                Err("failed miserably".to_owned().into())
            }
        }
    }
}
