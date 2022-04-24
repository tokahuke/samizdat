use quinn::Endpoint;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::task::JoinHandle;
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
}

impl HubAsNodeServer {
    pub fn new(partner: SocketAddr) -> HubAsNodeServer {
        HubAsNodeServer { partner }
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

        let candidates =
            candidates_for_resolution(ctx, self.partner, Resolution::clone(&resolution)).await;

        ResolutionResponse::Redirect(candidates)
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

/// Connects a new hub-as-node server to a partner.
async fn connect(
    partner: &crate::cli::AddrToResolve,
    endpoint: &Endpoint,
) -> Result<JoinHandle<()>, crate::Error> {
    let (_, partner) = partner.resolve().await?;
    let new_connection = samizdat_common::quic::connect(endpoint, partner).await?;

    log::info!(
        "hub-as-node connected to hub at {}",
        new_connection.connection.remote_address()
    );

    let transport = BincodeOverQuic::new(
        new_connection.connection.clone(),
        new_connection.uni_streams,
        MAX_TRANSFER_SIZE,
    );

    let server_task = server::BaseChannel::with_defaults(transport)
        .execute(HubAsNodeServer::new(partner).serve());

    Ok(tokio::spawn(server_task))
}

/// Runs a hub-as-node server forever.
pub async fn run(partner: &crate::cli::AddrToResolve, endpoint: &Endpoint) {
    let start = Duration::from_millis(100);
    let max = Duration::from_secs(100);
    let mut backoff = start;

    loop {
        match connect(&partner, &endpoint).await {
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
