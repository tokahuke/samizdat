//! Implements the RPC server part of the Hub API.

use futures::prelude::*;
use std::net::SocketAddr;
use std::pin::pin;
use std::sync::Arc;
use std::time::Instant;
use tarpc::context;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Duration, Interval, MissedTickBehavior};

use samizdat_common::address::{ChannelAddr, ChannelId};
use samizdat_common::keyed_channel::KeyedChannel;
use samizdat_common::rpc::*;

use crate::cli::cli;
use crate::models::CandidateLog;
use crate::models::ConnectionLog;
use crate::models::QueryLog;
use crate::models::{Id, Indexable};
use crate::rpc::ROOM;

use super::{announce_edition, candidates_for_resolution, edition_for_request, REPLAY_RESISTANCE};

/// The Hub server side of a client-server RPC connection.
struct HubServerInner {
    /// Id of this particular connection.
    connection_id: Id,
    /// Limits the number of simultaneous queries a node can make.
    call_semaphore: Semaphore,
    /// Limits the frequency of queries a node can make.
    call_throttle: Mutex<Interval>,
    /// The address of the node.
    addr: SocketAddr,
    /// The channel of peers that can answer queries for this node.
    candidate_channels: KeyedChannel<Candidate>,
}

/// The Hub server side of a client-server RPC connection.
#[derive(Clone)]
pub struct HubServer(Arc<HubServerInner>);

impl HubServer {
    pub fn new(addr: SocketAddr, candidate_channels: KeyedChannel<Candidate>) -> HubServer {
        let mut call_throttle =
            interval(Duration::from_secs_f64(1. / cli().max_query_rate_per_node));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        HubServer(Arc::new(HubServerInner {
            connection_id: ConnectionLog::new(addr).insert(),
            call_semaphore: Semaphore::new(cli().max_queries_per_node),
            // Store the configured throttle. The previous version built a configured
            // `call_throttle` and then dropped it on the floor, instantiating a fresh
            // `interval(...)` here with the default `Burst` missed-tick behavior, which
            // turned the per-node rate limit into "fire many ticks at once after an idle
            // period". Now we keep the one we set up.
            call_throttle: Mutex::new(call_throttle),
            addr,
            candidate_channels,
        }))
    }

    /// Does the whole API throttling thing. Using `Box` denies any allocations to the throttled
    /// client. This may mitigate DoS.
    async fn throttle<'a, F, Fut, T>(&'a self, f: F) -> T
    where
        F: 'a + Send + FnOnce(&'a Self) -> Fut,
        Fut: 'a + Future<Output = T>,
    {
        // // First, make sure we are not being trolled:
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

    async fn do_query(
        &self,
        ctx: context::Context,
        query: &Query,
        query_log_id: Id,
    ) -> QueryResponse {
        let client_addr = self.0.addr;

        // Create a channel address from peer address:
        let channel_id = ChannelId::random();
        let channel_addr = ChannelAddr::new(self.0.addr, channel_id);

        // Se if you are not being replayed:
        match REPLAY_RESISTANCE.check(query) {
            Ok(true) => {}
            Ok(false) => return QueryResponse::Replayed,
            Err(err) => {
                tracing::error!("replay-resistance check failed: {err}");
                return QueryResponse::InternalError;
            }
        }

        // If query is empty, nothing to be done:
        if query.content_riddles.is_empty() {
            tracing::debug!("query riddle empty");
            return QueryResponse::EmptyQuery;
        }

        // Now, prepare resolution request:
        let location_message_riddle = query.location_riddle.riddle_for(channel_addr);
        let resolution = Resolution {
            content_riddles: query.content_riddles.clone(),
            location_message_riddle,
            validation_nonces: vec![],
            hint: query.hint.clone(),
            kind: query.kind,
        };

        // And then create a candidate channel to forward candidate peers:
        let candidate_channel: ChannelId = ChannelId::random();

        // Get the node for the client address:
        let Some(node) = ROOM.get(client_addr).await else {
            return QueryResponse::NoReverseConnection;
        };

        // Forward all candidate peers:
        let candidate_channels = self.0.candidate_channels.clone();
        tokio::spawn(async move {
            // TODO: maybe wait some millis to make sure query response has arrived?
            let mut candidates = pin!(candidates_for_resolution(
                ctx,
                client_addr,
                resolution,
                candidate_channels.clone(),
            ));

            while let Some(candidate) = candidates.next().await {
                // Need to get socket address now because it will be moved:
                let socket_addr = candidate.socket_addr;

                // Insert the candidate log for the first time (no outcome yet):
                let mut candidate_log = CandidateLog::new(query_log_id, candidate.clone());
                candidate_log.insert();

                // Forward the candidate:
                let start = Instant::now();
                let outcome = node
                    .client
                    .recv_candidate(ctx, candidate_channel, candidate)
                    .await;

                // Update the candidate log with the outcome:
                candidate_log.update_with_outcome(
                    outcome.as_ref().map(|_| ()).map_err(|e| e.to_string()),
                    start.elapsed(),
                );
                candidate_log.insert();

                // Log the outcome:
                if let Err(err) = outcome {
                    tracing::warn!(
                        "Error sending candidate {socket_addr} to {}: {err}",
                        node.addr
                    );
                }
            }
        });

        tracing::debug!("query done");

        QueryResponse::Resolved {
            candidate_channel,
            channel_id,
        }
    }
}

impl Hub for HubServer {
    // Saving for future use.
    async fn set_property(
        self,
        _: context::Context,
        _key: String,
        _value: serde_json::Value,
    ) -> SetPropertyResponse {
        self.throttle(|_| async { SetPropertyResponse::Unsupported })
            .await
    }

    async fn query(self, ctx: context::Context, query: Query) -> QueryResponse {
        self.throttle(move |server| async move {
            tracing::debug!("got {:?}", query);

            // Insert the query log for the first time (no response yet):
            let mut query_log = QueryLog::new(server.0.connection_id, query.clone());
            query_log.insert();

            // Do the query:
            let start = Instant::now();
            let response = server.do_query(ctx, &query, query_log.id()).await;

            // Update the query log with the response:
            query_log.update_with_response(response.clone(), start.elapsed());
            query_log.insert();

            response
        })
        .await
    }

    async fn recv_candidate(
        self,
        _: context::Context,
        candidate_channel: ChannelId,
        candidate: Candidate,
    ) {
        self.throttle(|server| async move {
            server
                .0
                .candidate_channels
                .send(candidate_channel, candidate)
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
            // Se if you are not being replayed; treat a DB error as fail-closed.
            match REPLAY_RESISTANCE.check(&request) {
                Ok(true) => {}
                Ok(false) => return vec![],
                Err(err) => {
                    tracing::error!("replay-resistance check failed: {err}");
                    return vec![];
                }
            }

            // Now broadcast the request:
            edition_for_request(ctx, client_addr, Arc::new(request)).await
        })
        .await
    }

    async fn announce_edition(self, ctx: context::Context, announcement: EditionAnnouncement) {
        let client_addr = self.0.addr;
        self.throttle(|_| async move {
            // Se if you are not being replayed; on DB error, drop the announcement.
            match REPLAY_RESISTANCE.check(&announcement) {
                Ok(true) => {}
                Ok(false) => return,
                Err(err) => {
                    tracing::error!("replay-resistance check failed: {err}");
                    return;
                }
            }

            // Now, broadcast the announcement:
            announce_edition(ctx, client_addr, Arc::new(announcement)).await
        })
        .await
    }
}
