//! Implmentation of the node behavior in the Samizdat network, both with hubs and with
//! other nodes.

mod file_transfer;
mod node_server;
mod reconnect;
mod transport;

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

use samizdat_common::cipher::TransferCipher;
use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::{ContentRiddle, Hash};

use crate::cli;
use crate::models::{ObjectRef, SeriesItem, SeriesRef};

use node_server::NodeServer;
use transport::{ChannelManager, ConnectionManager};

/// A single connection instance, which will be recreates by [`Reconnect`] on connection loss.
pub struct HubConnectionInner {
    client: HubClient,
    // connection_manager: Arc<ConnectionManager>,
    channel_manager: Arc<ChannelManager>,
}

impl HubConnectionInner {
    /// Creates the RPC connection from the Node to the Hub.
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

    /// Creates the RPC connection from the Hub to the Node.
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
                channel_manager: Arc::new(ChannelManager::new(connection_manager.clone())),
            }
            .serve(),
        );
        tokio::spawn(async move {
            server_task.await;
            server_reset_trigger.send(()).ok();
        });

        Ok(server_reset_recv)
    }

    /// Creates the two connections between hub and node: RPC from node to hub and RPC from
    /// hub to node.
    async fn connect(
        direct_addr: SocketAddr,
        reverse_addr: SocketAddr,
    ) -> Result<(HubConnectionInner, impl Future<Output = ()>), crate::Error> {
        // Connect and create connection manager:
        let (endpoint, incoming) = quic::new_default("[::]:0".parse().expect("valid address"));
        let connection_manager = Arc::new(ConnectionManager::new(endpoint, incoming));
        let channel_manager = Arc::new(ChannelManager::new(connection_manager.clone()));
        let (client, client_reset_recv) =
            Self::connect_direct(direct_addr, connection_manager.clone()).await?;
        let server_reset_recv =
            Self::connect_reverse(reverse_addr, connection_manager.clone()).await?;

        let reset_trigger = future::select(server_reset_recv, client_reset_recv).map(|_| ());

        Ok((
            HubConnectionInner {
                client,
                // connection_manager,
                channel_manager,
            },
            reset_trigger,
        ))
    }
}

/// A connection to a single node, already resilient to reconnects.
pub struct HubConnection {
    name: &'static str,
    inner: Reconnect<HubConnectionInner>,
}

impl HubConnection {
    /// Creates a connection to the hub.
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

    /// Makes a query to this hub.
    pub async fn query(
        &self,
        content_hash: Hash,
        kind: QueryKind,
    ) -> Result<Option<ObjectRef>, crate::Error> {
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
                    kind,
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
        let channel = stream::iter(candidates)
            .map(|candidate| {
                let channel_manager = inner.channel_manager.clone();
                Box::pin(async move {
                    channel_manager
                        .expect(candidate)
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

        // TODO: minor improvement... could we tee the object stream directly to the user? By now,
        // we are waiting for the whole object to arrive, which is fine for most files, but ca be
        // a pain for the bigger ones...
        let outcome = match channel {
            Some((_sender, receiver)) => Ok(Some(match kind {
                QueryKind::Object => file_transfer::recv_object(receiver, content_hash).await?,
                QueryKind::Item => file_transfer::recv_item(receiver, content_hash).await?,
            })),
            None => Err(crate::Error::AllCandidatesFailed),
        };

        log::info!("query done: {:?} {}", kind, content_hash);

        outcome
    }

    /// Tries to resolve the latest item of a given series.
    pub async fn get_latest(&self, series: &SeriesRef) -> Result<Option<SeriesItem>, crate::Error> {
        let key_riddle = ContentRiddle::new(&series.public_key.hash());
        let inner = self.inner.get().await;

        let response = inner
            .client
            .get_latest(context::current(), LatestRequest { key_riddle })
            .await?;

        let mut most_recent: Option<SeriesItem> = None;

        for candidate in response {
            let cipher = TransferCipher::new(&series.public_key.hash(), &candidate.rand);
            let candidate_item: SeriesItem = candidate.series.decrypt_with(&cipher)?;

            if !candidate_item.is_valid() {
                log::warn!("received invalid candidate item: {:?}", candidate_item);
                continue;
            }

            if let Some(most_recent) = most_recent.as_mut() {
                if candidate_item.freshness() > most_recent.freshness() {
                    *most_recent = candidate_item;
                }
            } else {
                most_recent = Some(candidate_item);
            }
        }

        if let Some(mut most_recent) = most_recent {
            most_recent.make_fresh();
            Ok(Some(most_recent))
        } else {
            Ok(None)
        }
    }
}

/// Set of all hub connection from this node.
pub struct Hubs {
    hubs: Vec<Arc<HubConnection>>,
}

impl Hubs {
    /// Initiates the set of all hub connections.
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

    /// Makes a query to all inscribed hubs.
    pub async fn query(&self, content_hash: Hash, kind: QueryKind) -> Option<ObjectRef> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.query(content_hash, kind).await) })
            .buffer_unordered(cli().max_parallel_hubs);

        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(Some(found)) => return Some(found),
                Ok(None) => {
                    log::info!("got no result from {}", hub_name)
                }
                Err(err) => {
                    log::error!("Error while querying {}: {}", hub_name, err)
                }
            }
        }

        None
    }

    /// Tries to resolve the latest item of a given series.
    pub async fn get_latest(&self, series: &SeriesRef) -> Option<SeriesItem> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.get_latest(series).await) })
            .buffer_unordered(cli().max_parallel_hubs);

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
