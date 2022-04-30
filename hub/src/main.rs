#![feature(ip)]

mod cli;
mod db;
mod http;
mod replay_resistance;
mod rpc;
mod slow_compiler_workaround;
mod utils;

pub use db::db;
pub use samizdat_common::Error;

use std::panic;
use structopt::StructOpt;
use tokio::task;

use samizdat_common::logger;

lazy_static::lazy_static! {
    pub static ref CLI: cli::Cli = cli::Cli::from_args();
    // pub static ref DB: rocksdb::DB = rocksdb::DB::open_default(&CLI.db_path).unwrap();
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
    let _ = logger::init_logger(CLI.verbose);

    db::init_db()?;

    // Spawn services:
    let direct_rpc_server = tokio::spawn(crate::rpc::run_direct(CLI.direct_addresses.clone()));
    let reverse_rpc_server = tokio::spawn(crate::rpc::run_reverse(CLI.reverse_addresses.clone()));
    let http_server = tokio::spawn(http::serve());
    let partners = tokio::spawn(crate::rpc::run_partners());

    // Await for services to end:
    maybe_resume_panic(direct_rpc_server.await);
    maybe_resume_panic(reverse_rpc_server.await);
    maybe_resume_panic(http_server.await);
    maybe_resume_panic(partners.await);

    // Exit:
    Ok(())
}
