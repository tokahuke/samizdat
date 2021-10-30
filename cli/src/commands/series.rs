use serde_derive::{Deserialize, Serialize};
use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use super::show_table;

pub async fn new(series_name: String) -> Result<(), crate::Error> {
    #[derive(Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/_seriesowners", crate::server()))
        .json(&Request {
            series_owner_name: &*series_name,
        })
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Series key: {}", response.text().await?);

    Ok(())
}

pub async fn rm(series_name: String) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .delete(format!(
            "{}/_seriesowners/{}",
            crate::server(), series_name
        ))
        .send()
        .await?;

    println!("Status: {}", response.status());

    Ok(())
}

pub async fn show(series_name: String) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "{}/_seriesowners/{}",
            crate::server(), series_name
        ))
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Series public key: {}", response.text().await?);

    Ok(())
}

pub async fn list(series_owner_name: &Option<String>) -> Result<(), crate::Error> {
    pub async fn series_list_series(_series_owner_name: &str) -> Result<(), crate::Error> {
        // let client = reqwest::Client::new();
        todo!()
    }

    pub async fn series_list_all() -> Result<(), crate::Error> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/_seriesowners", crate::server()))
            .send()
            .await?;

        log::info!("Status: {}", response.status());

        #[derive(Debug, Deserialize)]
        struct SeriesOwner {
            name: String,
            keypair: ed25519_dalek::Keypair,
            default_ttl: std::time::Duration,
        }

        #[derive(Tabled)]
        struct Row {
            name: String,
            public_key: Key,
            private_key: PrivateKey,
            default_ttl: String,
        }

        show_table(
            response
                .json::<Vec<SeriesOwner>>()
                .await?
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
