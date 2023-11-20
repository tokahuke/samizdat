//! Implementation of the node behavior in the Samizdat network, both with hubs and with
//! other nodes.

mod node_server;
mod reconnect;
mod transport;

pub use file_transfer::{ReceivedItem, ReceivedObject};
pub use reconnect::{ConnectionStatus, Reconnect};
use samizdat_common::Hint;
use tokio::time::Instant;
pub use transport::PEER_CONNECTIONS;

use futures::prelude::*;
use futures::stream;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;
use tarpc::client::NewClient;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use samizdat_common::address::{ChannelAddr, HubAddr};
use samizdat_common::cipher::TransferCipher;
use samizdat_common::keyed_channel::KeyedChannel;
use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::{Hash, Riddle};

use crate::cli;
use crate::models;
use crate::models::{Edition, SeriesRef};

use self::node_server::NodeServer;
use self::transport::{file_transfer, ChannelManager, ConnectionManager};

/// A single connection instance, which will be recreated by [`Reconnect`] on connection loss.
pub struct HubConnectionInner {
    client: HubClient,
    // connection_manager: Arc<ConnectionManager>,
    channel_manager: Arc<ChannelManager>,
    candidate_channels: KeyedChannel<Candidate>,
}

impl HubConnectionInner {
    /// Creates the RPC connection from the Node to the Hub.
    async fn connect_direct(
        direct_addr: SocketAddr,
        connection_manager: Arc<ConnectionManager>,
    ) -> Result<(HubClient, oneshot::Receiver<()>), crate::Error> {
        let (client_reset_trigger, client_reset_recv) = oneshot::channel();

        // Create transport for client and create client:
        let transport = connection_manager.transport(direct_addr).await?;
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
        candidate_channels: KeyedChannel<Candidate>,
    ) -> Result<JoinHandle<()>, crate::Error> {
        // Create transport for server and spawn server:
        let transport = connection_manager.transport(reverse_addr).await?;
        let server_task = server::BaseChannel::with_defaults(transport).execute(
            NodeServer {
                channel_manager: Arc::new(ChannelManager::new(connection_manager.clone())),
                candidate_channels,
            }
            .serve(),
        );
        let handler = tokio::spawn(server_task);

        Ok(handler)
    }

    /// Creates the two connections between hub and node: RPC from node to hub and RPC from
    /// hub to node.
    async fn connect(
        hub_addr: HubAddr,
    ) -> Result<(HubConnectionInner, impl Future<Output = ()>), crate::Error> {
        // Connect and create connection manager:
        let endpoint = quic::new_default("[::]:0".parse().expect("valid address"));

        if let Ok(local_addr) = endpoint.local_addr() {
            log::info!("Hub connection bound to {local_addr}");
        }

        let connection_manager = Arc::new(ConnectionManager::new(endpoint));
        let channel_manager = Arc::new(ChannelManager::new(connection_manager.clone()));
        let candidate_channels = KeyedChannel::new();
        let (client, client_reset_recv) =
            Self::connect_direct(hub_addr.direct_addr(), connection_manager.clone()).await?;
        let server_reset_recv = Self::connect_reverse(
            hub_addr.reverse_addr(),
            connection_manager.clone(),
            candidate_channels.clone(),
        )
        .await?;

        let reset_trigger = future::select(server_reset_recv, client_reset_recv).map(|_| ());

        Ok((
            HubConnectionInner {
                client,
                // connection_manager,
                channel_manager,
                candidate_channels,
            },
            reset_trigger,
        ))
    }
}

/// A connection to a single node, already resilient to reconnects.
pub struct HubConnection {
    name: String,
    hub_addr: HubAddr,
    inner: Reconnect<HubConnectionInner>,
}

impl HubConnection {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn status(&self) -> ConnectionStatus {
        self.inner.status()
    }

    pub fn address(&self) -> HubAddr {
        self.hub_addr
    }

