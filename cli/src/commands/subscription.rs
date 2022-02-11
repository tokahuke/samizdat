use tabled::Tabled;

use samizdat_common::Key;

use crate::api;

use super::show_table;

pub async fn new(public_key: String) -> Result<(), anyhow::Error> {
    api::post_subscription(api::PostSubscriptionRequest {
        public_key: &public_key,
    })
    .await?;

    Ok(())
}

pub async fn rm(public_key: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_subscription(&public_key).await?;

    if !removed {
        println!("NOTE: subscription to {public_key} does not exist.");
    }

    Ok(())
}

pub async fn ls(public_key: Option<String>) -> Result<(), anyhow::Error> {
    pub async fn list_subscription(_public_key: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    pub async fn list_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_subscriptions().await?;

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
