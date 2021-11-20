use serde_derive::Serialize;

use crate::access_token;

pub async fn grant(scope: String, granted_rights: Vec<String>) -> Result<(), crate::Error> {
    #[derive(Serialize)]
    struct Request {
        granted_rights: Vec<String>,
    }

    let client = reqwest::Client::new();
    let response = client.patch(format!(
        "{}/_auth/{}",
        crate::server(),
        scope,
    ))
    .header("Authorization", format!("Bearer {}", access_token()))
    .json(&Request { granted_rights })
    .send()
    .await?
    ;

    println!("Status: {}", response.status());

    Ok(())
}


pub async fn revoke(scope: String) -> Result<(), crate::Error> {

    let client = reqwest::Client::new();
    let response = client.delete(format!(
        "{}/_auth/{}",
        crate::server(),
        scope,
    ))
    .header("Authorization", format!("Bearer {}", access_token()))
    .send()
    .await?
    ;

    println!("Status: {}", response.status());

    Ok(())
}

