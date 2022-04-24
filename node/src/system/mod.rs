//! Implementation of the node behavior in the Samizdat network, both with hubs and with
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
use tokio::task::JoinHandle;
use tokio::time::Duration;

use samizdat_common::cipher::TransferCipher;
use samizdat_common::quic;
use samizdat_common::rpc::*;
use samizdat_common::{Hash, Riddle};

use crate::cli;
use crate::models::Identity;
use crate::models::IdentityRef;
use crate::models::{Edition, ObjectRef, SeriesRef};

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
    ) -> Result<JoinHandle<()>, crate::Error> {
        // Create transport for server and spawn server:
        let transport = connection_manager.transport(reverse_addr).await?;
        let server_task = server::BaseChannel::with_defaults(transport).execute(
            NodeServer {
                channel_manager: Arc::new(ChannelManager::new(connection_manager.clone())),
            }
            .serve(),
        );
        let handler = tokio::spawn(server_task);

        Ok(handler)
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
        let content_riddles = (0..cli().riddles_per_query)
            .map(|_| Riddle::new(&content_hash))
            .collect();
        let location_riddle = Riddle::new(&content_hash);

        let inner = self.inner.get().await;

        let query_response = inner
            .client
            .query(
                context::current(),
                Query {
                    content_riddles,
                    location_riddle,
                    kind,
                },
            )
            .await?;

        let candidates = match query_response {
            QueryResponse::Replayed => return Err("hub has suspected replay attack".into()),
            QueryResponse::EmptyQuery => return Err("hub has received an empty query".into()),
            QueryResponse::InternalError => {
                return Err("hub has experienced an internal error".into())
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

    /// Tries to resolve the latest edition of a given series.
    pub async fn get_latest(&self, series: &SeriesRef) -> Option<Edition> {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.get_edition(series).await) })
            .buffer_unordered(cli().max_parallel_hubs);

        // Even though we should have to go through *aaaaaaall* the hubs to get the best answer, we
        // can wait for changes to propagate eventually.
        // In other words, this might be inaccurate, but it is faster.
        while let Some((hub_name, result)) = results.next().await {
            match result {
                Ok(Some(found)) => return Some(found),
                Ok(None) => {}
                Err(err) => {
                    log::error!("Error while querying {hub_name}: {err}")
                }
            }
        }

        None
    }

    pub async fn announce_edition(&self, announcement: &EditionAnnouncement) {
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.announce_edition(announcement).await) })
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
        let mut results = stream::iter(self.hubs.iter().cloned())
            .map(|hub| async move { (hub.name, hub.get_identity(identity).await) })
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
