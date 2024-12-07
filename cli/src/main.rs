//! Command line interface for Samizdat node and hub.

#![feature(try_blocks, once_cell_try)]

mod access_token;
mod api;
mod cli;
mod commands;
mod html;
mod identity_dapp;
mod manifest;
mod util;
mod ws;

pub use access_token::access_token;
pub use cli::server;
pub use manifest::{Manifest, PrivateManifest};

#[tokio::main]
async fn main() {
    let outcome: Result<(), anyhow::Error> = try {
        if cli::cli().verbose {
            tracing_subscriber::fmt().init();
        }
        
        access_token::init_port()?;

        api::validate_node_is_up().await?;
        cli::cli().clone().command.execute().await?;
    };

    if let Err(err) = outcome {
        println!("Error: {err:?}");
    }
}
