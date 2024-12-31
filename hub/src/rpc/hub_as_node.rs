//! Implements the RPC client part of the Hub API _for the Samizdat Hub_. This describes
//! how a Samizdat Hub can behave as another node to another Samizdat Hub. This confers
//! recursion to the Samizdat network.

use futures::future::Either;
use futures::prelude::*;
use samizdat_common::keyed_channel::KeyedChannel;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client::NewClient;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::oneshot;
use tokio::task::{JoinError, JoinHandle};
use tokio::time;

use samizdat_common::address::ChannelId;
use samizdat_common::quinn::{Connection, Endpoint};
use samizdat_common::{quic, rpc::*, transport};

use crate::cli::cli;

use super::{announce_edition, candidates_for_resolution, edition_for_request, REPLAY_RESISTANCE};

/// The maximum length in bytes that a message in the RPC connections can have. This is
/// set to a low value because all messages sent and received through the RPC are quite
/// small. A such, this may change in the future to a bigger value.
const MAX_TRANSFER_SIZE: usize = super::MAX_LENGTH;

/// The RPC server of the Samizdat Node, but implemented by a Samizdat Hub.
#[derive(Debug, Clone)]
pub struct HubAsNodeServer {
    /// The socket address of the _other_ Samizdat Hub this hub is connecting to.
    partner: SocketAddr,
    /// The raw RPC client.
    client: HubClient,
    /// The channel of peers that can answer queries for this node.
    candidate_channels: KeyedChannel<Candidate>,
}

impl HubAsNodeServer {
    /// Creates a new RPC server.
    pub fn new(
        partner: SocketAddr,
        client: HubClient,
        candidate_channels: KeyedChannel<Candidate>,
    ) -> HubAsNodeServer {
        HubAsNodeServer {
            partner,
            client,
            candidate_channels,
        }
    }
}

impl Node for HubAsNodeServer {
    async fn config(self, _: context::Context) -> NodeConfig {
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
        tracing::info!("got {:?}", resolution);

        // Se if you are not being replayed:
        if !REPLAY_RESISTANCE.lock().await.check(&*resolution) {
            return ResolutionResponse::NotFound;
        }

        if resolution.content_riddles.is_empty() {
            return ResolutionResponse::EmptyResolution;
        }

        let candidate_channel: ChannelId = rand::random::<u32>().into();

        tokio::spawn(async move {
            let candidates = candidates_for_resolution(
                ctx,
                self.partner,
                Resolution::clone(&resolution),
                self.candidate_channels.clone(),
            );
            let mut pinned = Box::pin(candidates);

            while let Some(candidate) = pinned.next().await {
                let outcome = self
                    .client
                    .recv_candidate(ctx, candidate_channel, candidate)
                    .await;

                if let Err(err) = outcome {
                    tracing::error!(
                        "Failed to send candidate to channel {candidate_channel}: {err}"
                    );
                }
            }
        });

        ResolutionResponse::Redirect(candidate_channel)
    }

    async fn recv_candidate(
        self,
        _: context::Context,
        candidate_channel: ChannelId,
        candidate: Candidate,
    ) {
        self.candidate_channels.send(candidate_channel, candidate);
    }

    async fn get_edition(
        self,
        ctx: context::Context,
        request: Arc<EditionRequest>,
    ) -> Vec<EditionResponse> {
        // Se if you are not being replayed:
        if !REPLAY_RESISTANCE.lock().await.check(&*request) {
            return vec![];
        }

        edition_for_request(ctx, self.partner, request).await
    }

    async fn announce_edition(self, ctx: context::Context, announcement: Arc<EditionAnnouncement>) {
        // Se if you are not being replayed:
        if !REPLAY_RESISTANCE.lock().await.check(&*announcement) {
            return;
        }

        announce_edition(ctx, self.partner, announcement).await
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
