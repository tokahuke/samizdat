pub mod node_sampler;

mod hub_as_node;
mod hub_server;
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

use samizdat_common::rpc::*;
use samizdat_common::BincodeOverQuic;
use samizdat_common::{quic, Riddle};

use crate::replay_resistance::ReplayResistance;
use crate::utils;
use crate::CLI;

use self::hub_server::HubServer;
use self::node_sampler::{EditionSampler, QuerySampler, Statistics, UniformSampler};
use self::room::Room;

const MAX_LENGTH: usize = 2_048;

lazy_static! {
    pub static ref ROOM: Room = Room::new();
    pub static ref REPLAY_RESISTANCE: Mutex<ReplayResistance> = Mutex::new(ReplayResistance::new());
}

#[derive(Debug)]
pub struct Node {
    query_statistics: Statistics,
    edition_statistics: Statistics,
    client: NodeClient,
    addr: SocketAddr,
}

impl Node {
    fn new(addr: SocketAddr, client: NodeClient) -> Node {
        Node {
            query_statistics: Statistics::default(),
            edition_statistics: Statistics::default(),
            client,
            // Make tunneled IPv4 addresses actual IPv4 addresses.
            addr,
        }
    }
}

async fn candidates_for_resolution(
    ctx: context::Context,
    client_addr: SocketAddr,
    mut resolution: Resolution,
) -> Vec<Candidate> {
    log::debug!("Client {client_addr} requested {resolution:?}");

    // Go one step down with the resolution:
    let validation_riddle = resolution
        .content_riddles
        .pop()
        .expect("non-empty resolution");
    resolution.validation_nonces.push(validation_riddle.rand);
    let resolution = dbg!(Arc::new(resolution));

    // Ooops! Empty query resolution...
    if resolution.content_riddles.is_empty() {
        log::info!("Content riddles empty");
        return vec![];
    }

    // Then query peers:
    ROOM.with_peers(QuerySampler, client_addr, move |peer_id, peer| {
        log::debug!("Pairing client {client_addr} with peer {peer_id}");
        let resolution = resolution.clone();
        let validation_riddle = validation_riddle.clone();

        async move {
            log::info!("starting resolve for {peer_id}");

            peer.query_statistics.start_request();

            let start = Instant::now();
            let outcome = peer.client.resolve(ctx, resolution.clone()).await;
            let elapsed = start.elapsed();

            let response = match outcome {
                Ok(response) => response,
                Err(err) => {
                    log::warn!("error asking {peer_id} to resolve: {err}");
                    peer.query_statistics.end_request_with_failure();
                    return None;
                }
            };

            log::info!("resolve done for {peer_id}");

            let validate_riddles = |riddles: &[Riddle]| {
                // `>=`: there can be more added nonces down the line because of further redirects.
                riddles.len() >= resolution.validation_nonces.len()
                    // Check that *your* riddle is correct
                    && &riddles[resolution.validation_nonces.len() - 1] == &validation_riddle
                    // Although you don't know the riddles before you, at least check that the nonces
                    // match.
                    && riddles
                        .iter()
                        .zip(&resolution.validation_nonces)
                        .all(|(riddle, nonce)| riddle.rand == *nonce)
            };

            match response {
                ResolutionResponse::Found(validation_riddles)
                    if validate_riddles(&validation_riddles) =>
                {
                    peer.query_statistics.end_request_with_success(elapsed);
                    Some(vec![Candidate {
                        peer_addr: peer.addr,
                        validation_riddles,
                    }])
                }
                ResolutionResponse::Redirect(candidates) => {
                    let valid_candidates = candidates
                        .into_iter()
                        .filter(|candidate| validate_riddles(&candidate.validation_riddles))
                        .filter(|candidate| {
                            // IPv6 with IPv6; IPv4 with IPv4!
                            candidate.peer_addr.ip().is_ipv6() == client_addr.ip().is_ipv6()
                        })
                        .collect::<Vec<_>>();

                    if !valid_candidates.is_empty() {
                        peer.query_statistics.end_request_with_success(elapsed);
                        Some(valid_candidates)
                    } else {
                        peer.query_statistics.end_request_with_failure();
                        None
                    }
                }
                _ => {
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
                    log::warn!("error announcing to peer {peer_id}: {err}");
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

pub async fn run_direct(addrs: Vec<impl Into<SocketAddr>>) -> Result<(), io::Error> {
    let all_incoming = addrs
        .into_iter()
        .map(|addr| {
            let (endpoint, incoming) = samizdat_common::quic::new_default(addr.into());
            log::info!("Direct server started at {}", endpoint.local_addr()?);

            Ok(incoming)
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    stream::iter(all_incoming)
        .flatten()
        .filter_map(|connecting| async move {
            connecting
                .await
                .map_err(|err| log::warn!("failed to establish QUIC connection: {}", err))
                .ok()
        })
        .map(|new_connection| {
            // Get peer address:
            let client_addr =
                utils::socket_to_canonical(new_connection.connection.remote_address());

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

pub async fn run_reverse(addrs: Vec<impl Into<SocketAddr>>) -> Result<(), io::Error> {
    let all_incoming = addrs
        .into_iter()
        .map(|addr| {
            let (endpoint, incoming) = samizdat_common::quic::new_default(addr.into());
            log::info!("Reverse server started at {}", endpoint.local_addr()?);

            Ok(incoming)
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    stream::iter(all_incoming)
        .flatten()
        .filter_map(|connecting| async move {
            connecting
                .await
                .map_err(|err| log::warn!("failed to establish QUIC connection: {err}"))
                .ok()
        })
        .for_each_concurrent(Some(CLI.max_connections), |new_connection| async move {
            // Get peer address:
            let client_addr =
                utils::socket_to_canonical(new_connection.connection.remote_address());

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
