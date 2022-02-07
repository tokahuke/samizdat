use std::sync::Arc;
use tarpc::context;
use std::net::{SocketAddr};

use samizdat_common::rpc::*;

use super::{REPLAY_RESISTANCE, candidates_for_resolution};

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
        let client_addr = self.partner;
        log::debug!("got {:?}", resolution);

        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.lock().await.check(&*resolution) {
            Ok(true) => { /* valid */ },
            Ok(false) => return QueryResponse::Replayed,
            Err(err) => {
                log::error!("error while checking for replay: {}", err);
                return QueryResponse::InternalError;
            }
        }

        let candidates = candidates_for_resolution(ctx, client_addr, resolution).await;

        todo!()
    }

    async fn resolve_latest(
        self,
        _: context::Context,
        latest: Arc<LatestRequest>,
    ) -> Option<LatestResponse> {
        todo!()
    }

    async fn announce_edition(self, _: context::Context, announcement: Arc<EditionAnnouncement>) {
        todo!()
    }
}
