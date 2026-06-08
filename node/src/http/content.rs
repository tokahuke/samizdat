//! HTTP handlers for content served at per-series subdomain origins.
//!
//! Routes:
//! - `GET /` -- dispatches on the [`HostScope`] extractor: bare loopback
//!   returns the welcome HTML; series and identity subdomains resolve the
//!   "root" of the corresponding collection.
//! - `GET /{*name}` -- same dispatch with a content path.
//!
//! The bare-loopback case for `/` is what previously lived in
//! `node/src/http/mod.rs::serve`'s root route (the `index.html` welcome
//! page). Folding it in here is what avoids having to write a separate
//! routing dispatcher; both routes share the same `HostScope` extractor.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use tokio::time::Instant;

use crate::http::host_scope::HostScope;
use crate::http::resolvers::{resolve_identity, resolve_series};
use crate::http::{PageResponse, SamizdatTimeout};
use crate::models::SeriesRef;

/// Welcome HTML served at `GET /` on the bare-loopback admin host. Same
/// content as previously served from `serve()` directly.
const WELCOME_HTML: &str = include_str!("../index.html");

/// Handles `GET /`.
pub async fn content_root(
    scope: HostScope,
    SamizdatTimeout(timeout): SamizdatTimeout,
) -> Response {
    match scope {
        HostScope::BareLoopback => Html(WELCOME_HTML).into_response(),
        HostScope::Series(key) => {
            let series = SeriesRef::new(key);
            PageResponse(resolve_series(series, "".into(), [], Instant::now() + timeout).await)
                .into_response()
        }
        HostScope::Identity(handle) => PageResponse(
            resolve_identity(&handle, "".into(), [], Instant::now() + timeout).await,
        )
        .into_response(),
    }
}

/// Handles `GET /{*name}`.
pub async fn content_path(
    scope: HostScope,
    Path(name): Path<String>,
    SamizdatTimeout(timeout): SamizdatTimeout,
) -> Response {
    match scope {
        HostScope::BareLoopback => (
            StatusCode::NOT_FOUND,
            "samizdat-node serves content only on series and identity \
             subdomains (`<base32-key>.localhost:<port>` or \
             `<identity>.localhost:<port>`); the bare loopback host is for \
             admin endpoints only",
        )
            .into_response(),
        HostScope::Series(key) => {
            let series = SeriesRef::new(key);
            PageResponse(
                resolve_series(series, name.as_str().into(), [], Instant::now() + timeout).await,
            )
            .into_response()
        }
        HostScope::Identity(handle) => PageResponse(
            resolve_identity(&handle, name.as_str().into(), [], Instant::now() + timeout).await,
        )
        .into_response(),
    }
}
