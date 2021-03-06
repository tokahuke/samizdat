use futures::future::Either;
use futures::prelude::*;
use quinn::Endpoint;
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

use samizdat_common::rpc::*;
use samizdat_common::BincodeOverQuic;

use super::{
    announce_edition, candidates_for_resolution, edition_for_request, get_identity,
    REPLAY_RESISTANCE,
};

const MAX_TRANSFER_SIZE: usize = 2_048;

#[derive(Debug, Clone)]
pub struct HubAsNodeServer {
    partner: SocketAddr,
    client: HubClient,
    candidate_channels: KeyedChannel<Candidate>,
}

impl HubAsNodeServer {
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

#[tarpc::server]
impl Node for HubAsNodeServer {
    async fn resolve(
        self,
        ctx: context::Context,
        resolution: Arc<Resolution>,
    ) -> ResolutionResponse {
        log::info!("got {:?}", resolution);

        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*resolution) {
            Ok(true) => { /* valid */ }
            Ok(false) => return ResolutionResponse::NotFound,
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return ResolutionResponse::NotFound;
            }
        }

        if resolution.content_riddles.is_empty() {
            return ResolutionResponse::EmptyResolution;
        }

        let candidate_channel: CandidateChannelId = rand::random();

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
                    .recv_candidate(ctx.clone(), candidate_channel, candidate)
                    .await;

                if let Err(err) = outcome {
                    log::error!("Failed to send candidate to channel {candidate_channel}: {err}");
                }
            }
        });

        ResolutionResponse::Redirect(candidate_channel)
    }

    async fn recv_candidate(
        self,
        _: context::Context,
        candidate_channel: CandidateChannelId,
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
        match REPLAY_RESISTANCE.lock().await.check(&*request) {
            Ok(true) => { /* valid */ }
            Ok(false) => return vec![],
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return vec![];
            }
        }

        edition_for_request(ctx, self.partner, request).await
    }

    async fn announce_edition(self, ctx: context::Context, announcement: Arc<EditionAnnouncement>) {
        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*announcement) {
            Ok(true) => { /* valid */ }
            Ok(false) => return,
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return;
            }
        }

        announce_edition(ctx, self.partner, announcement).await
    }

    async fn get_identity(
        self,
        ctx: context::Context,
        request: Arc<IdentityRequest>,
    ) -> Vec<IdentityResponse> {
        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*request) {
            Ok(true) => { /* valid */ }
            Ok(false) => return vec![],
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return vec![];
            }
        }

        get_identity(ctx, self.partner, request).await
    }

    async fn announce_identity(
        self,
        _ctx: context::Context,
        _announcement: Arc<IdentityAnnouncement>,
    ) {
        // TODO: this is a no-op by now.
    }
}

/// Connects a new hub-as-node as client to a partner hub.
async fn connect_direct(
    direct_addr: SocketAddr,
    endpoint: &Endpoint,
) -> Result<(HubClient, oneshot::Receiver<()>), crate::Error> {
    let new_connection = samizdat_common::quic::connect(endpoint, direct_addr).await?;

    log::info!(
        "hub-as-node connected to hub (as client) at {}",
        new_connection.connection.remote_address()
    );

    let transport = BincodeOverQuic::new(
        new_connection.connection.clone(),
        new_connection.uni_streams,
        MAX_TRANSFER_SIZE,
    );

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
    reverse_addr: SocketAddr,
    endpoint: &Endpoint,
    client: HubClient,
    candidate_channels: KeyedChannel<Candidate>,
) -> Result<JoinHandle<()>, crate::Error> {
    let new_connection = samizdat_common::quic::connect(endpoint, reverse_addr).await?;

    log::info!(
        "hub-as-node connected to hub (as server) at {}",
        new_connection.connection.remote_address()
    );

    let transport = BincodeOverQuic::new(
        new_connection.connection.clone(),
        new_connection.uni_streams,
        MAX_TRANSFER_SIZE,
    );

    let server_task = server::BaseChannel::with_defaults(transport)
        .execute(HubAsNodeServer::new(reverse_addr, client, candidate_channels).serve());

    Ok(tokio::spawn(server_task))
}

/// Connects a new hub-as-node to a partner hub.
async fn connect(
    endpoint: &Endpoint,
    direct_addr: SocketAddr,
    reverse_addr: SocketAddr,
) -> Result<impl Future<Output = Result<(), JoinError>>, crate::Error> {
    let candidate_channels = KeyedChannel::new();
    let (client, client_reset_recv) = connect_direct(direct_addr, endpoint).await?;
    let server_reset_recv =
        connect_reverse(reverse_addr, endpoint, client, candidate_channels.clone()).await?;

    let reset_trigger =
        future::select(server_reset_recv, client_reset_recv).map(|selected| match selected {
            Either::Left((server_exited, _)) => server_exited,
            Either::Right((_, _)) => Ok(()),
        });

    Ok(reset_trigger)
}

/// Runs a hub-as-node server forever.
pub async fn run(partner: &crate::cli::AddrToResolve, endpoint: &Endpoint) {
    // Set up addresses
    let (_, partner) = match partner.resolve().await {
        Ok(resolved) => resolved,
        Err(err) => {
            log::error!("Failed to connect to partner {partner}: {err}");
            return;
        }
    };
    let direct_addr = partner;
    let reverse_addr = (partner.ip(), partner.port() + 1).into();

    // Set up exponential backoff
    let start = Duration::from_millis(100);
    let max = Duration::from_secs(100);
    let mut backoff = start;

    // Exponential backoff
    loop {
        match connect(endpoint, direct_addr, reverse_addr).await {
            Ok(handle) => match handle.await {
                Ok(()) => {
                    log::info!("Hub-as-node server finished for {partner}");
                    backoff = start;
                }
                Err(err) => log::error!("Hub-as-node server panicked for {partner}: {err}"),
            },
            Err(err) => {
                log::error!("Failed to connect as hub-as-node to {partner}: {err}")
            }
        }

        time::sleep(backoff).await;
        backoff *= 2;
        backoff = if backoff > max { max } else { backoff };
    }
}
