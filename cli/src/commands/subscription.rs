//! Subscription command implementations for the Samizdat CLI.

use tabled::Tabled;

use samizdat_common::Key;

use super::show_table;
use crate::api;

/// Creates a new subscription to a series.
///
/// # Arguments
/// * `public_key` - Public key of the series to subscribe to
/// * `max_size_mb` - Optional cap (MB) on the size of a single edition for this series.
///   `None` falls back to the node's `default_max_edition_size_mb`. Enforced atomically
///   by the cap module at object-fetch time.
pub async fn new(public_key: String, max_size_mb: Option<u64>) -> Result<(), anyhow::Error> {
    api::post_subscription(api::PostSubscriptionRequest {
        public_key: &public_key,
        max_size_mb,
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
    #[derive(Tabled)]
    struct Row {
        /// Public key of the subscribed series
        public_key: Key,
        /// Type of subscription
        kind: String,
        /// Per-edition size cap in MB, or "default" when the
        /// operator's `default_max_edition_size_mb` applies.
        max_size: String,
    }

    /// Renders an optional byte count as a human-readable MB string,
    /// returning "default" when None. Suitable for any column that
    /// shows a configured-or-defaulted size; not specific to caps.
    fn render_size_or_default(bytes: Option<u64>) -> String {
        match bytes {
            None => "default".to_owned(),
            Some(bytes) => format!("{} MB", bytes / 1_000_000),
        }
    }

    async fn list_subscription(public_key: String) -> Result<(), anyhow::Error> {
        let subscription = api::get_subscription(&public_key).await?;

        show_table(vec![Row {
            public_key: subscription.public_key,
            kind: subscription.kind,
            max_size: render_size_or_default(subscription.max_bytes),
        }]);

        Ok(())
    }

    /// Lists all subscriptions.
    async fn list_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_subscriptions().await?;

        show_table(
            response
                .into_iter()
                .map(|subscription| Row {
                    public_key: subscription.public_key,
                    kind: subscription.kind,
                    max_size: render_size_or_default(subscription.max_bytes),
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
