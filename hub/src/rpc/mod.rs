//! The RPC server that is the core of the Samizdat Hub.

pub mod node_sampler;

mod hub_as_node;
mod hub_server;
mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use samizdat_common::keyed_channel::KeyedChannel;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Interval, MissedTickBehavior};

use samizdat_common::{quinn, rpc::*, transport};
// use samizdat_common::BincodeOverQuic;
use samizdat_common::{quic, Riddle};

use crate::cli::cli;
use crate::models::blacklisted_ip::BlacklistedIp;
use crate::replay_resistance::ReplayResistance;
use crate::utils;

use self::hub_server::HubServer;
use self::node_sampler::{EditionSampler, QuerySampler, Statistics, UniformSampler};
use self::room::Room;

/// The maximum length in bytes that a message in the RPC connections can have. This is
/// set to a low value because all messages sent and received through the RPC are quite
/// small. A such, this may change in the future to a bigger value.
const MAX_LENGTH: usize = 2_048;

lazy_static! {
    /// The main pool of nodes.
    pub static ref ROOM: Room = Room::new();
    /// The replay resistance that tracks nonces that are being sent to the server.
    pub static ref REPLAY_RESISTANCE: Mutex<ReplayResistance> = Mutex::new(ReplayResistance::new());
}

/// Represents a connection to a Samizdat node.
#[derive(Debug)]
pub struct Node {
    /// Gathers statics on the ability of this node to answer queries.
    query_statistics: Statistics,
    /// Gather statistics on the ability of this node to answer editions.
    edition_statistics: Statistics,
    /// The RPC client of this node.
    client: NodeClient,
    /// The socket address of the node.
    addr: SocketAddr,
    /// Limits the number of simultaneous queries a node can make.
    call_semaphore: Semaphore,
    /// Limits the frequency of queries a node can make.
    call_throttle: Mutex<Interval>,
}

impl Node {
    /// Creates a new node from a socket address and a raw RPC client.
    fn new(addr: SocketAddr, client: NodeClient, config: NodeConfig) -> Node {
        let mut call_throttle = interval(Duration::from_secs_f64(1. / config.max_query_rate));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        Node {
            query_statistics: Statistics::default(),
            edition_statistics: Statistics::default(),
            client,
            addr,
            call_semaphore: Semaphore::new(config.max_queries),
            call_throttle: Mutex::new(call_throttle),
        }
    }

    // This looked like a good idea, but is a bad idea, actually.
    // /// Guesses if a call to [`Node::throttle`] will throttle or not.
    // fn will_throttle(&self) -> bool {
    //     if let Ok(mut guard) = self.call_throttle.try_lock() {
    //         // Instant::tick is cancellation-safe.
    //         if guard.tick().now_or_never().is_none() {
    //             return true;
    //         }
    //     } else {
    //         return true;
    //     }

    //     self.call_semaphore.try_acquire().is_err()
    // }

    /// Does the whole API throttling thing. Using `Box` denies any allocations to the throttled
    /// client. This may mitigate DoS.
    async fn throttle<'a, F, Fut, T>(&'a self, f: F) -> T
    where
        F: 'a + Send + FnOnce(&'a Self) -> Fut,
        Fut: 'a + Future<Output = T>,
    {
        // First, make sure we are not being trolled:
        self.call_throttle.lock().await.tick().await;
        let permit = self
            .call_semaphore
            .acquire()
            .await
            .expect("semaphore never closed");

        let outcome = f(self).await;

        drop(permit);
        outcome
    }
}

