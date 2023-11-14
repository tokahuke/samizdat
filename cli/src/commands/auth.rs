use serde_derive::Serialize;
use tabled::Tabled;

use super::show_table;
use crate::api::{self, get_auths};

pub async fn grant(scope: String, granted_rights: Vec<String>) -> Result<(), anyhow::Error> {
    #[derive(Serialize)]
    struct Request {
        granted_rights: Vec<String>,
    }

    let granted = api::patch_auth(&scope, api::PatchAuthRequest { granted_rights }).await?;

    if !granted {
        println!("NOTE: scope {scope} already has granted rights. Revoke them to grant new rights");
    }

    Ok(())
}

pub async fn revoke(scope: String) -> Result<(), anyhow::Error> {
    let revoked = api::delete_auth(&scope).await?;

    if !revoked {
        println!("NOTE: scope {scope} had no granted rights");
    }

    Ok(())
}

pub async fn ls() -> Result<(), anyhow::Error> {
    let auths = get_auths().await?;

    #[derive(Tabled)]
    struct Row {
        scope: String,
        granted_rights: String,
    }

    show_table(
        auths
            .into_iter()
            .map(|auth| Row {
                scope: if auth.entity.r#type == "_identity" {
                    format!("/{}", auth.entity.identifier)
                } else {
                    format!("/{}/{}", auth.entity.r#type, auth.entity.identifier)
                },
                granted_rights: auth.granted_rights.join(", "),
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
