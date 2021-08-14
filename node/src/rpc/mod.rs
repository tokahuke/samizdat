mod connection_manager;
mod file_transfer;
mod reconnect;

pub use reconnect::Reconnect;

use futures::prelude::*;
use futures::stream;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::client::NewClient;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::oneshot;
use tokio::time::Duration;

use samizdat_common::quic;
use samizdat_common::rpc::{
    HubClient, Node, Query, QueryKind, QueryResponse, Resolution, ResolutionResponse,
};
use samizdat_common::{ContentRiddle, Hash};

use crate::cache::ObjectRef;

use connection_manager::{ConnectionManager, DropMode};

#[derive(Clone)]
struct NodeServer {
    connection_manager: Arc<ConnectionManager>,
}

#[tarpc::server]
impl Node for NodeServer {
    async fn resolve(self, _: context::Context, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got {:?}", resolution);

        let object = match ObjectRef::find(&resolution.content_riddle) {
            Some(object) => object,
            None => {
                log::info!("hash not found for resolution");
                return ResolutionResponse::NOT_FOUND;
            }
        };

        // Code smell?
        let hash = object.hash;

        log::info!("found hash {}", object.hash);
        let peer_addr = match resolution.message_riddle.resolve(&hash) {
            Some(message) => message.socket_addr,
            None => {
                log::warn!("failed to resolve message riddle after resolving content riddle");
                return ResolutionResponse::FOUND;
            }
        };

        log::info!("found peer at {}", peer_addr);

        tokio::spawn(
            async move {
                let new_connection = self
                    .connection_manager
                    .punch_hole_to(peer_addr, DropMode::DropIncoming)
                    .await?;
                file_transfer::send(&new_connection.connection, object).await
            }
            .map(move |outcome| {
                outcome
                    .map_err(|err| log::error!("failed to send {} to {}: {}", hash, peer_addr, err))
            }),
        );

        return ResolutionResponse::FOUND;
    }
}

pub struct HubConnectionInner {
    client: HubClient,
    connection_manager: Arc<ConnectionManager>,
}

impl HubConnectionInner {
    async fn connect_direct(
        direct_addr: SocketAddr,
        connection_manager: Arc<ConnectionManager>,
    ) -> Result<(HubClient, oneshot::Receiver<()>), crate::Error> {
        let (client_reset_trigger, client_reset_recv) = oneshot::channel();

        // Create transport for client and create client:
        let transport = connection_manager
            .transport(&direct_addr, "localhost")
            .await?;
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

    async fn connect_reverse(
        reverse_addr: SocketAddr,
        connection_manager: Arc<ConnectionManager>,
    ) -> Result<oneshot::Receiver<()>, crate::Error> {
        let (server_reset_trigger, server_reset_recv) = oneshot::channel();

        // Create transport for server and spawn server:
        let transport = connection_manager
            .transport(&reverse_addr, "localhost")
            .await?;
        let server_task = server::BaseChannel::with_defaults(transport).execute(
            NodeServer {
                connection_manager: connection_manager.clone(),
            }
            .serve(),
        );
        tokio::spawn(async move {
            server_task.await;
            server_reset_trigger.send(()).ok();
        });

        Ok(server_reset_recv)
    }

    async fn connect(
        direct_addr: SocketAddr,
        reverse_addr: SocketAddr,
    ) -> Result<(HubConnectionInner, impl Future<Output = ()>), crate::Error> {
        // Connect and create connection manager:
        let (endpoint, incoming) = quic::new_default("[::]:0".parse().expect("valid address"));
        let connection_manager = Arc::new(ConnectionManager::new(endpoint, incoming));

        let (client, client_reset_recv) =
            Self::connect_direct(direct_addr, connection_manager.clone()).await?;
        let server_reset_recv =
            Self::connect_reverse(reverse_addr, connection_manager.clone()).await?;

        let reset_trigger = future::select(server_reset_recv, client_reset_recv).map(|_| ());

        Ok((
            HubConnectionInner {
                client,
                connection_manager,
            },
            reset_trigger,
        ))
    }
}

pub struct HubConnection {
    name: &'static str,
    inner: Reconnect<HubConnectionInner>,
}

impl HubConnection {
    pub async fn connect(
        name: &'static str,
        direct_addr: SocketAddr,
        reverse_addr: SocketAddr,
    ) -> Result<HubConnection, crate::Error> {
        Ok(HubConnection {
            name,
            inner: Reconnect::init(
                move || HubConnectionInner::connect(direct_addr, reverse_addr),
                || {
                    reconnect::exponential_backoff(
                        Duration::from_millis(100),
                        Duration::from_secs(100),
                    )
                },
            )
            .await?,
        })
    }

    pub async fn query(&self, content_hash: Hash) -> Result<Option<ObjectRef>, crate::Error> {
        let content_riddle = ContentRiddle::new(&content_hash);
        let location_riddle = ContentRiddle::new(&content_hash);

        let inner = self.inner.get().await;

        let query_response = inner
            .client
            .query(
                context::current(),
                Query {
                    content_riddle,
                    location_riddle,
                    kind: QueryKind::Object,
                },
            )
            .await?;

        let candidates = match query_response {
            QueryResponse::Replayed => {
                return Err(format!("hub has suspected replay attack").into())
            }
            QueryResponse::InternalError => {
                return Err(format!("hub has experienced an internal error").into())
            }
            QueryResponse::Resolved { candidates } => candidates,
        };

        log::info!("Candidates for {}: {:?}", content_hash, candidates);

        if candidates.is_empty() {
            // Forget it!
            return Ok(None);
        }

        let n_candidates = candidates.len();
        let new_connection = stream::iter(candidates)
            .map(|candidate| {
                let connection_manager = inner.connection_manager.clone();
                Box::pin(async move {
                    connection_manager
                        .punch_hole_to(candidate, DropMode::DropOutgoing)
                        .await
                        .map_err(|err| {
                            log::warn!("hole punching with {} failed: {}", candidate, err)
                        })
                        .ok()
                })
            })
            .buffer_unordered(n_candidates)
            .filter_map(|done| Box::pin(async move { done })) // pointless box, compiler!
            .next()
            .await;

        match new_connection {
            Some(mut new_connection) => Ok(Some(
                file_transfer::recv(&mut new_connection.uni_streams, content_hash).await?,
            )),
            None => Err(crate::Error::AllCandidatesFailed),
        }
    }
}

pub struct Hubs {
    hubs: Vec<Arc<HubConnection>>,
}

impl Hubs {
    pub async fn init<I>(addrs: I) -> Result<Hubs, crate::Error>
    where
        I: IntoIterator<Item = (&'static str, SocketAddr)>,
    {
        let hubs = stream::iter(addrs)
            .map(|(name, addr)| {
                let direct_addr = addr;
                let mut reverse_addr = addr;
                reverse_addr.set_port(reverse_addr.port() + 1);

                HubConnection::connect(name, direct_addr, reverse_addr)
            })
            .buffer_unordered(10) // 'cause 10!
            .map(|outcome| outcome.map(Arc::new))
            .try_collect::<Vec<_>>()
            .await?;

        Ok(Hubs { hubs })
    }

    pub async fn query(&self, content_hash: Hash) -> Option<ObjectRef> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.query(content_hash).await) })
            .buffer_unordered(self.hubs.len());

        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(Some(found)) => return Some(found),
                Ok(None) => {}
                Err(err) => {
                    log::error!("Error while querying {}: {}", hub_name, err)
                }
            }
        }

        None
    }
}
