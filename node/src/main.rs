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
pub use db::db;

use std::{panic, sync::OnceLock};
use tokio::task;

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

/// Utility for propagating panics through tasks.
fn maybe_resume_panic<T>(r: Result<T, task::JoinError>) {
    if let Err(err) = r {
        if let Ok(panic) = err.try_into_panic() {
            panic::resume_unwind(panic);
        }
    }
}

/// The entrypoint of the Samizdat node.
#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    init_cli()?;

    // Init logger:
    tracing_subscriber::fmt().init();

    tracing::info!(
        "Starting SAMIZDAT node in folder {:?}",
        cli().data.canonicalize()?
    );

    // Init resources:
    init_db()?;
    init_access_token()?;
    init_identity_provider()?;
    init_hubs().await?;

    // Start vacuum:
    tokio::spawn(crate::vacuum::run_vacuum_daemon());

    // Run public server:
    let server = tokio::spawn(http::serve());

    maybe_resume_panic(server.await);

    // Exit:
    Ok(())
}
