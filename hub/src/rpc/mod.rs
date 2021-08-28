mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use quinn::Endpoint;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Duration, Interval, MissedTickBehavior};

use samizdat_common::rpc::*;
use samizdat_common::BincodeOverQuic;

use crate::replay_resistance::ReplayResistance;
use crate::CLI;

use room::Room;

const MAX_LENGTH: usize = 2_048;

lazy_static! {
    static ref ROOM: Room = Room::new();
    static ref REPLAY_RESISTANCE: Mutex<ReplayResistance> = Mutex::new(ReplayResistance::new());
}

#[derive(Debug)]
struct Node {
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
    async fn throttle<'a, Fut, T>(&'a self, f: Box<dyn 'a + Send + FnOnce(&'a Self) -> Fut>) -> T
    where
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
    // async fn reverse_port(self, _: context::Context) -> u16 {
    //     CLI.reverse_port
    // }

    async fn query(self, ctx: context::Context, query: Query) -> QueryResponse {
        let client_addr = self.0.addr;
        self.throttle(Box::new(|server| async move {
            log::debug!("got {:?}", query);

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
            let message_riddle = query.content_riddle.riddle_for(server.0.addr);
            let resolution = Arc::new(Resolution {
                content_riddle: query.content_riddle,
                message_riddle,
                kind: query.kind,
            });

            // And then send the request to the peers:
            let kind = query.kind;
            let candidates = ROOM
                .with_peers(client_addr, move |peer_id, peer| {
                    let resolution = resolution.clone();
                    async move {
                        log::debug!("starting resolve for {}", peer_id);

                        let outcome = match kind {
                            QueryKind::Object => peer.client.resolve_object(ctx, resolution).await,
                            QueryKind::Item => peer.client.resolve_item(ctx, resolution).await,
                        };

                        let response = match outcome {
                            Ok(response) => response,
                            Err(err) => {
                                log::warn!("error asking {} to resolve: {}", peer_id, err);
                                return None;
                            }
                        };

                        log::debug!("resolve done for {}", peer_id);

                        if let ResolutionStatus::Found = response.status {
                            Some(peer.addr)
                        } else {
                            None
                        }
                    }
                })
                .await;

            log::debug!("query done");

            QueryResponse::Resolved { candidates }
        }))
        .await
    }

    async fn get_latest(self, ctx: context::Context, latest: LatestRequest) -> Vec<LatestResponse> {
        let client_addr = self.0.addr;

        // Now broadcast the request:
        let latest = Arc::new(latest);

        let responses = ROOM
            .with_peers(client_addr, |peer_id, peer| {
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

                    response
                }
            })
            .await;

        responses
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
        .for_each(|new_connection| async move {
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
                dispatch: uninstrumented_client.dispatch.map(move |outcome| {
                    ROOM.remove(client_addr);
                    outcome
                }),
            }
            .spawn();

            log::info!("Connection from node (as client) {} accepted", client_addr);

            ROOM.insert(
                client_addr,
                Node {
                    client,
                    addr: client_addr,
                },
            );
        })
        // Max number of channels.
        .await;

    Ok(())
}
