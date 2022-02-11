use serde_derive::Serialize;

use crate::api;

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
