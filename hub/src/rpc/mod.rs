mod hub_as_node;
mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use quinn::Endpoint;
use rand_distr::Distribution;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Duration, Interval, MissedTickBehavior};

use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::{BincodeOverQuic, ChannelAddr};

use crate::replay_resistance::ReplayResistance;
use crate::CLI;

use self::room::Room;

const MAX_LENGTH: usize = 2_048;

lazy_static! {
    static ref ROOM: Room = Room::new();
    static ref REPLAY_RESISTANCE: Mutex<ReplayResistance> = Mutex::new(ReplayResistance::new());
}

#[derive(Debug, Default)]
struct Statistics {
    requests: AtomicUsize,
    successes: AtomicUsize,
    errors: AtomicUsize,
    total_latency: AtomicUsize,
}

impl Statistics {
    fn rand_priority(&self) -> f64 {
        let requests = self.requests.load(Ordering::Relaxed) as f64;
        let successes = self.successes.load(Ordering::Relaxed) as f64;
        let beta =
            rand_distr::Beta::new(successes + 1., requests + 1.).expect("valid beta distribution");

        beta.sample(&mut rand::thread_rng())
    }
}

#[derive(Debug)]
struct Node {
    statistics: Statistics,
    client: NodeClient,
    addr: SocketAddr,
}

struct HubServerInner {
    call_semaphore: Semaphore,
    call_throttle: Mutex<Interval>,
    addr: SocketAddr,
}

#[derive(Clone)]
struct HubServer(Arc<HubServerInner>);

impl HubServer {
    fn new(addr: SocketAddr) -> HubServer {
        let mut call_throttle = interval(Duration::from_secs_f64(1. / CLI.max_query_rate_per_node));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        HubServer(Arc::new(HubServerInner {
            call_semaphore: Semaphore::new(CLI.max_queries_per_node),
            call_throttle: Mutex::new(interval(Duration::from_secs_f64(
                1. / CLI.max_query_rate_per_node,
            ))),
            addr,
        }))
    }

    /// Does the whole API throttling thing. Using `Box` denies any allocations to the throttled
    /// client. This may mitigate DoS.
    async fn throttle<'a, F, Fut, T>(&'a self, f: F) -> T
    where
        F: 'a + Send + FnOnce(&'a Self) -> Fut,
        Fut: 'a + Future<Output = T>,
    {
        // First, make sure we are not being trolled:
        self.0.call_throttle.lock().await.tick().await;
        let permit = self
            .0
            .call_semaphore
            .acquire()
            .await
            .expect("semaphore never closed");

        let outcome = f(self).await;

        drop(permit);
        outcome
    }
}

