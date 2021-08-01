mod cli;
mod db;
mod flatbuffers;
mod http;
mod rpc;

pub use samizdat_common::Error;

pub use cli::cli;
pub use db::{db, Table};

use std::panic;
use tokio::task;
use warp::Filter;

use samizdat_common::logger;

use cli::init_cli;
use db::init_db;

static mut HUB: Option<rpc::HubConnection> = None;

async fn init_hub() -> Result<(), crate::Error> {
    let hub = rpc::HubConnection::connect(([0, 0, 0, 0], 4511)).await?;

    unsafe {
        HUB = Some(hub);
    }

    Ok(())
}

pub fn hub<'a>() -> &'a rpc::HubConnection {
    unsafe { HUB.as_ref().expect("hub connection not initialized") }
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
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    init_cli()?;
    init_db()?;
    init_hub().await?;

    // Describe server:
    let server = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html"))
        .or(http::get_hash())
        .or(http::post_content())
        .with(warp::log("api"));

    // Run server:
    let http_server = tokio::spawn(warp::serve(server).run(([0, 0, 0, 0], cli().port)));
    // let rpc_server = tokio::spawn(crate::rpc::run(([0, 0, 0, 0], 4511)));

    maybe_resume_panic(http_server.await);
    // maybe_resume_panic(rpc_server.await);

    // Exit:
    Ok(())
}
