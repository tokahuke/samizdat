//! Command line interface for Samizdat node and hub.

#![feature(try_blocks)]

mod access_token;
mod api;
mod cli;
mod commands;
mod html;
mod logger;
mod manifest;
mod util;
mod ws;

pub use access_token::access_token;
pub use cli::server;
pub use manifest::{Manifest, PrivateManifest};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    cli::init_cli()?;

    let _ = logger::init_logger(cli::cli().verbose);

    access_token::init_access_token()?;
    access_token::init_port()?;

    api::validate_node_is_up().await?;
    if let Err(err) = cli::cli().clone().command.execute().await {
        println!("Error: {err:?}");
    }

    Ok(())
}
