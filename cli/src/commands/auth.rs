use serde_derive::Serialize;

use crate::api;

pub async fn grant(scope: String, granted_rights: Vec<String>) -> Result<(), anyhow::Error> {
    #[derive(Serialize)]
    struct Request {
        granted_rights: Vec<String>,
    }

    let response: bool =
        api::patch(format!("/_auth/{}", scope), Request { granted_rights }).await?;
    println!("Status: {}", response);

    Ok(())
}

pub async fn revoke(scope: String) -> Result<(), anyhow::Error> {
    let response: bool = api::delete(format!("/_auth/{}", scope)).await?;
    println!("Status: {}", response);

    Ok(())
}
