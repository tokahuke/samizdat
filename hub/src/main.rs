#![feature(ip)]

mod cli;
mod db;
mod http;
mod models;
mod replay_resistance;
mod rpc;
mod utils;

use cli::cli;
pub use samizdat_common::Error;

use std::panic;
use tokio::task;

use samizdat_common::keyed_channel::KeyedChannel;

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
    cli::init_cli()?;
    samizdat_common::logger::init();
    db::init_db::<crate::db::Table>(&cli().data)?;

    tracing::info!("Starting SAMIZDAT hub in folder {:?}", cli().data);

    // Resolve hubs:
    let mut hubs = vec![];

    for addr in &cli().addresses {
        hubs.extend(
            cli()
                .resolution_mode
                .resolve(addr)
                .await?
                .into_iter()
                .map(|tuple| tuple.1),
        );
    }

    // Spawn services:
    let candidate_channels = KeyedChannel::new();
    let rpc_server = tokio::spawn(crate::rpc::run(hubs, candidate_channels.clone()));
    let partners = tokio::spawn(crate::rpc::run_partners());
    let http_server = tokio::spawn(http::serve());

    // Await for services to end:
    maybe_resume_panic(rpc_server.await);
    maybe_resume_panic(http_server.await);
    maybe_resume_panic(partners.await);

    // Exit:
    Ok(())
}
