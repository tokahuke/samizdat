mod cli;
mod db;
mod flatbuffers;
mod replay_resistance;
mod rpc;

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
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    let _ = &*CLI;
    db::init_db()?;

    let direct_rpc_server = tokio::spawn(crate::rpc::run_direct((CLI.address, CLI.direct_port)));
    let reverse_rpc_server = tokio::spawn(crate::rpc::run_reverse((CLI.address, CLI.reverse_port)));

    maybe_resume_panic(direct_rpc_server.await);
    maybe_resume_panic(reverse_rpc_server.await);

    // Exit:
    Ok(())
}
