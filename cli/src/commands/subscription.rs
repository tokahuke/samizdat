use serde_derive::{Deserialize, Serialize};
use tabled::Tabled;

use samizdat_common::Key;

use crate::access_token;

use super::show_table;

pub async fn new(public_key: String) -> Result<(), crate::Error> {
    #[derive(Serialize)]
    struct Request<'a> {
        public_key: &'a str,
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/_subscriptions", crate::server()))
        .json(&Request {
            public_key: &public_key,
        })
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await?;

    log::info!("Status: {}", response.status());
    println!("Response: {}", response.text().await?);

    Ok(())
}

pub async fn rm(public_key: String) -> Result<(), crate::Error> {
    let client = reqwest::Client::new();
    let response = client
        .delete(format!("{}/_subscriptions/{}", crate::server(), public_key))
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await?;

    println!("Status: {}", response.status());

    Ok(())
}

pub async fn ls(public_key: Option<String>) -> Result<(), crate::Error> {
    pub async fn list_subscription(_public_key: String) -> Result<(), crate::Error> {
        // let client = reqwest::Client::new();
        todo!()
    }

    pub async fn list_all() -> Result<(), crate::Error> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/_subscriptions", crate::server()))
            .header("Authorization", format!("Bearer {}", access_token()))
            .send()
            .await?;

        log::info!("Status: {}", response.status());

        #[derive(Debug, Deserialize)]
        struct Subscription {
            public_key: Key,
            kind: String,
        }

        #[derive(Tabled)]
        struct Row {
            public_key: Key,
            kind: String,
        }

        show_table(
            response
                .json::<Vec<Subscription>>()
                .await?
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
