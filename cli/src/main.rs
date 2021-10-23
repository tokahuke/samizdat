//! TODO: this code was hastily written. Good place for a big refactry.

mod cli;
mod commands;
mod error;
mod logger;
mod manifest;
mod util;
mod html;

pub use error::Error;
pub use manifest::{Manifest, PrivateManifest};

async fn validate_node_is_up() -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client.get("http://localhost:4510/").send().await;

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
    validate_node_is_up().await?;
    cli::cli().execute().await?;

    Ok(())
}
