//! Pinner's HTTP control surface.
//!
//! `POST /pin`, `GET /pin`, `GET /pin/{series}`, `DELETE /pin/{series}`. All
//! routes require an `X-Api-Key` header that matches the configured shared
//! key. There is no `/api/` prefix because there is no other surface on
//! this daemon to disambiguate from.

use axum::extract::{Path, Request};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use samizdat_common::Key;
use serde_derive::{Deserialize, Serialize};
use std::num::NonZeroU32;

use crate::cli::cli;
use crate::{db, node_client};

pub fn router() -> Router {
    let auth = middleware::from_fn(require_api_key);

    Router::new()
        .route("/pin", post(pin).get(list_pins))
        .route("/pin/{series}", get(get_pin).delete(unpin))
        .route_layer(auth)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

#[derive(Deserialize)]
struct PinRequest {
    series_key: String,
    /// Days from now until expiry. Re-POSTing extends the existing
    /// expiry to `max(existing, now + days)`, so a customer renews by
    /// repeating the same request. `NonZeroU32` rejects `0` at the
    /// type level so a "renew" request can never accidentally schedule
    /// the subscription for immediate reaping.
    days: NonZeroU32,
}

#[derive(Serialize)]
struct PinResponse {
    series_key: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

async fn pin(Json(req): Json<PinRequest>) -> Result<Json<PinResponse>, ApiError> {
    let key: Key = req
        .series_key
        .parse()
        .map_err(|e| ApiError::BadRequest(format!("bad series_key: {e}")))?;

    node_client::get()
        .add_subscription(&key)
        .await
        .map_err(ApiError::NodeAdmin)?;

    let new_expires = Utc::now() + Duration::days(i64::from(req.days.get()));
    let effective_expires = db::upsert(&key, new_expires, None).map_err(ApiError::Storage)?;

    Ok(Json(PinResponse {
        series_key: key.to_string(),
        expires_at: effective_expires,
    }))
}

#[derive(Serialize)]
struct PinRow {
    series_key: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    customer: Option<String>,
}

async fn list_pins() -> Result<Json<Vec<PinRow>>, ApiError> {
    let rows = db::list().map_err(ApiError::Storage)?;
    Ok(Json(
        rows.into_iter()
            .map(|(key, row)| PinRow {
                series_key: key.to_string(),
                expires_at: row.expires_at,
                created_at: row.created_at,
                customer: row.customer,
            })
            .collect(),
    ))
}

async fn get_pin(Path(series): Path<String>) -> Result<Json<Option<PinRow>>, ApiError> {
    let key: Key = series
        .parse()
        .map_err(|e| ApiError::BadRequest(format!("bad series key: {e}")))?;
    let row = db::get(&key).map_err(ApiError::Storage)?;
    Ok(Json(row.map(|row| PinRow {
        series_key: key.to_string(),
        expires_at: row.expires_at,
        created_at: row.created_at,
        customer: row.customer,
    })))
}

async fn unpin(Path(series): Path<String>) -> Result<StatusCode, ApiError> {
    let key: Key = series
        .parse()
        .map_err(|e| ApiError::BadRequest(format!("bad series key: {e}")))?;

    node_client::get()
        .drop_subscription(&key)
        .await
        .map_err(ApiError::NodeAdmin)?;

    db::delete(&key).map_err(ApiError::Storage)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn require_api_key(headers: HeaderMap, req: Request, next: Next) -> Response {
    let Some(configured) = cli().api_key.as_deref() else {
        return ApiError::Storage(anyhow::anyhow!("pinner has no api_key configured"))
            .into_response();
    };
    let supplied = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // Constant-time-ish: short-circuit only on length, but the comparison
    // itself runs through a constant-time helper. The cost of a real
    // constant-time check here is negligible against the wallclock noise
    // of network ingress.
    if supplied.len() != configured.len() {
        return ApiError::Unauthorized.into_response();
    }
    let mut diff = 0u8;
    for (a, b) in supplied.bytes().zip(configured.bytes()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return ApiError::Unauthorized.into_response();
    }
    next.run(req).await
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
    Unauthorized,
    /// The local samizdat-node rejected the admin call (auth issue,
    /// malformed key on the node side, 5xx from the node). The caller
    /// usually wants to retry or have the operator rotate the admin
    /// token; surface as 502 Bad Gateway so it's distinguishable from
    /// a pinner-side failure.
    NodeAdmin(anyhow::Error),
    /// Pinner's own local persistence layer failed. Operator action
    /// (restart, disk space) is required; 500 Internal Server Error.
    Storage(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "missing or wrong X-Api-Key".into())
            }
            ApiError::NodeAdmin(err) => {
                tracing::warn!("pinner node-admin failure: {err:#}");
                (StatusCode::BAD_GATEWAY, "node admin call failed".into())
            }
            ApiError::Storage(err) => {
                tracing::error!("pinner storage failure: {err:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "storage error".into())
            }
        };
        (status, body).into_response()
    }
}
