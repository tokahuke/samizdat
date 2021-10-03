mod cli;
mod html;
mod http;
mod logger;
mod slow_compiler_workaround;

use std::io;
use warp::Filter;

//use samizdat_common::logger;

use cli::cli;

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    let _ = logger::init_logger();

    cli::init_cli()?;

    // Describe server:
    let server = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::with_header(include_str!("index.html"), "Content-Type", "text/html"))
        .or(http::api())
        .with(warp::log("api"));

    // Run server:
    let http_server = tokio::spawn(warp::serve(server).run(([0, 0, 0, 0], cli().port)));

    http_server.await?;

    Ok(())
}
