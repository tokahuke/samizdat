mod connection_manager;
mod file_transfer;

use lazy_static::lazy_static;
use rocksdb::IteratorMode;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::sync::{oneshot, RwLock};

use samizdat_common::quic;
use samizdat_common::rpc::{HubClient, Node, Query, QueryResponse, Resolution, ResolutionResponse};
use samizdat_common::Hash;

use crate::{db, Table};

use connection_manager::{ConnectionManager, DropMode};

#[derive(Clone)]
struct NodeServer {
    connection_manager: Arc<ConnectionManager>,
}

#[tarpc::server]
impl Node for NodeServer {
    async fn resolve(self, _: context::Context, resolution: Arc<Resolution>) -> ResolutionResponse {
        log::info!("got {:?}", resolution);
        let iter = db().iterator_cf(Table::Hashes.get(), IteratorMode::Start);

        for (key, _) in iter {
            let hash: Hash = match key.as_ref().try_into() {
                Ok(hash) => hash,
                Err(err) => {
                    log::warn!("{}", err);
                    continue;
                }
            };

            if resolution.content_riddle.resolves(&hash) {
                log::info!("found {:?}", hash);
                let peer_addr = match resolution.location_riddle.resolve(&hash) {
                    Some(peer_addr) => peer_addr,
                    None => {
                        log::warn!(
                            "failed to resolve location riddle after resolving content riddle"
                        );
                        return ResolutionResponse::FOUND;
                    }
                };

                log::info!("found peer at {}", peer_addr);

                tokio::spawn(async move {
                    let new_connection = self
                        .connection_manager
                        .punch_hole_to(peer_addr, DropMode::DropIncoming)
                        .await
                        .expect("failed to punch hole");
                    let content = db()
                        .get_cf(Table::Content.get(), hash)
                        .expect("db error")
                        .expect("content exists");
                    file_transfer::send(&new_connection.connection, hash, &content)
                        .await
                        .expect("failed to send file")
                });

                return ResolutionResponse::FOUND;
            }
        }

        log::info!("hash not found for resolution");

        ResolutionResponse::NOT_FOUND
    }
}

lazy_static! {
    static ref POST_BACK: RwLock<BTreeMap<Hash, oneshot::Sender<Vec<u8>>>> = RwLock::default();
}

pub struct HubConnection {
    client: HubClient,
    connection_manager: Arc<ConnectionManager>,
}

impl HubConnection {
    pub async fn connect(
        direct_addr: impl Into<SocketAddr>,
        reverse_addr: impl Into<SocketAddr>,
    ) -> Result<HubConnection, crate::Error> {
        // Define relevant addresses:
        let direct_addr = direct_addr.into();
        let reverse_addr = reverse_addr.into();

        // Connect and create connection manager:
        let (endpoint, incoming) = quic::new_default(([0, 0, 0, 0], 0).into());
        let connection_manager = Arc::new(ConnectionManager::new(endpoint, incoming));

        // Create transport for client and create client:
        let transport = connection_manager
            .transport(&direct_addr, "localhost")
            .await?;
        let client = HubClient::new(tarpc::client::Config::default(), transport).spawn();

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
        tokio::spawn(server_task);

        Ok(HubConnection {
            client,
            connection_manager,
        })
    }

    pub async fn query(&self, content_hash: Hash) -> Result<Option<Vec<u8>>, crate::Error> {
        let content_riddle = content_hash.gen_riddle();
        let location_riddle = content_hash.gen_riddle();

        let QueryResponse { candidates } = self
            .client
            .query(
                context::current(),
                Query {
                    content_riddle,
                    location_riddle,
                },
            )
            .await?;

        log::info!("Candidates for {}: {:?}", content_hash, candidates);

        if candidates.is_empty() {
            // Forget it!
            POST_BACK.write().await.remove(&content_hash);
            return Ok(None);
        }

        // Only one candidate: experiment....
        let candidate = candidates[0];
        let mut new_connection = self
            .connection_manager
            .punch_hole_to(candidate, DropMode::DropOutgoing)
            .await?;

        // TODO: timeout.
        Ok(Some(
            file_transfer::recv(&mut new_connection.uni_streams, content_hash).await?,
        ))
    }
}
