mod cli;
mod error;
mod flatbuffers;
mod http;
mod logger;
mod rpc;
mod hash;

pub use error::Error;

use std::panic;
use structopt::StructOpt;
use tokio::task;
use warp::Filter;

static mut HUB: Option<rpc::HubConnection> = None;

async fn init_hub() -> Result<(), crate::Error> {
    let hub = rpc::HubConnection::connect(([0, 0, 0, 0], 4511)).await?;

    unsafe {
        HUB = Some(hub);
    }

    Ok(())
}

pub fn hub<'a>() -> &'a rpc::HubConnection {
    unsafe { 
        HUB.as_ref().expect("hub connection not initialized")
    }
}

static mut CLI: Option<cli::Cli> = None;

fn init_cli() -> Result<(), crate::Error> {
    let cli = cli::Cli::from_args();

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

fn cli<'a>() -> &'a cli::Cli {
    unsafe {
        CLI.as_ref().expect("cli not initialized")
    }
}

static mut DB: Option<rocksdb::DB> = None;

fn init_db() -> Result<(), crate::Error> {
    let db = rocksdb::DB::open_default(&cli().db_path)?;

    unsafe {
        DB = Some(db);
    }

    Ok(())
}

fn db<'a>() -> &'a rocksdb::DB {
    unsafe {
        DB.as_ref().expect("db not initialized")
    }
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
    let http_server = tokio::spawn(warp::serve(server).run(([0, 0, 0, 0], 4510)));
    // let rpc_server = tokio::spawn(crate::rpc::run(([0, 0, 0, 0], 4511)));

    maybe_resume_panic(http_server.await);
    // maybe_resume_panic(rpc_server.await);

    // Exit:
    Ok(())
}
