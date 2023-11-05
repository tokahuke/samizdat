use tabled::Tabled;

use crate::api::{self, PostHubRequest};

use super::show_table;

pub async fn new(address: String, resolution_mode: String) -> Result<(), anyhow::Error> {
    api::post_hub(PostHubRequest {
        address,
        resolution_mode,
    })
    .await?;

    Ok(())
}

pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_hubs().await?;

    #[derive(Tabled)]
    struct Row {
        address: String,
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

pub async fn rm(address: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_hub(&address).await?;

    if !removed {
        println!("NOTE: hub {address} does not exist.");
    }

    Ok(())
}