async fn candidates_for_resolution(
    ctx: context::Context,
    client_addr: SocketAddr,
    resolution: Arc<Resolution>,
) -> Vec<Candidate> {
    ROOM.with_peers(client_addr, move |peer_id, peer| {
        let resolution = resolution.clone();
        async move {
            log::debug!("starting resolve for {}", peer_id);

            peer.statistics.requests.fetch_add(1, Ordering::Relaxed);

            let start = Instant::now();
            let outcome = peer.client.resolve(ctx, resolution).await;
            let elapsed = start.elapsed().as_millis() as usize;
            peer.statistics
                .total_latency
                .fetch_add(elapsed, Ordering::Relaxed);

            let response = match outcome {
                Ok(response) => response,
                Err(err) => {
                    log::warn!("error asking {} to resolve: {}", peer_id, err);
                    peer.statistics.errors.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };

            log::debug!("resolve done for {}", peer_id);

            match response {
                ResolutionResponse::Found(validation_riddle) => {
                    peer.statistics.successes.fetch_add(1, Ordering::Relaxed);
                    Some(vec![Candidate {
                        peer_addr: peer.addr,
                        validation_riddle,
                    }])
                }
                ResolutionResponse::Redirect(candidates) => {
                    peer.statistics.successes.fetch_add(1, Ordering::Relaxed);
                    Some(candidates)
                }
                ResolutionResponse::NotFound => None,
            }
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
}

async fn latest_for_request(
    ctx: context::Context,
    client_addr: SocketAddr,
    latest: Arc<LatestRequest>,
) -> Vec<LatestResponse> {
    ROOM.with_peers(client_addr, |peer_id, peer| {
        let latest = latest.clone();
        async move {
            let outcome = peer.client.resolve_latest(ctx, latest).await;
            let response = match outcome {
                Ok(response) => response,
                Err(err) => {
                    log::warn!("error asking {} for latest: {}", peer_id, err);
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
    ROOM.with_peers(client_addr, |peer_id, peer| {
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

#[tarpc::server]
impl Hub for HubServer {
    async fn query(self, ctx: context::Context, query: Query) -> QueryResponse {
        let client_addr = self.0.addr;
        self.throttle(|server| async move {
            log::debug!("got {:?}", query);

            // Create a channel address from peer address:
            let channel = rand::random();
            let channel_addr = ChannelAddr::new(server.0.addr, channel);

            // Se if you are not being replayed:
            match REPLAY_RESISTANCE.lock().await.check(&query) {
                Ok(false) => return QueryResponse::Replayed,
                Err(err) => {
                    log::error!("error while checking for replay: {}", err);
                    return QueryResponse::InternalError;
                }
                _ => {}
            }

            // Now, prepare resolution request:
            let message_riddle = query.content_riddle.riddle_for(channel_addr);
            let resolution = Arc::new(Resolution {
                content_riddle: query.content_riddle,
                message_riddle,
                validation_nonce: query.validation_riddle.rand,
                kind: query.kind,
            });

            // And then send the request to the peers:
            let candidates = candidates_for_resolution(ctx, client_addr, resolution).await;

            log::debug!("query done");

            QueryResponse::Resolved {
                candidates: candidates
                    .into_iter()
                    .filter(|candidate| candidate.validation_riddle == query.validation_riddle)
                    .map(|candidate| ChannelAddr::new(candidate.peer_addr, channel))
                    .collect(),
            }
        })
        .await
    }

    async fn get_latest(
        self,
        ctx: context::Context,
        latest_request: LatestRequest,
    ) -> Vec<LatestResponse> {
        let client_addr = self.0.addr;
        self.throttle(|_| async move {
            // Se if you are not being replayed:
            match REPLAY_RESISTANCE.lock().await.check(&latest_request) {
                Ok(false) => return vec![],
                Err(err) => {
                    log::error!("error while checking for replay: {}", err);
                    return vec![];
                }
                _ => {}
            }

            // Now broadcast the request:
            latest_for_request(ctx, client_addr, Arc::new(latest_request)).await
        })
        .await
    }

    async fn announce_edition(self, ctx: context::Context, announcement: EditionAnnouncement) {
        let client_addr = self.0.addr;
        self.throttle(|_| async move {
            // Se if you are not being replayed:
            match REPLAY_RESISTANCE.lock().await.check(&announcement) {
                Ok(false) => return,
                Err(err) => {
                    log::error!("error while checking for replay: {}", err);
                    return;
                }
                _ => {}
            }

            // Now, broadcast the announcement:
            announce_edition(ctx, client_addr, Arc::new(announcement)).await
        })
        .await
    }
}

pub async fn run_direct(addr: impl Into<SocketAddr>) -> Result<(), io::Error> {
    let mut endpoint_builder = Endpoint::builder();
    endpoint_builder.listen(samizdat_common::quic::server_config());
    endpoint_builder.default_client_config(samizdat_common::quic::insecure());

    let (endpoint, incoming) = endpoint_builder.bind(&addr.into()).expect("failed to bind");

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

            log::debug!("Incoming connection from {}", client_addr);

            let transport = BincodeOverQuic::new(
                new_connection.connection,
                new_connection.uni_streams,
                MAX_LENGTH,
            );

            // Set up server:
            let server = HubServer::new(client_addr);
            let server_task = server::BaseChannel::with_defaults(transport).execute(server.serve());

            log::info!("Connection from node (as server) {} accepted", client_addr);

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
    let mut endpoint_builder = Endpoint::builder();
    endpoint_builder.listen(samizdat_common::quic::server_config());
    endpoint_builder.default_client_config(samizdat_common::quic::insecure());

    let (endpoint, incoming) = endpoint_builder.bind(&addr.into()).expect("failed to bind");

    log::info!("Reverse server started at {}", endpoint.local_addr()?);

    incoming
        .filter_map(|connecting| async move {
            connecting
                .await
                .map_err(|err| log::warn!("failed to establish QUIC connection: {}", err))
                .ok()
        })
        .for_each_concurrent(Some(CLI.max_connections), |new_connection| async move {
            // Get peer address:
            let client_addr = new_connection.connection.remote_address();

            log::debug!("Incoming connection from {}", client_addr);

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

            log::info!("Connection from node (as client) {} accepted", client_addr);

            ROOM.insert(
                client_addr,
                Node {
                    statistics: Statistics::default(),
                    client,
                    addr: client_addr,
                },
            )
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
