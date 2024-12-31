//! Editions API.

use axum::routing::get;
use axum::Router;
use futures::FutureExt;
use samizdat_common::db::readonly_tx;

use crate::models::Edition;
use crate::{access::AccessRight, security_scope};

use super::ApiResponse;

/// The entrypoint of the series API.
pub fn api() -> Router {
    Router::new().merge(editions())
}

fn editions() -> Router {
    Router::new().route(
        "/",
        get(|| async move { readonly_tx(|tx| Edition::get_all(tx)) }.map(ApiResponse))
            .layer(security_scope!(AccessRight::ManageSeries)),
    )
}
