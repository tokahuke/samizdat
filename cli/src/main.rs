//! TODO: this code was hastily written. Good place for a big refactry.

mod access_token;
mod cli;
mod commands;
mod error;
mod html;
mod logger;
mod manifest;
mod util;

pub use access_token::access_token;
pub use cli::server;
pub use error::Error;
pub use manifest::{Manifest, PrivateManifest};

async fn validate_node_is_up() -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client.get(format!("{}/", crate::server())).send().await;

    if let Err(error) = response {
        if error.is_connect() {
            return Err(crate::Error::Message(
                "Failed to connect to your local node. Check if samizdat-node is up and running"
                    .to_owned(),
            ));
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let _ = logger::init_logger();

    cli::init_cli()?;
    access_token::init_access_token()?;

    validate_node_is_up().await?;

    cli::cli().clone().command.execute().await?;

    Ok(())
}
