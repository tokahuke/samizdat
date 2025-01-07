#![feature(ip, try_blocks, result_flattening)]

mod access;
mod cli;
mod db;
mod http;
mod identity_dapp;
mod models;
mod system;
mod utils;
mod vacuum;

pub use samizdat_common::Error;

pub use cli::cli;

use std::sync::OnceLock;

use access::init_access_token;
use cli::init_cli;
use db::init_db;
use identity_dapp::init_identity_provider;
use system::Hubs;

/// The variable holding a list of all the connections to the hubs.
static HUBS: OnceLock<Hubs> = OnceLock::new();

/// Initiates [`HUBS`] by connecting to all hubs defined in the command line.
async fn init_hubs() -> Result<(), crate::Error> {
    let hubs = Hubs::init().await?;
    HUBS.set(hubs).ok();

    Ok(())
}

/// Retrieves a reference to the list of hubs. Needs to be called just after initialization.
pub fn hubs<'a>() -> &'a Hubs {
    HUBS.get().expect("hubs not initialized")
}

/// The entrypoint of the Samizdat node.
#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    init_cli()?;

    // Init logger:
    samizdat_common::logger::init();

    tracing::info!(
        "Starting SAMIZDAT node in folder {:?}",
        cli().data.canonicalize()?
    );

    // Init resources:
    init_db::<crate::db::Table>(&cli().data.to_string_lossy())?;
    init_access_token()?;
    init_identity_provider()?;
    init_hubs().await?;

    // Start vacuum:
    tokio::spawn(crate::vacuum::run_vacuum_daemon());

    // Run public server:
    http::serve().await?;

    // Exit:
    Ok(())
}
