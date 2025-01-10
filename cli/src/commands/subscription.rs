//! Subscription command implementations for the Samizdat CLI.

use tabled::Tabled;

use samizdat_common::Key;

use crate::api;
use super::show_table;

/// Creates a new subscription to a series.
///
/// # Arguments
/// * `public_key` - Public key of the series to subscribe to
pub async fn new(public_key: String) -> Result<(), anyhow::Error> {
    api::post_subscription(api::PostSubscriptionRequest {
        public_key: &public_key,
    })
    .await?;

    Ok(())
}

/// Refreshes a subscription to a series.
///
/// # Arguments
/// * `public_key` - Public key of the series subscription to refresh
pub async fn refresh(public_key: String) -> Result<(), anyhow::Error> {
    api::get_subscription_refresh(&public_key).await?;
    Ok(())
}

/// Removes a subscription to a series.
///
/// # Arguments
/// * `public_key` - Public key of the series subscription to remove
pub async fn rm(public_key: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_subscription(&public_key).await?;

    if !removed {
        println!("NOTE: subscription to {public_key} does not exist.");
    }

    Ok(())
}

/// Lists subscriptions, either all or for a specific series.
///
/// # Arguments
/// * `public_key` - Optional public key of the series to list subscriptions for
pub async fn ls(public_key: Option<String>) -> Result<(), anyhow::Error> {
    async fn list_subscription(_public_key: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// Lists all subscriptions.
    async fn list_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_subscriptions().await?;

        #[derive(Tabled)]
        struct Row {
            /// Public key of the subscribed series
            public_key: Key,
            /// Type of subscription
            kind: String,
        }

        show_table(
            response
                .into_iter()
                .map(|subscription| Row {
                    public_key: subscription.public_key,
                    kind: subscription.kind,
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(public_key) = public_key {
        list_subscription(public_key).await
    } else {
        list_all().await
    }
}
