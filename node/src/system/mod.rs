//! Implementation of the node behavior in the Samizdat network, both with hubs and with
//! other nodes.

mod node_server;
mod reconnect;
mod transport;

pub use file_transfer::ReceivedObject;
pub use reconnect::Reconnect;

use futures::prelude::*;
use futures::stream;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;
use tarpc::client::NewClient;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio::time::{timeout_at, Duration};

use samizdat_common::address::{ChannelAddr, HubAddr};
use samizdat_common::cipher::TransferCipher;
use samizdat_common::keyed_channel::KeyedChannel;
use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::{Hash, Riddle};

use crate::cli;
use crate::models::Identity;
use crate::models::IdentityRef;
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
    name: &'static str,
    inner: Reconnect<HubConnectionInner>,
}

impl HubConnection {
    /// Creates a connection to the hub.
    pub async fn connect(
        name: &'static str,
        hub_addr: HubAddr,
    ) -> Result<HubConnection, crate::Error> {
        Ok(HubConnection {
            name,
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

    /// Makes a query to this hub.
    pub async fn query(
        &self,
        content_hash: Hash,
        kind: QueryKind,
    ) -> Result<ReceivedObject, crate::Error> {
        // Create riddles for query:
        let content_riddles = (0..cli().riddles_per_query)
            .map(|_| Riddle::new(&content_hash))
            .collect();
        let location_riddle = Riddle::new(&content_hash);

        // Acquire hub connection:
        let inner = self.inner.get().await;

        // Get the deadline of the request:
        let context = context::current();
        let request_duration = context
            .deadline
            .duration_since(SystemTime::now())
            .expect("deadline is in the future");
        let deadline = Instant::now() + request_duration;

        // Do the RPC call:
        let query_response = inner
            .client
            .query(
                context,
                Query {
                    content_riddles,
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
            "Candidate channel for {}: {:x}",
            content_hash,
            candidate_channel
        );

        // Stream of peer candidates:
        let mut candidates = inner
            .candidate_channels
            .recv_stream(candidate_channel)
            .map(|candidate| {
                // TODO: check if candidate is valid. However, seems to be unnecessary, since
                // transport will make sure no naughty people are involved.
                let channel_addr = ChannelAddr::new(candidate.socket_addr, channel_id);
                log::info!("Got candidate {channel_addr} for channel {candidate_channel:x}");
                let channel_manager = inner.channel_manager.clone();
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
            .buffer_unordered(4) // TODO: create CLI knob
            .filter_map(|done| Box::pin(async move { done }));

        // For each candidate, "do the thing":
        let outcome = loop {
            match timeout_at(deadline, candidates.next()).await {
                Ok(Some((_sender, receiver))) => {
                    // TODO: minor improvement... could we tee the object stream directly to the
                    // user? By now, we are waiting for the whole object to arrive, which is fine
                    // for most files, but can be a pain for the bigger ones...
                    let receive_outcome = match kind {
                        QueryKind::Object => {
                            file_transfer::recv_object(receiver, content_hash).await
                        }
                        QueryKind::Item => file_transfer::recv_item(receiver, content_hash).await,
                    };

                    match receive_outcome {
                        Ok(outcome) => break Ok(outcome),
                        Err(err) => {
                            log::warn!(
                                "Candidate for query {kind:?} {content_hash} failed with: {err}"
                            );
                        }
                    }
                }
                Ok(None) => {
                    log::info!("Candidate channel {candidate_channel:x} dried");
                    break Err(crate::Error::AllCandidatesFailed);
                }
                Err(_) => {
                    break Err(crate::Error::Timeout);
                }
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
        let inner = self.inner.get().await;

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
        let inner = self.inner.get().await;

        inner
            .client
            .announce_edition(context::current(), announcement.clone())
            .await?;

        Ok(())
    }

    pub async fn get_identity(
        &self,
        identity: &IdentityRef,
    ) -> Result<Option<Identity>, crate::Error> {
        let identity_riddle = Riddle::new(&identity.hash());
        let inner = self.inner.get().await;

        let candidates = inner
            .client
            .get_identity(context::current(), IdentityRequest { identity_riddle })
            .await?;

        let mut most_worked_on: Option<Identity> = None;

        for candidate in candidates {
            let cipher = TransferCipher::new(&identity.hash(), &candidate.rand);
            let candidate_identity: Identity = candidate.identity.decrypt_with(&cipher)?;

            if !candidate_identity.is_valid() || candidate_identity.identity() != identity {
                log::warn!("received invalid candidate identity: {candidate_identity:?}",);
                continue;
            }

            if let Some(most_worked_on) = most_worked_on.as_mut() {
                if candidate_identity.work_done() > most_worked_on.work_done() {
                    *most_worked_on = candidate_identity;
                }
            } else {
                most_worked_on = Some(candidate_identity);
            }
        }

        Ok(most_worked_on)
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
        I: IntoIterator<Item = (&'static str, HubAddr)>,
    {
        let hubs = stream::iter(addrs)
            .map(|(name, addr)| HubConnection::connect(name, addr))
            .buffer_unordered(10) // 'cause 10!
            .map(|outcome| outcome.map(Arc::new))
            .try_collect::<Vec<_>>()
            .await?;

        Ok(Hubs { hubs })
    }

    /// Makes a query to all inscribed hubs.
    pub async fn query(&self, content_hash: Hash, kind: QueryKind) -> Option<ReceivedObject> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Querying {} for {kind:?} {content_hash}", hub.name);
                (hub.name, hub.query(content_hash, kind).await)
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

    /// Tries to resolve the latest edition of a given series.
    pub async fn get_latest(&self, series: &SeriesRef) -> Option<Edition> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Querying {} for latest edition of {series}", hub.name);
                (hub.name, hub.get_edition(series).await)
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
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Announcing {announcement:?} to {}", hub.name);
                (hub.name, hub.announce_edition(announcement).await)
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

    pub async fn get_identity(&self, identity: &IdentityRef) -> Option<Identity> {
        log::info!("HERE!");

        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move {
                log::debug!("Querying {} for identity {identity}", hub.name);
                (hub.name, hub.get_identity(identity).await)
            })
            .buffer_unordered(cli().max_parallel_hubs);

        let mut most_worked_on: Option<Identity> = None;

        // Here, we need to go through *aaaaaall* the hubs to find the best match.
        // In other words, this *must* be correct. Let's not cut any corners here.
        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(Some(found)) => {
                    if let Some(most_worked_on) = most_worked_on.as_mut() {
                        if found.work_done() > most_worked_on.work_done() {
                            *most_worked_on = found;
                        }
                    } else {
                        most_worked_on = Some(found);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    log::error!("Error while querying {hub_name}: {err}")
                }
            }
        }

        most_worked_on
    }
}
