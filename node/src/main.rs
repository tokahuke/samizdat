
mod cli;
mod error;
mod http;
mod logger;
mod flatbuffers;

pub use error::Error;

use structopt::StructOpt;
use warp::Filter;

lazy_static::lazy_static! {
    pub static ref CLI: cli::Cli = cli::Cli::from_args();
    pub static ref DB: rocksdb::DB = rocksdb::DB::open_default(&CLI.db_path).unwrap();
}

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    // Init logger:
    let _ = logger::init_logger();

    // Init resources:
    &*CLI;
    &*DB;

    // Describe server:
    let server = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html"))
        .or(http::get_hash())
        .or(http::post_content())
        .with(warp::log("api"));

    // Run server:
    warp::serve(server).run(([0, 0, 0, 0], 4510)).await;

    // Exit:
    Ok(())
}
