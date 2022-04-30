use futures::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Duration, Interval, MissedTickBehavior};

use samizdat_common::rpc::*;
use samizdat_common::ChannelAddr;

use crate::CLI;

use super::{
    announce_edition, candidates_for_resolution, edition_for_request, get_identity,
    REPLAY_RESISTANCE,
};

struct HubServerInner {
    call_semaphore: Semaphore,
    call_throttle: Mutex<Interval>,
    addr: SocketAddr,
}

#[derive(Clone)]
pub struct HubServer(Arc<HubServerInner>);

impl HubServer {
    pub fn new(addr: SocketAddr) -> HubServer {
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

            // If query is empty, nothing to be done:
            if query.content_riddles.is_empty() {
                log::debug!("query riddle empty");
                return QueryResponse::EmptyQuery;
            }

            // Now, prepare resolution request:
            let location_message_riddle = query.location_riddle.riddle_for(channel_addr);
            let resolution = Resolution {
                content_riddles: query.content_riddles,
                location_message_riddle,
                validation_nonces: vec![],
                kind: query.kind,
            };

            // And then send the request to the peers:
            let candidates = candidates_for_resolution(ctx, client_addr, resolution).await;

            log::debug!("query done");

            QueryResponse::Resolved {
                candidates: candidates
                    .into_iter()
                    .map(|candidate| ChannelAddr::new(candidate.peer_addr, channel))
                    .collect(),
            }
        })
        .await
    }

    async fn get_edition(
        self,
        ctx: context::Context,
        request: EditionRequest,
    ) -> Vec<EditionResponse> {
        let client_addr = self.0.addr;
        self.throttle(|_| async move {
            // Se if you are not being replayed:
            match REPLAY_RESISTANCE.lock().await.check(&request) {
                Ok(false) => return vec![],
                Err(err) => {
                    log::error!("error while checking for replay: {}", err);
                    return vec![];
                }
                _ => {}
            }

            // Now broadcast the request:
            edition_for_request(ctx, client_addr, Arc::new(request)).await
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

    async fn get_identity(
        self,
        ctx: context::Context,
        request: IdentityRequest,
    ) -> Vec<IdentityResponse> {
        let client_addr = self.0.addr;
        self.throttle(|_| async move {
            // Se if you are not being replayed:
            match REPLAY_RESISTANCE.lock().await.check(&request) {
                Ok(false) => return vec![],
                Err(err) => {
                    log::error!("error while checking for replay: {}", err);
                    return vec![];
                }
                _ => {}
            }

            // Now, broadcast the announcement:
            get_identity(ctx, client_addr, Arc::new(request)).await
        })
        .await
    }

    async fn announce_identity(self, _ctx: context::Context, _announcement: IdentityAnnouncement) {
        unimplemented!()
    }
}
