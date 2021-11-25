use serde_derive::{Deserialize, Serialize};
use tabled::Tabled;

use samizdat_common::Key;

use crate::api;

use super::show_table;

pub async fn new(public_key: String) -> Result<(), crate::Error> {
    #[derive(Serialize)]
    struct Request<'a> {
        public_key: &'a str,
    }

    api::post(
        "/_subscriptions",
        Request {
            public_key: &public_key,
        },
    )
    .await??;

    Ok(())
}

pub async fn rm(public_key: String) -> Result<(), crate::Error> {
    api::delete(format!("/_subscriptions/{}", public_key)).await??;

    Ok(())
}

pub async fn ls(public_key: Option<String>) -> Result<(), crate::Error> {
    pub async fn list_subscription(_public_key: String) -> Result<(), crate::Error> {
        todo!()
    }

    pub async fn list_all() -> Result<(), crate::Error> {
        #[derive(Debug, Deserialize)]
        struct Subscription {
            public_key: Key,
            kind: String,
        }

        let response: Vec<Subscription> = api::get("/_subscriptions").await??;

        #[derive(Tabled)]
        struct Row {
            public_key: Key,
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
