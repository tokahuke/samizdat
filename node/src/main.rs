#![feature(ip)]

mod access;
mod cli;
mod db;
mod http;
mod models;
mod replay_resistance;
mod slow_compiler_workaround;
mod system;
mod utils;
mod vacuum;

pub use samizdat_common::Error;

pub use cli::cli;
pub use db::db;

use futures::{prelude::*, TryStreamExt};
use std::panic;
use tokio::task;

use samizdat_common::logger;

use access::init_access_token;
use cli::init_cli;
use db::init_db;
use system::Hubs;

/// The variable holding a list of all the connections to the hubs.
static mut HUBS: Option<Hubs> = None;

/// Initiates [`HUBS`] by connecting to all hubs defined in the command line.
async fn init_hubs() -> Result<(), crate::Error> {
    let sockets = cli()
        .hubs
        .iter()
        .map(|to_resolve| to_resolve.resolve(cli().resolution_mode));
    let resolved = stream::iter(sockets)
        .buffer_unordered(cli().hubs.len())
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten();
    let hubs = Hubs::init(resolved).await?;

    unsafe {
        HUBS = Some(hubs);
    }

    Ok(())
}

/// Retrieves a reference to the list of hubs. Needs to be called just after initialization.
pub fn hubs<'a>() -> &'a Hubs {
    unsafe { HUBS.as_ref().expect("hubs not initialized") }
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
    let _ = logger::init_logger(cli().verbose);

    log::info!(
        "Starting SAMIZDAT node in folder {:?}",
        cli().data.canonicalize()?
    );

    // Init resources:
    init_db()?;
    init_access_token()?;
    init_hubs().await?;

    // Start vacuum:
    tokio::spawn(crate::vacuum::run_vacuum_daemon());

    // Run public server:
    let server = tokio::spawn(http::serve());

    maybe_resume_panic(server.await);

    // Exit:
    Ok(())
}
