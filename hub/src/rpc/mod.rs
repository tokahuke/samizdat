use futures::prelude::*;
use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use tarpc::server::{self, Channel, Incoming};
use tarpc::{context, serde_transport};

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
    let mut listen =
        serde_transport::tcp::listen(addr.into(), tokio_serde::formats::Bincode::default)
            .await
            .unwrap();

    listen.config_mut().max_frame_length(usize::MAX);

    listen
        // Ignore accept errors.
        .filter_map(|r| async move { r.ok() })
        .map(server::BaseChannel::with_defaults)
        .map(|channel| {
            let server = HubServer;
            println!("here");
            channel.execute(server.serve())
        })
        // Max 1_000 channels.
        .buffer_unordered(1_000)
        .for_each(|_| async {})
        .await;
}
