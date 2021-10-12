use serde_derive::{Deserialize, Serialize};
use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use crate::manifest::{Manifest, PrivateManifest};

use super::show_table;

pub async fn new(series_name: String) -> Result<(), crate::Error> {
    #[derive(Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
    }

    let client = reqwest::Client::new();
    let response = client
        .post("http://localhost:4510/_seriesowners")
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
            "http://localhost:4510/_seriesowners/{}",
            series_name
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
            "http://localhost:4510/_seriesowners/{}",
            series_name
        ))
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Series public key: {}", response.text().await?);

    Ok(())
}

pub async fn import() -> Result<(), crate::Error> {
    let manifest = Manifest::find()?;
    let private_manifest = PrivateManifest::find()?;

    #[derive(Serialize)]
    struct KeyPair<'a> {
        public_key: &'a str,
        private_key: &'a str,
    }

    #[derive(Serialize)]
    struct Request<'a> {
        series_owner_name: &'a str,
        keypair: KeyPair<'a>,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://localhost:4510/_seriesowners"))
        .json(&Request {
            series_owner_name: &manifest.series.name,
            keypair: KeyPair {
                public_key: &manifest.series.public_key,
                private_key: &private_manifest.private_key,
            },
        })
        .send()
        .await?;

    let _debug_response = client
        .post(format!("http://localhost:4510/_seriesowners"))
        .json(&Request {
            series_owner_name: &manifest.debug.name,
            keypair: KeyPair {
                public_key: &manifest.debug.public_key,
                private_key: &private_manifest.private_key_debug,
            },
        })
        .send()
        .await?;

    println!("Status: {}", response.status());
    // println!("Response: {}", response.text().await?);

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
            .get("http://localhost:4510/_seriesowners")
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
