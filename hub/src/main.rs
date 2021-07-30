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
    &*CLI;
    //&*DB;

    let rpc_server = tokio::spawn(crate::rpc::run(([0, 0, 0, 0], 4511)));

    maybe_resume_panic(rpc_server.await);

    // Exit:
    Ok(())
}
