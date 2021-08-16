mod cache;
mod cli;
mod db;
mod http;
//mod object;
mod public_folder;
mod rpc;

pub use samizdat_common::Error;

pub use cli::cli;
pub use db::{db, Table};

use futures::prelude::*;
use std::panic;
use tokio::task;
use warp::Filter;

use samizdat_common::logger;

use cli::init_cli;
use db::init_db;
use rpc::Hubs;

static mut HUBS: Option<Hubs> = None;

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

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    init_cli()?;
    init_db()?;
    init_hubs().await?;

    // Describe server:
    let server = warp::filters::addr::remote()
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
        .or(warp::get()
            .and(warp::path::end())
            .map(|| {
                warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html")
            })
            .or(http::get_object())
            .or(http::post_object())
            .or(http::delete_object())
            .or(http::post_collection())
            .or(http::get_item()))
        .with(warp::log("api"));

    // Run server:
    let http_server = tokio::spawn(warp::serve(server).run(([0, 0, 0, 0], cli().port)));

    maybe_resume_panic(http_server.await);

    // Exit:
    Ok(())
}
