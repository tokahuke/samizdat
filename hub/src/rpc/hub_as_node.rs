//! Implements the RPC client part of the Hub API _for the Samizdat Hub_. This describes
//! how a Samizdat Hub can behave as another node to another Samizdat Hub. This confers
//! recursion to the Samizdat network.

use futures::future::Either;
use futures::prelude::*;
use samizdat_common::keyed_channel::KeyedChannel;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client::NewClient;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::{oneshot, Mutex, Semaphore};
use tokio::task::{JoinError, JoinHandle};
use tokio::time;
use tokio::time::{interval, Interval, MissedTickBehavior};

use samizdat_common::address::ChannelId;
use samizdat_common::quinn::{Connection, Endpoint};
use samizdat_common::{quic, rpc::*, transport};

use crate::cli::cli;

use super::{announce_edition, candidates_for_resolution, edition_for_request, REPLAY_RESISTANCE};

/// The maximum length in bytes that a message in the RPC connections can have. This is
/// set to a low value because all messages sent and received through the RPC are quite
/// small. A such, this may change in the future to a bigger value.
const MAX_TRANSFER_SIZE: usize = super::MAX_LENGTH;

/// Shared per-partner state holding the throttle and concurrency cap. Lives
/// in an `Arc` so the `tarpc`-required by-value `self` in trait methods only
/// clones the handle.
#[derive(Debug)]
struct HubAsNodeServerInner {
    /// The socket address of the _other_ Samizdat Hub this hub is connecting to.
    partner: SocketAddr,
    /// The raw RPC client.
    client: HubClient,
    /// The channel of peers that can answer queries for this node.
    candidate_channels: KeyedChannel<Candidate>,
    /// Limits the number of simultaneous RPCs a partner hub can make.
    call_semaphore: Semaphore,
    /// Limits the per-second rate of RPCs a partner hub can make.
    call_throttle: Mutex<Interval>,
}

/// The RPC server of the Samizdat Node, but implemented by a Samizdat Hub.
#[derive(Debug, Clone)]
pub struct HubAsNodeServer(Arc<HubAsNodeServerInner>);

impl HubAsNodeServer {
    /// Creates a new RPC server.
    pub fn new(
        partner: SocketAddr,
        client: HubClient,
        candidate_channels: KeyedChannel<Candidate>,
    ) -> HubAsNodeServer {
        let mut call_throttle =
            interval(Duration::from_secs_f64(1. / cli().max_query_rate_per_hub));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        HubAsNodeServer(Arc::new(HubAsNodeServerInner {
            partner,
            client,
            candidate_channels,
            call_semaphore: Semaphore::new(cli().max_queries_per_hub),
            call_throttle: Mutex::new(call_throttle),
        }))
    }

