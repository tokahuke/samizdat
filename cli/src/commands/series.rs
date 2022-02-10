use serde_derive::{Deserialize, Serialize};
use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use crate::api;

use super::show_table;

pub async fn new(series_name: String) -> Result<(), anyhow::Error> {
    #[derive(Debug, Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
    }

    api::post(
        "/_seriesowners",
        Request {
            series_owner_name: &*series_name,
        },
    )
    .await?;

    Ok(())
}

pub async fn rm(series_name: String) -> Result<(), anyhow::Error> {
    api::delete(format!("/_seriesowners/{}", series_name)).await?;
    Ok(())
}

pub async fn show(series_name: String) -> Result<(), anyhow::Error> {
    api::get(format!("/_seriesowners/{}", series_name)).await?;
    Ok(())
}

pub async fn list(series_owner_name: Option<String>) -> Result<(), anyhow::Error> {
    pub async fn series_list_series(_series_owner_name: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    pub async fn series_list_all() -> Result<(), anyhow::Error> {
        #[derive(Debug, Deserialize)]
        struct SeriesOwner {
            name: String,
            keypair: ed25519_dalek::Keypair,
            default_ttl: std::time::Duration,
        }

        let response: Vec<SeriesOwner> = api::get("/_seriesowners").await?;

        #[derive(Tabled)]
        struct Row {
            name: String,
            public_key: Key,
            private_key: PrivateKey,
            default_ttl: String,
        }

        show_table(
            response
                .into_iter()
                .map(|series_owner| Row {
                    name: series_owner.name,
                    public_key: series_owner.keypair.public.into(),
                    private_key: series_owner.keypair.secret.into(),
                    default_ttl: format!("{:?}", series_owner.default_ttl),
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_owner_name) = series_owner_name {
        series_list_series(series_owner_name).await
    } else {
        series_list_all().await
    }
}
