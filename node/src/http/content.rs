//! HTTP handlers for content served at typed-subdomain origins.
//!
//! Routes:
//! - `GET /` -- dispatches on the [`HostScope`] extractor: bare loopback
//!   returns the welcome HTML; series/identity/object/collection/edition
//!   subdomains resolve their respective content.
//! - `GET /{*name}` -- same dispatch with a content path. Object hosts
//!   ignore the path (objects are atomic blobs).

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use tokio::time::Instant;

use samizdat_common::db::readonly_tx;
use samizdat_common::Hash;

use crate::http::host_scope::HostScope;
use crate::http::resolvers::{resolve_identity, resolve_item, resolve_object, resolve_series};
use crate::http::{PageResponse, SamizdatTimeout};
use crate::models::{CollectionRef, Edition, ObjectRef, SeriesRef};

/// Welcome HTML served at `GET /` on the bare-loopback admin host.
const WELCOME_HTML: &str = include_str!("../index.html");

/// Handles `GET /`.
pub async fn content_root(
    scope: HostScope,
    SamizdatTimeout(timeout): SamizdatTimeout,
) -> Response {
    serve(scope, "", timeout).await
}

/// Handles `GET /{*name}`.
pub async fn content_path(
    scope: HostScope,
    Path(name): Path<String>,
    SamizdatTimeout(timeout): SamizdatTimeout,
) -> Response {
    serve(scope, name.as_str(), timeout).await
}

async fn serve(scope: HostScope, name: &str, timeout: std::time::Duration) -> Response {
    let deadline = Instant::now() + timeout;
    match scope {
        HostScope::BareLoopback => {
            if name.is_empty() {
                Html(WELCOME_HTML).into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    "samizdat-node serves content only on typed subdomains \
                     (`series-<key>.localhost`, `object-<hash>.localhost`, \
                     `collection-<hash>.localhost`, `edition-<id>.localhost`, \
                     or `<identity>.localhost`); the bare loopback host is for \
                     admin endpoints only",
                )
                    .into_response()
            }
        }
        HostScope::Series(key) => {
            let series = SeriesRef::new(key);
            PageResponse(resolve_series(series, name.into(), [], deadline).await).into_response()
        }
        HostScope::Identity(handle) => {
            PageResponse(resolve_identity(&handle, name.into(), [], deadline).await)
                .into_response()
        }
        HostScope::Object(hash) => {
            // Objects are atomic blobs; the request path is ignored.
            let _ = name;
            PageResponse(resolve_object(ObjectRef::new(hash), [], deadline).await).into_response()
        }
        HostScope::Collection(hash) => {
            let collection = CollectionRef::new(hash);
            let locator = collection.locator_for(name.into());
            PageResponse(resolve_item(locator, [], deadline).await).into_response()
        }
        HostScope::Edition(id) => {
            let parsed = match id.parse::<Hash>() {
                Ok(h) => h,
                Err(err) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("edition id is not a valid hash: {err}"),
                    )
                        .into_response();
                }
            };
            let edition = match readonly_tx(|tx| Edition::by_id(&parsed, tx)) {
                Ok(Some(e)) => e,
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        format!("no edition with id `{parsed}` is indexed locally"),
                    )
                        .into_response();
                }
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("edition lookup failed: {err}"),
                    )
                        .into_response();
                }
            };
            let collection = edition.collection();
            let locator = collection.locator_for(name.into());
            PageResponse(resolve_item(locator, [], deadline).await).into_response()
        }
    }
}
