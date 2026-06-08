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
    /// repeating the same request.
    days: u32,
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
        .map_err(|e| ApiError::Internal(format!("add_subscription: {e}")))?;

    let new_expires = Utc::now() + Duration::days(i64::from(req.days));
    let effective_expires = db::upsert(&key, new_expires, None)
        .map_err(|e| ApiError::Internal(format!("db upsert: {e}")))?;

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
    let rows = db::list().map_err(|e| ApiError::Internal(format!("db list: {e}")))?;
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
    let row = db::get(&key).map_err(|e| ApiError::Internal(format!("db get: {e}")))?;
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
        .map_err(|e| ApiError::Internal(format!("drop_subscription: {e}")))?;

    db::delete(&key).map_err(|e| ApiError::Internal(format!("db delete: {e}")))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn require_api_key(headers: HeaderMap, req: Request, next: Next) -> Response {
    let Some(configured) = cli().api_key.as_deref() else {
        return ApiError::Internal("pinner has no api_key configured".into()).into_response();
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
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "missing or wrong X-Api-Key".into()),
            ApiError::Internal(msg) => {
                tracing::error!("pinner internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, body).into_response()
    }
}
