use futures::prelude::*;
use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tarpc::server::{self, Channel, Incoming};
use tarpc::{context};

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
            println!("{:?}", t.peer_addr());
            server::BaseChannel::with_defaults(
                transport::Multiplex::new(t).channel(0).await.unwrap()
            )
        })
        // .map(server::BaseChannel::with_defaults)
        .map(|channel| {
            println!("starting to serve");
            let server = HubServer;
            channel.execute(server.serve())
        })
        // // Max 1_000 channels.
        .buffer_unordered(1_000)
        .for_each(|_| async {})
        .await;
}
