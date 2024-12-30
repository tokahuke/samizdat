#![feature(ip)]

mod cli;
mod db;
mod http;
mod models;
mod replay_resistance;
mod rpc;
mod utils;

pub use samizdat_common::Error;

use std::panic;
use structopt::StructOpt;
use tokio::task;

use samizdat_common::keyed_channel::KeyedChannel;

lazy_static::lazy_static! {
    /// The command line arguments.
    pub static ref CLI: cli::Cli = cli::Cli::from_args();
}

/// Utility for propagating panics through tasks.
fn maybe_resume_panic<T>(r: Result<T, task::JoinError>) {
    if let Err(err) = r {
        if let Ok(panic) = err.try_into_panic() {
            panic::resume_unwind(panic);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    // Init resources:
    let _ = &*CLI;

    // Init logger:
    tracing_subscriber::fmt().init();

    db::init_db()?;

    // Resolve hubs:
    let mut hubs = vec![];

    for addr in &CLI.addresses {
        hubs.extend(
            CLI.resolution_mode
                .resolve(addr)
                .await?
                .into_iter()
                .map(|tuple| tuple.1),
        );
    }

    // Spawn services:
    let candidate_channels = KeyedChannel::new();
    let direct_rpc_server = tokio::spawn(crate::rpc::run_direct(hubs, candidate_channels.clone()));
    // let reverse_rpc_server = tokio::spawn(crate::rpc::run_reverse(
    //     CLI.addresses
    //         .iter()
    //         .map(|addr| addr.reverse_addr())
    //         .collect(),
    // ));
    let partners = tokio::spawn(crate::rpc::run_partners());
    let http_server = tokio::spawn(http::serve());

    // Await for services to end:
    maybe_resume_panic(direct_rpc_server.await);
    // maybe_resume_panic(reverse_rpc_server.await);
    maybe_resume_panic(http_server.await);
    maybe_resume_panic(partners.await);

    // Exit:
    Ok(())
}
