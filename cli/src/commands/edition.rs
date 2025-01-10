//! Edition command implementations for the Samizdat CLI.
use tabled::Tabled;

use samizdat_common::{Hash, Key};

use super::show_table;
use crate::api;

/// Lists editions, either for a specific series or all editions.
///
/// # Arguments
/// * `series_key` - Optional public key of the series to list editions for. If None,
///   lists all editions across all series.
pub async fn ls(series_key: Option<String>) -> Result<(), anyhow::Error> {
    /// Lists editions for a specific series.
    pub async fn ls_series(_series_key: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    /// Lists all editions across all series.
    pub async fn ls_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_editions().await?;

        #[derive(Tabled)]
        struct Row {
            /// Public key of the series
            public_key: Key,
            /// Whether this is a draft edition
            is_draft: bool,
            /// Hash of the collection
            collection: Hash,
            /// Creation timestamp
            timestamp: chrono::DateTime<chrono::Utc>,
            /// Time-to-live duration
            ttl: String,
        }

        show_table(
            response
                .into_iter()
                .map(|edition| Row {
                    public_key: edition.public_key,
                    is_draft: edition.is_draft,
                    collection: edition.signed.collection.hash,
                    timestamp: edition.signed.timestamp,
                    ttl: format!("{:?}", edition.signed.ttl),
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_key) = series_key {
        ls_series(series_key).await
    } else {
        ls_all().await
    }
}
