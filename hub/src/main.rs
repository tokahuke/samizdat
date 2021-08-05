mod cli;
mod error;
mod flatbuffers;
mod rpc;

pub use error::Error;

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
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    let _ = &*CLI;
    //let _ = &*DB;

    let direct_rpc_server = tokio::spawn(crate::rpc::run_direct(([127, 0, 0, 1], CLI.direct_port)));
    let reverse_rpc_server =
        tokio::spawn(crate::rpc::run_reverse(([127, 0, 0, 1], CLI.reverse_port)));

    maybe_resume_panic(direct_rpc_server.await);
    maybe_resume_panic(reverse_rpc_server.await);

    // Exit:
    Ok(())
}
