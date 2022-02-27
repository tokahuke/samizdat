//! TODO: this code was hastily written. Good place for a big refactry.

mod access_token;
mod api;
mod cli;
mod commands;
// mod error;
mod html;
mod logger;
mod manifest;
mod util;

pub use access_token::access_token;
pub use cli::server;
// pub use error::Error;
pub use manifest::{Manifest, PrivateManifest};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    cli::init_cli()?;

    let _ = logger::init_logger(cli::cli().verbose);

    access_token::init_access_token()?;

    api::validate_node_is_up().await?;
    if let Err(err) = cli::cli().clone().command.execute().await {
        println!("Error: {err:?}");
    }

    Ok(())
}
