mod access_token;
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

use futures::prelude::*;
use std::panic;
use tokio::task;
use warp::Filter;

use samizdat_common::logger;

use access_token::init_access_token;
use cli::init_cli;
use db::init_db;
use system::Hubs;

/// The variable holding a list of all the connections to the hubs.
static mut HUBS: Option<Hubs> = None;

/// Initiates [`HUBS`] by connecting to all hubs defined in the command line.
async fn init_hubs() -> Result<(), crate::Error> {
    let resolved = futures::stream::iter(&cli().hubs)
        .map(cli::AddrToResolve::resolve)
        .buffer_unordered(cli().hubs.len())
        .try_collect::<Vec<_>>()
        .await?;
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
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    init_cli()?;
    init_access_token()?;
    init_db()?;
    init_hubs().await?;

    // Start vacuum:
    tokio::spawn(crate::vacuum::run_vacuum_daemon());

    // Describe server:
    let public_server = warp::filters::addr::remote()
        .and_then(|addr: Option<std::net::SocketAddr>| async move {
            if let Some(addr) = addr {
                if addr.ip().is_loopback() {
                    return Err(warp::reject::not_found());
                }
            }

            Ok(warp::reply::with_status(
                "cannot connect outside loopback",
                ::http::StatusCode::FORBIDDEN,
            ))
        })
        .or(warp::get().and(warp::path::end()).map(|| {
            warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html")
        }))
        .or(http::api())
        .with(warp::log("api"));

    // Run public server:
    let server = tokio::spawn(warp::serve(public_server).run(([0, 0, 0, 0], cli().port)));

    maybe_resume_panic(server.await);

    // Exit:
    Ok(())
}
