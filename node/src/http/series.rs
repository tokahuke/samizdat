//! Series admin API.
//!
//! Series content is served at
//! `http://series-<base32-key>.localhost:<port>/<path>` by the dispatcher in
//! `node/src/http/content.rs`. This module only carries the admin "list all
//! known public keys" endpoint.

use axum::routing::get;
use axum::Router;
use futures::FutureExt;
use samizdat_common::db::readonly_tx;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::SeriesRef;
use crate::security_scope;

/// The entrypoint of the series admin API.
pub fn api() -> Router {
    Router::new().route(
        // Lists all known public keys the node has seen, be they locally
        // owned or not.
        "/",
        get(|| async move { readonly_tx(|tx| SeriesRef::get_all(tx)) }.map(ApiResponse))
            .layer(security_scope!(read; AccessRight::ManageSeries)),
    )
}
