use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use crate::api;

use super::show_table;

pub async fn new(series_name: String, is_draft: bool) -> Result<(), anyhow::Error> {
    api::post_series_owner(api::PostSeriesOwnerRequest {
        series_owner_name: &*series_name,
        keypair: None,
        is_draft,
    })
    .await?;

    Ok(())
}

pub async fn rm(series_name: String) -> Result<(), anyhow::Error> {
    let removed = api::delete_series_owner(&series_name).await?;

    if !removed {
        println!("NOTE: series {series_name} does not exist.");
    }

    Ok(())
}

pub async fn show(series_name: String) -> Result<(), anyhow::Error> {
    api::get_series_owner(&series_name).await?;
    Ok(())
}

pub async fn ls(series_owner_name: Option<String>) -> Result<(), anyhow::Error> {
    pub async fn ls_series(_series_owner_name: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    pub async fn ls_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series_owners().await?;

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
        ls_series(series_owner_name).await
    } else {
        ls_all().await
    }
}


pub async fn ls_cached(series_name: Option<String>) -> Result<(), anyhow::Error> {
    pub async fn ls_cached_series(_series_name: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    pub async fn ls_cached_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series().await?;

        #[derive(Tabled)]
        struct Row {
            public_key: Key,
        }

        show_table(
            response
                .into_iter()
                .map(|series| Row {
                    public_key: series.public_key,
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_name) = series_name {
        ls_cached_series(series_name).await
    } else {
        ls_cached_all().await
    }
}
