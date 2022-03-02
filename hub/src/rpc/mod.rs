mod hub_as_node;
mod hub_server;
mod node_sampler;
mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::Mutex;

use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::BincodeOverQuic;

use crate::replay_resistance::ReplayResistance;
use crate::CLI;

use self::hub_server::HubServer;
use self::node_sampler::{EditionSampler, QuerySampler, Statistics, UniformSampler};
use self::room::Room;

const MAX_LENGTH: usize = 2_048;

lazy_static! {
    static ref ROOM: Room = Room::new();
    static ref REPLAY_RESISTANCE: Mutex<ReplayResistance> = Mutex::new(ReplayResistance::new());
}

#[derive(Debug)]
struct Node {
    query_statistics: Statistics,
    edition_statistics: Statistics,
    client: NodeClient,
    addr: SocketAddr,
}

impl Node {
    fn new(client_addr: SocketAddr, client: NodeClient) -> Node {
        Node {
            query_statistics: Statistics::default(),
            edition_statistics: Statistics::default(),
            client,
            addr: client_addr,
        }
    }
}

async fn candidates_for_resolution(
    ctx: context::Context,
    client_addr: SocketAddr,
    resolution: Arc<Resolution>,
) -> Vec<Candidate> {
    ROOM.with_peers(QuerySampler, client_addr, move |peer_id, peer| {
        let resolution = resolution.clone();
        async move {
            log::debug!("starting resolve for {peer_id}");

            peer.query_statistics.start_request();

            let start = Instant::now();
            let outcome = peer.client.resolve(ctx, resolution).await;
            let elapsed = start.elapsed();

            let response = match outcome {
                Ok(response) => response,
                Err(err) => {
                    log::warn!("error asking {peer_id} to resolve: {err}");
                    peer.query_statistics.end_request_with_failure();
                    return None;
                }
            };

            log::debug!("resolve done for {peer_id}");

            match response {
                ResolutionResponse::Found(validation_riddle) => {
                    peer.query_statistics.end_request_with_success(elapsed);
                    Some(vec![Candidate {
                        peer_addr: peer.addr,
                        validation_riddle,
                    }])
                }
                ResolutionResponse::Redirect(candidates) => {
                    peer.query_statistics.end_request_with_success(elapsed);
                    Some(candidates)
                }
                ResolutionResponse::NotFound => {
                    peer.query_statistics.end_request_with_failure();
                    None
                }
            }
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
}

async fn edition_for_request(
    ctx: context::Context,
    client_addr: SocketAddr,
    latest: Arc<EditionRequest>,
) -> Vec<EditionResponse> {
    ROOM.with_peers(EditionSampler, client_addr, |peer_id, peer| {
        let latest = latest.clone();
        async move {
            log::debug!("starting resolve latest edition for {peer_id}");

            peer.edition_statistics.start_request();

            let start = Instant::now();
            let outcome = peer.client.get_edition(ctx, latest).await;
            let elapsed = start.elapsed();

            let response = match outcome {
                Ok(response) => {
                    peer.edition_statistics.end_request_with_success(elapsed);
                    response
                }
                Err(err) => {
                    log::warn!("error asking {peer_id} for latest: {err}");
                    peer.edition_statistics.end_request_with_failure();
                    return None;
                }
            };

            Some(response)
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
}

async fn announce_edition(
    ctx: context::Context,
    client_addr: SocketAddr,
    announcement: Arc<EditionAnnouncement>,
) {
    ROOM.with_peers(UniformSampler, client_addr, |peer_id, peer| {
        let announcement = announcement.clone();
        async move {
            let outcome = peer.client.announce_edition(ctx, announcement).await;

            match outcome {
                Ok(_) => Some(()),
                Err(err) => {
                    log::warn!("error announcing to peer {}: {}", peer_id, err);
                    None
                }
            }
        }
    })
    .await;
}

async fn get_identity(
    ctx: context::Context,
    client_addr: SocketAddr,
    request: Arc<IdentityRequest>,
) -> Vec<IdentityResponse> {
    // TODO: create dedicated sampler....
    ROOM.with_peers(EditionSampler, client_addr, |peer_id, peer| {
        let request = request.clone();
        async move {
            peer.edition_statistics.start_request();

            let start = Instant::now();
            let outcome = peer.client.get_identity(ctx, request).await;
            let elapsed = start.elapsed();

            let response = match outcome {
                Ok(response) => {
                    peer.edition_statistics.end_request_with_success(elapsed);
                    response
                }
                Err(err) => {
                    log::warn!("error asking {peer_id} for latest: {err}");
                    peer.edition_statistics.end_request_with_failure();
                    return None;
                }
            };

            Some(response)
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
}

pub async fn run_direct(addr: impl Into<SocketAddr>) -> Result<(), io::Error> {
    let (endpoint, incoming) = samizdat_common::quic::new_default(addr.into());

    log::info!("Direct server started at {}", endpoint.local_addr()?);

    incoming
        .filter_map(|connecting| async move {
            connecting
                .await
                .map_err(|err| log::warn!("failed to establish QUIC connection: {}", err))
                .ok()
        })
        .map(|new_connection| {
            // Get peer address:
            let client_addr = new_connection.connection.remote_address();

            log::debug!("Incoming connection from {client_addr}");

            let transport = BincodeOverQuic::new(
                new_connection.connection,
                new_connection.uni_streams,
                MAX_LENGTH,
            );

            // Set up server:
            let server = HubServer::new(client_addr);
            let server_task = server::BaseChannel::with_defaults(transport).execute(server.serve());

            log::info!("Connection from node (as server) {client_addr} accepted");

            Some(server_task)
        })
        .filter_map(|maybe_server| async move { maybe_server })
        // Max number of channels.
        .buffer_unordered(CLI.max_connections)
        .for_each(|_| async {})
        .await;

    Ok(())
}

pub async fn run_reverse(addr: impl Into<SocketAddr>) -> Result<(), io::Error> {
    let (endpoint, incoming) = samizdat_common::quic::new_default(addr.into());

    log::info!("Reverse server started at {}", endpoint.local_addr()?);

    incoming
        .filter_map(|connecting| async move {
            connecting
                .await
                .map_err(|err| log::warn!("failed to establish QUIC connection: {err}"))
                .ok()
        })
        .for_each_concurrent(Some(CLI.max_connections), |new_connection| async move {
            // Get peer address:
            let client_addr = new_connection.connection.remote_address();

            log::debug!("Incoming connection from {client_addr}");

            let transport = BincodeOverQuic::new(
                new_connection.connection,
                new_connection.uni_streams,
                MAX_LENGTH,
            );

            // Set up client (remember to drop it when connection is severed):
            let uninstrumented_client =
                NodeClient::new(tarpc::client::Config::default(), transport);
            let client = tarpc::client::NewClient {
                client: uninstrumented_client.client,
                dispatch: uninstrumented_client
                    .dispatch
                    .then(move |outcome| async move {
                        ROOM.remove(client_addr).await;
                        outcome
                    }),
            }
            .spawn();

            log::info!("Connection from node (as client) {client_addr} accepted");

            ROOM.insert(client_addr, Node::new(client_addr, client))
                .await;
        })
        .await;

    Ok(())
}

pub async fn run_partners() {
    let (endpoint, _incoming) = quic::new_default("[::]:0".parse().expect("valid address"));

    log::info!(
        "Hub-as-node server started at {}",
        endpoint.local_addr().expect("local address exists")
    );

    // Resolve partner addresses (`CLI.partners` is an `Option`. Therefore, we flatten it!):
    let partners = stream::iter(CLI.partners.iter().flatten())
        .map(|partner| hub_as_node::run(partner, &endpoint))
        .collect::<Vec<_>>()
        .await;

    for partner in partners {
        partner.await;
    }
}
