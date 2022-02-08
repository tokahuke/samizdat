use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;

use samizdat_common::rpc::*;

use super::{announce_edition, candidates_for_resolution, latest_for_request, REPLAY_RESISTANCE};

#[derive(Debug, Clone)]
pub struct HubAsNodeServer {
    partner: SocketAddr,
}

#[tarpc::server]
impl Node for HubAsNodeServer {
    async fn resolve(
        self,
        ctx: context::Context,
        resolution: Arc<Resolution>,
    ) -> ResolutionResponse {
        log::debug!("got {:?}", resolution);

        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*resolution) {
            Ok(true) => { /* valid */ }
            Ok(false) => return ResolutionResponse::NotFound,
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return ResolutionResponse::NotFound;
            }
        }

        let candidates = candidates_for_resolution(ctx, self.partner, resolution).await;

        ResolutionResponse::Redirect(candidates)
    }

    async fn resolve_latest(
        self,
        ctx: context::Context,
        latest_request: Arc<LatestRequest>,
    ) -> Vec<LatestResponse> {
        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*latest_request) {
            Ok(true) => { /* valid */ }
            Ok(false) => return vec![],
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return vec![];
            }
        }

        latest_for_request(ctx, self.partner, latest_request).await
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
}
