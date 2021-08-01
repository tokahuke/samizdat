use futures::prelude::*;
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
use samizdat_common::transport::{BincodeTransport, QuicTransport};
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

use quinn::generic::{OpenBi, RecvStream, SendStream, ServerConfig};
use quinn::{crypto::Session, Connection, Endpoint, Incoming};

pub struct HubConnection {
    client: HubClient,
}

impl HubConnection {
    pub async fn connect(
        local_addr: impl Into<SocketAddr>,
        remote_addr: impl Into<SocketAddr>,
    ) -> Result<HubConnection, crate::Error> {
        let local_addr = local_addr.into();
        let remote_addr = remote_addr.into();

        let mut endpoint_builder = Endpoint::builder();
        endpoint_builder.listen(samizdat_common::quic::server_config());
        endpoint_builder.default_client_config(samizdat_common::quic::insecure());

        let (endpoint, _) = endpoint_builder.bind(&local_addr).expect("failed to bind");

        let new_connection = endpoint
            .connect_with(samizdat_common::quic::insecure(), &remote_addr, "localhost")
            .expect("failed to start connecting")
            .await
            .expect("failed to connect");
        let mut connection = new_connection.connection;
        let mut foo = connection
        .open_bi()
        .await
        .expect("failed to open bidirectional channel");

        foo.0.write(b"").await.unwrap();

        let mut bar = connection
        .open_bi()
        .await
        .expect("failed to open bidirectional channel");

        bar.0.write(b"").await.unwrap();

        let direct = BincodeTransport::new(QuicTransport::new(
            foo,
        ));
        let reverse = BincodeTransport::new(QuicTransport::new(
            bar
        ));

        let client = HubClient::new(tarpc::client::Config::default(), direct)
            .spawn()
            .map_err(|err| format!("failed to spawn client for {}: {}", remote_addr, err))?;

        let server_task = server::BaseChannel::with_defaults(reverse).execute(NodeServer.serve());
        tokio::spawn(server_task);

        log::info!("client connected to server at {}", connection.remote_address());

        Ok(HubConnection { client })
    }

    pub async fn query(&self, content_hash: Hash) -> Result<Vec<u8>, crate::Error> {
        log::info!("query for hash {}", content_hash);

        let content_riddle = content_hash.gen_riddle();
        let location_riddle = content_hash.gen_riddle();

        log::info!("inscrbing postback channel");
        let (sender, receiver) = oneshot::channel();
        POST_BACK.write().await.insert(content_hash, sender);

        log::info!("running rpc");
        self.client
            .query(
                context::current(),
                Query {
                    content_riddle,
                    location_riddle,
                },
            )
            .await?;
        log::info!("rpc done");

        // TODO: timeout.
        Ok(receiver.await.unwrap())
    }
}
