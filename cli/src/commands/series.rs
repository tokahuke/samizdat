//! Series command implementations for the Samizdat CLI.

use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use super::show_table;
use crate::api::{self, Keypair};

/// Creates a new series with the specified parameters.
///
/// # Arguments
/// * `series_name` - Name of the series to create
/// * `is_draft` - Whether this is a draft series. Draft series are not published to the
/// network.
/// * `public_key` - Optional public key for the series
/// * `private_key` - Optional private key for the series
///
/// # Panics
/// Panics if only one of public_key or private_key is provided without the other
pub async fn new(
    series_name: String,
    is_draft: bool,
    public_key: Option<String>,
    private_key: Option<String>,
) -> Result<(), anyhow::Error> {
    if public_key.is_some() && private_key.is_none() {
        anyhow::bail!("Missing private key")
    } else if public_key.is_none() && private_key.is_some() {
        anyhow::bail!("Missing public key")
    }

    let keypair = public_key
        .zip(private_key)
        .map(|(public_key, private_key)| Keypair {
            public_key,
            private_key,
        });

    api::post_series_owner(api::PostSeriesOwnerRequest {
        series_owner_name: &series_name,
        keypair,
        is_draft,
    })
    .await?;

    Ok(())
}

/// Removes a series.
///
/// # Arguments
/// * `series_name` - Name of the series to remove
pub async fn rm(series_name: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_series_owner(&series_name).await?;

    if !removed {
        println!("NOTE: series {series_name} does not exist.");
    }

    Ok(())
}

/// Shows information about a specific series.
///
/// # Arguments
/// * `series_name` - Name of the series to show
pub async fn show(series_name: String) -> Result<(), anyhow::Error> {
    api::get_series_owner(&series_name).await?;
    Ok(())
}

/// Lists series owners, either all or for a specific series.
///
/// # Arguments
/// * `series_owner_name` - Optional name of the series owner to list
pub async fn ls(series_owner_name: Option<String>) -> Result<(), anyhow::Error> {
    async fn ls_series(_series_owner_name: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    async fn ls_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series_owners().await?;

        #[derive(Tabled)]
        struct Row {
            /// Name of the series owner
            name: String,
            /// Public key of the series
            public_key: Key,
            /// Private key of the series
            private_key: PrivateKey,
            /// Default time-to-live duration
            default_ttl: String,
        }

        show_table(
            response
                .into_iter()
                .map(|series_owner| Row {
                    name: series_owner.name,
                    public_key: series_owner.keypair.verifying_key().into(),
                    private_key: series_owner.keypair.to_scalar_bytes().into(),
                    default_ttl: format!("{:?}", series_owner.default_ttl),
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_owner_name) = series_owner_name {
        ls_series(series_owner_name).await
    } else {
        ls_all().await
    }
}

/// Lists cached series information, either all or for a specific series.
///
/// # Arguments
/// * `series_name` - Optional name of the series to list cached information for
pub async fn ls_cached(series_name: Option<String>) -> Result<(), anyhow::Error> {
    async fn ls_cached_series(_series_name: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    async fn ls_cached_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series().await?;

        #[derive(Tabled)]
        struct Row {
            /// Public key of the series
            public_key: Key,
        }

        show_table(
            response
                .into_iter()
                .map(|series| Row {
                    public_key: series.public_key,
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_name) = series_name {
        ls_cached_series(series_name).await
    } else {
        ls_cached_all().await
    }
}