    /// Creates a connection to the hub.
    pub async fn connect(name: String, hub_addr: HubAddr) -> Result<HubConnection, crate::Error> {
        Ok(HubConnection {
            name,
            hub_addr,
            inner: Reconnect::init(
                move || HubConnectionInner::connect(hub_addr),
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

    /// Makes a query to this hub. Returns `Ok(None)` if the object is
    pub async fn query(
        &self,
        content_hash: Hash,
        kind: QueryKind,
        deadline: SystemTime,
    ) -> Result<ReceivedItem, crate::Error> {
        // Create riddles for query:
        let content_riddles = (0..cli().riddles_per_query)
            .map(|_| Riddle::new(&content_hash))
            .collect();
        let hint = Hint::new(content_hash, cli().hint_size as usize);
        let location_riddle = Riddle::new(&content_hash);

        // Acquire hub connection:
        let guard = self.inner.get().await;
        let inner = guard.as_ref().ok_or("Not yet connected")?;

        // Get the deadline of the request:
        let query_start = SystemTime::now();
        let mut context = context::current();
        context.deadline = deadline;
        let request_duration = deadline
            .duration_since(query_start)
            .expect("deadline is in the future");
        let deadline_instant = Instant::now() + request_duration;

        // Do the RPC call:
        let query_response = inner
            .client
            .query(
                context::current(),
                Query {
                    content_riddles,
                    hint,
                    location_riddle,
                    kind,
                },
            )
            .await?;

        // Interpret RPC response:
        let (candidate_channel, channel_id) = match query_response {
            QueryResponse::Replayed => return Err("hub has suspected replay attack".into()),
            QueryResponse::EmptyQuery => return Err("hub has received an empty query".into()),
            QueryResponse::NoReverseConnection => {
                return Err("hub said I have no reverse connection".into())
            }
            QueryResponse::InternalError => {
                return Err("hub has experienced an internal error".into())
            }
            QueryResponse::Resolved {
                candidate_channel,
                channel_id,
            } => (candidate_channel, channel_id),
        };

        log::info!(
            "Candidate channel for {}: {}",
            content_hash,
            candidate_channel
        );

        // Stream of peer candidates:
        let channel_manager = inner.channel_manager.clone();
        let candidates = inner
            .candidate_channels
            .recv_stream(candidate_channel)
            .map(move |candidate| {
                // TODO: check if candidate is valid. However, seems to be unnecessary, since
                // transport will make sure no naughty people are involved.
                let channel_addr = ChannelAddr::new(candidate.socket_addr, channel_id);
                log::info!("Got candidate {channel_addr} for channel {candidate_channel}");
                let channel_manager = channel_manager.clone();
                Box::pin(async move {
                    channel_manager
                        .expect(channel_addr)
                        .await
                        .map_err(|err| {
                            log::warn!("Hole punching with {channel_addr} failed: {err}")
                        })
                        .ok()
                })
            })
            .buffer_unordered(cli().concurrent_candidates)
            .filter_map(|done| Box::pin(async move { done }));

        let outcome = match kind {
            QueryKind::Object => {
                file_transfer::recv_object(candidates, content_hash, query_start, deadline_instant)
                    .await
                    .map(ReceivedItem::NewObject)
            }
            QueryKind::Item => {
                file_transfer::recv_item(candidates, content_hash, query_start, deadline_instant)
                    .await
            }
        };

        log::info!(
            "Query done: {kind:?} {content_hash} {:?}",
            outcome.as_ref().map(|tee| tee.object_ref())
        );

        outcome
    }

    /// Tries to resolve the latest edition of a given series.
    pub async fn get_edition(&self, series: &SeriesRef) -> Result<Option<Edition>, crate::Error> {
        let key_riddle = Riddle::new(&series.public_key.hash());
        let guard = self.inner.get().await;
        let inner = guard.as_ref().ok_or("Not yet connected")?;

        let response = inner
            .client
            .get_edition(context::current(), EditionRequest { key_riddle })
            .await?;

        let mut most_recent: Option<Edition> = None;

        for candidate in response {
            let cipher = TransferCipher::new(&series.public_key.hash(), &candidate.rand);
            let candidate_edition: Edition = candidate.series.decrypt_with(&cipher)?;

            if !candidate_edition.is_valid() {
                log::warn!("received invalid candidate edition: {candidate_edition:?}",);
                continue;
            }

            if let Some(most_recent) = most_recent.as_mut() {
                if candidate_edition.timestamp() > most_recent.timestamp() {
                    *most_recent = candidate_edition;
                }
            } else {
                most_recent = Some(candidate_edition);
            }
        }

        Ok(most_recent)
    }

    pub async fn announce_edition(
        &self,
        announcement: &EditionAnnouncement,
    ) -> Result<(), crate::Error> {
        let guard = self.inner.get().await;
        let inner = guard.as_ref().ok_or("Not yet connected")?;

        inner
            .client
            .announce_edition(context::current(), announcement.clone())
            .await?;

        Ok(())
    }
}

/// Set of all hub connection from this node.
pub struct Hubs {
    hubs: RwLock<Vec<Arc<HubConnection>>>,
}

impl Hubs {
    pub async fn remove(&self, name: &str) {
        let mut hubs = self.hubs.write().await;
        *hubs = hubs
            .iter()
            .filter(|&conn| conn.name != name)
            .cloned()
            .collect();
    }

    pub async fn insert(&self, hub_model: models::Hub) {
        let mut hubs = self.hubs.write().await;
        let mut resolved_addresses = vec![];

        let outcome: Result<(), crate::Error> = try {
            for (name, address) in hub_model.address.resolve(hub_model.resolution_mode).await? {
                let is_already_inserted = hubs.iter().any(|conn| {
                    conn.address().direct_addr() == address.direct_addr()
                        && conn.address().reverse_addr() == address.reverse_addr()
                });

                if !is_already_inserted {
                    resolved_addresses.push((name, address));
                }
            }
        };

        if let Err(err) = outcome {
            log::warn!("Failed to resolve address for {}: {err}", hub_model.address);
        }

        let hub_stream = stream::iter(resolved_addresses)
            .map(|(name, resolved)| async move {
                match HubConnection::connect(name.clone(), resolved).await {
                    Ok(conn) => Some(conn),
                    Err(err) => {
                        log::warn!("Failed to connect to hub {name} at {resolved}: {err}");
                        None
                    }
                }
            })
            .buffer_unordered(10)
            .filter_map(|maybe_conn| async move { maybe_conn })
            .map(Arc::new);

        log::debug!("Inserting connection(s) for {}", hub_model.address);

        *hubs = stream::iter(hubs.iter().cloned())
            .chain(hub_stream)
            .collect()
            .await;

        log::info!("Connection(s) for {} created", hub_model.address);
    }

    pub async fn snapshot(&self) -> Vec<Arc<HubConnection>> {
        let hubs = self.hubs.read().await;
        hubs.iter().cloned().collect()
    }

    /// Initiates the set of all hub connections.
    pub async fn init() -> Result<Hubs, crate::Error> {
        let all_hub_models = models::Hub::get_all()?;
        let mut resolved_addresses = vec![];

        for hub_model in all_hub_models {
            let outcome: Result<(), crate::Error> = try {
                for (name, address) in hub_model.address.resolve(hub_model.resolution_mode).await? {
                    // TODO: disallow creating more than one connection to the same HubAddr.
                    resolved_addresses.push((name, address));
                }
            };

            if let Err(err) = outcome {
                log::warn!("Failed to resolve address for {}: {err}", hub_model.address);
            }
        }

        let hub_stream = stream::iter(resolved_addresses)
            .map(|(name, resolved)| async move {
                match HubConnection::connect(name.clone(), resolved).await {
                    Ok(conn) => Some(conn),
                    Err(err) => {
                        log::warn!("Failed to connect to hub {name} at {resolved}: {err}");
                        None
                    }
                }
            })
            .buffer_unordered(10)
            .filter_map(|maybe_conn| async move { maybe_conn })
            .map(Arc::new);

        Ok(Hubs {
            hubs: RwLock::new(hub_stream.collect().await),
        })
    }

    /// Makes a query to all inscribed hubs.
    pub async fn query(
        &self,
        content_hash: Hash,
        kind: QueryKind,
        deadline: SystemTime,
    ) -> Option<ReceivedItem> {
        let hubs = self.hubs.read().await;
        let mut results = stream::iter(hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Querying {} for {kind:?} {content_hash}", hub.name);
                (
                    hub.name.clone(),
                    hub.query(content_hash, kind, deadline).await,
                )
            })
            .buffer_unordered(cli().max_parallel_hubs);

        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(found) => return Some(found),
                Err(err) => {
                    log::error!("Error while querying {}: {}", hub_name, err)
                }
            }
        }

        None
    }

    pub async fn query_with_retry<I>(
        &self,
        content_hash: Hash,
        kind: QueryKind,
        deadline: SystemTime,
        retries: I,
    ) -> Option<ReceivedItem>
    where
        I: IntoIterator<Item = Duration>,
    {
        if let Some(item) = self.query(content_hash, kind, deadline).await {
            return Some(item);
        }

        for duration in retries {
            tokio::time::sleep(duration).await;
            if let Some(item) = self.query(content_hash, kind, deadline).await {
                return Some(item);
            }
        }

        None
    }

    /// Tries to resolve the latest edition of a given series.
    pub async fn get_latest(&self, series: &SeriesRef) -> Option<Edition> {
        let hubs = self.hubs.read().await;
        let mut results = stream::iter(hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Querying {} for latest edition of {series}", hub.name);
                (hub.name.clone(), hub.get_edition(series).await)
            })
            .buffer_unordered(cli().max_parallel_hubs);

        // Even though we should have to go through *aaaaaaall* the hubs to get the best answer, we
        // can wait for changes to propagate eventually.
        // In other words, this might be inaccurate, but it is faster.
        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(Some(found)) => return Some(found),
                Ok(None) => {
                    log::debug!("got no result from {}", hub_name)
                }
                Err(err) => {
                    log::error!("Error while querying {hub_name}: {err}")
                }
            }
        }

        None
    }

    pub async fn announce_edition(&self, announcement: &EditionAnnouncement) {
        let hubs = self.hubs.read().await;
        let mut results = stream::iter(hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Announcing {announcement:?} to {}", hub.name);
                (hub.name.clone(), hub.announce_edition(announcement).await)
            })
            .buffer_unordered(cli().max_parallel_hubs);

        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(_) => {}
                Err(err) => {
                    log::error!("Error while querying {hub_name}: {err}")
                }
            }
        }
    }
}
