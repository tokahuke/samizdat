use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use crate::access_token::access_token;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError(pub String);

lazy_static! {
    pub static ref CLIENT: reqwest::Client = {
        reqwest::Client::new()
    };
}

pub async fn validate_node_is_up() -> Result<(), crate::Error> {
    let response = CLIENT.get(format!("{}/", crate::server())).send().await;

    if let Err(error) = response {
        if error.is_connect() {
            return Err(crate::Error::Message(
                "Failed to connect to your local node. Check if samizdat-node is up and running"
                    .to_owned(),
            ));
        }
    }

    Ok(())
}

pub async fn get<R, Q>(route: R) -> Result<Result<Q, ApiError>, crate::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;

    log::info!("{} GET {} {}", status, url, text);

    Ok(serde_json::from_str(&text)?)
}

pub async fn post<R, P, Q>(route: R, payload: P) -> Result<Result<Q, ApiError>, crate::Error>
where
    R: AsRef<str>,
    P: Serialize,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .json(&payload)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;

    log::info!("{} POST {} {}", status, url, text);

    Ok(serde_json::from_str(&text)?)
}

pub async fn patch<R, P, Q>(route: R, payload: P) -> Result<Result<Q, ApiError>, crate::Error>
where
    R: AsRef<str>,
    P: Serialize,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .patch(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .json(&payload)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;

    log::info!("{} POST {} {}", status, url, text);

    Ok(serde_json::from_str(&text)?)
}

pub async fn delete<R, Q>(route: R) -> Result<Result<Q, ApiError>, crate::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .delete(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;

    log::info!("{} GET {} {}", status, url, text);

    Ok(serde_json::from_str(&text)?)
}