/// Lists candidates that can answer to a given resolution.
/// TODO: code smell: big function.
fn candidates_for_resolution(
    ctx: context::Context,
    client_addr: SocketAddr,
    mut resolution: Resolution,
    candidate_channels: KeyedChannel<Candidate>,
) -> impl Send + Stream<Item = Candidate> {
    tracing::debug!("Client {client_addr} requested {resolution:?}");

    // Go one step down with the resolution:
    let validation_riddle = resolution
        .content_riddles
        .pop()
        .expect("non-empty resolution");
    resolution.validation_nonces.push(validation_riddle.rand);
    let resolution = Arc::new(resolution);

    // // TODO: streaming broke this...
    // if validation_riddle.is_empty() {
    //     // now what?
    // }

    // Then query peers:
    ROOM.with_peers(
        QuerySampler,
        client_addr,
        cli().max_resolutions_per_query,
        cli().max_candidates,
        move |peer_id, peer| {
            tracing::debug!("Pairing client {client_addr} with peer {peer_id}");
            let resolution = resolution.clone();
            let validation_riddle = validation_riddle.clone();
            let candidate_channels = candidate_channels.clone();

            async move {
                tracing::debug!("starting resolve for {peer_id}");
                let experiment = peer.query_statistics.start_experiment();
                let outcome = peer
                    .throttle(|peer| async { peer.client.resolve(ctx, resolution.clone()).await })
                    .await;

                let response = match outcome {
                    Ok(response) => response,
                    Err(err) => {
                        tracing::warn!("error asking {peer_id} to resolve: {err}");
                        return None;
                    }
                };

                tracing::debug!("resolve done for {peer_id}");

                let validate_riddles = move |riddles: &[Riddle]| {
                    // `>=`: there can be more added nonces down the line because of further redirects.
                    riddles.len() >= resolution.validation_nonces.len()
                    // Check that *your* riddle is correct
                    && riddles[resolution.validation_nonces.len() - 1] == validation_riddle
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
                        experiment.end_with_success();
                        Some(Box::pin(stream::once(async move {
                            Candidate {
                                socket_addr: peer.addr,
                                validation_riddles,
                            }
                        }))
                            as Pin<Box<dyn Send + Stream<Item = Candidate>>>)
                    }
                    ResolutionResponse::Redirect(candidate_channel) => {
                        let mut maybe_experiment = Some(experiment);
                        let valid_candidates = candidate_channels
                            .recv_stream(candidate_channel)
                            .filter(move |candidate| {
                                let is_valid = validate_riddles(&candidate.validation_riddles);
                                // IPv6 with IPv6; IPv4 with IPv4!
                                let ip_version_matches = candidate.socket_addr.ip().is_ipv6()
                                    == client_addr.ip().is_ipv6();

                                async move { is_valid && ip_version_matches }
                            })
                            .inspect(move |_| {
                                // End experiment with success on first received candidate
                                if let Some(experiment) = maybe_experiment.take() {
                                    experiment.end_with_success();
                                }
                            });

                        Some(Box::pin(valid_candidates)
                            as Pin<Box<dyn Send + Stream<Item = Candidate>>>)
                    }
                    _ => None,
                }
            }
        },
    )
    .flatten_unordered(10)
}

/// Lists editions that can answer to a given edition request.
async fn edition_for_request(
    ctx: context::Context,
    client_addr: SocketAddr,
    latest: Arc<EditionRequest>,
) -> Vec<EditionResponse> {
    let responses = ROOM
        .with_peers(
            EditionSampler,
            client_addr,
            cli().max_resolutions_per_query,
            usize::MAX,
            |peer_id, peer| {
                let latest = latest.clone();
                async move {
                    tracing::debug!("starting resolve latest edition for {peer_id}");
                    let experiment = peer.edition_statistics.start_experiment();
                    let outcome = peer
                        .throttle(|peer| async { peer.client.get_edition(ctx, latest).await })
                        .await;

                    match outcome {
                        Ok(response) => {
                            // Empty response is not a valid candidate.
                            if !response.is_empty() {
                                experiment.end_with_success();
                                Some(response)
                            } else {
                                None
                            }
                        }
                        Err(err) => {
                            tracing::warn!("error asking {peer_id} for latest: {err}");
                            None
                        }
                    }
                }
            },
        )
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    tracing::debug!("Client {client_addr} receives {responses:?}");

    responses
}

/// Announces a new edition to the network.
async fn announce_edition(
    ctx: context::Context,
    client_addr: SocketAddr,
    announcement: Arc<EditionAnnouncement>,
) {
    ROOM.with_peers(
        UniformSampler,
        client_addr,
        cli().max_resolutions_per_query,
        usize::MAX,
        |peer_id, peer| {
            let announcement = announcement.clone();
            async move {
                let outcome = peer
                    .throttle(|peer| async {
                        peer.client.announce_edition(ctx, announcement).await
                    })
                    .await;

                match outcome {
                    Ok(_) => Some(()),
                    Err(err) => {
                        tracing::warn!("error announcing to peer {peer_id}: {err}");
                        None
                    }
                }
            }
        },
    )
    .collect::<Vec<_>>()
    .await;
}

