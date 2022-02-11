use anyhow::Context;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

//use samizdat_common::Hash;

use crate::access_token::access_token;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError(pub String);

impl From<ApiError> for anyhow::Error {
    fn from(e: ApiError) -> anyhow::Error {
        anyhow::anyhow!("{}", e.0)
    }
}

lazy_static! {
    pub static ref CLIENT: reqwest::Client = reqwest::Client::new();
}

pub async fn validate_node_is_up() -> Result<(), anyhow::Error> {
    let response = CLIENT.get(format!("{}/", crate::server())).send().await;

    if let Err(error) = response {
        if error.is_connect() {
            return Err(anyhow::anyhow!(
                "Failed to connect to your local node. Check if samizdat-node is up and running"
            ));
        }
    }

    Ok(())
}

pub async fn post_object(
    content: Vec<u8>,
    content_type: &str,
    bookmark: bool,
    is_draft: bool,
) -> Result<String, anyhow::Error> {
    let url = format!("{}/_objects", crate::server());
    let response = CLIENT
        .post(&format!(
            "{}/_objects?bookmark={}&is-draft={}",
            crate::server(),
            bookmark,
            is_draft,
        ))
        .header("Content-Type", content_type)
        .header("Authorization", format!("Bearer {}", access_token()))
        .body(content)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST /_objects"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response POST /_objects"))?;

    log::info!("{} GET {} {}", status, url, text);

    let content: Result<String, ApiError> = serde_json::from_str(&text)
        .with_context(|| format!("error deserializing response from POST /_objects: {text}"))?;

    Ok(content?)
}

pub async fn get<R, Q>(route: R) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request GET {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response GET {}", route.as_ref()))?;

    log::info!("{} GET {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from GET {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

pub async fn post<R, P, Q>(route: R, payload: P) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    P: Serialize + std::fmt::Debug,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response POST {}", route.as_ref()))?;

    log::info!("{} POST {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from POST {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

pub async fn patch<R, P, Q>(route: R, payload: P) -> Result<Q, anyhow::Error>
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
        .await
        .with_context(|| format!("error from samizdat-node request PATCH {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response PATCH {}", route.as_ref()))?;

    log::info!("{} PATCH {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from PATCH {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

pub async fn delete<R, Q>(route: R) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server(), route.as_ref());
    let response = CLIENT
        .delete(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request DELETE {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response GET {}", route.as_ref()))?;

    log::info!("{} GET {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from DELETE {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}
