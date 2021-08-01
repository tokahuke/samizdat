mod room;

use futures::prelude::*;
use lazy_static::lazy_static;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, Interval, Duration, MissedTickBehavior};
use tokio_stream::wrappers::TcpListenerStream;

use samizdat_common::rpc::{Hub, NodeClient, Query, QueryResponse, Resolution, ResolutionStatus};
use samizdat_common::transport;

use crate::CLI;

use room::{Participant, Room};

lazy_static! {
    static ref ROOM: Room<Node> = Room::new();
}

struct Node {
    client: NodeClient,
    addr: SocketAddr,
}

struct HubServerInner {
    call_semaphore: Semaphore,
    call_throttle: Mutex<Interval>,
    client: Participant<Node>,
}

#[derive(Clone)]
struct HubServer(Arc<HubServerInner>);

impl HubServer {
    fn new(addr: SocketAddr, client: NodeClient) -> HubServer {
        let client = ROOM.insert(Node { client, addr });

        let mut call_throttle = interval(Duration::from_secs_f64(1. / CLI.max_query_rate_per_node));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        HubServer(Arc::new(HubServerInner {
            call_semaphore: Semaphore::new(CLI.max_queries_per_node),
            call_throttle: Mutex::new(interval(Duration::from_secs_f64(1. / CLI.max_query_rate_per_node))),
            client,
        }))
    }

    /// Does the whole API throttling thing. Using `Box` denies any allocations to the throttled
    /// client. This may mitigate DoS.
    async fn throttle<'a, Fut, T>(&'a self, f: Box<dyn 'a + Send + FnOnce(&'a Self) -> Fut>) -> T
    where
        Fut: 'a + Future<Output=T>
    {
        // First, make sure we are not being trolled:
        self.0.call_throttle.lock().await.tick().await;
        let permit = self.0.call_semaphore.acquire().await.expect("semaphore never closed");
        
        let outcome = f(self).await;

        drop(permit);
        outcome
    }
}

#[tarpc::server]
impl Hub for HubServer {
    async fn query(self, ctx: context::Context, query: Query) -> QueryResponse {
        self.throttle(Box::new(|server| async move {
            // Now, prepare resolution request:
            log::debug!("got {:?}", query);
            let client_id = server.0.client.id();
            let location_riddle = query.location_riddle.riddle_for_location(server.0.client.addr);
            let resolution = Arc::new(Resolution {
                content_riddle: query.content_riddle,
                location_riddle,
            });

            // And then send the request to the peers:
            let candidates = server.0.client
                .stream_peers()
                .map(|(peer_id, peer)| {
                    if peer_id != client_id {
                        // TODO
                    }

                    let resolution = resolution.clone();
                    async move {
                        log::debug!("starting resolve for {}", peer_id);
                        let response = peer.client.resolve(ctx, resolution).await.unwrap();
                        log::debug!("resolve done for {}", peer_id);
                        (peer.addr, response)
                    }
                })
                .buffer_unordered(CLI.max_resolutions_per_query)
                .filter_map(|(addr, response)| async move { if let ResolutionStatus::Found = response.status {
                    Some(addr)
                } else {
                    None
                }})
                .take(CLI.max_candidates)
                .collect::<Vec<_>>()
                .await;

            log::debug!("query done");

            QueryResponse { candidates }
        })).await
    }
}

pub async fn run(addr: impl Into<SocketAddr>) -> Result<(), io::Error> {
    let listener = TcpListener::bind(addr.into()).await?;

    TcpListenerStream::new(listener)
        .filter_map(|r| async move {
            r.map_err(|err| log::warn!("failed to establish TCP connection: {}", err))
                .ok()
        })
        .then(|t| async move {
            // Get peer address:
            let client_addr = t
                .peer_addr()
                .map_err(|err| log::warn!("could not get peer address for connection: {}", err))
                .ok()?;

            log::info!("Incoming connection from {}", client_addr);

            // Multiplex connection:
            let multiplex = transport::Multiplex::new(t);
            let direct = multiplex
                .channel(0)
                .await
                .expect("channel 0 in use unexpectedly");
            let reverse = multiplex
                .channel(1)
                .await
                .expect("channel 1 in use unexpectedly");

            // Set up client:
            let client = 
                NodeClient::new(tarpc::client::Config::default(), reverse)
                    .spawn()
                    .map_err(|err| {
                        log::warn!("failed to spawn client from {}: {}", client_addr, err)
                    })
                    .ok()?;
            
            // Set up server:
            let server = HubServer::new(client_addr, client);
            let server_task = server::BaseChannel::with_defaults(direct).execute(server.serve());

            Some(server_task)
        })
        .filter_map(|maybe_server| async move { maybe_server })
        // Max number of channels.
        .buffer_unordered(CLI.max_connections)
        .for_each(|_| async {})
        .await;

    Ok(())
}