/// Runs the "direct" server. This is the system where the Hub acts as a server and the
/// Node acts as a client. This is used for, e.g., the nodes to ask the server the
/// resolution to a given query.
pub async fn run_direct(
    addrs: Vec<impl Into<SocketAddr>>,
    candidate_channels: KeyedChannel<Candidate>,
) -> Result<(), io::Error> {
    let all_endpoints = addrs
        .into_iter()
        .map(|addr| {
            let endpoint = samizdat_common::quic::new_default(addr.into());
            tracing::info!("Direct server started at {}", endpoint.local_addr()?);

            Ok(endpoint)
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    stream::iter(all_endpoints)
        .flat_map(|endpoint| {
            futures::stream::unfold(endpoint, |endpoint| async move {
                endpoint
                    .accept()
                    .await
                    .map(|connecting| (connecting, endpoint))
            })
        })
        .filter_map(|connecting| async move {
            let remote_addr = utils::socket_to_canonical(connecting.remote_address());

            // Validate if address is not blacklisted:
            if BlacklistedIp::get(remote_addr.ip())
                .expect("db error")
                .is_some()
            {
                return None;
            }

            connecting
                .await
                .map_err(|err| {
                    tracing::warn!("failed to establish QUIC connection with {remote_addr}: {err}")
                })
                .ok()
        })
        .for_each(|connection| {
            let client_addr = utils::socket_to_canonical(connection.remote_address());
            let candidate_channels = candidate_channels.clone();

            tracing::info!("Incoming connection from {client_addr}");
            tokio::spawn(async move {
                if let Err(err) =
                    setup_connection(connection, client_addr, candidate_channels).await
                {
                    tracing::error!("failed to setup connection to {client_addr}: {err}")
                }
            });

            async {}
        })
        .await;

    Ok(())
}

async fn setup_connection(
    connection: quinn::Connection,
    client_addr: SocketAddr,
    candidate_channels: KeyedChannel<Candidate>,
) -> Result<(), crate::Error> {
    let (direct_transport, reverse_transport) =
        transport::accept_bincode_transports(connection, MAX_LENGTH).await?;

    tokio::spawn(async move {
        // Set up server:
        let server = HubServer::new(client_addr, candidate_channels);
        let server_task = server::BaseChannel::with_defaults(direct_transport)
            .execute(server.serve())
            .for_each(|request_task| async move {
                tokio::spawn(request_task);
            });

        tracing::info!("Connection from node (as server) {client_addr} accepted");

        server_task.await
    });

    // Client task:
    tokio::spawn(async move {
        // Set up client (remember to drop it when connection is severed):
        let uninstrumented_client =
            NodeClient::new(tarpc::client::Config::default(), reverse_transport);
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

        match client.config(context::current()).await {
            Ok(config) => {
                tracing::info!("Connection from node (as client) {client_addr} accepted");
                ROOM.insert(client_addr, Node::new(client_addr, client, config))
                    .await;
            }
            Err(err) => {
                tracing::warn!("Failed to get configuration from node at {client_addr}: {err}")
            }
        }
    });

    Ok(())
}

/// This runs the current Hub taking the role of a Node to other hubs. This is what makes the
/// network recursive.
pub async fn run_partners() {
    let endpoint = quic::new_default("[::]:0".parse().expect("valid address"));

    tracing::info!(
        "Hub-as-node server started at {}",
        endpoint.local_addr().expect("local address exists")
    );

    // Resolve partner addresses (`CLI.partners` is an `Option`. Therefore, we flatten it!):
    let partners = stream::iter(cli().partners.iter().flatten())
        .map(|partner| hub_as_node::run(partner, &endpoint))
        .collect::<Vec<_>>()
        .await;

    for partner in partners {
        partner.await
    }
}