    /// Per-partner rate-limit + concurrency cap, mirroring `HubServer::throttle`.
    /// The client-facing path has had this since day one; the federation path
    /// was missing it entirely, so a misbehaving partner could blast any of
    /// the four `Node` methods at line-rate. Partners are operator-curated
    /// trust relationships but they are also the highest-impact attacker in
    /// the network (federation amplification), so bounding their load is
    /// strictly defensive.
    async fn throttle<'a, F, Fut, T>(&'a self, f: F) -> T
    where
        F: 'a + Send + FnOnce(&'a Self) -> Fut,
        Fut: 'a + Future<Output = T>,
    {
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

impl Node for HubAsNodeServer {
    async fn config(self, _: context::Context) -> NodeConfig {
        // `config` is intentionally NOT throttled: it's the cheap handshake
        // the partner uses to learn our limits. Throttling it would deadlock
        // setup.
        NodeConfig {
            max_queries: cli().max_queries_per_hub,
            max_query_rate: cli().max_query_rate_per_hub,
        }
    }

    async fn resolve(
        self,
        ctx: context::Context,
        resolution: Arc<Resolution>,
    ) -> ResolutionResponse {
        self.throttle(|s| async move {
            tracing::info!("got {:?}", resolution);

            // Se if you are not being replayed; on DB error, fail closed.
            match REPLAY_RESISTANCE.check(&*resolution) {
                Ok(true) => {}
                Ok(false) => return ResolutionResponse::NotFound,
                Err(err) => {
                    tracing::error!("replay-resistance check failed: {err}");
                    return ResolutionResponse::NotFound;
                }
            }

            if resolution.content_riddles.is_empty() {
                return ResolutionResponse::EmptyResolution;
            }

            let candidate_channel: ChannelId = ChannelId::random();
            let inner = s.0.clone();

            tokio::spawn(async move {
                let mut candidates = pin!(candidates_for_resolution(
                    ctx,
                    inner.partner,
                    Resolution::clone(&resolution),
                    inner.candidate_channels.clone(),
                ));

                while let Some(candidate) = candidates.next().await {
                    let outcome = inner
                        .client
                        .recv_candidate(ctx, candidate_channel, candidate)
                        .await;

                    if let Err(err) = outcome {
                        // Partner side is gone (timeout, disconnect, or its
                        // own upstream gave up). Stop pulling more candidates
                        // from the inner stream; otherwise this task keeps
                        // running for as long as `candidates_for_resolution`
                        // keeps producing, pushing into a dead RPC and burning
                        // a tokio task slot per query for the full chain
                        // duration.
                        tracing::debug!(
                            "stopping forwarding for channel {candidate_channel}: {err}"
                        );
                        break;
                    }
                }
            });

            ResolutionResponse::Redirect(candidate_channel)
        })
        .await
    }

    // TODO(channel-id-binding): same class of bug as the client-facing
    // `HubServer::recv_candidate`. Any connected partner hub can spray
    // `recv_candidate` with random `ChannelId`s and inject candidates into the
    // shared `candidate_channels` map, poisoning downstream client queries.
    // The proper fix is to bind `channel_id` cryptographically (HMAC over
    // `(client_addr, peer_id, server_secret)`) so peers can only deliver to
    // channels they were legitimately assigned to. Tackled in a future pass
    // alongside the equivalent client-facing path.
    async fn recv_candidate(
        self,
        _: context::Context,
        candidate_channel: ChannelId,
        candidate: Candidate,
    ) {
        self.throttle(|s| async move {
            s.0.candidate_channels.send(candidate_channel, candidate);
        })
        .await
    }

    async fn get_edition(
        self,
        ctx: context::Context,
        request: Arc<EditionRequest>,
    ) -> Vec<EditionResponse> {
        self.throttle(|s| async move {
            // Se if you are not being replayed; on DB error, fail closed.
            match REPLAY_RESISTANCE.check(&*request) {
                Ok(true) => {}
                Ok(false) => return vec![],
                Err(err) => {
                    tracing::error!("replay-resistance check failed: {err}");
                    return vec![];
                }
            }

            edition_for_request(ctx, s.0.partner, request).await
        })
        .await
    }

    async fn announce_edition(self, ctx: context::Context, announcement: Arc<EditionAnnouncement>) {
        self.throttle(|s| async move {
            // Se if you are not being replayed; on DB error, drop the announcement.
            match REPLAY_RESISTANCE.check(&*announcement) {
                Ok(true) => {}
                Ok(false) => return,
                Err(err) => {
                    tracing::error!("replay-resistance check failed: {err}");
                    return;
                }
            }

            announce_edition(ctx, s.0.partner, announcement).await
        })
        .await
    }
}

/// Connects a new hub-as-node as client to a partner hub.
async fn connect_direct(
    connection: Connection,
) -> Result<(HubClient, oneshot::Receiver<()>), crate::Error> {
    let transport = transport::open_direct_bincode_transport(connection, MAX_TRANSFER_SIZE).await?;

    let (client_reset_trigger, client_reset_recv) = oneshot::channel();
    let uninstrumented_client = HubClient::new(tarpc::client::Config::default(), transport);
    let client = NewClient {
        client: uninstrumented_client.client,
        dispatch: uninstrumented_client.dispatch.inspect(|_| {
            client_reset_trigger.send(()).ok();
        }),
    }
    .spawn();

    Ok((client, client_reset_recv))
}

/// Connects a new hub-as-node as server to a partner hub.
async fn connect_reverse(
    partner: SocketAddr,
    connection: Connection,
    client: HubClient,
    candidate_channels: KeyedChannel<Candidate>,
) -> Result<JoinHandle<()>, crate::Error> {
    tracing::info!(
        "hub-as-node connected to hub (as server) at {}",
        connection.remote_address()
    );

    let transport =
        transport::open_reverse_bincode_transport(connection, MAX_TRANSFER_SIZE).await?;

    let server_task = server::BaseChannel::with_defaults(transport)
        .execute(HubAsNodeServer::new(partner, client, candidate_channels).serve())
        .for_each(|request_task| async move {
            tokio::spawn(request_task);
        });

    Ok(tokio::spawn(server_task))
}

/// Connects a new hub-as-node to a partner hub.
async fn connect(
    endpoint: &Endpoint,
    hub_addr: SocketAddr,
) -> Result<impl Future<Output = Result<(), JoinError>>, crate::Error> {
    let candidate_channels = KeyedChannel::new();
    let connection = quic::connect(endpoint, hub_addr, true).await?;
    let (client, client_reset_recv) = connect_direct(connection.clone()).await?;
    let server_reset_recv =
        connect_reverse(hub_addr, connection, client, candidate_channels.clone()).await?;

    let reset_trigger =
        future::select(server_reset_recv, client_reset_recv).map(|selected| match selected {
            Either::Left((server_exited, _)) => server_exited,
            Either::Right((_, _)) => Ok(()),
        });

    Ok(reset_trigger)
}

/// Runs a hub-as-node server forever.
pub async fn run(partner: &str, endpoint: &Endpoint) {
    // TODO: resolve _all_ possible addresses:
    // Set up addresses
    let (_, partner) = match cli().resolution_mode.resolve(partner).await {
        Ok(resolved) => resolved.into_iter().next().expect("iterator not empty"),
        Err(err) => {
            tracing::error!("Failed to connect to partner {partner}: {err}");
            return;
        }
    };

    // Set up exponential backoff
    let start = Duration::from_millis(100);
    let max = Duration::from_secs(100);
    let mut backoff = start;

    // Exponential backoff
    loop {
        match connect(endpoint, partner).await {
            Ok(handle) => match handle.await {
                Ok(()) => {
                    tracing::info!("Hub-as-node server finished for {partner}");
                    backoff = start;
                }
                Err(err) => tracing::error!("Hub-as-node server panicked for {partner}: {err}"),
            },
            Err(err) => {
                tracing::error!("Failed to connect as hub-as-node to {partner}: {err}")
            }
        }

        time::sleep(backoff).await;
        backoff *= 2;
        backoff = if backoff > max { max } else { backoff };
    }
}
