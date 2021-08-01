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

use samizdat_common::rpc::{Hub, NodeClient, Query, Resolution, Status};
use samizdat_common::transport;

use crate::CLI;

use room::{Participant, Room};

lazy_static! {
    static ref ROOM: Room<NodeClient> = Room::new();
}

struct HubServerInner {
    call_semaphore: Semaphore,
    call_throttle: Mutex<Interval>,
    client_addr: SocketAddr,
    client: Participant<NodeClient>,
}

#[derive(Clone)]
struct HubServer(Arc<HubServerInner>);

impl HubServer {
    fn new(client_addr: SocketAddr, client: NodeClient) -> HubServer {
        let client = ROOM.insert(client);

        let mut call_throttle = interval(Duration::from_secs_f64(1. / CLI.max_query_rate_per_node));
        call_throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);

        HubServer(Arc::new(HubServerInner {
            call_semaphore: Semaphore::new(CLI.max_queries_per_node),
            call_throttle: Mutex::new(interval(Duration::from_secs_f64(1. / CLI.max_query_rate_per_node))),
            client_addr,
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
    async fn query(self, ctx: context::Context, query: Query) -> Status {
        self.throttle(Box::new(|server| async move {
            // Now, prepare resolution request:
            log::debug!("got {:?}", query);
            let client_id = server.0.client.id();
            let location_riddle = query.location_riddle.riddle_for_location(server.0.client_addr);
            let resolution = Arc::new(Resolution {
                content_riddle: query.content_riddle,
                location_riddle,
            });

            // And then send the request to the peers:
            server.0.client
                .stream_peers()
                .for_each_concurrent(Some(CLI.max_resolutions_per_query), |(peer_id, peer)| {
                    if peer_id != client_id {
                        // TODO
                    }
                    let resolution = resolution.clone();
                    async move {
                        log::debug!("starting resolve");
                        peer.resolve(ctx, resolution).await.unwrap();
                        log::debug!("resolve done");
                    }
                })
                .await;

            log::debug!("query done");

            Status::Found
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
