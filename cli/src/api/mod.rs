//! API client implementation for interacting with the Samizdat node.
//!
//! Provides the core functionality for making HTTP requests to a Samizdat
//! node, including error handling, authentication, and basic request/response
//! processing. Strongly-typed helpers live in `calls`.
//!
//! TODO(robustness): response bodies are read into memory with `.text()` and
//! have no size cap; ANSI escapes in node-supplied strings are printed raw via
//! `println!`. Both are low-priority today because the trust boundary stops at
//! the local node (if the node is compromised the CLI is already at risk),
//! but if the CLI ever talks to a network-attached node, cap response bodies
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

/// HTTP client used for making requests to the Samizdat node.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// Routes whose response bodies can carry secret material (currently series owner
/// keypairs, which serialise the private bytes). When logging these responses we
/// substitute the body with `<redacted>` so that running the CLI with `--verbose`
/// does not write private keys into any configured `tracing` sink.
const SENSITIVE_BODY_ROUTES: &[&str] = &["/_series-owners"];

pub(super) fn redact_if_sensitive<'a>(route: &str, body: &'a str) -> &'a str {
    if SENSITIVE_BODY_ROUTES.iter().any(|p| route.starts_with(p)) {
        "<redacted: response may contain secret material>"
    } else {
        body
    }
}

/// Bail with a status-tagged error when the node returns a non-2xx HTTP
/// response. The body is included verbatim so the message is whatever
/// the node sent -- typically a JSON-shaped error from the API or a
/// plain string from axum's own deserialization layer for malformed
/// requests. Keeps callers from accidentally trying to deserialize an
/// error body as a success payload.
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

/// Deserialize a successful response body as `Result<Q, ApiError>` (the
/// node's wire format for success payloads). Failure here means the
/// node sent us a 2xx with a body that does not match the expected
/// shape -- a CLI/node mismatch, not a user error.
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

    tracing::info!("{} GET {} {}", status, url, redact_if_sensitive(route.as_ref(), &text));

    bail_on_http_error("GET", route.as_ref(), status, &text)?;
    deserialize_api_response("GET", route.as_ref(), status, &text)
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

    tracing::info!("{} POST {} {}", status, url, redact_if_sensitive(route.as_ref(), &text));

    bail_on_http_error("POST", route.as_ref(), status, &text)?;
    deserialize_api_response("POST", route.as_ref(), status, &text)
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

    tracing::info!("{} PUT {} {}", status, url, redact_if_sensitive(route.as_ref(), &text));

    bail_on_http_error("PUT", route.as_ref(), status, &text)?;
    deserialize_api_response("PUT", route.as_ref(), status, &text)
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

    tracing::info!("{} PATCH {} {}", status, url, redact_if_sensitive(route.as_ref(), &text));

    bail_on_http_error("PATCH", route.as_ref(), status, &text)?;
    deserialize_api_response("PATCH", route.as_ref(), status, &text)
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

    tracing::info!("{} DELETE {} {}", status, url, redact_if_sensitive(route.as_ref(), &text));

    bail_on_http_error("DELETE", route.as_ref(), status, &text)?;
    deserialize_api_response("DELETE", route.as_ref(), status, &text)
}
