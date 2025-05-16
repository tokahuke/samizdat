//! Identity management commands for interacting with the identity service.
//! Provides functionality to set/get providers and manage identity-entity mappings.

use crate::api::{get_polygon_provider, put_polygon_provider};

/// Sets the Polygon provider endpoint for identity operations.
pub async fn set_provider(endpoint: &str) -> Result<(), anyhow::Error> {
    put_polygon_provider(endpoint.to_owned()).await?;
    Ok(())
}

/// Retrieves and displays the current Polygon provider endpoint.
pub async fn get_provider() -> Result<(), anyhow::Error> {
    let endpoint = get_polygon_provider().await?.endpoint;
    print!("{endpoint}");
    Ok(())
}

/// Creates a new identity-entity mapping with the specified parameters.
pub async fn create(
    identity: String,
    entity: String,
    ttl: u64,
    endpoint: Option<String>,
) -> Result<(), anyhow::Error> {
    // Check if entity is a well-formed Samizdat public key.
    anyhow::ensure!(
        entity.parse::<samizdat_common::Key>().is_ok(),
        "Entity is not a valid series"
    );
    crate::identity_dapp::create(identity, entity, ttl, endpoint).await?;
    Ok(())
}

/// Updates an existing identity-entity mapping.
pub async fn update(
    identity: String,
    entity: String,
    ttl: u64,
    endpoint: Option<String>,
) -> Result<(), anyhow::Error> {
    // Check if entity is a well-formed Samizdat public key.
    anyhow::ensure!(
        entity.parse::<samizdat_common::Key>().is_ok(),
        "Entity is not a valid series"
    );
    crate::identity_dapp::update(identity, entity, ttl, endpoint).await?;
    Ok(())
}

/// Retrieves and displays the entity associated with the specified identity.
pub async fn get(identity: String, endpoint: Option<String>) -> Result<(), anyhow::Error> {
    let entity = crate::identity_dapp::get(identity, endpoint).await?;
    println!("{entity}");
    Ok(())
}
