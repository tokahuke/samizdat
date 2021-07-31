mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;

use samizdat_common::rpc::{Hub, NodeClient, Query, Resolution};
use samizdat_common::transport;

use room::{Participant, Room};

#[derive(Clone)]
struct HubServer {
    client_addr: SocketAddr,
    client: Participant<NodeClient>,
}

#[tarpc::server]
impl Hub for HubServer {
    async fn query(self, ctx: context::Context, query: Query) {
        log::debug!("got {:?}", query);
        let client_id = self.client.id();
        let location_riddle = query.location_riddle.riddle_for_location(self.client_addr);
        let resolution = Arc::new(Resolution {
            content_riddle: query.content_riddle,
            location_riddle,
        });

        self.client
            .for_each_peer(|peer_id, peer| {
                if peer_id != client_id {
                    // TODO
                }

                let peer = peer.clone();
                let resolution = resolution.clone();
                tokio::spawn(async move {
                    log::debug!("starting resolve");
                    peer.resolve(ctx, resolution).await.unwrap();
                    log::debug!("resolve done");
                });
            })
            .await;

        log::debug!("query done");
    }
}

lazy_static! {
    static ref ROOM: Room<NodeClient> = Room::new();
}

pub async fn run(addr: impl Into<SocketAddr>) -> Result<(), io::Error> {
    let listener = TcpListener::bind(addr.into()).await?;

    TcpListenerStream::new(listener)
        .filter_map(|r| async move {
            r.map_err(|err| log::warn!("failed to establish TCP connection: {}", err))
                .ok()
        })
        .then(|t| async move {
            // Get peer address:
            let client_addr = t
                .peer_addr()
                .map_err(|err| log::warn!("could not get peer address for connection: {}", err))
                .ok()?;

            log::info!("Incoming connection from {}", client_addr);

            // Multiplex connection:
            let multiplex = transport::Multiplex::new(t);
            let direct = multiplex
                .channel(0)
                .await
                .expect("channel 0 in use unexpectedly");
            let reverse = multiplex
                .channel(1)
                .await
                .expect("channel 1 in use unexpectedly");

            // Set up client:
            let client = ROOM
                .insert(
                    NodeClient::new(tarpc::client::Config::default(), reverse)
                        .spawn()
                        .map_err(|err| {
                            log::warn!("failed to spawn client from {}: {}", client_addr, err)
                        })
                        .ok()?,
                )
                .await;

            // Set up server:
            let server = HubServer {
                client_addr,
                client,
            };
            let server_task = server::BaseChannel::with_defaults(direct).execute(server.serve());

            Some(server_task)
        })
        .filter_map(|maybe_server| async move { maybe_server })
        // Max 1_000 channels.
        .buffer_unordered(1_000)
        .for_each(|_| async {})
        .await;

    Ok(())
}
