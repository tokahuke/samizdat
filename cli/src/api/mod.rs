//! HTTP client for the Samizdat node's API.
//!
//! Sends authenticated requests, handles errors, deserialises responses.
//! Strongly-typed wrappers live in `calls`.
//!
//! TODO(robustness): response bodies are read with `.text()` and have no
//! size cap; ANSI escapes in node-supplied strings are printed raw via
//! `println!`. Low priority today: the trust boundary stops at the local
//! node, so a compromised node already owns the CLI. If the CLI ever talks
//! to a remote node, cap response bodies
//! (`response.bytes_stream().take(MAX)`) and sanitise control characters
//! before display.

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

/// HTTP client for requests to the Samizdat node.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Routes whose response bodies can carry secret material (today: series-owner
/// keypairs, whose private bytes are serialised). When logging these responses
/// we replace the body with `<redacted>` so `--verbose` does not write private
/// keys into any configured `tracing` sink.
const SENSITIVE_BODY_ROUTES: &[&str] = &["/_series-owners"];

pub(super) fn redact_if_sensitive<'a>(route: &str, body: &'a str) -> &'a str {
    if SENSITIVE_BODY_ROUTES.iter().any(|p| route.starts_with(p)) {
        "<redacted: response may contain secret material>"
    } else {
        body
    }
}

/// Bail with a status-tagged error when the node returns a non-2xx HTTP
/// response. The body is included verbatim: the node usually sends a
/// JSON-shaped error, but axum's own deserialization layer can also send
/// a plain string for malformed requests. Keeps callers from
/// accidentally deserializing an error body as a success payload.
pub(super) fn bail_on_http_error(
    method: &str,
    route: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> Result<(), anyhow::Error> {
    if status.is_success() {
        return Ok(());
    }
    let trimmed = body.trim();
    let detail = if trimmed.is_empty() {
        "<empty body>".to_owned()
    } else {
        // Cap the body in the error message so a 10 MB error page does
        // not flood the terminal. Tracing still has the full version
        // when `--verbose` is on.
        let cap = 1024;
        if trimmed.len() > cap {
            format!("{}... ({} bytes total)", &trimmed[..cap], trimmed.len())
        } else {
            trimmed.to_owned()
        }
    };
    anyhow::bail!("{method} {route} returned HTTP {status}: {detail}")
}

/// Deserialize a 2xx response body as `Result<Q, ApiError>` (the node's
/// wire format for success payloads). A failure here means the node
/// sent a 2xx with a body that does not match the expected shape: a
/// CLI/node version mismatch, not a user error.
pub(super) fn deserialize_api_response<Q>(
    method: &str,
    route: &str,
    status: reqwest::StatusCode,
    text: &str,
) -> Result<Q, anyhow::Error>
where
    Q: for<'a> Deserialize<'a>,
{
    let content: Result<Q, ApiError> = serde_json::from_str(text).with_context(|| {
        let body_preview = if text.len() > 512 {
            format!("{}... ({} bytes)", &text[..512], text.len())
        } else {
            text.to_owned()
        };
        format!(
            "{method} {route} -> HTTP {status} but response body did not match expected shape: \
             {body_preview}"
        )
    })?;
    Ok(content?)
}

/// Pings the Samizdat node to check it is up and reachable.
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

/// GET `route` and deserialize the response as `Q`.
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

    tracing::info!(
        "{} GET {} {}",
        status,
        url,
        redact_if_sensitive(route.as_ref(), &text)
    );

    bail_on_http_error("GET", route.as_ref(), status, &text)?;
    deserialize_api_response("GET", route.as_ref(), status, &text)
}

/// POST `payload` as JSON to `route` and deserialize the response as `Q`.
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

    tracing::info!(
        "{} POST {} {}",
        status,
        url,
        redact_if_sensitive(route.as_ref(), &text)
    );

    bail_on_http_error("POST", route.as_ref(), status, &text)?;
    deserialize_api_response("POST", route.as_ref(), status, &text)
}

/// PUT `payload` as JSON to `route` and deserialize the response as `Q`.
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

    tracing::info!(
        "{} PUT {} {}",
        status,
        url,
        redact_if_sensitive(route.as_ref(), &text)
    );

    bail_on_http_error("PUT", route.as_ref(), status, &text)?;
    deserialize_api_response("PUT", route.as_ref(), status, &text)
}

/// PATCH `payload` as JSON to `route` and deserialize the response as `Q`.
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

    tracing::info!(
        "{} PATCH {} {}",
        status,
        url,
        redact_if_sensitive(route.as_ref(), &text)
    );

    bail_on_http_error("PATCH", route.as_ref(), status, &text)?;
    deserialize_api_response("PATCH", route.as_ref(), status, &text)
}

/// DELETE `route` and deserialize the response as `Q`.
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

    tracing::info!(
        "{} DELETE {} {}",
        status,
        url,
        redact_if_sensitive(route.as_ref(), &text)
    );

    bail_on_http_error("DELETE", route.as_ref(), status, &text)?;
    deserialize_api_response("DELETE", route.as_ref(), status, &text)
}
