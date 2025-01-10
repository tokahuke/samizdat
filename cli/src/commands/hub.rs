//! Hub command implementations for the Samizdat CLI.

use tabled::Tabled;

use super::show_table;
use crate::api::{self, PostHubRequest};

/// Creates a new hub connection with the specified address and resolution mode.
///
/// # Arguments
/// * `address` - Network address of the hub
/// * `resolution_mode` - Mode used for resolving the DNS or socket address
pub async fn new(address: String, resolution_mode: String) -> Result<(), anyhow::Error> {
    api::post_hub(PostHubRequest {
        address,
        resolution_mode,
    })
    .await?;

    Ok(())
}

/// Lists all registered hubs.
pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_hubs().await?;

    #[derive(Tabled)]
    struct Row {
        /// Network address of the hub
        address: String,
        /// Mode used for resolving the DNS or socket address
        resolution_mode: String,
    }

    show_table(
        response
            .into_iter()
            .map(|hub| Row {
                address: hub.address,
                resolution_mode: hub.resolution_mode,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}

/// Removes a hub connection.
///
/// # Arguments
/// * `address` - Network address of the hub to remove
pub async fn rm(address: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_hub(&address).await?;

    if !removed {
        println!("NOTE: hub {address} does not exist.");
    }

    Ok(())
}
