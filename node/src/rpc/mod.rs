use lazy_static::lazy_static;
use rocksdb::IteratorMode;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::net::SocketAddr;
use std::sync::Arc;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, RwLock};

use samizdat_common::rpc::{HubClient, Node, Query, Resolution, Status};
use samizdat_common::{transport, Hash};

use crate::{db, Table};

#[derive(Clone)]
struct NodeServer;

#[tarpc::server]
impl Node for NodeServer {
    async fn resolve(self, _: context::Context, resolution: Arc<Resolution>) -> Status {
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
                        return Status::Found;
                    }
                };

                log::info!("found peer at {}", peer_addr);

                return Status::Found;
            }
        }

        log::info!("hash not found for resolution");

        Status::NotFound
    }
}

lazy_static! {
    static ref POST_BACK: RwLock<BTreeMap<Hash, oneshot::Sender<Vec<u8>>>> = RwLock::default();
}

pub struct HubConnection {
    client: HubClient,
}

impl HubConnection {
    pub async fn connect(addr: impl Into<SocketAddr>) -> Result<HubConnection, crate::Error> {
        let addr = addr.into();
        let multiplex = transport::Multiplex::new(TcpStream::connect(addr).await?);
        let direct = multiplex
            .channel(0)
            .await
            .expect("channel 0 in use unexpectedly");
        let reverse = multiplex
            .channel(1)
            .await
            .expect("channel 0 in use unexpectedly");

        let client = HubClient::new(tarpc::client::Config::default(), direct)
            .spawn()
            .map_err(|err| format!("failed to spawn client for {}: {}", addr, err))?;

        let server_task = server::BaseChannel::with_defaults(reverse).execute(NodeServer.serve());
        tokio::spawn(server_task);

        Ok(HubConnection { client })
    }

    pub async fn query(&self, content_hash: Hash) -> Result<Vec<u8>, crate::Error> {
        let content_riddle = content_hash.gen_riddle();
        let location_riddle = content_hash.gen_riddle();

        let (sender, receiver) = oneshot::channel();
        POST_BACK.write().await.insert(content_hash, sender);

        self.client
            .query(
                context::current(),
                Query {
                    content_riddle,
                    location_riddle,
                },
            )
            .await?;

        // TODO: timeout.
        Ok(receiver.await.unwrap())
    }
}
