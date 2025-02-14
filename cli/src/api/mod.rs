//! API client implementation for interacting with the Samizdat node.
//!
//! This module provides the core functionality for making HTTP requests to a Samizdat node,
//! including error handling, authentication, and basic request/response processing. It also
//! provides strongly-typed functions for buiilding API calls.

mod calls;

pub use calls::*;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::access_token::access_token;

/// Error response from the API.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError(pub String);

impl From<ApiError> for anyhow::Error {
    fn from(e: ApiError) -> anyhow::Error {
        anyhow::anyhow!("{}", e.0)
    }
}

/// HTTP client used for making requests to the Samizdat node.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| reqwest::Client::new());

/// Validates that the Samizdat node is running and accessible.
pub async fn validate_node_is_up() -> Result<(), anyhow::Error> {
    let response = CLIENT.get(format!("{}/", crate::server()?)).send().await;

    if let Err(error) = response {
        if error.is_connect() {
            anyhow::bail!(
                "Failed to connect to node at {}. Check if samizdat-node is up and running",
                crate::server()?
            );
        } else {
            anyhow::bail!(
                "Unexpected error testing connection to node at {}: {error}",
                crate::server()?
            );
        }
    }

    Ok(())
}

/// Makes a GET request to the specified route.
///
/// # Type Parameters
/// * `R` - Type of the route
/// * `Q` - Type of the response
async fn get<R, Q>(route: R) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server()?, route.as_ref());
    let response = CLIENT
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request GET {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response GET {}", route.as_ref()))?;

    tracing::info!("{} GET {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from GET {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

/// Makes a POST request to the specified route with the given payload.
///
/// # Type Parameters
/// * `R` - Type of the route
/// * `P` - Type of the payload
/// * `Q` - Type of the response
async fn post<R, P, Q>(route: R, payload: P) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    P: Serialize + std::fmt::Debug,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server()?, route.as_ref());
    let response = CLIENT
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response POST {}", route.as_ref()))?;

    tracing::info!("{} POST {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from POST {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

/// Makes a PUT request to the specified route with the given payload.
///
/// # Type Parameters
/// * `R` - Type of the route
/// * `P` - Type of the payload
/// * `Q` - Type of the response
async fn put<R, P, Q>(route: R, payload: P) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    P: Serialize + std::fmt::Debug,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server()?, route.as_ref());
    let response = CLIENT
        .put(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response POST {}", route.as_ref()))?;

    tracing::info!("{} POST {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from POST {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

/// Makes a PATCH request to the specified route with the given payload.
///
/// # Type Parameters
/// * `R` - Type of the route
/// * `P` - Type of the payload
/// * `Q` - Type of the response
async fn patch<R, P, Q>(route: R, payload: P) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    P: Serialize,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server()?, route.as_ref());
    let response = CLIENT
        .patch(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request PATCH {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response PATCH {}", route.as_ref()))?;

    tracing::info!("{} PATCH {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from PATCH {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}

/// Makes a DELETE request to the specified route.
///
/// # Type Parameters
/// * `R` - Type of the route
/// * `Q` - Type of the response
async fn delete<R, Q>(route: R) -> Result<Q, anyhow::Error>
where
    R: AsRef<str>,
    Q: for<'a> Deserialize<'a>,
{
    let url = format!("{}{}", crate::server()?, route.as_ref());
    let response = CLIENT
        .delete(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request DELETE {}", route.as_ref()))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response GET {}", route.as_ref()))?;

    tracing::info!("{} GET {} {}", status, url, text);

    let content: Result<Q, ApiError> = serde_json::from_str(&text).with_context(|| {
        format!(
            "error deserializing response from DELETE {}: {text}",
            route.as_ref()
        )
    })?;

    Ok(content?)
}
