use tabled::Tabled;

use samizdat_common::{Hash, Key};

use crate::api::{self};

use super::show_table;

pub async fn ls(series_key: Option<String>) -> Result<(), anyhow::Error> {
    pub async fn ls_series(_series_key: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    pub async fn ls_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_editions().await?;

        #[derive(Tabled)]
        struct Row {
            public_key: Key,
            is_draft: bool,
            collection: Hash,
            timestamp: chrono::DateTime<chrono::Utc>,
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
