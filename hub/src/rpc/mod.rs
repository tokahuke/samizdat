use futures::prelude::*;
use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use tarpc::context;
use tarpc::server::{self, Channel};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;

use samizdat_common::transport;

#[derive(Debug, Serialize, Deserialize)]
struct Riddle {
    rand: [u8; 28],
    hash: [u8; 28],
}

#[tarpc::service]
trait Hub {
    /// Returns a greeting for name.
    async fn query(riddle: Riddle);
}

#[tarpc::service]
trait Node {
    async fn resolve(riddle: Riddle);
}

#[derive(Clone)]
struct HubServer;

#[tarpc::server]
impl Hub for HubServer {
    async fn query(self, _: context::Context, riddle: Riddle) {
        println!("got riddle {:?}", riddle);
    }
}

pub async fn run(addr: impl Into<SocketAddr>) {
    let listener = TcpListener::bind(addr.into()).await.unwrap();

    TcpListenerStream::new(listener)
        // Ignore accept errors.
        .filter_map(|r| async move { r.ok() })
        .then(|t| async move {
            log::info!("Incoming connection from{:?}", t.peer_addr());
            let multiplex = transport::Multiplex::new(t);
            let direct = multiplex.channel(0).await.unwrap();
            let reverse = multiplex.reverse_channel(0).await.unwrap();

            let client = NodeClient::new(tarpc::client::Config::default(), reverse)
                .spawn()
                .unwrap();

            let server_task = server::BaseChannel::with_defaults(direct).execute(HubServer.serve());

            server_task
        })
        // Max 1_000 channels.
        .buffer_unordered(1_000)
        .for_each(|_| async {})
        .await;
}
